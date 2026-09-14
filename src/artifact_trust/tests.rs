//! Synthetic authority is private to this test module. These tests do not claim cosign proof.
use super::*;
use tempfile::TempDir;
fn selected_fixture(path: &Path, kind: &str) -> Aggregate {
    let bytes = fs::read(path).unwrap();
    let raw = serde_json::json!({"schema_version":3,"version":"synthetic","binaries":{"enscrive-developer":{"source_version":"synthetic","kind":kind,"platforms":{"test-target":{"url":format!("file://{}",path.display()),"sha256":digest(&bytes),"size_bytes":bytes.len()}}}}});
    Aggregate {
        manifest: serde_json::from_value(raw).unwrap(),
        sha: "a".repeat(64),
        bundle_sha: "b".repeat(64),
        verifier_sha: "c".repeat(64),
        pinned: true,
        evidence: path.parent().unwrap().into(),
        file_root: Some(path.parent().unwrap().into()),
    }
}
pub(super) fn archive_fixture(path: &Path, entries: &[(&str, tar::EntryType, &[u8])]) {
    let gzip =
        flate2::write::GzEncoder::new(File::create(path).unwrap(), flate2::Compression::default());
    let mut tar = tar::Builder::new(gzip);
    for (name, kind, bytes) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(*kind);
        h.set_mode(0o644);
        h.set_size(bytes.len() as u64);
        h.set_cksum();
        tar.append_data(&mut h, name, *bytes).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap();
}
#[test]
fn ens5913_location_and_pin_rules() {
    for bad in [
        "http://example.com/x",
        "https://u:p@example.com/x",
        "https://example.com/x?secret=1",
        "https://example.com/x#f",
        "https://example.com/a/../b",
        "https://example.com/%2e",
        "file://relative",
        "file:///a/../b",
        "https://example.com/a/.\t./b",
        "https://example.com/a\r/b",
        "https://example.com/a\n/b",
        " https://example.com/a",
        "https://example.com/a ",
        "file:///tmp/a\t",
    ] {
        assert!(Location::parse(bad).is_err(), "{bad}");
    }
    assert!(Location::parse("https://example.com/releases/dev/latest.json").is_ok());
    assert!(valid_digest(&"a".repeat(64)));
    assert!(!valid_digest(&"A".repeat(64)));
}
#[test]
fn ens5913_target_and_name_selection_refuse() {
    let t = TempDir::new().unwrap();
    let p = t.path().join("input");
    fs::write(&p, b"binary").unwrap();
    let a = selected_fixture(&p, "binary");
    assert!(a.select("enscrive-developer", "wrong-target").is_err());
    assert!(a.select("../other", "test-target").is_err());
}
#[tokio::test]
async fn ens5913_plain_cache_and_force_preserve_generations() {
    let t = TempDir::new().unwrap();
    let p = t.path().join("input");
    fs::write(&p, b"binary-v1").unwrap();
    let a = selected_fixture(&p, "binary");
    let s = a.select("enscrive-developer", "test-target").unwrap();
    let root = t.path().join("output");
    let (first, proof) = s.install(&root, false).await.unwrap();
    assert_eq!(proof.authority, "signed-aggregate");
    assert_eq!(fs::read(&first).unwrap(), b"binary-v1");
    fs::write(&first, b"locally-mutated").unwrap();
    let (second, _) = s.install(&root, false).await.unwrap();
    assert_ne!(first, second);
    assert_eq!(fs::read(&second).unwrap(), b"binary-v1");
    assert_eq!(fs::read(&first).unwrap(), b"locally-mutated");
    fs::write(&p, b"wrong-input").unwrap();
    assert!(s.install(&root, true).await.is_err());
    assert_eq!(fs::read(&second).unwrap(), b"binary-v1");
    let cached = root.join("compressed-cache").join(&s.sha);
    fs::write(cached, b"bad-cache").unwrap();
    assert!(s.install(&root, false).await.is_err());
    assert_eq!(fs::read(&second).unwrap(), b"binary-v1");
}
#[tokio::test]
async fn ens5913_archive_reextracts_and_keeps_site_generation() {
    let t = TempDir::new().unwrap();
    let p = t.path().join("input.tgz");
    archive_fixture(
        &p,
        &[
            (".", tar::EntryType::Directory, b""),
            ("enscrive-developer", tar::EntryType::Regular, b"exe"),
            ("site/pkg/app.js", tar::EntryType::Regular, b"asset"),
            ("site", tar::EntryType::Directory, b""),
        ],
    );
    let a = selected_fixture(&p, "archive");
    let s = a.select("enscrive-developer", "test-target").unwrap();
    let root = t.path().join("output");
    let (first, proof) = s.install(&root, false).await.unwrap();
    let site = Path::new(proof.generation.as_ref().unwrap()).join("site/pkg/app.js");
    fs::write(&site, b"tampered").unwrap();
    let (second, next) = s.install(&root, false).await.unwrap();
    assert_ne!(first, second);
    assert_eq!(
        fs::read(Path::new(next.generation.as_ref().unwrap()).join("site/pkg/app.js")).unwrap(),
        b"asset"
    );
    assert_eq!(fs::read(site).unwrap(), b"tampered");
}
#[test]
fn ens5913_archive_duplicate_and_link_refuse() {
    let t = TempDir::new().unwrap();
    for (i, entries) in [
        vec![
            ("enscrive-developer", tar::EntryType::Regular, &b"exe"[..]),
            (
                "./enscrive-developer",
                tar::EntryType::Regular,
                &b"again"[..],
            ),
        ],
        vec![("enscrive-developer", tar::EntryType::Symlink, &b""[..])],
    ]
    .into_iter()
    .enumerate()
    {
        let p = t.path().join(format!("{i}.tgz"));
        archive_fixture(&p, &entries);
        let out = t.path().join(format!("out{i}"));
        private_dir(&out).unwrap();
        assert!(
            archive::extract(
                &p,
                &out,
                "enscrive-developer",
                Instant::now() + Duration::from_secs(30)
            )
            .is_err()
        );
    }
}
#[test]
fn ens5913_identity_rejects_links_and_expired_work() {
    let t = TempDir::new().unwrap();
    let p = t.path().join("file");
    fs::write(&p, b"bytes").unwrap();
    let l = t.path().join("link");
    std::os::unix::fs::symlink(&p, &l).unwrap();
    assert!(identity(&l, Instant::now() + Duration::from_secs(1), 100).is_err());
    assert!(identity(&p, Instant::now(), 100).is_err());
    fs::hard_link(&p, t.path().join("hard")).unwrap();
    assert!(identity(&p, Instant::now() + Duration::from_secs(1), 100).is_err());
}

