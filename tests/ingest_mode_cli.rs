//! ENS-5833: real CLI requests against a listening, bounded loopback fixture.
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn invoke(input: &[&str]) -> (std::process::Output, Vec<Value>) {
    let home = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let stdout = tempfile::tempfile().unwrap();
    let stderr = tempfile::tempfile().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_enscrive"));
    if !input.contains(&"--help") {
        command.args(["--output", "json"]);
    }
    let child = command
        .args(["ingest", "documents", "--corpus-id", "corpus-fixture"])
        .args(input)
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("XDG_DATA_HOME", home.path().join(".local/share"))
        .env("ENSCRIVE_API_KEY", "fixture-only-key")
        .env("ENSCRIVE_BASE_URL", endpoint)
        .current_dir(home.path())
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().unwrap())
        .stderr(stderr.try_clone().unwrap())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut requests = Vec::new();
    let status = loop {
        match listener.accept() {
            Ok((mut socket, _)) => {
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut raw = Vec::new();
                let (headers, body_start, length) = loop {
                    assert!(Instant::now() < deadline, "HTTP header deadline exceeded");
                    assert!(raw.len() < 65536, "fixture request too large");
                    let mut buf = [0u8; 4096];
                    let count = socket.read(&mut buf).unwrap();
                    assert!(count > 0, "incomplete HTTP request");
                    raw.extend_from_slice(&buf[..count]);
                    if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8(raw[..end].to_vec()).unwrap();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|s| s.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        assert!(length < 65536);
                        break (headers, end + 4, length);
                    }
                };
                assert!(
                    headers.starts_with("POST /v1/ingest HTTP/1.1\r\n"),
                    "{headers}"
                );
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("x-api-key: fixture-only-key")
                );
                while raw.len() < body_start + length {
                    assert!(Instant::now() < deadline, "HTTP body deadline exceeded");
                    let mut buf = [0u8; 4096];
                    let n = socket.read(&mut buf).unwrap();
                    assert!(n > 0);
                    raw.extend_from_slice(&buf[..n]);
                }
                requests
                    .push(serde_json::from_slice(&raw[body_start..body_start + length]).unwrap());
                let response = r#"{"status":"completed","fixture":true}"#;
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("listener failed: {e}"),
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            panic!("CLI exceeded 15s deadline");
        }
        thread::sleep(Duration::from_millis(5));
    };
    // Child is gone; drain any connection queued before its exit (invalid flags
    // must not even open a socket, not merely fail at an unreachable endpoint).
    assert!(
        listener.accept().is_err(),
        "unexpected additional HTTP connection"
    );
    use std::io::{Seek, SeekFrom};
    let mut stdout = stdout;
    let mut stderr = stderr;
    stdout.seek(SeekFrom::Start(0)).unwrap();
    stderr.seek(SeekFrom::Start(0)).unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    stdout.read_to_end(&mut out).unwrap();
    stderr.read_to_end(&mut err).unwrap();
    (
        std::process::Output {
            status,
            stdout: out,
            stderr: err,
        },
        requests,
    )
}

#[test]
fn document_modes_preserve_request_fields_and_omission() {
    let files = tempfile::tempdir().unwrap();
    let text = files.path().join("document.txt");
    fs::write(&text, "file café\nsecond line").unwrap();
    let multi = json!([{"id":"one","content":"first","metadata":{"source":"fixture"},"fingerprint":"given"},{"id":"two","content":"second","metadata":{"n":2},"fingerprint":"other"}]);
    let documents = files.path().join("documents.json");
    fs::write(&documents, multi.to_string()).unwrap();
    for mode in [None, Some("append"), Some("replace"), Some("upsert")] {
        for source in ["inline", "file", "multi-inline", "multi-file"] {
            let multi_text = multi.to_string();
            let mut args = vec![
                "--voice-id",
                "voice-fixture",
                "--dry-run",
                "--no-batch",
                "--sync",
            ];
            let expected_documents = match source {
                "inline" => {
                    args.extend([
                        "--document-id",
                        "stable-id",
                        "--content",
                        "inline café\ntext",
                    ]);
                    json!([{"id":"stable-id","content":"inline café\ntext","metadata":{},"fingerprint":""}])
                }
                "file" => {
                    args.extend([
                        "--document-id",
                        "stable-id",
                        "--content-file",
                        text.to_str().unwrap(),
                    ]);
                    json!([{"id":"stable-id","content":"file café\nsecond line","metadata":{},"fingerprint":""}])
                }
                "multi-inline" => {
                    args.extend(["--documents-json", &multi_text]);
                    multi.clone()
                }
                "multi-file" => {
                    args.extend(["--documents-file", documents.to_str().unwrap()]);
                    multi.clone()
                }
                _ => unreachable!(),
            };
            if let Some(mode) = mode {
                args.extend(["--mode", mode]);
            }
            let (out, requests) = invoke(&args);
            assert!(
                out.status.success(),
                "{mode:?}/{source}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(requests.len(), 1);
            let mut expected = json!({"corpus_id":"corpus-fixture","documents":expected_documents,"voice_id":"voice-fixture","dry_run":true,"sync":true,"no_batch":true});
            if let Some(mode) = mode {
                expected["mode"] = json!(if mode == "upsert" { "replace" } else { mode });
            }
            assert_eq!(requests[0], expected, "{mode:?}/{source}");
        }
    }
}

#[test]
fn invalid_document_mode_fails_before_any_http() {
    for mode in ["invalid", "", "Replace"] {
        let (out, requests) = invoke(&["--content", "fixture", "--mode", mode]);
        assert_eq!(out.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&out.stderr).contains("--mode"));
        assert!(requests.is_empty());
    }
}

#[test]
fn document_mode_help_explains_replacement() {
    let (out, requests) = invoke(&["--help"]);
    assert!(out.status.success());
    assert!(requests.is_empty());
    let help = String::from_utf8(out.stdout).unwrap();
    for expected in [
        "--mode",
        "append",
        "replace",
        "upsert",
        "trailing chunks",
        "document ID alone",
    ] {
        assert!(help.contains(expected), "missing {expected}: {help}");
    }
}
