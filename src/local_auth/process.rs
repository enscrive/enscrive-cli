//! Bounded read-only child protocol; pipes are nonblocking, including after exit.
use super::{error, path_text};
#[cfg(unix)]
use std::os::{
    fd::AsRawFd,
    unix::{fs::MetadataExt, process::CommandExt},
};
use std::{
    fs,
    io::{ErrorKind, Read},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[cfg(unix)]
unsafe extern "C" {
    fn geteuid() -> u32;
    fn access(path: *const std::ffi::c_char, mode: i32) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
}
#[cfg(unix)]
pub(super) fn uid() -> u32 {
    unsafe { geteuid() }
}

pub(super) fn stopped(p: &Path) -> Result<(), String> {
    path_text(p)?;
    let m = match fs::symlink_metadata(p) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(error("PID record unavailable")),
        Ok(m) => m,
    };
    #[cfg(unix)]
    if !m.is_file() || m.uid() != uid() || m.len() > 32 {
        return Err(error("invalid owned PID record"));
    }
    #[cfg(not(unix))]
    return Err(error("process gate unsupported on this platform"));
    let mut file = fs::File::open(p).map_err(|_| error("PID record unreadable"))?;
    #[cfg(unix)]
    {
        let opened = file
            .metadata()
            .map_err(|_| error("PID metadata unavailable"))?;
        if opened.dev() != m.dev() || opened.ino() != m.ino() {
            return Err(error("PID record changed"));
        }
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(33)
        .read_to_end(&mut bytes)
        .map_err(|_| error("PID record unreadable"))?;
    if bytes.len() > 32 {
        return Err(error("invalid PID record"));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| error("invalid PID record"))?;
    let pid = text
        .trim()
        .parse::<i32>()
        .map_err(|_| error("invalid positive PID"))?;
    if pid <= 0 {
        return Err(error("invalid positive PID"));
    }
    #[cfg(unix)]
    {
        let r = unsafe { kill(pid, 0) };
        if r == 0 {
            return Err(error(
                "application running: stop affected services and retry",
            ));
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(3) {
            return Err(error("process liveness unavailable"));
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn capture(
    binary: &Path,
    cwd: &Path,
    args: &[&str],
    limit: usize,
    inventory_deadline: Instant,
) -> Result<Vec<u8>, String> {
    let deadline = inventory_deadline.min(Instant::now() + Duration::from_secs(20));
    if Instant::now() >= deadline {
        return Err(error("inventory deadline exceeded"));
    }
    // Cleanup is part of the same child/inventory budget, not extra time.
    let remaining = deadline.saturating_duration_since(Instant::now());
    let cleanup_budget = Duration::from_secs(2).min(remaining / 2);
    let work_deadline = deadline - cleanup_budget;
    let mut cmd = Command::new(binary);
    // No inherited key, keyring, namespace or vault override. Absolute binary and
    // explicit paths make PATH/HOME unnecessary to the fixed read-only commands.
    cmd.env_clear()
        .env("LANG", "C.UTF-8")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = cmd.spawn().map_err(|_| error("ESM spawn unavailable"))?;
    let mut reaped = false;
    let result = (|| {
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| error("ESM stdout unavailable"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| error("ESM stderr unavailable"))?;
        #[cfg(target_os = "linux")]
        const NONBLOCK: i32 = 2048;
        #[cfg(not(target_os = "linux"))]
        const NONBLOCK: i32 = 4;
        for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
            let flags = unsafe { fcntl(fd, 3) };
            if flags < 0 || unsafe { fcntl(fd, 4, flags | NONBLOCK) } < 0 {
                return Err(error("ESM pipe unavailable"));
            }
        }
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut out_end = false;
        let mut err_end = false;
        loop {
            if Instant::now() >= work_deadline {
                return Err(error("ESM deadline exceeded"));
            }
            drain(&mut stdout, &mut out, &mut out_end, limit)?;
            drain(&mut stderr, &mut err, &mut err_end, 8192)?;
            if let Some(status) = child
                .try_wait()
                .map_err(|_| error("ESM wait unavailable"))?
            {
                reaped = true;
                if out_end && err_end {
                    return if status.success() {
                        Ok(out)
                    } else {
                        Err(error("ESM command unavailable"))
                    };
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    })();
    // Signal the owned group, including descendants retaining pipes. A signal
    // request is not evidence of reaping: poll only within the reserved budget.
    // No blocking wait or detached waiter may extend this operation's deadline.
    if result.is_err() {
        unsafe { kill(-(child.id() as i32), 9) };
        if !reaped {
            let _ = child.kill();
        }
    }
    while !reaped && Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => reaped = true,
            Ok(None) => std::thread::sleep(
                Duration::from_millis(2).min(deadline.saturating_duration_since(Instant::now())),
            ),
            Err(_) => return Err(error("ESM cleanup reap unavailable")),
        }
    }
    if !reaped {
        return Err(error("ESM cleanup deadline exceeded: reap unconfirmed"));
    }
    if result.is_ok() && Instant::now() >= deadline {
        return Err(error("ESM inventory deadline exceeded"));
    }
    result
}
#[cfg(not(unix))]
pub(super) fn capture(
    _: &Path,
    _: &Path,
    _: &[&str],
    _: usize,
    _: Instant,
) -> Result<Vec<u8>, String> {
    Err(error("bounded ESM transport unsupported on this platform"))
}
fn drain(
    r: &mut impl Read,
    bytes: &mut Vec<u8>,
    end: &mut bool,
    limit: usize,
) -> Result<(), String> {
    if *end {
        return Ok(());
    }
    // One bounded read per stream per turn; a noisy stdout cannot starve stderr
    // or the total deadline. The extra byte distinguishes exact cap from overflow.
    let mut buf = [0u8; 8192];
    let remaining = (limit + 1 - bytes.len()).min(buf.len());
    match r.read(&mut buf[..remaining]) {
        Ok(0) => *end = true,
        Ok(n) => {
            bytes.extend_from_slice(&buf[..n]);
            if bytes.len() > limit {
                return Err(error("ESM output overflow"));
            }
        }
        Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::Interrupted => {}
        Err(_) => return Err(error("ESM pipe unavailable")),
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn executable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { access(path.as_ptr(), 1) == 0 }
}
