//! ENS-651 — live round-trip tests for `enscrive revisions list/show` and
//! `enscrive restore --dry-run` against a running stack.
//!
//! Ignored by default; run explicitly with:
//!
//! ```sh
//! ENSCRIVE_API_KEY=… ENSCRIVE_BASE_URL=http://localhost:13000 \
//!     cargo test --test revisions_live -- --ignored --nocapture
//! ```
//!
//! Only read-only surfaces are exercised (`GET /v1/backups`,
//! `GET /v1/backups/{id}`, `POST /v1/restore/dry-run`) — a live restore is
//! never launched from this suite.

use std::process::{Command, Stdio};

fn live_env() -> Option<(String, String)> {
    let key = std::env::var("ENSCRIVE_API_KEY").ok()?;
    let base = std::env::var("ENSCRIVE_BASE_URL").ok()?;
    if key.trim().is_empty() || base.trim().is_empty() {
        return None;
    }
    Some((key, base))
}

fn enscrive_json(key: &str, base: &str, args: &[&str]) -> (Option<i32>, serde_json::Value) {
    let mut full_args = vec!["--output", "json"];
    full_args.extend_from_slice(args);
    let out = Command::new(env!("CARGO_BIN_EXE_enscrive"))
        .args(&full_args)
        .env("ENSCRIVE_API_KEY", key)
        .env("ENSCRIVE_BASE_URL", base)
        .env_remove("ENSCRIVE_PROFILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn enscrive binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "stdout was not a JSON envelope ({e}): stdout={stdout} stderr={}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code(), value)
}

#[test]
#[ignore = "requires live stack: ENSCRIVE_API_KEY + ENSCRIVE_BASE_URL"]
fn live_revisions_list_show_round_trip() {
    let Some((key, base)) = live_env() else {
        panic!("set ENSCRIVE_API_KEY and ENSCRIVE_BASE_URL to run this live test");
    };

    // List.
    let (code, envelope) = enscrive_json(&key, &base, &["revisions", "list", "--limit", "5"]);
    assert_eq!(code, Some(0), "revisions list failed: {envelope}");
    assert_eq!(envelope["ok"], serde_json::json!(true));
    let backups = envelope["data"]["backups"]
        .as_array()
        .expect("data.backups must be an array");
    eprintln!(
        "live: {} revision(s) listed (total={})",
        backups.len(),
        envelope["data"]["total"]
    );

    // Show (round-trip on the first listed revision, when one exists).
    let Some(first) = backups.first() else {
        eprintln!("live: no revisions exist for this tenant yet; show round-trip skipped");
        return;
    };
    let revision_id = first["backup_id"].as_str().expect("backup_id");
    let (code, envelope) = enscrive_json(&key, &base, &["revisions", "show", revision_id]);
    assert_eq!(code, Some(0), "revisions show failed: {envelope}");
    assert_eq!(envelope["ok"], serde_json::json!(true));
    assert_eq!(
        envelope["data"]["backup_id"],
        serde_json::json!(revision_id),
        "show must round-trip the listed revision id"
    );
    // Detail carries checksum evidence.
    assert!(
        envelope["data"]["collections"].is_object(),
        "revision detail must carry the collections/checksum map: {envelope}"
    );
    eprintln!("live: revisions show round-trip OK for {revision_id}");
}

#[test]
#[ignore = "requires live stack: ENSCRIVE_API_KEY + ENSCRIVE_BASE_URL"]
fn live_restore_dry_run_validates_without_executing() {
    let Some((key, base)) = live_env() else {
        panic!("set ENSCRIVE_API_KEY and ENSCRIVE_BASE_URL to run this live test");
    };

    let (code, envelope) = enscrive_json(&key, &base, &["revisions", "list", "--limit", "1"]);
    assert_eq!(code, Some(0), "revisions list failed: {envelope}");
    let Some(first) = envelope["data"]["backups"].as_array().and_then(|b| b.first()).cloned()
    else {
        eprintln!("live: no revisions exist for this tenant yet; dry-run skipped");
        return;
    };
    let revision_id = first["backup_id"].as_str().expect("backup_id").to_string();

    let (code, envelope) = enscrive_json(
        &key,
        &base,
        &["restore", "--revision", &revision_id, "--dry-run"],
    );
    assert_eq!(code, Some(0), "restore --dry-run failed: {envelope}");
    assert_eq!(envelope["ok"], serde_json::json!(true));
    assert_eq!(
        envelope["data"]["revision_id"],
        serde_json::json!(revision_id)
    );
    assert!(
        envelope["data"]["dry_run"]["can_restore"].is_boolean(),
        "dry-run must report can_restore: {envelope}"
    );
    eprintln!(
        "live: dry-run for {revision_id} → can_restore={}",
        envelope["data"]["dry_run"]["can_restore"]
    );
}
