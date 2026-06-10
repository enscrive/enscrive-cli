//! ENS-651 — subprocess tests for the Revisions command surface.
//!
//! These run the real `enscrive` binary (CARGO_BIN_EXE) with an isolated
//! HOME (no profiles.toml) and a deliberately unreachable endpoint
//! (`http://127.0.0.1:9`). Every refusal asserted here exits with
//! EXIT_CONFIRMATION_REQUIRED (5) — NOT a network failure (1) — which
//! proves the destructive-command gate fires BEFORE any API call is
//! attempted.

use std::process::{Command, Stdio};

const UNREACHABLE_ENDPOINT: &str = "http://127.0.0.1:9";
const EXIT_CONFIRMATION_REQUIRED: i32 = 5;

fn enscrive(temp_home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_enscrive"))
        .args(args)
        // Isolate from the developer's real ~/.config/enscrive/profiles.toml
        // so profile_mode resolves to local and no managed-token path kicks in.
        .env("HOME", temp_home)
        .env("XDG_CONFIG_HOME", temp_home.join(".config"))
        .env("XDG_DATA_HOME", temp_home.join(".local/share"))
        .env("ENSCRIVE_API_KEY", "test-key")
        .env("ENSCRIVE_BASE_URL", UNREACHABLE_ENDPOINT)
        .env_remove("ENSCRIVE_PROFILE")
        // Piped stdin == non-TTY, exactly like a script or CI runner.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn enscrive binary")
}

#[test]
fn restore_refused_on_non_tty_stdin_before_any_api_call() {
    let home = tempfile::tempdir().expect("tempdir");
    let out = enscrive(home.path(), &["restore", "--revision", "rev-123", "--confirm"]);

    assert_eq!(
        out.status.code(),
        Some(EXIT_CONFIRMATION_REQUIRED),
        "non-TTY restore must exit with the confirmation-required code, not a \
         network error — stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("FAIL_CONFIRMATION_REQUIRED"),
        "stderr must carry the failure class: {stderr}"
    );
    assert!(
        stderr.contains("non-TTY"),
        "stderr must explain the non-TTY refusal: {stderr}"
    );
}

#[test]
fn restore_refused_without_confirm_flag() {
    let home = tempfile::tempdir().expect("tempdir");
    let out = enscrive(home.path(), &["restore", "--revision", "rev-123"]);

    assert_eq!(out.status.code(), Some(EXIT_CONFIRMATION_REQUIRED));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--confirm"),
        "refusal must point at --confirm: {stderr}"
    );
}

#[test]
fn restore_refused_in_json_agent_mode() {
    let home = tempfile::tempdir().expect("tempdir");
    let out = enscrive(
        home.path(),
        &[
            "--output", "json", "restore", "--revision", "rev-123", "--confirm",
        ],
    );

    assert_eq!(out.status.code(), Some(EXIT_CONFIRMATION_REQUIRED));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let envelope: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("JSON envelope on stdout");
    assert_eq!(envelope["ok"], serde_json::json!(false));
    assert_eq!(
        envelope["failure_class"],
        serde_json::json!("FAIL_CONFIRMATION_REQUIRED")
    );
}

// ── ADR §10.2 naming rule: grep the shipped help text for banned words ──────

#[test]
fn revisions_help_text_carries_no_banned_vocabulary() {
    let home = tempfile::tempdir().expect("tempdir");
    let banned = ["rewind", "fast-forward", "playback", "recall"];
    for args in [
        vec!["revisions", "--help"],
        vec!["revisions", "list", "--help"],
        vec!["revisions", "show", "--help"],
        vec!["restore", "--help"],
    ] {
        let out = enscrive(home.path(), &args);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
        .to_lowercase();
        for word in banned {
            assert!(
                !text.contains(word),
                "banned word {word:?} found in `enscrive {}` help output",
                args.join(" ")
            );
        }
    }
}
