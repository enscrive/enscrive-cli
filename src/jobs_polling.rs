//! ENS-475 Wave A: shared polling helper for the `/v1` 202+`JobLaunchResponse`
//! contract.
//!
//! Every Class-A/C async CLI command sits on the same shape: POST returns
//! `{ job_id, status, poll_url }`, then the client polls
//! `GET /v1/jobs/{job_id}` with exponential backoff (2 → 15s) until the
//! server reports a terminal `.status` value. Before this module, three
//! near-identical loops lived in `main.rs::await_corpus_populate_job`,
//! `main.rs::run_evals_from_url`, and `evals2.rs::await_datasets_create_job`.
//!
//! Layering:
//!
//! * [`await_job_terminal`] — pure polling loop over a [`JobPoller`].
//!   Returns a [`PollOutcome`]. Decoupled from CLI emission so it can be
//!   unit-tested against in-memory fakes.
//! * [`emit_outcome`] — converts a [`PollOutcome`] into a
//!   [`CliResponse`] and exits. Pure decision logic; safe to call from
//!   any handler.
//! * [`await_and_emit`] / [`await_and_emit_launch_job`] — high-level
//!   convenience for the common "post launch body → poll → emit" flow.

use std::future::Future;

use serde_json::{json, Value};

use crate::client::{ApiError, EnscriveClient};
use crate::output::{CliResponse, FailureClass, OutputFormat, EXIT_FAILURE};

const INITIAL_DELAY_SECS: u64 = 2;
const MAX_DELAY_SECS: u64 = 15;

// ──────────────────────────────────────────────────────────────────────────
// Trait abstraction so the loop can be exercised against fakes.
// ──────────────────────────────────────────────────────────────────────────

/// Minimal polling surface — abstracts `EnscriveClient::get_json` so the
/// loop can be tested without a live HTTP server.
pub trait JobPoller {
    fn get_json(&self, path: &str) -> impl Future<Output = Result<Value, ApiError>> + Send;
}

impl JobPoller for EnscriveClient {
    fn get_json(&self, path: &str) -> impl Future<Output = Result<Value, ApiError>> + Send {
        EnscriveClient::get_json(self, path)
    }
}

