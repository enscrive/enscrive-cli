//! Shared manifest fetch + verify utility.
//!
//! Consumed by:
//! - `enscrive init --mode self-managed` (CLI-REL-014 / ENS-95) — via `src/local.rs`.
//! - `enscrive deploy fetch`              (RB-008 / ENS-94)       — the legacy
//!   copy in `src/deploy.rs` will be migrated here before DEPLOY-003 deletes it.
//!
//! Schema matches
//! `enscrive-governance/plans/RELEASE-INDUSTRIALIZATION-2026-04-23/DESIGN.md §2.3`.
//!
//! Principles (per DESIGN.md §2.6):
//! - Caller supplies `dest` explicitly; utility never writes outside `dest`.
//! - Idempotent: if `dest` already exists with a matching SHA256 the binary is
//!   not re-downloaded. Force re-fetch is the caller's responsibility.
//! - No knowledge of orchestration: this module only fetches, verifies, and
//!   places files on disk.
//!
//! `https://` and `file://` URLs are both accepted by `fetch_manifest` — the
//! latter lets the test suite and offline harnesses point at a fixture manifest
//! without hitting the network.
//!
//! TODO(ENS-82): verify cosign bundle signatures once the v0.1 GA signing
//! pipeline lands. For now SHA256 is the only integrity signal.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::release_channel;

/// Highest manifest schema version this CLI understands.
///
/// schema_version=1 — original (DESIGN.md §2.3).
/// schema_version=2 — adds per-binary `kind` field ("binary" | "archive").
///   "archive" entries are tar.gz packages containing a server binary plus
///   a `site/` directory of static assets (Leptos SSR + hydrate apps).
pub const SUPPORTED_SCHEMA_VERSION: u32 = 2;

/// A parsed release manifest.
///
/// See DESIGN.md §2.3 for the authoritative schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub version: String,
    #[serde(default)]
    pub released_at: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    pub binaries: HashMap<String, BinaryEntry>,
    #[serde(default)]
    pub compatibility: Option<Compat>,
    /// Reserved for the cosign Rekor reference once ENS-82 lands.
    #[serde(default)]
    pub signature: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    /// Plain executable. Place at the destination path; chmod 0755.
    Binary,
    /// tar.gz containing a server binary at the archive root plus a
    /// `site/` directory. Extract; place binary at the destination
    /// path; place site/ at a sibling location managed by the caller.
    Archive,
}

