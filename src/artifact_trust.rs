//! ENS5913: signed aggregate selection and immutable artifact generations.
//! Linux-only release fetch; explicit local binaries remain operator trusted.
use crate::fetch_verify::{ArtifactKind, Manifest};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
mod archive;
#[cfg(target_os = "linux")]
mod process;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod real_crypto_tests;
pub(crate) const IDENTITY: &str =
    "https://github.com/enscrive/enscrive-cli/.github/workflows/manifest.yml@refs/heads/main";
pub(crate) const ISSUER: &str = "https://token.actions.githubusercontent.com";
const MAX_COMPONENT: u64 = 2 * 1024 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;
fn fail(s: &str) -> String {
    format!("artifact trust: {s}")
}
fn check_time(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(fail("deadline exceeded"))
    } else {
        Ok(())
    }
}
fn digest(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
mod policy;
use policy::Location;
pub(crate) use policy::{Policy, Provenance};
#[derive(Clone, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    size: u64,
    sha256: String,
}
fn identity(path: &Path, deadline: Instant, max: u64) -> Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    check_time(deadline)?;
    let before = fs::symlink_metadata(path).map_err(|_| fail("artifact file unavailable"))?;
    if !before.is_file() || before.nlink() != 1 || before.len() > max {
        return Err(fail("artifact file kind/size invalid"));
    }
    let mut f = File::open(path).map_err(|_| fail("artifact file unavailable"))?;
    let actual = f
        .metadata()
        .map_err(|_| fail("artifact metadata unavailable"))?;
    if actual.dev() != before.dev() || actual.ino() != before.ino() {
        return Err(fail("artifact identity changed"));
    }
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut b = [0; 65536];
    loop {
        check_time(deadline)?;
        let n = f.read(&mut b).map_err(|_| fail("artifact read failed"))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > max {
            return Err(fail("artifact size exceeded"));
        }
        hash.update(&b[..n]);
    }
    let after = fs::symlink_metadata(path).map_err(|_| fail("artifact identity changed"))?;
    check_time(deadline)?;
    if total != before.len()
        || after.dev() != before.dev()
        || after.ino() != before.ino()
        || after.len() != before.len()
        || after.mtime() != before.mtime()
        || after.mtime_nsec() != before.mtime_nsec()
    {
        return Err(fail("artifact changed"));
    }
    Ok(Identity {
        dev: before.dev(),
        ino: before.ino(),
        size: total,
        sha256: format!("{:x}", hash.finalize()),
    })
}
fn private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    for parent in path.ancestors() {
        if fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(fail("directory symlink refused"));
        }
    }
    if !path.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|_| fail("private directory unavailable"))?;
    }
    let m = fs::symlink_metadata(path).map_err(|_| fail("private directory unavailable"))?;
    if !m.is_dir() || m.uid() != unsafe { geteuid() } {
        return Err(fail("private directory ownership invalid"));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| fail("private directory mode unavailable"))?;
    Ok(())
}
fn fresh_dir(root: &Path, prefix: &str) -> Result<PathBuf> {
    private_dir(root)?;
    for _ in 0..16 {
        let p = root.join(format!("{prefix}-{:032x}", rand::random::<u128>()));
        use std::os::unix::fs::DirBuilderExt;
        match fs::DirBuilder::new().mode(0o700).create(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(fail("generation staging unavailable")),
        }
    }
    Err(fail("unique staging unavailable"))
}
fn new_file(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| fail("private file creation failed"))
}
fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| fail("directory sync failed"))
}
fn copy_file(source: &Path, destination: &Path, max: u64, deadline: Instant) -> Result<()> {
    let mut input = File::open(source).map_err(|_| fail("accepted input unavailable"))?;
    let mut output = new_file(destination)?;
    let mut total = 0u64;
    let mut buffer = [0; 65536];
    loop {
        check_time(deadline)?;
        let n = input
            .read(&mut buffer)
            .map_err(|_| fail("accepted input read failed"))?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or_else(|| fail("copy limit exceeded"))?;
        if total > max {
            return Err(fail("copy limit exceeded"));
        }
        output
            .write_all(&buffer[..n])
            .map_err(|_| fail("copy write failed"))?;
    }
    output.sync_all().map_err(|_| fail("copy sync failed"))?;
    check_time(deadline)
}
async fn read_to(location: &Location, path: &Path, max: u64, deadline: Instant) -> Result<()> {
    let mut f = new_file(path)?;
    let mut total = 0u64;
    match location {
        Location::File(p) => {
            // Explicit operator file roots still require signature verification.
            for ancestor in p.ancestors() {
                if fs::symlink_metadata(ancestor).is_ok_and(|m| m.file_type().is_symlink()) {
                    return Err(fail("file location symlink refused"));
                }
            }
            let before = identity(p, deadline, max)?;
            let mut input = File::open(p).map_err(|_| fail("file input unavailable"))?;
            let mut b = [0; 65536];
            loop {
                check_time(deadline)?;
                let n = input.read(&mut b).map_err(|_| fail("file input failed"))?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > max {
                    return Err(fail("input limit exceeded"));
                }
                f.write_all(&b[..n])
                    .map_err(|_| fail("staging write failed"))?;
            }
            if identity(p, deadline, max)? != before {
                return Err(fail("file input changed"));
            }
        }
        Location::Https(url) => {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| fail("HTTPS client unavailable"))?;
            let work = async {
                let response = client
                    .get(url)
                    .send()
                    .await
                    .map_err(|_| fail("HTTPS read unavailable"))?;
                if !response.status().is_success() {
                    return Err(fail("HTTPS status refused"));
                }
                if response.content_length().is_some_and(|n| n > max) {
                    return Err(fail("input limit exceeded"));
                }
                write_chunks(response.bytes_stream(), &mut f, max, deadline).await
            };
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), work)
                .await
                .map_err(|_| fail("download deadline exceeded"))??;
        }
    }
    f.sync_all().map_err(|_| fail("staging sync failed"))?;
    check_time(deadline)
}
async fn write_chunks<S, B, E>(
    stream: S,
    output: &mut dyn Write,
    max: u64,
    deadline: Instant,
) -> Result<()>
where
    S: futures_util::Stream<Item = std::result::Result<B, E>>,
    B: AsRef<[u8]>,
{
    futures_util::pin_mut!(stream);
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        check_time(deadline)?;
        let bytes = chunk.map_err(|_| fail("HTTPS body unavailable"))?;
        let bytes = bytes.as_ref();
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| fail("input limit exceeded"))?;
        if total > max {
            return Err(fail("input limit exceeded"));
        }
        output
            .write_all(bytes)
            .map_err(|_| fail("staging write failed"))?;
    }
    check_time(deadline)
}
pub(crate) struct Aggregate {
    manifest: Manifest,
    sha: String,
    bundle_sha: String,
    verifier_sha: String,
    pinned: bool,
    evidence: PathBuf,
    file_root: Option<PathBuf>,
}
pub(crate) struct Selection<'a> {
    aggregate: &'a Aggregate,
    name: String,
    url: Location,
    sha: String,
    size: Option<u64>,
    archive: bool,
    target: String,
}
impl Aggregate {
    pub(crate) async fn load(policy: &Policy, root: &Path, default: &str) -> Result<Self> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (policy, root, default);
            return Err(fail(
                "release fetch currently supports Linux only; explicit operator binaries remain supported",
            ));
        }
        #[cfg(target_os = "linux")]
        {
            let verifier = process::Verifier::resolve()?;
            let origin = Location::parse(
                policy
                    .origin
                    .as_deref()
                    .unwrap_or("https://developer.enscrive.io"),
            )?;
            let discovery = match (&policy.manifest, &policy.expected) {
                (Some(url), _) => Location::parse(url)?,
                (None, Some(sha)) => {
                    origin.child(&format!("pinsets/sha256/{sha}/manifest.json"))?
                }
                _ => Location::parse(default)?,
            };
            let evidence = fresh_dir(root, "aggregate")?;
            let raw = evidence.join("discovery.json");
            read_to(
                &discovery,
                &raw,
                4 * 1024 * 1024,
                Instant::now() + Duration::from_secs(30),
            )
            .await?;
            let body = fs::read(&raw).map_err(|_| fail("manifest read failed"))?;
            let sha = digest(&body);
            if policy.expected.as_ref().is_some_and(|s| s != &sha)
                || discovery.pin_digest().is_some_and(|s| s != sha)
            {
                return Err(fail("manifest digest mismatch"));
            }
            let selected = origin.child(&format!("pinsets/sha256/{sha}"))?;
            let manifest_path = evidence.join("manifest.json");
            read_to(
                &selected.child("manifest.json")?,
                &manifest_path,
                4 * 1024 * 1024,
                Instant::now() + Duration::from_secs(30),
            )
            .await?;
            if fs::read(&manifest_path).map_err(|_| fail("manifest read failed"))? != body {
                return Err(fail("immutable manifest differs"));
            }
            let bundle = evidence.join("manifest.bundle");
            read_to(
                &selected.child("manifest.bundle")?,
                &bundle,
                4 * 1024 * 1024,
                Instant::now() + Duration::from_secs(30),
            )
            .await?;
            let raw_before = identity(
                &manifest_path,
                Instant::now() + Duration::from_secs(30),
                4 * 1024 * 1024,
            )?;
            let bundle_before = identity(
                &bundle,
                Instant::now() + Duration::from_secs(30),
                4 * 1024 * 1024,
            )?;
            verifier.verify(&manifest_path, &bundle, &evidence)?;
            if identity(
                &manifest_path,
                Instant::now() + Duration::from_secs(30),
                4 * 1024 * 1024,
            )? != raw_before
                || identity(
                    &bundle,
                    Instant::now() + Duration::from_secs(30),
                    4 * 1024 * 1024,
                )? != bundle_before
            {
                return Err(fail("signature inputs changed"));
            }
            let manifest: Manifest = serde_json::from_slice(&body)
                .map_err(|_| fail("signed manifest schema invalid"))?;
            if !(1..=crate::fetch_verify::SUPPORTED_SCHEMA_VERSION)
                .contains(&manifest.schema_version)
            {
                return Err(fail("unsupported signed manifest schema"));
            }
            Ok(Self {
                manifest,
                sha,
                bundle_sha: bundle_before.sha256,
                verifier_sha: verifier.digest().into(),
                pinned: policy.expected.is_some(),
                file_root: match &origin {
                    Location::File(p) => Some(p.clone()),
                    _ => None,
                },
                evidence,
            })
        }
    }
    pub(crate) fn select(&self, name: &str, target: &str) -> Result<Selection<'_>> {
        if !matches!(
            name,
            "enscrive-developer" | "enscrive-observe" | "enscrive-embed" | "enscrive-docs" | "esm"
        ) {
            return Err(fail("unsupported component name"));
        }
        let entry = self
            .manifest
            .binaries
            .get(name)
            .ok_or_else(|| fail("signed component missing"))?;
        let platform = entry
            .platforms
            .get(target)
            .ok_or_else(|| fail("signed target missing"))?;
        if !valid_digest(&platform.sha256)
            || platform
                .size_bytes
                .is_some_and(|n| n == 0 || n > MAX_COMPONENT)
        {
            return Err(fail("invalid signed component digest/size"));
        }
        let url = Location::parse(&platform.url)?;
        if let Location::File(path) = &url
            && !self
                .file_root
                .as_ref()
                .is_some_and(|root| path.starts_with(root))
        {
            return Err(fail("component file is outside the operator-trusted root"));
        }
        Ok(Selection {
            aggregate: self,
            name: name.into(),
            target: target.into(),
            url,
            sha: platform.sha256.clone(),
            size: platform.size_bytes,
            archive: matches!(entry.kind, ArtifactKind::Archive),
        })
    }
}
impl Selection<'_> {
    pub(crate) async fn install(&self, root: &Path, force: bool) -> Result<(String, Provenance)> {
        let deadline = Instant::now() + Duration::from_secs(300);
        let cache = root.join("compressed-cache");
        private_dir(&cache)?;
        let generations = root.join("generations");
        let staging = fresh_dir(&generations, "pending")?;
        let cached = cache.join(&self.sha);
        let input = staging.join(".accepted-input");
        if !force && cached.exists() {
            let before = identity(&cached, deadline, MAX_COMPONENT)?;
            self.validate_bytes(&before)?;
            if !self.archive {
                use std::os::unix::fs::PermissionsExt;
                if fs::metadata(&cached)
                    .map_err(|_| fail("cache metadata unavailable"))?
                    .permissions()
                    .mode()
                    & 0o111
                    == 0
                {
                    return Err(fail("plain cache is not executable"));
                }
            }
            copy_file(&cached, &input, MAX_COMPONENT, deadline)?;
            if identity(&cached, deadline, MAX_COMPONENT)? != before {
                return Err(fail("cache identity changed"));
            }
        } else {
            read_to(&self.url, &input, MAX_COMPONENT, deadline).await?;
        }
        let accepted = identity(&input, deadline, MAX_COMPONENT)?;
        self.validate_bytes(&accepted)?;
        // Cache stores only accepted bytes; it is never an extracted-tree authority.
        if !cached.exists() {
            let temp = cache.join(format!("pending-{:032x}", rand::random::<u128>()));
            copy_file(&input, &temp, MAX_COMPONENT, deadline)?;
            if !self.archive {
                executable(&temp)?;
            }
            File::open(&temp)
                .and_then(|f| f.sync_all())
                .map_err(|_| fail("cache sync failed"))?;
            fs::rename(&temp, &cached).map_err(|_| fail("cache publication failed"))?;
            sync_dir(&cache)?;
        }
        let tree = staging.join("tree");
        private_dir(&tree)?;
        let inventory = if self.archive {
            archive::extract(
                &input,
                &tree,
                &self.name,
                Instant::now() + Duration::from_secs(300),
            )?
        } else {
            let p = tree.join(&self.name);
            copy_file(&input, &p, MAX_COMPONENT, deadline)?;
            executable(&p)?;
            let got = identity(&p, deadline, MAX_COMPONENT)?;
            self.validate_bytes(&got)?;
            vec![
                serde_json::json!({"path":self.name,"type":"file","sha256":got.sha256,"bytes":got.size}),
            ]
        };
        if identity(
            &input,
            Instant::now() + Duration::from_secs(30),
            MAX_COMPONENT,
        )? != accepted
        {
            return Err(fail("accepted archive changed"));
        }
        fs::remove_file(&input).map_err(|_| fail("input cleanup failed"))?;
        let final_dir = generations.join(format!("accepted-{:032x}", rand::random::<u128>()));
        let evidence = serde_json::json!({"aggregate_sha256":self.aggregate.sha,"bundle_sha256":self.aggregate.bundle_sha,"verifier_sha256":self.aggregate.verifier_sha,"aggregate_evidence":self.aggregate.evidence,"component":self.name,"target":self.target,"component_sha256":self.sha,"archive":self.archive,"pinned":self.aggregate.pinned,"files":inventory});
        let mut record = new_file(&staging.join(".completion-pending.json"))?;
        record
            .write_all(
                serde_json::to_string(&evidence)
                    .map_err(|_| fail("completion serialization failed"))?
                    .as_bytes(),
            )
            .map_err(|_| fail("completion write failed"))?;
        record
            .sync_all()
            .map_err(|_| fail("completion sync failed"))?;
        sync_dir(&tree)?;
        sync_dir(&staging)?;
        if fs::symlink_metadata(&final_dir).is_ok() {
            return Err(fail("generation identity collision"));
        }
        fs::rename(&staging, &final_dir).map_err(|_| fail("generation publication failed"))?;
        sync_dir(&generations)?;
        fs::rename(
            final_dir.join(".completion-pending.json"),
            final_dir.join("completion.json"),
        )
        .map_err(|_| fail("completion publication failed"))?;
        sync_dir(&final_dir)?;
        Ok((
            final_dir
                .join("tree")
                .join(&self.name)
                .display()
                .to_string(),
            Provenance {
                authority: "signed-aggregate".into(),
                aggregate_sha256: Some(self.aggregate.sha.clone()),
                component_sha256: Some(self.sha.clone()),
                generation: Some(final_dir.join("tree").display().to_string()),
                archive: self.archive,
                pinned: self.aggregate.pinned,
            },
        ))
    }
    fn validate_bytes(&self, id: &Identity) -> Result<()> {
        if id.sha256 != self.sha || self.size.is_some_and(|n| n != id.size) || id.size == 0 {
            return Err(fail("component size/hash mismatch"));
        }
        Ok(())
    }
}
fn executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| fail("executable mode failed"))?;
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| fail("executable sync failed"))
}