impl<T: JobPoller + Sync> JobPoller for &T {
    fn get_json(&self, path: &str) -> impl Future<Output = Result<Value, ApiError>> + Send {
        (*self).get_json(path)
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Outcome types
// ──────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalKind {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone)]
pub enum PollOutcome {
    Terminal {
        kind: TerminalKind,
        raw_status: String,
        job: Value,
        poll_count: u64,
    },
    TimedOut {
        last_status: String,
        last_job: Value,
        poll_count: u64,
    },
    PollFailed {
        error: String,
        last_job: Value,
        poll_count: u64,
    },
}

/// Classify a job/run `.status` string. `None` means non-terminal —
/// caller should keep polling.
pub fn classify_status(status: &str) -> Option<TerminalKind> {
    match status {
        "complete" | "completed" | "succeeded" => Some(TerminalKind::Succeeded),
        "failed" | "cancelled" => Some(TerminalKind::Failed),
        _ => None,
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Config knobs
// ──────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PollConfig {
    pub initial_delay: std::time::Duration,
    pub max_delay: std::time::Duration,
    pub timeout: std::time::Duration,
    /// When true, print per-tick progress to stderr.
    pub progress: bool,
}

impl PollConfig {
    pub fn waited(timeout_secs: u64) -> Self {
        Self {
            initial_delay: std::time::Duration::from_secs(INITIAL_DELAY_SECS),
            max_delay: std::time::Duration::from_secs(MAX_DELAY_SECS),
            timeout: std::time::Duration::from_secs(timeout_secs),
            progress: true,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Core loop
// ──────────────────────────────────────────────────────────────────────────

/// Poll `poll_path` until the response carries a terminal `.status` or
/// the deadline elapses. Each tick reads the body via `poller.get_json`
/// and (optionally) renders a stderr progress line.
///
/// Exponential backoff: `cfg.initial_delay` doubled per tick, capped at
/// `cfg.max_delay`. A transient `get_json` error is retried until the
/// deadline; only after the deadline does it surface as `PollFailed`.
pub async fn await_job_terminal<P: JobPoller>(
    poller: &P,
    poll_path: &str,
    cfg: PollConfig,
) -> PollOutcome {
    let deadline = std::time::Instant::now() + cfg.timeout;
    let mut delay = cfg.initial_delay;
    let mut last_job = Value::Null;
    let mut poll_count: u64 = 0;

    loop {
        match poller.get_json(poll_path).await {
            Ok(job) => {
                last_job = job.clone();
                poll_count += 1;
                let status = job
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if cfg.progress {
                    print_poll_progress(poll_count, &job);
                }

                if let Some(kind) = classify_status(&status) {
                    return PollOutcome::Terminal {
                        kind,
                        raw_status: status,
                        job,
                        poll_count,
                    };
                }

                if std::time::Instant::now() >= deadline {
                    return PollOutcome::TimedOut {
                        last_status: status,
                        last_job: job,
                        poll_count,
                    };
                }

                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(cfg.max_delay);
            }
            Err(e) => {
                if std::time::Instant::now() >= deadline {
                    return PollOutcome::PollFailed {
                        error: e.to_string(),
                        last_job,
                        poll_count,
                    };
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(cfg.max_delay);
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Emission helpers
// ──────────────────────────────────────────────────────────────────────────

/// Take a [`PollOutcome`] and emit the canonical CLI response, exiting
/// the process via [`CliResponse::emit`].
///
/// `build_success_data` is applied to ALL three terminal/timeout
/// branches that have a job blob to work with — Succeeded, Failed, and
/// TimedOut — so callers that lift custom fields onto `.data` (e.g.
/// `evals from-url`'s `build_from_url_success_data` which surfaces
/// `dataset_id` / `corpus_count` / etc.) keep that shape symmetric
/// between success and failure. Failure + timeout branches overlay an
/// extra `.terminal_status` field on top of the builder's output.
///
/// `PollFailed` is a special case: there is no job blob, so the
/// builder is not invoked and a fixed `{launch, last_job}` shape is
/// emitted.
pub fn emit_outcome<F>(
    command: &'static str,
    launch: &Value,
    job_id: &str,
    timeout_secs: u64,
    outcome: PollOutcome,
    fmt: OutputFormat,
    build_success_data: F,
) -> !
where
    F: FnOnce(&Value, &Value) -> Value,
{
    match outcome {
        PollOutcome::Terminal {
            kind: TerminalKind::Succeeded,
            job,
            ..
        } => {
            let data = build_success_data(launch, &job);
            CliResponse::success(command, data).emit(fmt);
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
            let mut data = build_success_data(launch, &job);
            overlay_terminal_status(&mut data, &raw_status);
            let mut resp = CliResponse::fail(
                command,
                format!("job {} {}: {}", job_id, raw_status, error_message),
                FailureClass::Bug,
                EXIT_FAILURE,
            );
            resp.data = Some(data);
            resp.emit(fmt);
        }
        PollOutcome::TimedOut {
            last_status,
            last_job,
            ..
        } => {
            let mut data = build_success_data(launch, &last_job);
            overlay_terminal_status(&mut data, &last_status);
            let mut resp = CliResponse::fail(
                command,
                format!(
                    "timed out after {}s polling job {} (last status: {})",
                    timeout_secs, job_id, last_status
                ),
                FailureClass::Bug,
                EXIT_FAILURE,
            );
            resp.data = Some(data);
            resp.emit(fmt);
        }
        PollOutcome::PollFailed {
            error, last_job, ..
        } => {
            let mut resp = CliResponse::fail(
                command,
                format!("poll failed after timeout: {error}"),
                FailureClass::Bug,
                EXIT_FAILURE,
            );
            resp.data = Some(json!({
                "launch": launch,
                "last_job": last_job,
            }));
            resp.emit(fmt);
        }
    }
}

/// Set `data["terminal_status"] = status`. If `data` is not a JSON
/// object (a builder is free to return any Value), wrap it as
/// `{"data": <original>, "terminal_status": status}` so the field is
/// always reachable at the top level.
fn overlay_terminal_status(data: &mut Value, status: &str) {
    if let Some(obj) = data.as_object_mut() {
        obj.insert(
            "terminal_status".to_string(),
            Value::String(status.to_string()),
        );
    } else {
        *data = json!({
            "data": data.clone(),
            "terminal_status": status,
        });
    }
}

/// Poll `/v1/jobs/{job_id}` to terminal, then emit the resulting
/// `CliResponse` using `build_success_data` to shape the success branch.
pub async fn await_and_emit<P, F>(
    poller: &P,
    command: &'static str,
    launch: Value,
    job_id: &str,
    timeout_secs: u64,
    fmt: OutputFormat,
    build_success_data: F,
) -> !
where
    P: JobPoller,
    F: FnOnce(&Value, &Value) -> Value,
{
    let poll_path = format!("/v1/jobs/{}", job_id);
    let outcome = await_job_terminal(poller, &poll_path, PollConfig::waited(timeout_secs)).await;
    emit_outcome(command, &launch, job_id, timeout_secs, outcome, fmt, build_success_data)
}

/// Convenience wrapper that uses the canonical `{ launch, job }` success
/// shape. The vast majority of retrofit sites want exactly this.
pub async fn await_and_emit_launch_job<P: JobPoller>(
    poller: &P,
    command: &'static str,
    launch: Value,
    job_id: &str,
    timeout_secs: u64,
    fmt: OutputFormat,
) -> ! {
    await_and_emit(poller, command, launch, job_id, timeout_secs, fmt, |launch, job| {
        json!({ "launch": launch, "job": job })
    })
    .await
}

/// Branch on the launch response shape:
///
///   * If `launch.job_id` is absent, the server responded synchronously
///     (e.g. `--sync`, `--dry-run`, or an endpoint that hasn't been
///     converted to 202+`JobLaunchResponse` for this particular input).
///     Emit `launch` as the response body and exit.
///   * If `launch.job_id` is present and `r#async` is true, emit
///     `launch` (the caller is opting into the async fire-and-forget
///     contract).
///   * Otherwise (`launch.job_id` present, `r#async` false), poll the
///     job to terminal via [`await_and_emit_launch_job`].
///
/// This is the canonical dispatch for every Class-A/C async CLI
/// command sitting on top of an endpoint that *may* return either
/// shape depending on input flags.
pub async fn maybe_await_async_launch<P: JobPoller>(
    poller: &P,
    command: &'static str,
    launch: Value,
    r#async: bool,
    timeout_secs: u64,
    fmt: OutputFormat,
) -> ! {
    let job_id = launch.get("job_id").and_then(Value::as_str).map(String::from);
    match job_id {
        Some(job_id) if !r#async => {
            await_and_emit_launch_job(poller, command, launch, &job_id, timeout_secs, fmt).await
        }
        _ => CliResponse::success(command, launch).emit(fmt),
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Progress rendering
// ──────────────────────────────────────────────────────────────────────────

/// Print a poll-tick progress line to stderr with optional sub-batch
/// breakdown. Sub-batches render only when `.sub_batches[]` is present
/// on the job blob — so this is safe to call on any job/run shape.
pub fn print_poll_progress(poll_count: u64, job: &Value) {
    let status = job.get("status").and_then(Value::as_str).unwrap_or("unknown");
    let pct = job
        .get("progress_percent")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let ingested = job
        .get("documents_ingested")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let total = job
        .get("total_documents")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    eprintln!(
        "[poll {}] Job {} \u{2014} {:.1}% ({}/{})",
        poll_count,
        status,
        pct,
        format_num(ingested),
        format_num(total)
    );

    if let Some(subs) = job.get("sub_batches").and_then(Value::as_array) {
        let total_subs = subs.len();
        for sb in subs {
            let idx = sb.get("index").and_then(Value::as_u64).unwrap_or(0);
            let size = sb.get("size").and_then(Value::as_u64).unwrap_or(0);
            let sb_status = sb.get("status").and_then(Value::as_str).unwrap_or("unknown");
            let completed = sb.get("completed").and_then(Value::as_u64).unwrap_or(0);
            let icon = match sb_status {
                "completed" => "\u{2713}",
                "in_progress" | "storing" => "\u{25CF}",
                "failed" => "\u{2717}",
                _ => "\u{25CB}",
            };
            if sb_status == "pending" {
                eprintln!("  {} batch {}/{}  pending", icon, idx, total_subs);
            } else {
                eprintln!(
                    "  {} batch {}/{}  {}/{}",
                    icon,
                    idx,
                    total_subs,
                    format_num(completed as i64),
                    format_num(size as i64)
                );
            }
        }
    }
}

/// Format a number with thousands separators for CLI display.
fn format_num(n: i64) -> String {
    if n < 0 {
        return format!("-{}", format_num(-n));
    }
    let s = n.to_string();
    let len = s.len();
    if len <= 3 {
        return s;
    }
    let mut out = String::with_capacity(len + len / 3);
    let first_chunk = len % 3;
    if first_chunk > 0 {
        out.push_str(&s[..first_chunk]);
        if len > first_chunk {
            out.push(',');
        }
    }
    let mut i = first_chunk;
    while i < len {
        out.push_str(&s[i..i + 3]);
        i += 3;
        if i < len {
            out.push(',');
        }
    }
    out
}

// ──────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Drives the polling loop with a scripted sequence of responses.
    /// Each `poll` consumes the next entry; `Ok(Value)` returns it as
    /// the job blob, `Err(message)` simulates a transient HTTP failure.
    /// When the scripted queue is exhausted the last entry is repeated
    /// — real services hold a stable status once they reach one, and the
    /// repeat-on-exhaust semantics let the timeout / poll-failed tests
    /// drive the loop to its deadline without padding the script.
    struct ScriptedPoller {
        responses: Mutex<std::collections::VecDeque<Result<Value, &'static str>>>,
        last: Mutex<Option<Result<Value, &'static str>>>,
        calls: Mutex<Vec<String>>,
    }

    impl ScriptedPoller {
        fn new(seq: Vec<Result<Value, &'static str>>) -> Self {
            Self {
                responses: Mutex::new(seq.into()),
                last: Mutex::new(None),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }

        fn last_path(&self) -> Option<String> {
            self.calls.lock().unwrap().last().cloned()
        }
    }

    impl JobPoller for ScriptedPoller {
        async fn get_json(&self, path: &str) -> Result<Value, ApiError> {
            self.calls.lock().unwrap().push(path.to_string());
            let next = {
                let mut queue = self.responses.lock().unwrap();
                if let Some(front) = queue.pop_front() {
                    *self.last.lock().unwrap() = Some(front.clone());
                    front
                } else {
                    self.last
                        .lock()
                        .unwrap()
                        .clone()
                        .expect("ScriptedPoller has no responses at all")
                }
            };
            match next {
                Ok(v) => Ok(v),
                Err(msg) => Err(ApiError::InvalidResponse {
                    status: 500,
                    body: msg.to_string(),
                }),
            }
        }
    }

    fn fast_cfg() -> PollConfig {
        PollConfig {
            initial_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(1),
            timeout: std::time::Duration::from_secs(5),
            progress: false,
        }
    }

    #[test]
    fn classify_status_terminal_succeeded() {
        assert_eq!(classify_status("completed"), Some(TerminalKind::Succeeded));
        assert_eq!(classify_status("complete"), Some(TerminalKind::Succeeded));
        assert_eq!(classify_status("succeeded"), Some(TerminalKind::Succeeded));
    }

    #[test]
    fn classify_status_terminal_failed() {
        assert_eq!(classify_status("failed"), Some(TerminalKind::Failed));
        assert_eq!(classify_status("cancelled"), Some(TerminalKind::Failed));
    }

    #[test]
    fn classify_status_non_terminal() {
        assert_eq!(classify_status("pending"), None);
        assert_eq!(classify_status("running"), None);
        assert_eq!(classify_status("queued"), None);
        assert_eq!(classify_status(""), None);
        assert_eq!(classify_status("unknown_future_value"), None);
    }

    #[tokio::test]
    async fn pending_then_running_then_complete() {
        let poller = ScriptedPoller::new(vec![
            Ok(json!({ "status": "pending" })),
            Ok(json!({ "status": "running", "documents_ingested": 5 })),
            Ok(json!({ "status": "completed", "documents_ingested": 10 })),
        ]);
        let outcome = await_job_terminal(&poller, "/v1/jobs/abc", fast_cfg()).await;
        match outcome {
            PollOutcome::Terminal {
                kind, raw_status, poll_count, job, ..
            } => {
                assert_eq!(kind, TerminalKind::Succeeded);
                assert_eq!(raw_status, "completed");
                assert_eq!(poll_count, 3);
                assert_eq!(job.get("documents_ingested").and_then(Value::as_i64), Some(10));
            }
            other => panic!("expected Terminal::Succeeded, got {other:?}"),
        }
        assert_eq!(poller.call_count(), 3);
        assert_eq!(poller.last_path().as_deref(), Some("/v1/jobs/abc"));
    }

    #[tokio::test]
    async fn pending_then_failed() {
        let poller = ScriptedPoller::new(vec![
            Ok(json!({ "status": "pending" })),
            Ok(json!({
                "status": "failed",
                "error_message": "embedding rate-limit exceeded",
            })),
        ]);
        let outcome = await_job_terminal(&poller, "/v1/jobs/xyz", fast_cfg()).await;
        match outcome {
            PollOutcome::Terminal {
                kind, raw_status, job, ..
            } => {
                assert_eq!(kind, TerminalKind::Failed);
                assert_eq!(raw_status, "failed");
                assert_eq!(
                    job.get("error_message").and_then(Value::as_str),
                    Some("embedding rate-limit exceeded")
                );
            }
            other => panic!("expected Terminal::Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancelled_classified_as_failed() {
        let poller = ScriptedPoller::new(vec![Ok(json!({ "status": "cancelled" }))]);
        let outcome = await_job_terminal(&poller, "/v1/jobs/c", fast_cfg()).await;
        assert!(matches!(
            outcome,
            PollOutcome::Terminal {
                kind: TerminalKind::Failed,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn times_out_when_no_terminal_status() {
        let poller = ScriptedPoller::new(vec![
            Ok(json!({ "status": "pending" })),
            Ok(json!({ "status": "running" })),
            Ok(json!({ "status": "running" })),
        ]);
        let cfg = PollConfig {
            initial_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(1),
            timeout: std::time::Duration::from_millis(5),
            progress: false,
        };
        let outcome = await_job_terminal(&poller, "/v1/jobs/t", cfg).await;
        match outcome {
            PollOutcome::TimedOut {
                last_status,
                poll_count,
                ..
            } => {
                assert_eq!(last_status, "running");
                // 5ms deadline with 1ms initial+max delay should produce
                // 3+ polls (the first three are scripted; ScriptedPoller
                // then repeats the last entry until deadline).
                assert!(
                    poll_count >= 3,
                    "should have consumed all three scripted polls; got {poll_count}",
                );
            }
            other => panic!("expected TimedOut, got {other:?}"),
        }
    }

    #[test]
    fn overlay_terminal_status_on_object() {
        let mut data = json!({ "dataset_id": "abc", "job_id": "xyz" });
        overlay_terminal_status(&mut data, "failed");
        assert_eq!(data["dataset_id"], "abc");
        assert_eq!(data["job_id"], "xyz");
        assert_eq!(data["terminal_status"], "failed");
    }

    #[test]
    fn overlay_terminal_status_wraps_non_object() {
        let mut data = json!([1, 2, 3]);
        overlay_terminal_status(&mut data, "cancelled");
        assert_eq!(data["data"], json!([1, 2, 3]));
        assert_eq!(data["terminal_status"], "cancelled");
    }

    #[tokio::test]
    async fn transient_error_retried_then_succeeds() {
        let poller = ScriptedPoller::new(vec![
            Err("transient 500"),
            Ok(json!({ "status": "pending" })),
            Ok(json!({ "status": "completed" })),
        ]);
        let outcome = await_job_terminal(&poller, "/v1/jobs/r", fast_cfg()).await;
        assert!(matches!(
            outcome,
            PollOutcome::Terminal {
                kind: TerminalKind::Succeeded,
                ..
            }
        ));
        assert_eq!(poller.call_count(), 3);
    }

    #[tokio::test]
    async fn poll_failed_after_deadline() {
        let poller = ScriptedPoller::new(vec![
            Err("persistent 500"),
            Err("persistent 500"),
            Err("persistent 500"),
        ]);
        let cfg = PollConfig {
            initial_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(1),
            timeout: std::time::Duration::from_millis(5),
            progress: false,
        };
        let outcome = await_job_terminal(&poller, "/v1/jobs/p", cfg).await;
        match outcome {
            PollOutcome::PollFailed { error, .. } => {
                assert!(error.contains("persistent 500"), "got: {error}");
            }
            other => panic!("expected PollFailed, got {other:?}"),
        }
    }
}
