//! ENS-651 — the Revisions capability (TENANT-DATA-BACKUP-STRATEGY ADR).
//!
//! Tenant-scoped surface over the ENS-649 endpoints (enscrive-developer
//! PR #68), under normal API-key auth (no Admin capability):
//!
//! * `enscrive revisions list`        → `GET  /v1/backups`
//! * `enscrive revisions show <id>`   → `GET  /v1/backups/{id}`
//! * `enscrive restore --revision <id>`            → `POST /v1/restore`
//! * `enscrive restore --revision <id> --dry-run`  → `POST /v1/restore/dry-run`
//!
//! Restore is TENANT-WIDE and point-in-time: the server re-hydrates every
//! substrate collection for the tenant to the named revision's moment.
//! It is destructive, so the full CLI-TIER-013/014 destructive-command
//! gate applies (`--confirm` + interactive TTY re-type of the revision id;
//! non-TTY and JSON/agent mode are refused; managed mode additionally
//! requires `--confirm-token`). The pre-prompt refusals run BEFORE any
//! API call so a refused invocation provably never touches the server.
//!
//! The launched job (`job_kind = 'tenant_restore'`) carries
//! `params.verified` once terminal: `true` only when the server's
//! per-corpus convergence verification passed. The CLI exit code reflects
//! that honestly — a job that completes without `params.verified == true`
//! is reported as a failure, never silently stamped OK.

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::client::EnscriveClient;
use crate::jobs_polling::{self, PollConfig, PollOutcome, TerminalKind};
use crate::output::{
    CliResponse, EXIT_CONFIRMATION_REQUIRED, EXIT_FAILURE, FailureClass, OutputFormat,
};

// ---------------------------------------------------------------------------
// Command surface
// ---------------------------------------------------------------------------

/// `enscrive revisions …` — point-in-time restore points for your tenant.
#[derive(Subcommand)]
pub enum RevisionsSubcommand {
    /// List revisions (point-in-time restore points) for your tenant
    List(RevisionsListArgs),

    /// Show one revision in detail, including content checksums
    Show(RevisionsShowArgs),
}

#[derive(Args)]
pub struct RevisionsListArgs {
    /// Page size (server default 20)
    #[arg(long)]
    pub limit: Option<u32>,

    /// Pagination cursor — pass the previous page's `next_cursor`
    #[arg(long)]
    pub cursor: Option<String>,
}

#[derive(Args)]
pub struct RevisionsShowArgs {
    /// Revision ID (see `enscrive revisions list`)
    pub revision_id: String,
}

/// Args for `enscrive restore` — restore tenant data to a revision.
#[derive(Args)]
pub struct RestoreArgs {
    /// Revision to restore to (see `enscrive revisions list`)
    #[arg(long = "revision", value_name = "REVISION_ID")]
    pub revision_id: String,

    /// Validate the restore server-side without executing it
    #[arg(long = "dry-run", default_value_t = false)]
    pub dry_run: bool,

    /// Required to proceed with the destructive restore. Without this the
    /// command refuses (CLI-TIER-013); in an interactive TTY you are then
    /// prompted to re-type the revision id.
    #[arg(long, default_value_t = false)]
    pub confirm: bool,

    /// Confirmation token (required in managed mode; obtain via portal at https://enscrive.io/portal/confirmations)
    #[arg(long = "confirm-token", value_name = "TOKEN")]
    pub confirm_token: Option<String>,

    /// Return immediately with the launched job instead of polling to
    /// terminal status. Check progress with `enscrive jobs get`.
    #[arg(long = "async", default_value_t = false)]
    pub r#async: bool,