impl Default for ArtifactKind {
    fn default() -> Self {
        Self::Binary
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryEntry {
    pub source_version: String,
    /// schema_version >= 2 carries this; absent on schema_version=1
    /// manifests where it defaults to `Binary` (back-compat).
    #[serde(default)]
    pub kind: ArtifactKind,
    pub platforms: HashMap<String, PlatformEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformEntry {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compat {
    pub min_cli_version: String,
}

#[derive(Debug)]
pub enum FetchError {
    ManifestRead(String),
    ManifestParse(String),
    SchemaVersionUnsupported { found: u32, max_supported: u32 },
    BinaryNotInManifest(String),
    PlatformMissing(String),
    Download(String),
    ChecksumMismatch {
        artifact: String,
        expected: String,
        got: String,
    },
    Io(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::ManifestRead(m) => write!(f, "read manifest: {m}"),
            FetchError::ManifestParse(m) => write!(f, "parse manifest: {m}"),
            FetchError::SchemaVersionUnsupported {
                found,
                max_supported,
            } => write!(
                f,
                "unsupported manifest schema_version {found}; this CLI understands up to {max_supported}. Upgrade the CLI."
            ),
            FetchError::BinaryNotInManifest(n) => {
                write!(f, "manifest has no entry for binary '{n}'")
            }
            FetchError::PlatformMissing(m) => write!(f, "{m}"),
            FetchError::Download(m) => write!(f, "download: {m}"),
            FetchError::ChecksumMismatch {
                artifact,
                expected,
                got,
            } => write!(
                f,
                "sha256 mismatch for '{artifact}': expected {expected}, got {got}"
            ),
            FetchError::Io(m) => write!(f, "io: {m}"),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<FetchError> for String {
    fn from(e: FetchError) -> Self {
        e.to_string()
    }
}

/// Fetch the JSON manifest at `manifest_url` and deserialize into `Manifest`.
///
/// Accepts `http://`, `https://`, and `file://` URLs. The `file://` path is
/// intended for offline tests and local-file harnesses — production callers
/// pass the dev-channel URL `https://dev.enscrive.io/releases/dev/latest.json`
/// today, and a `https://enscrive.io/...` URL once the prod CloudFront is
/// provisioned.
pub async fn fetch_manifest(manifest_url: &str) -> Result<Manifest, FetchError> {
    let body = read_url_bytes(manifest_url).await?;
    let manifest: Manifest = serde_json::from_slice(&body).map_err(|e| {
        FetchError::ManifestParse(format!(
            "{e}; url='{manifest_url}' body_snippet='{}'",
            snippet(&body)
        ))
    })?;

    if manifest.schema_version > SUPPORTED_SCHEMA_VERSION {
        return Err(FetchError::SchemaVersionUnsupported {
            found: manifest.schema_version,
            max_supported: SUPPORTED_SCHEMA_VERSION,
        });
    }

    Ok(manifest)
}

/// Download `binary.platforms[target]` to `dest`, verifying SHA256 on the way.
///
/// If `dest` already exists and its SHA256 matches the manifest, no download
/// happens and the existing file is left in place (idempotent).
///
/// On SHA256 mismatch the partial download is deleted and a descriptive error
/// is returned.
pub async fn fetch_and_verify(
    binary: &BinaryEntry,
    target: &str,
    dest: &Path,
) -> Result<(), FetchError> {
    let platform = binary.platforms.get(target).ok_or_else(|| {
        let available: Vec<String> = binary.platforms.keys().cloned().collect();
        FetchError::PlatformMissing(release_channel::format_platform_missing(
            "<manifest>",
            target,
            &available,
        ))
    })?;

    if dest.exists()
        && sha256_file(dest)
            .map(|h| h.eq_ignore_ascii_case(&platform.sha256))
            .unwrap_or(false)
    {
        // Already present with matching hash — nothing to do.
        set_executable(dest)?;
        return Ok(());
    }

    let bytes = read_url_bytes(&platform.url)
        .await
        .map_err(|e| match e {
            FetchError::ManifestRead(m) => FetchError::Download(m),
            other => other,
        })?;

    let got = format!("{:x}", Sha256::digest(&bytes));
    if !got.eq_ignore_ascii_case(&platform.sha256) {
        return Err(FetchError::ChecksumMismatch {
            artifact: platform.url.clone(),
            expected: platform.sha256.clone(),
            got,
        });
    }

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            FetchError::Io(format!("create dest parent '{}': {e}", parent.display()))
        })?;
    }

    // Write to a sibling temp file, then atomic rename. Drop partials on
    // failure so we never leave half-written binaries behind.
    let tmp = temp_sibling(dest);
    {
        let mut file = fs::File::create(&tmp)
            .map_err(|e| FetchError::Io(format!("create temp '{}': {e}", tmp.display())))?;
        file.write_all(&bytes)
            .map_err(|e| FetchError::Io(format!("write temp '{}': {e}", tmp.display())))?;
        file.sync_all()
            .map_err(|e| FetchError::Io(format!("sync temp '{}': {e}", tmp.display())))?;
    }

    if let Err(e) = fs::rename(&tmp, dest) {
        let _ = fs::remove_file(&tmp);
        return Err(FetchError::Io(format!(
            "rename '{}' -> '{}': {e}",
            tmp.display(),
            dest.display()
        )));
    }

    set_executable(dest)?;
    Ok(())
}

/// Download `binary.platforms[target]` (a tar.gz archive), verify SHA256 of
/// the archive itself, and extract its contents into `dest_root`.
///
/// The archive is expected to contain at minimum a top-level executable named
/// `<binary_basename>` and (optionally) a top-level `site/` directory of
/// static assets. After extraction:
///
/// - `<dest_root>/<binary_basename>` is the executable (chmod 0755 set).
/// - `<dest_root>/site/...` mirrors the archive's site tree.
///
/// `dest_root` is created if missing. Existing contents are preserved unless
/// they conflict with archive entries, in which case the archive entry wins
/// (overwritten in place). The caller is responsible for choosing whether to
/// clear `dest_root` before calling this; idempotent re-runs against the same
/// archive content are safe.
///
/// Mismatch on the archive's SHA256 deletes the temp tar.gz and errors;
/// nothing is extracted in that case.
pub async fn fetch_and_extract_archive(
    binary: &BinaryEntry,
    target: &str,
    dest_root: &Path,
    binary_basename: &str,
) -> Result<(), FetchError> {
    let platform = binary.platforms.get(target).ok_or_else(|| {
        let available: Vec<String> = binary.platforms.keys().cloned().collect();
        FetchError::PlatformMissing(release_channel::format_platform_missing(
            "<manifest>",
            target,
            &available,
        ))
    })?;

    let bytes = read_url_bytes(&platform.url)
        .await
        .map_err(|e| match e {
            FetchError::ManifestRead(m) => FetchError::Download(m),
            other => other,
        })?;

    let got = format!("{:x}", Sha256::digest(&bytes));
    if !got.eq_ignore_ascii_case(&platform.sha256) {
        return Err(FetchError::ChecksumMismatch {
            artifact: platform.url.clone(),
            expected: platform.sha256.clone(),
            got,
        });
    }

    fs::create_dir_all(dest_root).map_err(|e| {
        FetchError::Io(format!(
            "create dest_root '{}': {e}",
            dest_root.display()
        ))
    })?;

    // Decompress + untar in one shot. Refuse paths that escape dest_root
    // (`..` traversal) — defensive belt-and-suspenders even though our own
    // workflows produce well-formed archives.
    let cursor = std::io::Cursor::new(&bytes);
    let gz = flate2::read::GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(gz);
    archive.set_preserve_mtime(false);

    for entry in archive.entries().map_err(|e| {
        FetchError::Io(format!("read tar entries from '{}': {e}", platform.url))
    })? {
        let mut entry = entry
            .map_err(|e| FetchError::Io(format!("read tar entry: {e}")))?;
        let entry_path = entry
            .path()
            .map_err(|e| FetchError::Io(format!("decode tar entry path: {e}")))?
            .into_owned();
        if entry_path.is_absolute()
            || entry_path.components().any(|c| {
                matches!(c, std::path::Component::ParentDir)
            })
        {
            return Err(FetchError::Io(format!(
                "refusing tar entry with traversal path: '{}'",
                entry_path.display()
            )));
        }
        let dst = dest_root.join(&entry_path);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                FetchError::Io(format!(
                    "create entry parent '{}': {e}",
                    parent.display()
                ))
            })?;
        }
        entry
            .unpack(&dst)
            .map_err(|e| FetchError::Io(format!("unpack '{}': {e}", dst.display())))?;
    }

