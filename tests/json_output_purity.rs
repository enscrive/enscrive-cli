//! `--output json` stdout purity (ENS-3054 W4 §4.1).
//!
//! Anything consuming this CLI programmatically — an agent following
//! `.enscrive/AGENT.md`, a CI step, a `jq` pipeline — parses **stdout**. A
//! single stray human-readable line makes the whole invocation unparseable,
//! and the failure surfaces far from the `println!` that caused it.
//!
//! The rule this file locks in: **in `--output json` mode, stdout is
//! exactly one JSON document and nothing else.** Human-facing prose,
//! progress ticks, and warnings belong on stderr.
//!
//! Purity held across these commands when the test was written; this is a
//! regression lock, not a bug report. It runs the real binary
//! (`CARGO_BIN_EXE_enscrive`) against a mock `/v1` rather than asserting on
//! source, because the thing that matters is the bytes a caller receives.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

/// Mock `/v1` that answers every request with a generic JSON body. The
/// shapes are deliberately uninteresting: this test asserts on stdout
/// framing, not on any endpoint's contract.
fn spawn_mock() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let body = br#"{"ok":true,"items":[],"results":[],"data":{},"corpora":[]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.flush();
        }
    });

    port
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_enscrive"))
}

/// Every command that can complete against the mock without a running
/// stack. Each is a full argv tail appended after the shared global flags.
fn json_commands() -> Vec<Vec<&'static str>> {
    vec![
        vec!["status"],
        vec!["health"],
        vec!["search", "--query", "anything"],
        vec!["corpus", "list"],
        vec![
            "corpus",
            "ensure",
            "--name",
            "purity-probe",
            "--embedding-model",
            "text-embedding-3-large",
        ],
        vec!["corpus", "stats", "--id", "c-1"],
        vec!["corpus", "documents", "--id", "c-1"],
        vec!["wallet", "balance"],
        vec!["jobs", "list"],
        vec!["voices", "list"],
        vec!["agents", "list"],
        vec!["ratecard", "show"],
        vec!["license", "status"],
        vec!["models", "list"],
    ]
}

#[test]
fn json_mode_stdout_is_exactly_one_json_document() {
    let port = spawn_mock();
    let endpoint = format!("http://127.0.0.1:{port}");

    // Isolated config root: `status` needs a profile to resolve, and no
    // test may read or write the developer's real ~/.config/enscrive.
    let config_home = tempfile::tempdir().expect("tempdir");
    let profile_dir = config_home.path().join("enscrive");
    std::fs::create_dir_all(&profile_dir).expect("create profile dir");
    std::fs::write(
        profile_dir.join("profiles.toml"),
        format!(
            "version = 1\ndefault_profile = \"probe\"\n\n\
             [profiles.probe]\nmode = \"managed\"\nendpoint = \"{endpoint}\"\napi_key = \"probe-key\"\n"
        ),
    )
    .expect("write profiles.toml");

    let mut impure: Vec<String> = Vec::new();

    for tail in json_commands() {
        let output = Command::new(binary())
            .args(["--output", "json", "--endpoint", &endpoint])
            .args(&tail)
            // A project marker anywhere above the test's cwd must not
            // redirect these probes; pin the profile explicitly.
            .args(["--profile", "probe"])
            .env("XDG_CONFIG_HOME", config_home.path())
            .env_remove("ENSCRIVE_API_KEY")
            .env_remove("ENSCRIVE_PROFILE")
            .env_remove("ENSCRIVE_BASE_URL")
            .output()
            .unwrap_or_else(|e| panic!("run `enscrive {}`: {e}", tail.join(" ")));

        let stdout = String::from_utf8_lossy(&output.stdout);
        let label = tail.join(" ");

        // An empty stdout is fine only when clap rejected the arguments
        // before dispatch — that is a usage error, correctly on stderr.
        if stdout.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<serde_json::Value>(&stdout) {
            Ok(value) => {
                // One document, not a stream: the envelope is always an
                // object carrying `ok`.
                if !value.is_object() || value.get("ok").is_none() {
                    impure.push(format!(
                        "`enscrive {label}` stdout parsed but is not a CliResponse envelope:\n{stdout}"
                    ));
                }
            }
            Err(e) => impure.push(format!(
                "`enscrive {label}` stdout is not valid JSON ({e}). \
                 Something printed to stdout that should have gone to stderr:\n{stdout}"
            )),
        }
    }

    assert!(
        impure.is_empty(),
        "--output json must emit exactly one JSON document on stdout:\n\n{}",
        impure.join("\n\n")
    );
}

/// The complement: a failure must also be machine-readable. An agent that
/// cannot parse the error path has to fall back to scraping human text.
#[test]
fn json_mode_failures_are_machine_readable_on_stdout() {
    let config_home = tempfile::tempdir().expect("tempdir");

    // Port 1 is reserved and never listening, so this is a transport
    // failure — the noisiest error path there is.
    let output = Command::new(binary())
        .args([
            "--output",
            "json",
            "--endpoint",
            "http://127.0.0.1:1",
            "--api-key",
            "probe-key",
            "search",
            "--query",
            "anything",
        ])
        .env("XDG_CONFIG_HOME", config_home.path())
        .env_remove("ENSCRIVE_API_KEY")
        .env_remove("ENSCRIVE_PROFILE")
        .output()
        .expect("run enscrive search");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("a failing command must still emit parseable JSON on stdout ({e}):\n{stdout}")
    });

    assert_eq!(parsed["ok"], false, "failure envelope must carry ok=false");
    assert!(
        parsed.get("error").and_then(|v| v.as_str()).is_some(),
        "failure envelope must carry an `error` string: {stdout}"
    );
    assert!(
        parsed.get("failure_class").is_some(),
        "failure envelope must carry a `failure_class`: {stdout}"
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "a failing command must exit non-zero"
    );
}