    /// Poll timeout for the wait path. Ignored when `--async` is set.
    #[arg(long = "timeout-secs", default_value_t = 1800)]
    pub timeout_secs: u64,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn handle_revisions(
    client: &EnscriveClient,
    sub: &RevisionsSubcommand,
    fmt: OutputFormat,
) -> ! {
    match sub {
        RevisionsSubcommand::List(args) => {
            let query = build_revisions_list_query(args);
            match client.get_json_with_query("/v1/backups", &query).await {
                Ok(data) => {
                    if matches!(fmt, OutputFormat::Human) {
                        if let Some(table) = render_revisions_table(&data) {
                            println!("{table}");
                        }
                    }
                    CliResponse::success("revisions list", data).emit(fmt)
                }
                Err(e) => crate::request_failure("revisions list", e).emit(fmt),
            }
        }
        RevisionsSubcommand::Show(args) => {
            let path = format!("/v1/backups/{}", args.revision_id);
            match client.get_json(&path).await {
                Ok(data) => {
                    if matches!(fmt, OutputFormat::Human) {
                        if let Some(detail) = render_revision_detail(&data) {
                            println!("{detail}");
                        }
                    }
                    CliResponse::success("revisions show", data).emit(fmt)
                }
                Err(e) => crate::request_failure("revisions show", e).emit(fmt),
            }
        }
    }
}

pub async fn handle_restore(
    client: &EnscriveClient,
    args: &RestoreArgs,
    deployment_mode: &str,
    fmt: OutputFormat,
) -> ! {
    let revision_id = args.revision_id.as_str();

    // ── --dry-run: read-only validation, no confirmation needed ──────────
    if args.dry_run {
        // The dry-run endpoint validates a point in time, so resolve the
        // revision's timestamp first (also fails loud on an unknown id).
        let detail = match client
            .get_json(&format!("/v1/backups/{revision_id}"))
            .await
        {
            Ok(d) => d,
            Err(e) => crate::request_failure("restore dry-run", e).emit(fmt),
        };
        let Some(target_time) = detail.get("timestamp").and_then(Value::as_str) else {
            CliResponse::fail(
                "restore dry-run",
                format!("revision {revision_id} has no timestamp field; cannot validate"),
                FailureClass::Bug,
                EXIT_FAILURE,
            )
            .emit(fmt);
        };
        let body = json!({ "target_time": target_time });
        match client.post_json("/v1/restore/dry-run", body).await {
            Ok(result) => CliResponse::success(
                "restore dry-run",
                json!({
                    "revision_id": revision_id,
                    "target_time": target_time,
                    "dry_run": result,
                }),
            )
            .emit(fmt),
            Err(e) => crate::request_failure("restore dry-run", e).emit(fmt),
        }
    }

    // ── Managed mode requires a confirmation token (CLI-TIER-014) ────────
    match crate::require_managed_confirmation(deployment_mode, args.confirm_token.as_deref(), "restore") {
        Err(FailureClass::ConfirmationRequired) => {
            CliResponse::fail(
                "restore",
                "'restore' requires a confirmation token in managed mode.\nObtain one at https://enscrive.io/portal/confirmations\n(or run locally with --mode self-managed and use --confirm)".to_string(),
                FailureClass::ConfirmationRequired,
                EXIT_CONFIRMATION_REQUIRED,
            ).emit(fmt);
        }
        Err(_) => unreachable!(),
        Ok(_) => {}
    }

    // ── Pre-prompt refusals (CLI-TIER-013) BEFORE any API call ───────────
    // Missing --confirm, JSON/agent mode, and non-TTY stdin are all refused
    // here, so a refused restore provably never reaches the server.
    {
        use std::io::IsTerminal;
        if let Some(msg) = crate::confirmation_preprompt_refusal(
            revision_id,
            fmt,
            args.confirm,
            std::io::stdin().is_terminal(),
        ) {
            CliResponse::fail(
                "restore",
                msg,
                FailureClass::ConfirmationRequired,
                EXIT_CONFIRMATION_REQUIRED,
            )
            .emit(fmt);
        }
    }

    // ── Resolve the revision and say exactly what will happen ────────────
    let detail = match client
        .get_json(&format!("/v1/backups/{revision_id}"))
        .await
    {
        Ok(d) => d,
        Err(e) => crate::request_failure("restore", e).emit(fmt),
    };
    eprintln!("{}", describe_restore(revision_id, &detail));

    // ── Interactive TTY confirmation: re-type the revision id ────────────
    // (Same CLI-TIER-013 gate as `corpus delete` / `voices delete`; a
    // mismatch aborts here — the POST below is never reached.)
    crate::require_local_confirmation(revision_id, "restore", fmt, args.confirm);

    // ── Launch the restore (202 + JobLaunchResponse) ──────────────────────
    let mut body = json!({
        "backup_id": revision_id,
        "confirm": revision_id,
    });
    if let Some(token) = &args.confirm_token {
        body["confirm_token"] = json!(token);
    }
    let launch = match client.post_json("/v1/restore", body).await {
        Ok(l) => l,
        Err(e) => crate::request_failure("restore", e).emit(fmt),
    };

    let job_id = launch
        .get("job_id")
        .and_then(Value::as_str)
        .map(String::from);
    match job_id {
        Some(job_id) if !args.r#async => {
            let poll_path = format!("/v1/jobs/{job_id}");
            let outcome = jobs_polling::await_job_terminal(
                client,
                &poll_path,
                PollConfig::waited(args.timeout_secs),
            )
            .await;
            let resp =
                restore_outcome_response("restore", &launch, &job_id, args.timeout_secs, outcome);
            if resp.ok && matches!(fmt, OutputFormat::Human) {
                eprintln!("Restore complete — convergence VERIFIED.");
            }
            resp.emit(fmt)
        }
        // --async (or a server that answered synchronously): emit as-is.
        _ => CliResponse::success("restore", launch).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// Pure decision: terminal restore job → CliResponse (unit-tested below)
// ---------------------------------------------------------------------------

/// Decide the final response for a polled restore job, honoring the
/// server's convergence verification honestly:
///
/// * terminal success AND `params.verified == true` → success;
/// * terminal success WITHOUT verification → failure (the data may not
///   match the catalog; never claim a verified restore that wasn't);
/// * terminal failure → failure carrying the server's shortfall message;
/// * timeout / poll failure → failure with resume guidance.
pub fn restore_outcome_response(
    command: &'static str,
    launch: &Value,
    job_id: &str,
    timeout_secs: u64,
    outcome: PollOutcome,
) -> CliResponse {
    match outcome {
        PollOutcome::Terminal {
            kind: TerminalKind::Succeeded,
            raw_status,
            job,
            ..
        } => {
            let verified = job
                .get("params")
                .and_then(|p| p.get("verified"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if verified {
                CliResponse::success(
                    command,
                    json!({ "launch": launch, "job": job, "verified": true }),
                )
            } else {
                let mut resp = CliResponse::fail(
                    command,
                    format!(
                        "restore job {job_id} reports status {raw_status:?} but convergence \
                         verification did not pass (params.verified is not true); treat the \
                         restored data as UNVERIFIED"
                    ),
                    FailureClass::FalseClaim,
                    EXIT_FAILURE,
                );
                resp.data = Some(json!({ "launch": launch, "job": job, "verified": false }));
                resp
            }
        }
        PollOutcome::Terminal {
            kind: TerminalKind::Failed,
            raw_status,
            job,
            ..
        } => {
            let error_message = job
                .get("error_message")
                .and_then(Value::as_str)
                .unwrap_or("job terminated without error_message")
                .to_string();
            let mut resp = CliResponse::fail(
                command,
                format!("restore job {job_id} {raw_status}: {error_message}"),
                FailureClass::Bug,
                EXIT_FAILURE,
            );
            resp.data = Some(json!({
                "launch": launch,
                "job": job,
                "verified": false,
                "terminal_status": raw_status,
            }));
            resp
        }
        PollOutcome::TimedOut {
            last_status,
            last_job,
            ..
        } => {
            let mut resp = CliResponse::fail(
                command,
                format!(
                    "timed out after {timeout_secs}s polling restore job {job_id} (last status: \
                     {last_status}); the restore may still be running server-side — check \
                     `enscrive jobs get --id {job_id}`"
                ),
                FailureClass::Bug,
                EXIT_FAILURE,
            );
            resp.data = Some(json!({
                "launch": launch,
                "last_job": last_job,
                "verified": false,
            }));
            resp
        }
        PollOutcome::PollFailed {
            error, last_job, ..
        } => {
            let mut resp = CliResponse::fail(
                command,
                format!(
                    "poll failed for restore job {job_id}: {error}; check \
                     `enscrive jobs get --id {job_id}`"
                ),
                FailureClass::Bug,
                EXIT_FAILURE,
            );
            resp.data = Some(json!({
                "launch": launch,
                "last_job": last_job,
                "verified": false,
            }));
            resp
        }
    }
}

// ---------------------------------------------------------------------------
// Rendering (human output)
// ---------------------------------------------------------------------------

fn build_revisions_list_query(args: &RevisionsListArgs) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(limit) = args.limit {
        query.push(("limit", limit.to_string()));
    }
    if let Some(cursor) = &args.cursor {
        query.push(("cursor", cursor.clone()));
    }
    query
}

/// Render `GET /v1/backups` as an aligned table:
/// REVISION / WHEN / TYPE / SIZE / POINTS. Returns `None` when the payload
/// doesn't carry a `backups` array (the raw JSON still gets emitted).
pub(crate) fn render_revisions_table(data: &Value) -> Option<String> {
    let backups = data.get("backups")?.as_array()?;
    if backups.is_empty() {
        return Some("No revisions found for this tenant yet.".to_string());
    }

    let header = ["REVISION", "WHEN", "TYPE", "SIZE", "POINTS"];
    let mut rows: Vec<[String; 5]> = Vec::with_capacity(backups.len());
    for b in backups {
        rows.push([
            str_or_dash(b, "backup_id"),
            str_or_dash(b, "timestamp"),
            str_or_dash(b, "backup_type"),
            b.get("compressed_bytes")
                .and_then(Value::as_u64)
                .map(format_bytes)
                .unwrap_or_else(|| "-".to_string()),
            b.get("total_points")
                .and_then(Value::as_u64)
                .map(format_count)
                .unwrap_or_else(|| "-".to_string()),
        ]);
    }

    let mut widths = header.map(str::len);
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }

    let render_row = |cells: [&str; 5]| -> String {
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                line.push_str("  ");
            }
            line.push_str(cell);
            if i < 4 {
                line.push_str(&" ".repeat(widths[i] - cell.len()));
            }
        }
        line
    };