    // Ensure the binary at dest_root/<basename> is executable.
    let bin_path = dest_root.join(binary_basename);
    if bin_path.is_file() {
        set_executable(&bin_path)?;
    } else {
        return Err(FetchError::Io(format!(
            "archive at '{}' did not contain expected top-level binary '{}'",
            platform.url,
            binary_basename
        )));
    }
    Ok(())
}

/// Formatted error for "binary entry exists but has no row for our target".
///
/// Lists the platforms the manifest DOES ship for that binary so operators can
/// eyeball whether they're on a supported host.
pub fn platform_missing_error(binary_name: &str, target: &str, entry: &BinaryEntry) -> String {
    let available: Vec<String> = entry.platforms.keys().cloned().collect();
    format!(
        "binary '{binary_name}' in release manifest has no platform entry for '{target}'.\n{}",
        release_channel::format_platform_missing("<manifest>", target, &available)
    )
}

// --- helpers --------------------------------------------------------------

async fn read_url_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    if let Some(path) = url.strip_prefix("file://") {
        return fs::read(path)
            .map_err(|e| FetchError::ManifestRead(format!("read file '{path}': {e}")));
    }

    if url.starts_with("http://") || url.starts_with("https://") {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|e| FetchError::ManifestRead(format!("build http client: {e}")))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| FetchError::ManifestRead(format!("GET {url}: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            return Err(FetchError::ManifestRead(format!(
                "GET {url} returned HTTP {status}: {}",
                truncate(&body, 240)
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| FetchError::ManifestRead(format!("read body from {url}: {e}")))?;
        return Ok(bytes.to_vec());
    }

    // Bare paths are treated as local files too, which helps test fixtures
    // that pass through `PathBuf::display().to_string()` without a scheme.
    if url.starts_with('/') || url.starts_with("./") {
        return fs::read(url)
            .map_err(|e| FetchError::ManifestRead(format!("read path '{url}': {e}")));
    }

    Err(FetchError::ManifestRead(format!(
        "unsupported URL scheme for '{url}' (expected http, https, or file)"
    )))
}

