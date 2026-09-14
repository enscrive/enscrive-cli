//! Independently operator-trusted Linux verifier; no automatic bootstrap.
use super::{Identity, Result, check_time, fail, identity};
use std::{
    env,
    ffi::OsString,
    fs,
    io::{ErrorKind, Read},
    os::fd::AsRawFd,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    fn access(path: *const std::ffi::c_char, mode: i32) -> i32;
}
pub(super) struct Verifier {
    path: PathBuf,
    before: Identity,
    home: OsString,
    path_env: OsString,
}
impl Verifier {
    pub(super) fn resolve() -> Result<Self> {
        let path_env = env::var_os("PATH").ok_or_else(|| fail("verifier PATH absent"))?;
        let home = env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| fail("verifier HOME absent"))?;
        let choice = match env::var_os("ENSCRIVE_COSIGN_BIN") {
            Some(v) => {
                let s = v
                    .to_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| fail("invalid verifier override"))?;
                PathBuf::from(s)
            }
            None => env::split_paths(&path_env)
                .map(|p| p.join("cosign"))
                .find(|p| {
                    fs::metadata(p)
                        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                })
                .ok_or_else(|| {
                    fail("cosign unavailable; install an independently trusted verifier")
                })?,
        };
        let path = fs::canonicalize(choice).map_err(|_| fail("verifier unavailable"))?;
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| fail("invalid verifier path"))?;
        if unsafe { access(c.as_ptr(), 1) } != 0 {
            return Err(fail("verifier not executable"));
        }
        let before = identity(
            &path,
            Instant::now() + Duration::from_secs(30),
            super::MAX_COMPONENT,
        )?;
        Ok(Self {
            path,
            before,
            home,
            path_env,
        })
    }
    pub(super) fn digest(&self) -> &str {
        &self.before.sha256
    }
    pub(super) fn verify(&self, manifest: &Path, bundle: &Path, tmp: &Path) -> Result<()> {
        self.verify_bounded(manifest, bundle, tmp, Duration::from_secs(60))
    }
    fn verify_bounded(
        &self,
        manifest: &Path,
        bundle: &Path,
        tmp: &Path,
        duration: Duration,
    ) -> Result<()> {
        self.verify_with_expectations(manifest, bundle, tmp, duration, super::IDENTITY, super::ISSUER)
    }
    #[cfg(test)]
    pub(super) fn verify_test_expectations(
        &self,
        manifest: &Path,
        bundle: &Path,
        tmp: &Path,
        identity: &str,
        issuer: &str,
    ) -> Result<()> {
        self.verify_with_expectations(manifest, bundle, tmp, Duration::from_secs(60), identity, issuer)
    }
    fn verify_with_expectations(
        &self,
        manifest: &Path,
        bundle: &Path,
        tmp: &Path,
        duration: Duration,
        expected_identity: &str,
        expected_issuer: &str,
    ) -> Result<()> {
        let deadline = Instant::now() + duration;
        if identity(
            &self.path,
            deadline.min(Instant::now() + Duration::from_secs(30)),
            super::MAX_COMPONENT,
        )? != self.before
        {
            return Err(fail("verifier changed"));
        }
        let mut cmd = Command::new(&self.path);
        cmd.args(["verify-blob", "--bundle"])
            .arg(bundle)
            .args([
                "--certificate-identity",
                expected_identity,
                "--certificate-oidc-issuer",
                expected_issuer,
            ])
            .arg(manifest)
            .env_clear()
            .env("PATH", &self.path_env)
            .env("HOME", &self.home)
            .env("TMPDIR", tmp)
            .env("LANG", "C.UTF-8")
            .current_dir(tmp)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = cmd.spawn().map_err(|_| fail("verifier spawn failed"))?;
        let work = deadline - std::cmp::min(Duration::from_secs(2), duration / 4);
        let mut reaped = false;
        let result = (|| {
            let mut out = child
                .stdout
                .take()
                .ok_or_else(|| fail("verifier pipe failed"))?;
            let mut err = child
                .stderr
                .take()
                .ok_or_else(|| fail("verifier pipe failed"))?;
            for fd in [out.as_raw_fd(), err.as_raw_fd()] {
                let f = unsafe { fcntl(fd, 3) };
                if f < 0 || unsafe { fcntl(fd, 4, f | 2048) } < 0 {
                    return Err(fail("verifier pipe failed"));
                }
            }
            let mut total = 0usize;
            let (mut oe, mut ee) = (false, false);
            loop {
                check_time(work)?;
                for (stream, eof) in [
                    (&mut out as &mut dyn Read, &mut oe),
                    (&mut err as &mut dyn Read, &mut ee),
                ] {
                    let mut b = [0; 4096];
                    loop {
                        match stream.read(&mut b) {
                            Ok(0) => {
                                *eof = true;
                                break;
                            }
                            Ok(n) => {
                                total += n;
                                if total > 65536 {
                                    return Err(fail("verifier output overflow"));
                                }
                                #[cfg(test)]
                                DIAGNOSTIC.with(|slot| {
                                    if let Some(bytes) = slot.borrow_mut().as_mut() {
                                        bytes.extend_from_slice(&b[..n]);
                                    }
                                });
                            }
                            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                            Err(_) => return Err(fail("verifier pipe failed")),
                        }
                        check_time(work)?;
                    }
                }
                if let Some(status) = child.try_wait().map_err(|_| fail("verifier wait failed"))? {
                    reaped = true;
                    if oe && ee {
                        check_time(work)?;
                        return if status.success() {
                            Ok(())
                        } else {
                            Err(fail("signature rejected"))
                        };
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        })();
        // Always signal the owned group, including a successful leader's descendants.
        unsafe { kill(-(child.id() as i32), 9) };
        while Instant::now() < deadline {
            if !reaped {
                reaped = child
                    .try_wait()
                    .map_err(|_| fail("verifier reap unavailable"))?
                    .is_some();
            }
            if reaped && !group_live(child.id())? {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if !reaped || group_live(child.id())? {
            return Err(fail("verifier cleanup unconfirmed"));
        }
        check_time(deadline)?;
        if identity(
            &self.path,
            deadline.min(Instant::now() + Duration::from_secs(30)),
            super::MAX_COMPONENT,
        )? != self.before
        {
            return Err(fail("verifier changed"));
        }
        result
    }
}
fn group_live(group: u32) -> Result<bool> {
    for e in fs::read_dir("/proc").map_err(|_| fail("process membership unavailable"))? {
        let e = e.map_err(|_| fail("process membership unavailable"))?;
        if e.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        let text = match fs::read_to_string(e.path().join("stat")) {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(_) => return Err(fail("process membership unavailable")),
        };
        let (_, tail) = text
            .rsplit_once(')')
            .ok_or_else(|| fail("process membership invalid"))?;
        let f: Vec<_> = tail.split_whitespace().collect();
        if f.len() < 3 {
            return Err(fail("process membership invalid"));
        }
        if f[2].parse::<u32>().ok() == Some(group) && !matches!(f[0], "Z" | "X") {
            return Ok(true);
        }
    }
    Ok(false)
}

// Test observation only: same bounded stream, no second invocation and no production output.
#[cfg(test)]
thread_local! {
    static DIAGNOSTIC: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(super) fn begin_diagnostic() {
    DIAGNOSTIC.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Vec::new());
    });
}
#[cfg(test)]
pub(super) fn take_diagnostic() -> Vec<u8> {
    DIAGNOSTIC.with(|slot| slot.borrow_mut().take().expect("diagnostic scope"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(script: &str) -> (tempfile::TempDir, Verifier, PathBuf, PathBuf) {
        let t = tempfile::TempDir::new().unwrap();
        let bin = t.path().join("verifier");
        fs::write(
            &bin,
            format!("#!/bin/sh\nprintf '%s\\n' \"$$\" > \"$TMPDIR/group.pid\"\n{script}\n"),
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o700)).unwrap();
        let before = identity(
            &bin,
            Instant::now() + Duration::from_secs(2),
            super::super::MAX_COMPONENT,
        )
        .unwrap();
        let v = Verifier {
            path: bin,
            before,
            home: t.path().as_os_str().into(),
            path_env: "/usr/bin:/bin".into(),
        };
        let m = t.path().join("manifest");
        let b = t.path().join("bundle");
        fs::write(&m, b"synthetic").unwrap();
        fs::write(&b, b"synthetic").unwrap();
        (t, v, m, b)
    }
    #[test]
    fn ens5913_verifier_exact_arguments_and_clean_environment() {
        let _lock = crate::test_support::lock_env();
        let sentinel = std::ffi::OsStr::new("synthetic-must-not-reach-verifier");
        let _env = super::super::tests::EnvRestore::set(&[
            ("OPENAI_API_KEY", Some(sentinel)),
            ("ESM_MASTER_KEY", Some(sentinel)),
            ("ENSCRIVE_COSIGN_BIN", Some(sentinel)),
        ]);
        let script = concat!(
            "test \"$1\" = verify-blob || exit 1\n",
            "test \"$2\" = --bundle || exit 1\n",
            "test \"$4\" = --certificate-identity || exit 1\n",
            "test \"$5\" = https://github.com/enscrive/enscrive-cli/.github/workflows/manifest.yml@refs/heads/main || exit 1\n",
            "test \"$6\" = --certificate-oidc-issuer || exit 1\n",
            "test \"$7\" = https://token.actions.githubusercontent.com || exit 1\n",
            "test $# -eq 8 || exit 1\n",
            "test -z \"${OPENAI_API_KEY+x}${ESM_MASTER_KEY+x}${ENSCRIVE_COSIGN_BIN+x}\" || exit 1\n"
        );
        let (t, v, m, b) = fixture(script);
        assert!(
            v.verify_bounded(&m, &b, t.path(), Duration::from_secs(2))
                .is_ok()
        );
        settled(t.path());
    }
    fn settled(t: &Path) {
        let group: u32 = fs::read_to_string(t.join("group.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            !group_live(group).unwrap(),
            "owned group retains live processes"
        );
        assert!(
            !Path::new("/proc").join(group.to_string()).exists(),
            "direct child was not reaped"
        );
    }
    #[test]
    fn ens5913_verifier_nonzero_overflow_deadline_and_mutation() {
        for (script, expected) in [
            ("exit 1", "signature rejected"),
            (
                "while :; do printf '0123456789012345678901234567890123456789012345678901234567890123456789'; done",
                "verifier output overflow",
            ),
            ("while :; do :; done", "deadline exceeded"),
        ] {
            let (t, v, m, b) = fixture(script);
            let started = Instant::now();
            assert_eq!(
                v.verify_bounded(&m, &b, t.path(), Duration::from_millis(800))
                    .unwrap_err(),
                fail(expected)
            );
            assert!(started.elapsed() < Duration::from_secs(2));
            settled(t.path());
        }
        let (t, v, m, b) = fixture("exit 0");
        fs::write(&v.path, b"changed").unwrap();
        assert_eq!(
            v.verify_bounded(&m, &b, t.path(), Duration::from_secs(2))
                .unwrap_err(),
            fail("verifier changed")
        );
        assert!(!t.path().join("group.pid").exists());
    }
    #[test]
    fn ens5913_successful_leader_live_descendant_is_settled() {
        let (t, v, m, b) = fixture(
            "(while :; do :; done) >/dev/null 2>&1 &\nprintf '%s\\n' \"$!\" > \"$TMPDIR/descendant.pid\"\nexit 0",
        );
        let started = Instant::now();
        assert_eq!(
            v.verify_bounded(&m, &b, t.path(), Duration::from_secs(2)),
            Ok(())
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        settled(t.path());
        let pid: u32 = fs::read_to_string(t.path().join("descendant.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
            let state = stat
                .rsplit_once(')')
                .unwrap()
                .1
                .split_whitespace()
                .next()
                .unwrap();
            assert!(matches!(state, "Z" | "X"), "descendant is still executing");
        }
    }
}

