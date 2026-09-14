// Included inside local::tests: preserve the original two public libtest names.
// Prepared synthetic authentication; no service/provider or real ESM invocation.
struct ArtifactRegressionEnvironment(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl ArtifactRegressionEnvironment {
    fn install(root: &Path) -> Self {
        let names = ["HOME", "PATH", "ENSCRIVE_COSIGN_BIN", "XDG_CONFIG_HOME", "XDG_DATA_HOME"];
        let saved = Self(names.iter().map(|&name| (name, env::var_os(name))).collect());
        let verifier = root.join("synthetic-verifier");
        fs::write(&verifier, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&verifier, fs::Permissions::from_mode(0o700)).unwrap();
        }
        unsafe {
            env::set_var("HOME", root);
            env::set_var("PATH", "/usr/bin:/bin");
            env::set_var("ENSCRIVE_COSIGN_BIN", verifier);
            env::set_var("XDG_CONFIG_HOME", root.join("config"));
            env::set_var("XDG_DATA_HOME", root.join("data"));
        }
        saved
    }
}
impl Drop for ArtifactRegressionEnvironment {
    fn drop(&mut self) {
        for (name, value) in &self.0 {
            unsafe {
                match value { Some(value) => env::set_var(name, value), None => env::remove_var(name) }
            }
        }
    }
}
const REGRESSION_BINARIES: [(&str, &[u8]); 4] = [
    (BINARY_DEVELOPER, b"synthetic developer bytes"),
    (BINARY_OBSERVE, b"synthetic observe bytes"),
    (BINARY_EMBED, b"synthetic embed bytes"),
    (BINARY_DOCS, b"synthetic docs bytes"),
];
fn regression_sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
fn regression_signed_origin(root: &Path, missing_target: bool) -> (SelfManagedInitOptions, std::collections::BTreeMap<String, String>) {
    use std::io::Write;
    let origin = root.join(if missing_target { "missing-origin" } else { "valid-origin" });
    fs::create_dir_all(&origin).unwrap();
    let mut binaries = serde_json::Map::new();
    let mut hashes = std::collections::BTreeMap::new();
    for (name, executable) in REGRESSION_BINARIES {
        let bytes = if name == BINARY_DEVELOPER {
            let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            let mut archive = tar::Builder::new(gzip);
            for (path, contents) in [(BINARY_DEVELOPER, executable), ("site/pkg/current.js", &b"exact site javascript"[..])] {
                let mut header = tar::Header::new_gnu();
                header.set_size(contents.len() as u64);
                header.set_mode(if path == BINARY_DEVELOPER { 0o755 } else { 0o644 });
                header.set_cksum();
                archive.append_data(&mut header, path, contents).unwrap();
            }
            archive.into_inner().unwrap().finish().unwrap()
        } else { executable.to_vec() };
        let path = origin.join(format!("{name}.artifact"));
        fs::File::create(&path).unwrap().write_all(&bytes).unwrap();
        let digest = regression_sha(&bytes);
        hashes.insert(name.to_owned(), digest.clone());
        let target = if missing_target && name == BINARY_DEVELOPER { "not-the-current-target" } else { crate::release_channel::current_target() };
        let mut platforms = serde_json::Map::new();
        platforms.insert(target.to_owned(), json!({"url":format!("file://{}",path.display()),"sha256":digest,"size_bytes":bytes.len()}));
        binaries.insert(name.to_owned(), json!({"source_version":"synthetic-regression","kind":if name==BINARY_DEVELOPER {"archive"} else {"binary"},"platforms":platforms}));
    }
    let raw = serde_json::to_vec(&json!({"schema_version":3,"version":"synthetic-regression","binaries":binaries})).unwrap();
    let pin = regression_sha(&raw);
    let directory = origin.join("pinsets/sha256").join(&pin);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("manifest.json"), raw).unwrap();
    fs::write(directory.join("manifest.bundle"), b"synthetic verifier fixture, not a real signature").unwrap();
    (SelfManagedInitOptions {
        profile_name: Some("local".into()), with_grafana: false, developer_port: None,
        developer_bin: None, observe_bin: None, embed_bin: None, docs_bin: None, esm_bin: None,
        // Prepared init writes synthetic configuration only; no provider is contacted.
        openai_api_key: Some("synthetic-provider".into()), anthropic_api_key: None, voyage_api_key: None, nebius_api_key: None,
        set_default: true, manifest_url: None, expected_manifest_sha256: Some(pin),
        pinset_origin: Some(format!("file://{}", origin.display())), force_refetch: false,
    }, hashes)
}
fn regression_profile() -> LocalBinaries {
    load_profiles_raw().unwrap().profiles["local"].local.as_ref().unwrap().binaries.clone()
}
fn regression_assert_profile(binaries: &LocalBinaries, opts: &SelfManagedInitOptions, hashes: &std::collections::BTreeMap<String, String>) {
    let home = cli_home().unwrap();
    for ((name, expected), path) in REGRESSION_BINARIES.into_iter().zip([&binaries.developer, &binaries.observe, &binaries.embed, &binaries.docs]) {
        assert_eq!(fs::read(path).unwrap(), expected);
        let proof = &binaries.provenance[name];
        assert_eq!(proof.authority, "signed-aggregate");
        assert_eq!(proof.aggregate_sha256.as_ref(), opts.expected_manifest_sha256.as_ref());
        assert_eq!(proof.component_sha256.as_ref(), hashes.get(name));
        assert!(proof.pinned);
        assert_eq!(proof.archive, name == BINARY_DEVELOPER);
        let generation = PathBuf::from(proof.generation.as_ref().unwrap());
        assert!(generation.starts_with(home.data_root.join("release-artifacts/generations")));
        assert_eq!(Path::new(path), generation.join(name));
        assert!(generation.parent().unwrap().join("completion.json").is_file());
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }
    assert_eq!(binaries.provenance.len(), 5);
    assert_eq!(binaries.provenance[BINARY_ESM].authority, "operator-trusted");
    assert!(Path::new(&binaries.esm).is_file());
    let site = discover_developer_site_root(&home, binaries).unwrap();
    assert_eq!(site, Path::new(&binaries.developer).parent().unwrap().join("site"));
    assert_eq!(fs::read(site.join("pkg/current.js")).unwrap(), b"exact site javascript");
    let parsed = load_profiles_raw().unwrap();
    assert_eq!(parsed.default_profile.as_deref(), Some("local"));
    let local = parsed.profiles["local"].local.as_ref().unwrap();
    assert_eq!(read_env_value(&PathBuf::from(&local.config_dir).join("developer.env"), "LEPTOS_SITE_ROOT"), Some(site.display().to_string()));
}
fn regression_snapshot(binaries: &LocalBinaries) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut values = std::collections::BTreeMap::new();
    for path in [&binaries.developer, &binaries.observe, &binaries.embed, &binaries.docs] {
        values.insert(PathBuf::from(path), fs::read(path).unwrap());
        let completion = Path::new(path).parent().unwrap().parent().unwrap().join("completion.json");
        values.insert(completion.clone(), fs::read(completion).unwrap());
    }
    let site = Path::new(&binaries.developer).parent().unwrap().join("site/pkg/current.js");
    values.insert(site.clone(), fs::read(site).unwrap());
    values
}
fn regression_stale_globals(home: &CliHome) -> (PathBuf, PathBuf) {
    let binary = home.data_root.join("bin/enscrive-developer");
    fs::create_dir_all(binary.parent().unwrap()).unwrap(); fs::write(&binary, b"stale global binary").unwrap();
    let site = installed_developer_site_root(home).join("pkg/stale.js");
    fs::create_dir_all(site.parent().unwrap()).unwrap(); fs::write(&site, b"stale global site").unwrap();
    (binary, site)
}
#[cfg(target_os = "linux")]
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn init_self_managed_fetches_binaries_from_file_manifest() {
    let _lock = crate::test_support::lock_env();
    let temp = TempDir::new().unwrap(); let _env = ArtifactRegressionEnvironment::install(temp.path());
    let (mut opts, hashes) = regression_signed_origin(temp.path(), false);
    let globals = regression_stale_globals(&cli_home().unwrap());
    init_prepared_fixture(opts.clone()).await.unwrap();
    let first = regression_profile(); regression_assert_profile(&first, &opts, &hashes);
    // Damage only an extracted tree. An intact compressed cache may legitimately be reused.
    fs::write(&first.developer, b"corrupted old extracted tree").unwrap();
    let first_snapshot = regression_snapshot(&first);
    init_prepared_fixture(opts.clone()).await.unwrap();
    let second = regression_profile(); regression_assert_profile(&second, &opts, &hashes);
    assert_ne!(first.developer, second.developer);
    for (path, bytes) in &first_snapshot { assert_eq!(fs::read(path).unwrap(), *bytes); }
    let second_snapshot = regression_snapshot(&second);
    opts.force_refetch = true;
    init_prepared_fixture(opts.clone()).await.unwrap();
    let third = regression_profile(); regression_assert_profile(&third, &opts, &hashes);
    for (old, new) in [&second.developer, &second.observe, &second.embed, &second.docs].into_iter().zip([&third.developer, &third.observe, &third.embed, &third.docs]) { assert_ne!(old, new); }
    for (path, bytes) in first_snapshot.iter().chain(second_snapshot.iter()) { assert_eq!(fs::read(path).unwrap(), *bytes); }
    assert_eq!(fs::read(globals.0).unwrap(), b"stale global binary");
    assert_eq!(fs::read(globals.1).unwrap(), b"stale global site");
}
#[cfg(target_os = "linux")]
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn init_self_managed_reports_platform_missing_error() {
    let _lock = crate::test_support::lock_env();
    let temp = TempDir::new().unwrap(); let _env = ArtifactRegressionEnvironment::install(temp.path());
    let (good, hashes) = regression_signed_origin(temp.path(), false);
    init_prepared_fixture(good.clone()).await.unwrap();
    let original = regression_profile(); regression_assert_profile(&original, &good, &hashes);
    let snapshots = regression_snapshot(&original);
    let home = cli_home().unwrap(); let globals = regression_stale_globals(&home);
    let profile_path = home.config_root.join("profiles.toml"); let profile = fs::read(&profile_path).unwrap();
    let (bad, _) = regression_signed_origin(temp.path(), true);
    assert_eq!(init_prepared_fixture(bad).await.unwrap_err(), "artifact trust: signed target missing");
    assert_eq!(fs::read(profile_path).unwrap(), profile);
    for (path, bytes) in snapshots { assert_eq!(fs::read(path).unwrap(), bytes); }
    assert_eq!(fs::read(globals.0).unwrap(), b"stale global binary");
    assert_eq!(fs::read(globals.1).unwrap(), b"stale global site");
}