fn sha256_file(path: &Path) -> Result<String, FetchError> {
    let mut file = fs::File::open(path)
        .map_err(|e| FetchError::Io(format!("open '{}': {e}", path.display())))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| FetchError::Io(format!("read '{}': {e}", path.display())))?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn temp_sibling(dest: &Path) -> PathBuf {
    let mut name = dest
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("download"));
    name.push(".partial");
    dest.with_file_name(name)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), FetchError> {
    let mut perms = fs::metadata(path)
        .map_err(|e| FetchError::Io(format!("stat '{}': {e}", path.display())))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)
        .map_err(|e| FetchError::Io(format!("chmod '{}': {e}", path.display())))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), FetchError> {
    Ok(())
}

fn snippet(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    truncate(&s, 160).replace(char::is_control, " ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

// --- tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture_manifest_json() -> String {
        // Mirrors DESIGN.md §2.3 verbatim enough to exercise parse paths.
        r#"{
            "schema_version": 1,
            "version": "v0.1.0-beta.1",
            "released_at": "2026-05-10T14:00:00Z",
            "channel": "beta",
            "binaries": {
                "enscrive-developer": {
                    "source_version": "v0.1.0-beta.1",
                    "platforms": {
                        "x86_64-unknown-linux-gnu": {
                            "url": "https://enscrive.io/releases/v0.1.0-beta.1/x86_64-unknown-linux-gnu/enscrive-developer",
                            "sha256": "abc123",
                            "size_bytes": 14852196
                        }
                    }
                },
                "enscrive-observe": {
                    "source_version": "v0.1.0-beta.1",
                    "platforms": {}
                },
                "enscrive-embed": {
                    "source_version": "v0.1.0-beta.1",
                    "platforms": {}
                }
            },
            "compatibility": {
                "min_cli_version": "v0.1.0-beta.1"
            },
            "signature": null
        }"#
        .to_string()
    }

    #[tokio::test]
    async fn fetch_manifest_parses_design_schema_from_file_url() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("manifest.json");
        fs::write(&path, fixture_manifest_json()).unwrap();

        let url = format!("file://{}", path.display());
        let manifest = fetch_manifest(&url).await.unwrap();

        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.version, "v0.1.0-beta.1");
        assert_eq!(manifest.channel.as_deref(), Some("beta"));
        assert_eq!(manifest.binaries.len(), 3);
        let dev = manifest.binaries.get("enscrive-developer").unwrap();
        assert_eq!(dev.source_version, "v0.1.0-beta.1");
        let gnu = dev.platforms.get("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(gnu.sha256, "abc123");
        assert_eq!(gnu.size_bytes, Some(14852196));
        assert_eq!(
            manifest.compatibility.as_ref().unwrap().min_cli_version,
            "v0.1.0-beta.1"
        );
    }

    #[tokio::test]
    async fn fetch_manifest_rejects_future_schema_version() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("manifest.json");
        fs::write(
            &path,
            r#"{
                "schema_version": 9999,
                "version": "v9.9.9",
                "binaries": {}
            }"#,
        )
        .unwrap();
        let url = format!("file://{}", path.display());

        match fetch_manifest(&url).await {
            Err(FetchError::SchemaVersionUnsupported { found, max_supported }) => {
                assert_eq!(found, 9999);
                assert_eq!(max_supported, SUPPORTED_SCHEMA_VERSION);
            }
            other => panic!("expected SchemaVersionUnsupported, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn fetch_and_verify_accepts_matching_sha() {
        let temp = TempDir::new().unwrap();

        // Create a fake "binary" and compute its real SHA256.
        let src_path = temp.path().join("src-binary");
        let payload = b"fake binary content\n";
        fs::write(&src_path, payload).unwrap();
        let real_sha = format!("{:x}", Sha256::digest(payload));

        let mut platforms = HashMap::new();
        platforms.insert(
            "x86_64-unknown-linux-gnu".to_string(),
            PlatformEntry {
                url: format!("file://{}", src_path.display()),
                sha256: real_sha.clone(),
                size_bytes: Some(payload.len() as u64),
            },
        );
        let entry = BinaryEntry {
            kind: ArtifactKind::Binary,
            source_version: "v-test".to_string(),
            platforms,
        };

        let dest = temp.path().join("bin").join("enscrive-developer");
        fetch_and_verify(&entry, "x86_64-unknown-linux-gnu", &dest)
            .await
            .expect("fetch should succeed");

        assert!(dest.exists());
        assert_eq!(fs::read(&dest).unwrap(), payload);

        #[cfg(unix)]
        {
            let mode = fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "binary should be chmod 0755");
        }
    }

    #[tokio::test]
    async fn fetch_and_verify_rejects_mismatched_sha() {
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("src-binary");
        fs::write(&src_path, b"actual content").unwrap();

        let mut platforms = HashMap::new();
        platforms.insert(
            "x86_64-unknown-linux-gnu".to_string(),
            PlatformEntry {
                url: format!("file://{}", src_path.display()),
                // Deliberately wrong.
                sha256: "00000000000000000000000000000000000000000000000000000000deadbeef"
                    .to_string(),
                size_bytes: None,
            },
        );
        let entry = BinaryEntry {
            kind: ArtifactKind::Binary,
            source_version: "v-test".to_string(),
            platforms,
        };

        let dest = temp.path().join("bin").join("enscrive-developer");
        let err = fetch_and_verify(&entry, "x86_64-unknown-linux-gnu", &dest)
            .await
            .expect_err("should reject sha mismatch");

        match err {
            FetchError::ChecksumMismatch { expected, got, .. } => {
                assert!(expected.contains("deadbeef"));
                assert_ne!(got, expected);
            }
            other => panic!("expected ChecksumMismatch, got {other}"),
        }
        // Destination must not exist on mismatch — we never renamed in.
        assert!(!dest.exists(), "dest should not be created on mismatch");
        // And no stray .partial left over.
        let partial = temp.path().join("bin").join("enscrive-developer.partial");
        assert!(!partial.exists(), "partial should be cleaned up");
    }

    #[tokio::test]
    async fn fetch_and_verify_is_idempotent_when_sha_matches() {
        let temp = TempDir::new().unwrap();
        let payload = b"already-installed binary";
        let real_sha = format!("{:x}", Sha256::digest(payload));

        // Pre-seed the destination.
        let dest = temp.path().join("bin").join("enscrive-developer");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(&dest, payload).unwrap();

        // Point the "download" URL at a nonexistent file — if the idempotent
        // fast-path is broken, the test fails via an IO error.
        let mut platforms = HashMap::new();
        platforms.insert(
            "x86_64-unknown-linux-gnu".to_string(),
            PlatformEntry {
                url: "file:///nonexistent/should/not/be/read".to_string(),
                sha256: real_sha,
                size_bytes: None,
            },
        );
        let entry = BinaryEntry {
            kind: ArtifactKind::Binary,
            source_version: "v-test".to_string(),
            platforms,
        };

        fetch_and_verify(&entry, "x86_64-unknown-linux-gnu", &dest)
            .await
            .expect("idempotent path should not attempt download");

        assert_eq!(fs::read(&dest).unwrap(), payload);
    }

    #[tokio::test]
    async fn fetch_and_verify_missing_platform_surfaces_error() {
        let entry = BinaryEntry {
            kind: ArtifactKind::Binary,
            source_version: "v-test".to_string(),
            platforms: HashMap::new(),
        };
        let err = fetch_and_verify(&entry, "x86_64-unknown-linux-gnu", Path::new("/tmp/unused"))
            .await
            .expect_err("should error when platform missing");
        match err {
            FetchError::PlatformMissing(msg) => {
                assert!(msg.contains("x86_64-unknown-linux-gnu"));
            }
            other => panic!("expected PlatformMissing, got {other}"),
        }
    }

    #[test]
    fn platform_missing_error_lists_available_platforms() {
        let mut platforms = HashMap::new();
        platforms.insert(
            "aarch64-apple-darwin".to_string(),
            PlatformEntry {
                url: "https://example/a".to_string(),
                sha256: "aa".to_string(),
                size_bytes: None,
            },
        );
        platforms.insert(
            "x86_64-unknown-linux-gnu".to_string(),
            PlatformEntry {
                url: "https://example/b".to_string(),
                sha256: "bb".to_string(),
                size_bytes: None,
            },
        );
        let entry = BinaryEntry {
            kind: ArtifactKind::Binary,
            source_version: "v".to_string(),
            platforms,
        };
        let msg = platform_missing_error("enscrive-observe", "riscv64-unknown-linux-gnu", &entry);
        assert!(msg.contains("enscrive-observe"));
        assert!(msg.contains("riscv64-unknown-linux-gnu"));
        assert!(msg.contains("aarch64-apple-darwin"));
        assert!(msg.contains("x86_64-unknown-linux-gnu"));
    }
}
