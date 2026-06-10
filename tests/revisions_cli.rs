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
const EXIT_FAILURE: i32 = 1;
const EXIT_CONFIG: i32 = 3;
const EXIT_CONFIRMATION_REQUIRED: i32 = 5;
/// A syntactically valid revision id (revision ids are UUIDs); the gate
/// refusals under test fire before any lookup, so it never needs to exist.
const REVISION_ID: &str = "1b4e28ba-2fa1-11d2-883f-0016d3cca427";

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
    let out = enscrive(home.path(), &["restore", "--revision", REVISION_ID, "--confirm"]);

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
    let out = enscrive(home.path(), &["restore", "--revision", REVISION_ID]);

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
            "--output", "json", "restore", "--revision", REVISION_ID, "--confirm",
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

// ── ENS-651 review finding 3: path-traversal revision ids rejected ──────────

#[test]
fn restore_rejects_non_uuid_revision_id_before_any_request() {
    let home = tempfile::tempdir().expect("tempdir");
    for bad in ["../admin", "../../v1/admin/restore", "rev-123"] {
        let out = enscrive(home.path(), &["restore", "--revision", bad, "--confirm"]);
        // EXIT_CONFIG (usage error), not a network error — against the
        // unreachable endpoint that proves no request was constructed.
        assert_eq!(
            out.status.code(),
            Some(EXIT_CONFIG),
            "{bad:?} must be rejected as invalid — stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("invalid revision id") && stderr.contains("UUID"),
            "rejection must be clear about the expected format: {stderr}"
        );
    }
}

#[test]
fn revisions_show_rejects_non_uuid_revision_id() {
    let home = tempfile::tempdir().expect("tempdir");
    let out = enscrive(home.path(), &["revisions", "show", "../admin"]);
    assert_eq!(out.status.code(), Some(EXIT_CONFIG));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid revision id"), "stderr: {stderr}");
}

// ── ENS-651 review finding 4: managed-mode token satisfies the gate ─────────

#[test]
fn restore_with_managed_token_passes_gate_on_non_tty() {
    // A managed-mode profile whose default profile drives profile_mode.
    let home = tempfile::tempdir().expect("tempdir");
    let cfg_dir = home.path().join(".config").join("enscrive");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir config");
    std::fs::write(
        cfg_dir.join("profiles.toml"),
        format!(
            "version = 1\ndefault_profile = \"managed\"\n\n\
             [profiles.managed]\nmode = \"managed\"\n\
             endpoint = \"{UNREACHABLE_ENDPOINT}\"\napi_key = \"test-key\"\n"
        ),
    )
    .expect("write profiles.toml");

    // Non-TTY stdin + no --confirm: with a token the gate must be satisfied
    // and execution must proceed past it. The next step is the revision
    // lookup against the unreachable endpoint — so the proof of finding 4
    // is exit code 1 (network failure AFTER the gate), never 5 (refusal).
    let out = enscrive(
        home.path(),
        &[
            "restore",
            "--revision",
            REVISION_ID,
            "--confirm-token",
            "ecf_live_token",
        ],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(EXIT_FAILURE),
        "token-satisfied non-TTY restore must pass the gate and fail only at \
         the (unreachable) network layer — stderr: {stderr}"
    );
    assert!(
        !stderr.contains("FAIL_CONFIRMATION_REQUIRED"),
        "token-carrying automation must not be refused by the interactive \
         gate: {stderr}"
    );
    assert!(
        stderr.contains("request failed"),
        "failure must be the network call after the gate: {stderr}"
    );

    // Control: same managed profile WITHOUT a token is still refused.
    let out = enscrive(home.path(), &["restore", "--revision", REVISION_ID]);
    assert_eq!(out.status.code(), Some(EXIT_CONFIRMATION_REQUIRED));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("confirmation token in managed mode"),
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