pub(super) struct EnvRestore(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl EnvRestore {
    pub(super) fn set(values: &[(&'static str, Option<&std::ffi::OsStr>)]) -> Self {
        let previous = values
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        for (k, v) in values {
            unsafe {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
        Self(previous)
    }
}
impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (k, v) in &self.0 {
            unsafe {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn ens5913_aggregate_exact_bytes_expected_pin_bundle_and_verifier_refusal() {
    use std::os::unix::fs::PermissionsExt;
    let _lock = crate::test_support::lock_env();
    let t = TempDir::new().unwrap();
    let verifier = t.path().join("verifier");
    // Synthetic verifier checks exact CLI argv and fixed fixture bundle. It is not cosign.
    fs::write(&verifier,format!("#!/bin/sh\ntest \"$5\" = '{}' || exit 1\ntest \"$7\" = '{}' || exit 1\nIFS= read -r b < \"$3\"\ntest \"$b\" = accepted-synthetic-bundle\n",IDENTITY,ISSUER)).unwrap();
    fs::set_permissions(&verifier, fs::Permissions::from_mode(0o700)).unwrap();
    let _env = EnvRestore::set(&[
        ("ENSCRIVE_COSIGN_BIN", Some(verifier.as_os_str())),
        ("HOME", Some(t.path().as_os_str())),
        ("PATH", Some(std::ffi::OsStr::new("/usr/bin:/bin"))),
        ("ENSCRIVE_MANIFEST_URL", None),
        ("ENSCRIVE_EXPECTED_MANIFEST_SHA256", None),
        ("ENSCRIVE_PINSET_ORIGIN", None),
    ]);
    let body = b"{\"schema_version\":3,\"version\":\"test\",\"binaries\":{}}";
    let sha = digest(body);
    let pin = t.path().join("pinsets/sha256").join(&sha);
    fs::create_dir_all(&pin).unwrap();
    fs::write(pin.join("manifest.json"), body).unwrap();
    fs::write(pin.join("manifest.bundle"), b"accepted-synthetic-bundle\n").unwrap();
    let origin = format!("file://{}", t.path().display());
    let policy = Policy::new(None, Some(sha.clone()), Some(origin.clone())).unwrap();
    let output = t.path().join("evidence");
    let result = Aggregate::load(&policy, &output, "must-not-be-used")
        .await
        .unwrap();
    assert_eq!(result.sha, sha);
    assert!(result.pinned);
    fs::write(pin.join("manifest.bundle"), b"wrong\n").unwrap();
    assert!(Aggregate::load(&policy, &output, "unused").await.is_err());
    fs::remove_file(pin.join("manifest.bundle")).unwrap();
    assert!(Aggregate::load(&policy, &output, "unused").await.is_err());
    fs::write(pin.join("manifest.bundle"), b"accepted-synthetic-bundle\n").unwrap();
    fs::write(pin.join("manifest.json"), [body.as_slice(), b" "].concat()).unwrap();
    assert!(Aggregate::load(&policy, &output, "unused").await.is_err());
    let bad = Policy::new(
        Some(format!("file://{}/manifest.json", pin.display())),
        Some("0".repeat(64)),
        Some(origin),
    )
    .unwrap();
    assert!(Aggregate::load(&bad, &output, "unused").await.is_err());
    assert!(!t.path().join("generations").exists());
}

#[test]
fn ens5913_policy_empty_nonunicode_and_explicit_bad_verifier() {
    use std::os::unix::ffi::OsStringExt;
    let _lock = crate::test_support::lock_env();
    let bad = std::ffi::OsString::from_vec(vec![255]);
    let _env = EnvRestore::set(&[
        ("ENSCRIVE_MANIFEST_URL", Some(&bad)),
        ("ENSCRIVE_EXPECTED_MANIFEST_SHA256", None),
        ("ENSCRIVE_PINSET_ORIGIN", None),
        ("ENSCRIVE_COSIGN_BIN", Some(std::ffi::OsStr::new(""))),
    ]);
    assert!(Policy::new(None, None, None).is_err());
    assert!(Policy::new(Some(String::new()), None, None).is_err());
    assert!(process::Verifier::resolve().is_err());
}

#[test]
fn ens5913_gnu_longname_and_pax_path_effective_entries() {
    let t = TempDir::new().unwrap();
    let input = t.path().join("long.tgz");
    let long = format!("site/{}/asset.js", "a".repeat(150));
    archive_fixture(
        &input,
        &[
            ("enscrive-developer", tar::EntryType::Regular, b"exe"),
            (&long, tar::EntryType::Regular, b"asset"),
        ],
    );
    let out = t.path().join("long-out");
    private_dir(&out).unwrap();
    archive::extract(
        &input,
        &out,
        "enscrive-developer",
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(fs::read(out.join(long)).unwrap(), b"asset");
    let input = t.path().join("pax.tgz");
    let gzip = flate2::write::GzEncoder::new(
        File::create(&input).unwrap(),
        flate2::Compression::default(),
    );
    let mut tar = tar::Builder::new(gzip);
    tar.append_pax_extensions([("path", &b"enscrive-developer"[..])])
        .unwrap();
    let mut h = tar::Header::new_ustar();
    h.set_mode(0o644);
    h.set_size(3);
    h.set_cksum();
    tar.append_data(&mut h, "ignored", &b"exe"[..]).unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    let out = t.path().join("pax-out");
    private_dir(&out).unwrap();
    archive::extract(
        &input,
        &out,
        "enscrive-developer",
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(fs::read(out.join("enscrive-developer")).unwrap(), b"exe");
}

#[tokio::test]
async fn ens5913_plain_cache_mode_and_force_bypass() {
    use std::os::unix::fs::PermissionsExt;
    let t = TempDir::new().unwrap();
    let p = t.path().join("input");
    fs::write(&p, b"binary").unwrap();
    let a = selected_fixture(&p, "binary");
    let s = a.select("enscrive-developer", "test-target").unwrap();
    let root = t.path().join("output");
    let (old, _) = s.install(&root, false).await.unwrap();
    let cache = root.join("compressed-cache").join(&s.sha);
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(s.install(&root, false).await.is_err());
    let (new, _) = s.install(&root, true).await.unwrap();
    assert_ne!(old, new);
    assert_eq!(fs::read(old).unwrap(), b"binary");
    assert_eq!(fs::read(new).unwrap(), b"binary");
}

#[test]
fn ens5913_signed_https_authority_cannot_select_local_files() {
    let t = TempDir::new().unwrap();
    let p = t.path().join("input");
    fs::write(&p, b"binary").unwrap();
    let mut a = selected_fixture(&p, "binary");
    a.file_root = None;
    assert!(a.select("enscrive-developer", "test-target").is_err());
    a.file_root = Some(t.path().join("other-root"));
    assert!(a.select("enscrive-developer", "test-target").is_err());
}

#[tokio::test]
async fn ens5913_exact_declared_size_and_stream_caps() {
    let t = TempDir::new().unwrap();
    let input = t.path().join("input");
    fs::write(&input, b"12345").unwrap();
    let mut a = selected_fixture(&input, "binary");
    a.manifest
        .binaries
        .get_mut("enscrive-developer")
        .unwrap()
        .platforms
        .get_mut("test-target")
        .unwrap()
        .size_bytes = Some(4);
    let selected = a.select("enscrive-developer", "test-target").unwrap();
    assert_eq!(
        selected
            .install(&t.path().join("out"), false)
            .await
            .unwrap_err(),
        fail("component size/hash mismatch")
    );
    // The source digest matches exactly; only the signed declared length disagrees.
    let copy = t.path().join("copy");
    assert_eq!(
        copy_file(&input, &copy, 4, Instant::now() + Duration::from_secs(2)).unwrap_err(),
        fail("copy limit exceeded")
    );
    assert_eq!(fs::read(copy).unwrap().len(), 0);
    let source = Location::parse(&format!("file://{}", input.display())).unwrap();
    assert_eq!(
        read_to(
            &source,
            &t.path().join("read"),
            4,
            Instant::now() + Duration::from_secs(2)
        )
        .await
        .unwrap_err(),
        fail("artifact file kind/size invalid")
    );
}

#[tokio::test]
async fn ens5913_actual_download_stream_writer_limit_and_failure() {
    let d = Instant::now() + Duration::from_secs(2);
    let mut output = Vec::new();
    let chunks = futures_util::stream::iter([Ok::<_, ()>(vec![1, 2, 3]), Ok(vec![4, 5, 6])]);
    assert_eq!(
        write_chunks(chunks, &mut output, 5, d).await.unwrap_err(),
        fail("input limit exceeded")
    );
    assert_eq!(output, vec![1, 2, 3]);
    let mut output = Vec::new();
    let chunks = futures_util::stream::iter([
        Ok(vec![1, 2]),
        Err::<Vec<u8>, _>("synthetic remote diagnostic"),
    ]);
    assert_eq!(
        write_chunks(chunks, &mut output, 5, d).await.unwrap_err(),
        fail("HTTPS body unavailable")
    );
    assert_eq!(output, vec![1, 2]);
}
