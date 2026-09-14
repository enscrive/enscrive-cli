use super::*;
use tempfile::TempDir;
fn opts() -> SelfManagedInitOptions {
    SelfManagedInitOptions {
        profile_name: Some("local".into()),
        with_grafana: false,
        developer_port: None,
        developer_bin: None,
        observe_bin: None,
        embed_bin: None,
        esm_bin: Some("/nonexistent/ens5903-esm".into()),
        docs_bin: None,
        openai_api_key: None,
        anthropic_api_key: None,
        voyage_api_key: None,
        nebius_api_key: None,
        set_default: true,
        manifest_url: Some("http://must-not-resolve.invalid".into()),
        force_refetch: false,
    }
}
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn ens5903_missing_gate_is_read_only() {
    let _guard = crate::test_support::lock_env();
    let t = TempDir::new().unwrap();
    unsafe {
        env::set_var("XDG_CONFIG_HOME", t.path().join("config"));
        env::set_var("XDG_DATA_HOME", t.path().join("data"));
    }
    assert!(init_self_managed(opts()).await.is_err());
    assert!(!cli_home().unwrap().config_root.exists());
    assert!(!cli_home().unwrap().data_root.exists());
    let root = cli_home().unwrap().config_root;
    fs::create_dir_all(&root).unwrap();
    let p = root.join("profiles.toml");
    fs::write(&p, "SYNTHETIC_PRIVATE_PROFILE='").unwrap();
    let err = load_profiles_raw().err().unwrap();
    assert!(!err.contains("SYNTHETIC_PRIVATE_PROFILE"));
    assert_eq!(
        fs::read_to_string(&p).unwrap(),
        "SYNTHETIC_PRIVATE_PROFILE='"
    );
}
#[tokio::test]
async fn ens5903_setter_and_repair_protected_namespace() {
    let t = TempDir::new().unwrap();
    for name in local_auth::NAMES {
        let err = esm_set(
            "/not-executed",
            t.path(),
            &t.path().join("master"),
            name,
            "private-value",
        )
        .await
        .unwrap_err();
        assert!(err.contains("operator preparation"));
    }
    let p = t.path().join("config/developer.env");
    local_auth::protected_write(
        &p,
        "# exact comment\nAES_KEY=bad\nUNRELATED=007\nOBSERVE_GRPC_DEVELOPER_KEY=old\n",
    )
    .unwrap();
    ensure_valid_developer_env(&p, None, Some("new-output")).unwrap();
    let text = fs::read_to_string(&p).unwrap();
    assert!(text.starts_with("# exact comment\n"));
    assert!(text.contains("UNRELATED=007\n"));
    assert!(text.contains("OBSERVE_GRPC_DEVELOPER_KEY=old\n"));
    assert_eq!(
        fs::metadata(&p).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
#[tokio::test]
#[ignore = "requires separately verified pinned ESM and disposable-vault execution gate"]
async fn ens5903_actual_esm_parser_and_synthesis_preserves_auth() {
    use sha2::{Digest, Sha256};
    let binary =
        PathBuf::from(env::var_os("ENS5903_TEST_ESM").expect("gated pinned binary required"));
    let digest = format!("{:x}", Sha256::digest(fs::read(&binary).unwrap()));
    assert_eq!(
        digest,
        "b593e8d2d216515a63b140d85bf3e0276e5a08c970b62a398edcd0b718ffda2b"
    );
    let t = TempDir::new().unwrap();
    let runtime = t.path().join("runtime");
    fs::create_dir(&runtime).unwrap();
    let master = runtime.join(".esm-master-key");
    fs::write(&master, "  synthetic-parser-passphrase\n").unwrap();
    let services = ["enscrive-developer", "enscrive-observe", "enscrive-embed"];
    let keys = [
        vec![(local_auth::NAMES[1], "01".repeat(32))],
        vec![
            (local_auth::NAMES[1], "01".repeat(32)),
            (local_auth::NAMES[2], "02".repeat(32)),
            (local_auth::NAMES[3], "03".repeat(32)),
        ],
        vec![(local_auth::NAMES[2], "02".repeat(32))],
    ];
    for (service, values) in services.iter().zip(keys) {
        let cwd = esm_vault_dir(&runtime, service);
        fs::create_dir_all(&cwd).unwrap();
        let vault = esm_vault_file(&runtime, service);
        let prefix = [
            "--vault-path",
            vault.to_str().unwrap(),
            "--key-file",
            master.to_str().unwrap(),
            "--no-keyring",
        ];
        let mut args = prefix.to_vec();
        args.push("init");
        local_auth::fixtures::actual_command(&binary, &cwd, &args).unwrap();
        for (name, value) in values {
            let mut args = prefix.to_vec();
            args.extend(["set", name, &value]);
            local_auth::fixtures::actual_command(&binary, &cwd, &args).unwrap();
        }
    }
    let vaults = services.map(|s| esm_vault_file(&runtime, s));
    let before = vaults
        .iter()
        .map(|p| fs::read(p).unwrap())
        .collect::<Vec<_>>();
    let prepared = Prepared::load(
        local_auth::Executable::at(&binary).unwrap(),
        &runtime,
        chrono::Utc::now(),
    )
    .unwrap();
    for (p, b) in vaults.iter().zip(&before) {
        assert_eq!(&fs::read(p).unwrap(), b);
    }
    let config = t.path().join("config");
    local_auth::protected_dir(&config).unwrap();
    let env_files = [
        config.join("developer.env"),
        config.join("observe.env"),
        config.join("embed.env"),
    ];
    for (p, r) in env_files
        .iter()
        .zip([prepared.developer(), prepared.observe(), prepared.embed()])
    {
        let mut doc = EnvDocument::parse("# unchanged\nUNRELATED_TEST=007\n").unwrap();
        r.refresh(&mut doc);
        doc.write(p).unwrap();
    }
    synthesize_local_esm_vaults(
        &prepared,
        &runtime,
        [&env_files[0], &env_files[1], &env_files[2]],
    )
    .await
    .unwrap();
    for (p, b) in vaults.iter().zip(&before) {
        let old: toml::Value = toml::from_str(std::str::from_utf8(b).unwrap()).unwrap();
        let new: toml::Value = toml::from_str(&fs::read_to_string(p).unwrap()).unwrap();
        for record in old["secrets"].as_array().unwrap() {
            let name = record["name"].as_str().unwrap();
            let found = new["secrets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"].as_str() == Some(name))
                .unwrap();
            assert_eq!(
                record, found,
                "unrelated synthesis changed existing auth ciphertext/metadata"
            );
        }
    }
    // Authenticated list absence/derived/duplicate outcomes against the actual
    // pinned parser, using only explicitly owned synthetic encrypted fixtures.
    let original = fs::read(&vaults[1]).unwrap();
    for mode in ["missing", "derived", "duplicate"] {
        let mut doc: toml::Value = toml::from_str(std::str::from_utf8(&original).unwrap()).unwrap();
        let rows = doc.get_mut("secrets").unwrap().as_array_mut().unwrap();
        let position = rows
            .iter()
            .position(|r| r["name"].as_str().unwrap().ends_with(local_auth::NAMES[3]))
            .unwrap();
        match mode {
            "missing" => {
                rows.remove(position);
            }
            "derived" => {
                rows[position]
                    .as_table_mut()
                    .unwrap()
                    .insert("derived".into(), toml::Value::Boolean(true));
            }
            "duplicate" => {
                rows.push(rows[position].clone());
            }
            _ => unreachable!(),
        }
        fs::write(&vaults[1], toml::to_string(&doc).unwrap()).unwrap();
        let staged = fs::read(&vaults[1]).unwrap();
        let error = Prepared::load(
            local_auth::Executable::at(&binary).unwrap(),
            &runtime,
            chrono::Utc::now(),
        )
        .unwrap_err();
        assert!(error.contains(if mode == "missing" {
            "missing"
        } else {
            "derived or duplicate"
        }));
        assert_eq!(fs::read(&vaults[1]).unwrap(), staged);
        fs::write(&vaults[1], &original).unwrap();
    }
    let cwd = esm_vault_dir(&runtime, "enscrive-developer");
    let valid_dev = fs::read(&vaults[0]).unwrap();
    for value in ["04".repeat(32), "01".repeat(32) + "\n"] {
        local_auth::fixtures::actual_command(
            &binary,
            &cwd,
            &[
                "--vault-path",
                vaults[0].to_str().unwrap(),
                "--key-file",
                master.to_str().unwrap(),
                "--no-keyring",
                "set",
                local_auth::NAMES[1],
                &value,
            ],
        )
        .unwrap();
        let staged = fs::read(&vaults[0]).unwrap();
        assert!(
            Prepared::load(
                local_auth::Executable::at(&binary).unwrap(),
                &runtime,
                chrono::Utc::now()
            )
            .is_err()
        );
        assert_eq!(fs::read(&vaults[0]).unwrap(), staged);
        fs::write(&vaults[0], &valid_dev).unwrap();
    }
    let after = vaults
        .iter()
        .map(|p| fs::read(p).unwrap())
        .collect::<Vec<_>>();
    fs::write(&master, "wrong-master").unwrap();
    assert!(
        Prepared::load(
            local_auth::Executable::at(&binary).unwrap(),
            &runtime,
            chrono::Utc::now()
        )
        .is_err()
    );
    for (p, b) in vaults.iter().zip(after) {
        assert_eq!(fs::read(p).unwrap(), b);
    }
}

#[test]
fn ens5903_actual_child_namespace_and_role_binding() {
    let _guard = crate::test_support::lock_env();
    let t = TempDir::new().unwrap();
    let binary = local_auth::fixtures::prepare(t.path());
    let prepared = Prepared::load(
        local_auth::Executable::at(&binary).unwrap(),
        t.path(),
        chrono::Utc::now(),
    )
    .unwrap();
    struct Restore(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                unsafe {
                    match v {
                        Some(v) => env::set_var(k, v),
                        None => env::remove_var(k),
                    }
                }
            }
        }
    }
    let _restore = Restore(
        local_auth::NAMES
            .iter()
            .map(|&k| (k, env::var_os(k)))
            .collect(),
    );
    for k in local_auth::NAMES {
        unsafe {
            env::set_var(k, "ambient-collision");
        }
    }
    let script = t.path().join("capture-app");
    fs::write(&script,"#!/usr/bin/python3\nimport os,json,pathlib\nvalue={k:v for k,v in os.environ.items() if k.startswith('OBSERVE_GRPC_') or k=='UNRELATED'}\npathlib.Path(os.environ['ENS5903_CAPTURE']).write_text(json.dumps(value))\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let log = t.path().join("logs");
    local_auth::protected_dir(&log).unwrap();
    let envfile = t.path().join("env/service.env");
    let mut text = String::from("# untouched\nUNRELATED=file\n");
    for k in local_auth::NAMES {
        text.push_str(&format!("{k}=file-collision\n"));
    }
    local_auth::protected_write(&envfile, &text).unwrap();
    for (service, recipient, own) in [
        ("enscrive-developer", prepared.developer(), 1),
        ("enscrive-observe", prepared.observe(), 0),
        ("enscrive-embed", prepared.embed(), 2),
    ] {
        let capture = t.path().join(format!("{service}.json"));
        let mut extras = local_auth::NAMES
            .iter()
            .map(|&k| (k, "extra-collision".into()))
            .collect::<Vec<_>>();
        extras.push(("UNRELATED", "extra".into()));
        extras.push(("ENS5903_CAPTURE", capture.to_str().unwrap().into()));
        let result = spawn_service_with_extra_env(
            service,
            script.to_str().unwrap(),
            &envfile,
            &log,
            &extras,
            recipient,
        )
        .unwrap();
        let pid = result["pid"].as_i64().unwrap() as i32;
        unsafe extern "C" {
            fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
            fn kill(pid: i32, sig: i32) -> i32;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut status = 0;
        loop {
            let done = unsafe { waitpid(pid, &mut status, 1) };
            if done == pid {
                break;
            }
            assert!(done >= 0);
            if Instant::now() >= deadline {
                unsafe {
                    kill(pid, 9);
                    waitpid(pid, &mut status, 0);
                }
                panic!("owned capture app deadline");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(status, 0);
        let values: BTreeMap<String, String> =
            serde_json::from_str(&fs::read_to_string(capture).unwrap()).unwrap();
        assert_eq!(values["UNRELATED"], "extra");
        assert_eq!(values[local_auth::NAMES[0]], "1");
        for i in 1..10 {
            if own == 0 && i <= 3 || own == i {
                assert_eq!(values[local_auth::NAMES[i]], format!("{i:02}").repeat(32));
            } else {
                assert!(!values.contains_key(local_auth::NAMES[i]));
            }
        }
    }
    assert!(
        spawn_service_with_extra_env(
            "enscrive-observe",
            script.to_str().unwrap(),
            &envfile,
            &log,
            &[],
            prepared.developer()
        )
        .unwrap_err()
        .contains("mismatch")
    );
    assert!(
        spawn_service_full(
            "enscrive-developer",
            script.to_str().unwrap(),
            &[],
            &envfile,
            &log,
            &[]
        )
        .is_err()
    );
}