    let mut out = render_row(header);
    for row in &rows {
        out.push('\n');
        out.push_str(&render_row([
            &row[0], &row[1], &row[2], &row[3], &row[4],
        ]));
    }

    if let Some(total) = data.get("total").and_then(Value::as_u64) {
        out.push_str(&format!("\n\ntotal: {total}"));
    }
    if let Some(cursor) = data.get("next_cursor").and_then(Value::as_str) {
        out.push_str(&format!("\nnext page: --cursor {cursor}"));
    }
    Some(out)
}

/// Render `GET /v1/backups/{id}` as a detail block, including the
/// per-collection checksums the revision carries.
pub(crate) fn render_revision_detail(data: &Value) -> Option<String> {
    let id = data.get("backup_id")?.as_str()?;
    let mut out = String::new();
    out.push_str(&format!("Revision:   {id}\n"));
    out.push_str(&format!("When:       {}\n", str_or_dash(data, "timestamp")));
    out.push_str(&format!("Type:       {}\n", str_or_dash(data, "backup_type")));
    out.push_str(&format!(
        "Points:     {}\n",
        data.get("total_points")
            .and_then(Value::as_u64)
            .map(format_count)
            .unwrap_or_else(|| "-".to_string())
    ));
    out.push_str(&format!(
        "Size:       {}\n",
        data.get("compressed_bytes")
            .and_then(Value::as_u64)
            .map(format_bytes)
            .unwrap_or_else(|| "-".to_string())
    ));
    let encrypted = data
        .get("encrypted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let key_version = data
        .get("encryption_key_version")
        .and_then(Value::as_u64)
        .map(|v| format!(" (key v{v})"))
        .unwrap_or_default();
    out.push_str(&format!(
        "Encrypted:  {}{}\n",
        if encrypted { "yes" } else { "no" },
        if encrypted { key_version } else { String::new() }
    ));
    out.push_str(&format!(
        "Parent:     {}\n",
        str_or_dash(data, "parent_backup_id")
    ));
    if let Some(label) = data.get("label").and_then(Value::as_str) {
        out.push_str(&format!("Label:      {label}\n"));
    }
    if let Some(expires) = data.get("expires_at").and_then(Value::as_str) {
        out.push_str(&format!("Expires:    {expires}\n"));
    }

    if let Some(collections) = data.get("collections").and_then(Value::as_object) {
        if !collections.is_empty() {
            out.push_str("Checksums:\n");
            let mut names: Vec<&String> = collections.keys().collect();
            names.sort();
            for name in names {
                let info = &collections[name];
                let points = info
                    .get("point_count")
                    .and_then(Value::as_u64)
                    .map(format_count)
                    .unwrap_or_else(|| "-".to_string());
                let checksum = info
                    .get("checksum")
                    .and_then(Value::as_str)
                    .unwrap_or("-");
                out.push_str(&format!("  {name}  points={points}  checksum={checksum}\n"));
            }
        }
    }
    Some(out.trim_end().to_string())
}

/// Spell out exactly what a restore will do before any confirmation prompt.
pub(crate) fn describe_restore(revision_id: &str, detail: &Value) -> String {
    let when = str_or_dash(detail, "timestamp");
    let backup_type = str_or_dash(detail, "backup_type");
    let points = detail
        .get("total_points")
        .and_then(Value::as_u64)
        .map(format_count)
        .unwrap_or_else(|| "unknown".to_string());
    let size = detail
        .get("compressed_bytes")
        .and_then(Value::as_u64)
        .map(format_bytes)
        .unwrap_or_else(|| "unknown size".to_string());
    format!(
        "About to restore tenant data to revision {revision_id}.\n\
         \n\
         This restore is TENANT-WIDE and point-in-time: every corpus in this\n\
         tenant is restored to its state as of {when}\n\
         ({backup_type}, {points} points, {size}). Data written after that\n\
         moment will not be present once the restore completes.\n\
         \n\
         To validate without executing, run:\n\
         \x20 enscrive restore --revision {revision_id} --dry-run\n"
    )
}

fn str_or_dash(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string()
}

/// Humanize a byte count (binary units, one decimal above bytes).
pub(crate) fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Thousands-separated point count.
fn format_count(n: u64) -> String {
    jobs_polling::format_num(i64::try_from(n).unwrap_or(i64::MAX))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::EXIT_SUCCESS;

    fn sample_list() -> Value {
        json!({
            "backups": [
                {
                    "backup_id": "bk-20260610-aaaa",
                    "timestamp": "2026-06-10T03:00:00Z",
                    "backup_type": "full",
                    "total_points": 12345,
                    "compressed_bytes": 7340032u64,
                    "encrypted": true,
                    "label": null,
                    "expires_at": null
                },
                {
                    "backup_id": "bk-20260609-bbbb",
                    "timestamp": "2026-06-09T03:00:00Z",
                    "backup_type": "incremental",
                    "total_points": 120,
                    "compressed_bytes": 2048u64,
                    "encrypted": true,
                    "label": "pre-migration",
                    "expires_at": "2026-09-09T03:00:00Z"
                }
            ],
            "total": 2,
            "next_cursor": "abc123"
        })
    }

    fn sample_detail() -> Value {
        json!({
            "backup_id": "bk-20260610-aaaa",
            "tenant_id": "t-1",
            "timestamp": "2026-06-10T03:00:00Z",
            "backup_type": "full",
            "parent_backup_id": null,
            "s3_key": "backups/t-1/bk-20260610-aaaa.tar.zst",
            "total_points": 12345,
            "compressed_bytes": 7340032u64,
            "collections": {
                "corpus_a": { "point_count": 12000, "checksum": "sha256:deadbeef" },
                "corpus_b": { "point_count": 345, "checksum": "sha256:cafef00d" }
            },
            "encrypted": true,
            "encryption_key_version": 3,
            "label": "nightly",
            "expires_at": "2026-09-10T03:00:00Z"
        })
    }

    #[test]
    fn list_table_renders_all_columns() {
        let table = render_revisions_table(&sample_list()).expect("table");
        // Header row.
        assert!(table.contains("REVISION"));
        assert!(table.contains("WHEN"));
        assert!(table.contains("TYPE"));
        assert!(table.contains("SIZE"));
        assert!(table.contains("POINTS"));
        // Both rows, with humanized size + separated counts.
        assert!(table.contains("bk-20260610-aaaa"));
        assert!(table.contains("bk-20260609-bbbb"));
        assert!(table.contains("7.0 MiB"));
        assert!(table.contains("2.0 KiB"));
        assert!(table.contains("12,345"));
        assert!(table.contains("incremental"));
        // Pagination footer.
        assert!(table.contains("total: 2"));
        assert!(table.contains("--cursor abc123"));
    }

    #[test]
    fn list_table_empty_and_malformed() {
        let empty = render_revisions_table(&json!({"backups": [], "total": 0}))
            .expect("empty table message");
        assert!(empty.contains("No revisions found"));
        assert!(render_revisions_table(&json!({"unexpected": true})).is_none());
    }

    #[test]
    fn detail_block_includes_checksums() {
        let block = render_revision_detail(&sample_detail()).expect("detail");
        assert!(block.contains("Revision:   bk-20260610-aaaa"));
        assert!(block.contains("When:       2026-06-10T03:00:00Z"));
        assert!(block.contains("Type:       full"));
        assert!(block.contains("Points:     12,345"));
        assert!(block.contains("Size:       7.0 MiB"));
        assert!(block.contains("Encrypted:  yes (key v3)"));
        assert!(block.contains("Label:      nightly"));
        assert!(block.contains("Checksums:"));
        assert!(block.contains("corpus_a  points=12,000  checksum=sha256:deadbeef"));
        assert!(block.contains("corpus_b  points=345  checksum=sha256:cafef00d"));
    }

    #[test]
    fn describe_restore_states_tenant_wide_point_in_time() {
        let text = describe_restore("bk-20260610-aaaa", &sample_detail());
        assert!(text.contains("TENANT-WIDE"));
        assert!(text.contains("point-in-time"));
        assert!(text.contains("2026-06-10T03:00:00Z"));
        assert!(text.contains("--dry-run"));
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(7340032), "7.0 MiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    // ── restore_outcome_response: honest VERIFIED handling ───────────────

    fn launch() -> Value {
        json!({"job_id": "job-1", "status": "pending", "poll_url": "/v1/jobs/job-1"})
    }

    fn terminal(kind: TerminalKind, raw_status: &str, job: Value) -> PollOutcome {
        PollOutcome::Terminal {
            kind,
            raw_status: raw_status.to_string(),
            job,
            poll_count: 1,
        }
    }

    #[test]
    fn restore_complete_and_verified_is_success() {
        let job = json!({"status": "complete", "params": {"verified": true}});
        let resp = restore_outcome_response(
            "restore",
            &launch(),
            "job-1",
            1800,
            terminal(TerminalKind::Succeeded, "complete", job),
        );
        assert!(resp.ok);
        assert_eq!(resp.exit_code, EXIT_SUCCESS);
        let data = resp.data.expect("data");
        assert_eq!(data["verified"], json!(true));
    }

    #[test]
    fn restore_complete_but_unverified_is_failure() {
        // A job stamped complete WITHOUT params.verified == true must not
        // be claimed as a verified restore.
        for params in [json!({}), json!({"verified": false}), json!(null)] {
            let job = json!({"status": "complete", "params": params});
            let resp = restore_outcome_response(
                "restore",
                &launch(),
                "job-1",
                1800,
                terminal(TerminalKind::Succeeded, "complete", job),
            );
            assert!(!resp.ok, "unverified restore must fail");
            assert_eq!(resp.exit_code, EXIT_FAILURE);
            assert_eq!(resp.failure_class, Some(FailureClass::FalseClaim));
            assert!(resp.error.as_deref().unwrap().contains("UNVERIFIED"));
            assert_eq!(resp.data.expect("data")["verified"], json!(false));
        }
    }

    #[test]
    fn restore_failed_job_carries_shortfall_message() {
        let job = json!({
            "status": "failed",
            "error_message": "post-restore divergence: substrate=100, catalog=120, corpus=c-1"
        });
        let resp = restore_outcome_response(
            "restore",
            &launch(),
            "job-1",
            1800,
            terminal(TerminalKind::Failed, "failed", job),
        );
        assert!(!resp.ok);
        assert_eq!(resp.exit_code, EXIT_FAILURE);
        let err = resp.error.as_deref().unwrap();
        assert!(err.contains("substrate=100, catalog=120, corpus=c-1"));
        assert_eq!(resp.data.expect("data")["verified"], json!(false));
    }

    #[test]
    fn restore_timeout_points_at_jobs_get() {
        let resp = restore_outcome_response(
            "restore",
            &launch(),
            "job-1",
            30,
            PollOutcome::TimedOut {
                last_status: "running".to_string(),
                last_job: json!({"status": "running"}),
                poll_count: 4,
            },
        );
        assert!(!resp.ok);
        let err = resp.error.as_deref().unwrap();
        assert!(err.contains("timed out after 30s"));
        assert!(err.contains("enscrive jobs get --id job-1"));
    }

    #[test]
    fn restore_poll_failed_is_failure() {
        let resp = restore_outcome_response(
            "restore",
            &launch(),
            "job-1",
            30,
            PollOutcome::PollFailed {
                error: "connection refused".to_string(),
                last_job: Value::Null,
                poll_count: 0,
            },
        );
        assert!(!resp.ok);
        assert!(resp.error.as_deref().unwrap().contains("connection refused"));
    }

    #[test]
    fn list_query_builder() {
        let q = build_revisions_list_query(&RevisionsListArgs {
            limit: Some(5),
            cursor: Some("abc".to_string()),
        });
        assert_eq!(
            q,
            vec![("limit", "5".to_string()), ("cursor", "abc".to_string())]
        );
        assert!(build_revisions_list_query(&RevisionsListArgs {
            limit: None,
            cursor: None
        })
        .is_empty());
    }

    // ── ADR §10.2 naming rule: banned vocabulary never appears here ──────

    #[test]
    fn revisions_register_bans_legacy_vocabulary() {
        let surface = concat!(
            // Help-text + user-facing strings in this module are compiled
            // into the source; scan the module source itself.
            include_str!("revisions.rs"),
        )
        .to_lowercase();
        for banned in ["rewind", "fast-forward", "playback", "recall"] {
            // The banned words appear exactly once each — inside this
            // test's own list literal. Anything more is a violation.
            let count = surface.matches(banned).count();
            assert!(
                count <= 1,
                "banned word {banned:?} appears {count} times in revisions.rs"
            );
        }
    }
}
