use super::*;
use std::ffi::OsStr;
use tempfile::TempDir;
fn maps() -> [BTreeMap<String, String>; 3] {
    let dev = BTreeMap::from([(NAMES[1].into(), "01".repeat(32))]);
    let emb = BTreeMap::from([(NAMES[2].into(), "02".repeat(32))]);
    let mut obs = dev.clone();
    obs.extend(emb.clone());
    obs.insert(NAMES[3].into(), "03".repeat(32));
    [dev, obs, emb]
}
fn checked(temp: &TempDir, values: [BTreeMap<String, String>; 3]) -> Result<Prepared, String> {
    let binary = fixtures::prepare(temp.path());
    let [d, o, e] = values;
    Prepared::validate(Executable::at(&binary)?, d, o, e, Utc::now())
}
#[test]
fn ens5903_snapshot_pairs_rotation_and_redaction() {
    let t = TempDir::new().unwrap();
    let p = checked(&t, maps()).unwrap();
    assert_eq!(format!("{p:?}"), "Prepared([REDACTED])");
    assert!(p.developer().require_service("enscrive-observe").is_err());
    for i in 0..3 {
        let mut values = maps();
        values[i].clear();
        assert!(checked(&t, values).is_err());
    }
    let mut values = maps();
    values[0].insert(NAMES[1].into(), "04".repeat(32));
    assert!(checked(&t, values).is_err());
    for bad in [
        "",
        "A",
        "private-synthetic-marker",
        &"FF".repeat(32),
        &("01".repeat(32) + "\n"),
    ] {
        let mut values = maps();
        values[1].insert(NAMES[3].into(), bad.into());
        let err = checked(&t, values).unwrap_err();
        assert!(!err.contains("private-synthetic-marker"));
    }
    let mut values = maps();
    values[1].insert(NAMES[3].into(), "01".repeat(32));
    assert!(checked(&t, values).is_err());
    let mut values = maps();
    values[1].insert(NAMES[4].into(), "04".repeat(32));
    assert!(checked(&t, values.clone()).is_err());
    values[1].insert(
        NAMES[7].into(),
        (Utc::now() - ChronoDuration::hours(1)).to_rfc3339(),
    );
    assert!(checked(&t, values.clone()).is_ok());
    values[1].insert(
        NAMES[7].into(),
        (Utc::now() + ChronoDuration::hours(25)).to_rfc3339(),
    );
    assert!(checked(&t, values).is_err());
    for name in NAMES.iter().skip(2) {
        let mut values = maps();
        values[0].insert((*name).into(), String::new());
        assert!(checked(&t, values).is_err());
    }
}
#[test]
fn ens5903_list_raw_shape_and_duplicate_derived() {
    assert_eq!(
        parse_list(
            br#"[{"name":"unrelated"},{"name":"OBSERVE_GRPC_DEVELOPER_KEY","derived":false}]"#
        )
        .unwrap(),
        vec![NAMES[1]]
    );
    for text in [
        r#"{}"#,
        r#"[{"name":"OBSERVE_GRPC_DEVELOPER_KEY"}]"#,
        r#"[{"name":"OBSERVE_GRPC_DEVELOPER_KEY","derived":true}]"#,
        r#"[{"name":"OBSERVE_GRPC_DEVELOPER_KEY","derived":false},{"name":"OBSERVE_GRPC_DEVELOPER_KEY","derived":false}]"#,
    ] {
        assert!(parse_list(text.as_bytes()).is_err());
    }
}
#[test]
fn ens5903_paths_pid_and_executable_refuse_without_mutation() {
    use std::os::unix::ffi::OsStringExt;
    let t = TempDir::new().unwrap();
    assert!(xdg(Some(OsString::from_vec(vec![255])), t.path().to_owned()).is_err());
    assert!(xdg(Some("".into()), t.path().to_owned()).is_err());
    assert!(Executable::at(Path::new("relative-esm")).is_err());
    let p = t.path().join("esm");
    fs::write(&p, "fixture").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(Executable::at(&p).is_err());
    let pid = t.path().join("enscrive-developer.pid");
    for bad in ["0", "-1", "2147483648", "private-marker"] {
        fs::write(&pid, bad).unwrap();
        assert!(stopped(t.path()).is_err());
        assert_eq!(fs::read_to_string(&pid).unwrap(), bad);
    }
    fs::write(&pid, std::process::id().to_string()).unwrap();
    assert!(stopped(t.path()).unwrap_err().contains("stop"));
    fs::set_permissions(&pid, fs::Permissions::from_mode(0o000)).unwrap();
    assert_ne!(
        process::uid(),
        0,
        "permission witness requires unprivileged runner"
    );
    assert!(stopped(t.path()).unwrap_err().contains("unreadable"));
    fs::set_permissions(&pid, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_file(&pid).unwrap();
    std::os::unix::fs::symlink(t.path().join("missing"), &pid).unwrap();
    assert!(stopped(t.path()).is_err());
    fs::remove_file(&pid).unwrap();
    assert!(stopped(t.path()).is_ok());
}
#[test]
fn ens5903_protected_env_comments_and_typed_namespace() {
    let t = TempDir::new().unwrap();
    let prepared = checked(&t, maps()).unwrap();
    let p = t.path().join("config/developer.env");
    let mut doc=EnvDocument::parse("# preserve exact comment\nUNRELATED=007\nOBSERVE_GRPC_EMBED_KEY=stale\nOBSERVE_GRPC_DEVELOPER_PREVIOUS_KEY=stale\n").unwrap();
    prepared.developer().refresh(&mut doc);
    doc.write(&p).unwrap();
    let text = fs::read_to_string(&p).unwrap();
    assert!(text.starts_with("# preserve exact comment\nUNRELATED=007\n"));
    assert!(!text.contains("stale"));
    assert_eq!(fs::metadata(&p).unwrap().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(p.parent().unwrap()).unwrap().mode() & 0o777,
        0o700
    );
    for bad in [
        "SYNTHETIC_PRIVATE_LINE",
        "OBSERVE_GRPC_DEVELOPER_KEY=a\nOBSERVE_GRPC_DEVELOPER_KEY=b",
    ] {
        let err = EnvDocument::parse(bad).err().unwrap();
        assert!(!err.contains("SYNTHETIC_PRIVATE_LINE"));
    }
    let mut cmd = Command::new("/unused");
    for k in NAMES {
        cmd.env(k, "inherited/file/extra-collision");
    }
    cmd.env("UNRELATED", "preserved");
    prepared.developer().apply(&mut cmd);
    let final_env = cmd.get_envs().collect::<BTreeMap<_, _>>();
    assert_eq!(
        final_env.get(OsStr::new(NAMES[1])).unwrap().unwrap(),
        "01".repeat(32).as_str()
    );
    for k in &NAMES[2..] {
        assert_eq!(final_env.get(OsStr::new(k)), Some(&None));
    }
    assert_eq!(
        final_env.get(OsStr::new("UNRELATED")).unwrap().unwrap(),
        "preserved"
    );
    fs::remove_file(&p).unwrap();
    std::os::unix::fs::symlink(t.path().join("victim"), &p).unwrap();
    assert!(doc.write(&p).is_err());
    assert!(!t.path().join("victim").exists());
}
#[test]
fn ens5903_real_gate_protocol_fake_and_immutable_inputs() {
    let t = TempDir::new().unwrap();
    let binary = fixtures::prepare(t.path());
    let paths = SERVICES.map(|s| t.path().join("secrets").join(s).join(".esm/secrets.esm"));
    let before = paths
        .iter()
        .map(|p| fs::read(p).unwrap())
        .collect::<Vec<_>>();
    let prepared = Prepared::load(Executable::at(&binary).unwrap(), t.path(), Utc::now()).unwrap();
    assert_eq!(prepared.observe().values.len(), 4);
    for (p, bytes) in paths.iter().zip(before) {
        assert_eq!(fs::read(p).unwrap(), bytes);
    }
    let source = fs::read_to_string(&binary).unwrap();
    let changed = source.replace(
        " print('open\\t'+a[4])",
        " pathlib.Path(a[4]).write_bytes(b'changed-during-inventory')\n print('open\\t'+a[4])",
    );
    assert_ne!(source, changed);
    fs::write(&binary, changed).unwrap();
    let err = Prepared::load(Executable::at(&binary).unwrap(), t.path(), Utc::now()).unwrap_err();
    assert!(err.contains("input changed"));
    fs::write(&binary, "changed").unwrap();
    assert!(prepared.executable().verify().is_err());
}
#[test]
fn ens5903_bounded_capture_overflow_deadline_and_held_pipe() {
    let t = TempDir::new().unwrap();
    let binary = t.path().join("probe");
    for (code, limit, deadline, kind) in [
        (
            "import sys;sys.stderr.write('x'*10000)",
            128,
            Duration::from_secs(2),
            "overflow",
        ),
        ("print('x'*10000)", 128, Duration::from_secs(2), "overflow"),
        (
            "import time;time.sleep(10)",
            128,
            Duration::from_millis(800),
            "deadline",
        ),
        (
            "import os,time;child=os.fork();time.sleep(10) if child==0 else None",
            128,
            Duration::from_millis(800),
            "deadline",
        ),
    ] {
        fs::write(
            &binary,
            format!("#!/usr/bin/python3\nimport os,pathlib\npathlib.Path('child.pid').write_text(str(os.getpid()))\n{code}\n"),
        ).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let started = Instant::now();
        let err =
            process::capture(&binary, t.path(), &[], limit, Instant::now() + deadline).unwrap_err();
        assert!(err.contains(kind));
        assert!(started.elapsed() < deadline + Duration::from_millis(250));
        let pid = fs::read_to_string(t.path().join("child.pid"))
            .unwrap()
            .parse::<i32>()
            .unwrap();
        unsafe extern "C" {
            fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
        }
        let mut status = 0;
        // ECHILD proves capture already reaped its child; this assertion does not
        // turn a missing cleanup into a successful test by waiting/reaping it.
        assert_eq!(unsafe { waitpid(pid, &mut status, 1) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(10));
        fs::remove_file(t.path().join("child.pid")).unwrap();
    }
}
