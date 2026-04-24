//! Evals 2.0 CLI surface (EV-013).
//!
//! Thin client over `/v1/datasets`, `/v1/eval-defs`, `/v1/eval-runs`, and
//! the EV-011/EV-012 voice diff + cost endpoints. Every read command
//! supports structured JSON output via the shared `CliResponse` pipeline.
//!
//! Namespaced at `datasets`, `eval-defs`, `eval-runs` to avoid colliding
//! with the legacy `enscrive evals campaigns` + `enscrive evals datasets`
//! subcommand trees still serving the older eval_campaigns surface.
//! A future cleanup can unify once EV-016 renames the server-side tables.

use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::client::{ApiError, EnscriveClient};
use crate::output::{CliResponse, FailureClass, OutputFormat, EXIT_CONFIG, EXIT_FAILURE};

// ──────────────────────────────────────────────────────────────────────────
// Subcommand trees
// ──────────────────────────────────────────────────────────────────────────

#[derive(Subcommand, Clone)]
pub enum Datasets2Subcommand {
    /// List datasets for the tenant + environment.
    List,
    /// Get a dataset by id.
    Get {
        #[arg(long)]
        id: String,
    },
    /// Describe a dataset (structured summary for agents).
    Describe {
        #[arg(long)]
        id: String,
    },
    /// Archive (hard-delete) a dataset.
    Delete {
        #[arg(long)]
        id: String,
    },
    /// Upload a BeIR-layout dataset from a local directory containing
    /// `corpus.jsonl`, `queries.jsonl`, `qrels.tsv`.
    Upload(DatasetsUploadArgs),
    /// Create a dataset by downloading from a HuggingFace BeIR URL.
    /// Writes `source_type=huggingface` and `source_url` at creation, so
    /// the dataset is eligible for the `/v1/eval-defs/{id}/publish` gate
    /// without admin SQL.
    Create(DatasetsCreateArgs),
}

#[derive(Args, Clone)]
pub struct DatasetsCreateArgs {
    /// HuggingFace URL. Accepts `huggingface:BeIR/fiqa`,
    /// `https://huggingface.co/datasets/BeIR/fiqa`, or short `BeIR/fiqa`.
    #[arg(long = "from-url")]
    pub from_url: String,
    /// Dataset display name.
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub description: Option<String>,
    /// full | stratified_random | explicit. Default full.
    #[arg(long, default_value = "full")]
    pub sample_strategy: String,
    /// Strategy params as JSON.
    #[arg(long)]
    pub sample_params: Option<String>,
    #[arg(long)]
    pub sample_seed: Option<i64>,
    #[arg(long)]
    pub selected_query_ids: Option<String>,
    #[arg(long)]
    pub selected_doc_ids: Option<String>,
    #[arg(long)]
    pub rationale: Option<String>,
}

#[derive(Args, Clone)]
pub struct DatasetsUploadArgs {
    /// Directory holding corpus.jsonl + queries.jsonl + qrels. Accepts both
    /// the flat layout (qrels.tsv at the top) and the canonical BEIR layout
    /// (qrels/{train,dev,test}.tsv); see --qrels-split.
    #[arg(long)]
    pub dir: String,
    /// Which qrels split to upload when the directory uses the BEIR layout
    /// (qrels/<split>.tsv). Default "test" — matches EV-003 baseline
    /// comparison. Ignored when a flat qrels.tsv is present.
    #[arg(long, default_value = "test")]
    pub qrels_split: String,
    /// Dataset display name.
    #[arg(long)]
    pub name: String,
    /// Optional description.
    #[arg(long)]
    pub description: Option<String>,
    /// Sample strategy: full | stratified_random | explicit. Defaults to full.
    #[arg(long, default_value = "full")]
    pub sample_strategy: String,
    /// Strategy params as JSON (e.g. '{"n_queries": 40, "distractor_ratio": 2}').
    #[arg(long)]
    pub sample_params: Option<String>,
    /// Seed for stratified_random (ignored by other strategies).
    #[arg(long)]
    pub sample_seed: Option<i64>,
    /// For `explicit`: comma-separated query IDs.
    #[arg(long)]
    pub selected_query_ids: Option<String>,
    /// For `explicit`: comma-separated doc IDs.
    #[arg(long)]
    pub selected_doc_ids: Option<String>,
    /// For `explicit`: free-form rationale (stored for audit).
    #[arg(long)]
    pub rationale: Option<String>,
}

#[derive(Subcommand, Clone)]
pub enum EvalDefsSubcommand {
    /// Create a new eval definition.
    Create(EvalDefsCreateArgs),
    /// List all eval definitions for the tenant + environment.
    List,
    /// Get a single eval definition.
    Get {
        #[arg(long)]
        id: String,
    },
    /// Delete (soft-archive) an eval definition.
    Delete {
        #[arg(long)]
        id: String,
    },
    /// Trigger a run and poll until terminal.
    Run(EvalDefsRunArgs),
    /// Per-run sub-commands.
    Runs {
        #[command(subcommand)]
        sub: EvalRunsSubcommand,
    },
    /// Publish a completed full-scope run as canonical (EV-017).
    Publish(EvalDefsPublishArgs),
    /// List active publications for an eval.
    Publications {
        #[arg(long)]
        id: String,
    },
    /// Unpublish a publication (soft delete — audit row remains).
    Unpublish {
        #[arg(long = "publication-id")]
        publication_id: String,
    },
}

#[derive(Args, Clone)]
pub struct EvalDefsPublishArgs {
    /// Eval definition UUID.
    #[arg(long)]
    pub id: String,
    /// Run UUID to mark as canonical.
    #[arg(long = "run-id")]
    pub run_id: String,
    /// Optional free-form reviewer notes (stored for audit).
    #[arg(long)]
    pub notes: Option<String>,
}

#[derive(Args, Clone)]
pub struct EvalDefsCreateArgs {
    /// Dataset UUID this eval targets.
    #[arg(long)]
    pub dataset: String,
    /// Collection UUID (where search runs).
    #[arg(long)]
    pub collection: String,
    /// Optional voice UUID.
    #[arg(long)]
    pub voice: Option<String>,
    /// Display name.
    #[arg(long)]
    pub name: String,
    /// Optional description.
    #[arg(long)]
    pub description: Option<String>,
    /// Optional methodology JSON (defaults to
    /// `{"k_values": [10, 100], "metrics": ["recall","precision","ndcg","mrr"]}`).
    #[arg(long)]
    pub methodology: Option<String>,
}

#[derive(Args, Clone)]
pub struct EvalDefsRunArgs {
    /// Eval definition UUID.
    #[arg(long)]
    pub id: String,
    /// Don't poll — return the accepted-response as soon as the run enqueues.
    #[arg(long, default_value_t = false)]
    pub no_follow: bool,
    /// Poll interval in seconds.
    #[arg(long, default_value_t = 3)]
    pub poll_secs: u64,
    /// Max total polling seconds before giving up (still returns what's
    /// known at that moment).
    #[arg(long, default_value_t = 3600)]
    pub timeout_secs: u64,
}

#[derive(Subcommand, Clone)]
pub enum EvalRunsSubcommand {
    /// List all runs for an eval definition.
    List {
        #[arg(long = "eval-id")]
        eval_id: String,
    },
    /// Fetch a run (aggregate metrics + status).
    Get {
        #[arg(long)]
        id: String,
    },
    /// Fetch per-query details for a run, sorted worst-first — the
    /// diagnose view.
    Diagnose(EvalRunsDiagnoseArgs),
}

#[derive(Args, Clone)]
pub struct EvalRunsDiagnoseArgs {
    #[arg(long)]
    pub id: String,
    #[arg(long, default_value_t = 20)]
    pub limit: i64,
    #[arg(long, default_value_t = 0)]
    pub offset: i64,
    /// Ordering: `worst` (default) by nDCG@10 asc, or `created` for insert
    /// order.
    #[arg(long, default_value = "worst")]
    pub order: String,
}

#[derive(Subcommand, Clone)]
pub enum VoiceDiff2Subcommand {
    /// Diff a voice against an earlier version (or between two versions).
    Diff(VoicesDiffArgs),
    /// Estimate money + time cost of applying the diff to a collection.
    DiffCost(VoicesDiffCostArgs),
    /// Diff the live voice against a proposed config from a JSON file.
    DiffProposal(VoicesDiffProposalArgs),
}

#[derive(Args, Clone)]
pub struct VoicesDiffArgs {
    #[arg(long)]
    pub id: String,
    /// Version to diff AGAINST (before side).
    #[arg(long)]
    pub against: u32,
    /// Optional "after" version (defaults to live voice from observe).
    #[arg(long)]
    pub from: Option<u32>,
}

#[derive(Args, Clone)]
pub struct VoicesDiffCostArgs {
    #[arg(long)]
    pub id: String,
    /// Version to diff AGAINST.
    #[arg(long)]
    pub against: u32,
    /// Target collection UUID.
    #[arg(long)]
    pub collection: String,
    /// Whether to model batch-API pricing (default true).
    #[arg(long, default_value_t = true)]
    pub batch: bool,
}

#[derive(Args, Clone)]
pub struct VoicesDiffProposalArgs {
    #[arg(long)]
    pub id: String,
    /// File containing the proposed VoiceConfigApi JSON.
    #[arg(long = "proposed-file")]
    pub proposed_file: String,
}

// ──────────────────────────────────────────────────────────────────────────
// Handlers — datasets
// ──────────────────────────────────────────────────────────────────────────

pub async fn run_datasets(
    client: &EnscriveClient,
    fmt: OutputFormat,
    sub: Datasets2Subcommand,
) -> i32 {
    match sub {
        Datasets2Subcommand::List => match client.get_json("/v1/datasets").await {
            Ok(data) => CliResponse::success("datasets list", data).emit(fmt),
            Err(e) => request_failure("datasets list", e).emit(fmt),
        },
        Datasets2Subcommand::Get { id } => {
            let path = format!("/v1/datasets/{id}");
            match client.get_json(&path).await {
                Ok(data) => CliResponse::success("datasets get", data).emit(fmt),
                Err(e) => request_failure("datasets get", e).emit(fmt),
            }
        }
        Datasets2Subcommand::Describe { id } => {
            let path = format!("/v1/datasets/{id}/describe");
            match client.get_json(&path).await {
                Ok(data) => CliResponse::success("datasets describe", data).emit(fmt),
                Err(e) => request_failure("datasets describe", e).emit(fmt),
            }
        }
        Datasets2Subcommand::Delete { id } => {
            let path = format!("/v1/datasets/{id}");
            match client.delete_json(&path).await {
                Ok(data) => CliResponse::success("datasets delete", data).emit(fmt),
                Err(e) => request_failure("datasets delete", e).emit(fmt),
            }
        }
        Datasets2Subcommand::Upload(args) => handle_datasets_upload(client, fmt, args).await,
        Datasets2Subcommand::Create(args) => handle_datasets_create(client, fmt, args).await,
    }
}

async fn handle_datasets_create(
    client: &EnscriveClient,
    fmt: OutputFormat,
    args: DatasetsCreateArgs,
) -> i32 {
    let mut body = json!({
        "name": args.name,
        "source_type": "huggingface",
        "source_url": args.from_url,
    });
    if let Some(d) = args.description {
        body["description"] = Value::String(d);
    }
    if args.sample_strategy != "full" {
        let mut sample = json!({ "strategy": args.sample_strategy });
        if let Some(p) = args.sample_params {
            match serde_json::from_str::<Value>(&p) {
                Ok(v) => sample["params"] = v,
                Err(e) => {
                    return CliResponse::fail(
                        "datasets create",
                        format!("--sample-params is not valid JSON: {e}"),
                        FailureClass::Bug,
                        EXIT_CONFIG,
                    )
                    .emit(fmt);
                }
            }
        }
        if let Some(seed) = args.sample_seed {
            sample["seed"] = json!(seed);
        }
        if let Some(ids) = args.selected_query_ids {
            let list: Vec<String> = ids.split(',').map(|s| s.trim().to_string()).collect();
            sample["selected_query_ids"] = json!(list);
        }
        if let Some(ids) = args.selected_doc_ids {
            let list: Vec<String> = ids.split(',').map(|s| s.trim().to_string()).collect();
            sample["selected_doc_ids"] = json!(list);
        }
        if let Some(r) = args.rationale {
            sample["rationale"] = Value::String(r);
        }
        body["sample"] = sample;
    }
    match client.post_json("/v1/datasets", body).await {
        Ok(data) => CliResponse::success("datasets create", data).emit(fmt),
        Err(e) => request_failure("datasets create", e).emit(fmt),
    }
}

async fn handle_datasets_upload(
    client: &EnscriveClient,
    fmt: OutputFormat,
    args: DatasetsUploadArgs,
) -> i32 {
    let dir = std::path::PathBuf::from(&args.dir);
    let corpus_path = dir.join("corpus.jsonl");
    let queries_path = dir.join("queries.jsonl");

    // Resolve qrels path. Canonical BEIR layout keeps train/test/dev splits
    // under <dir>/qrels/<split>.tsv; a flat layout puts a single
    // <dir>/qrels.tsv at the top. Prefer the split-aware layout when it
    // exists so the default works against a freshly-unzipped BEIR archive.
    let split_qrels_path = dir.join("qrels").join(format!("{}.tsv", args.qrels_split));
    let flat_qrels_path = dir.join("qrels.tsv");
    let qrels_path = if split_qrels_path.exists() {
        split_qrels_path
    } else if flat_qrels_path.exists() {
        flat_qrels_path
    } else {
        return CliResponse::fail(
            "datasets upload",
            format!(
                "missing qrels: expected either {} or {}",
                flat_qrels_path.display(),
                split_qrels_path.display(),
            ),
            FailureClass::Bug,
            EXIT_CONFIG,
        )
        .emit(fmt);
    };

    for p in [&corpus_path, &queries_path] {
        if !p.exists() {
            return CliResponse::fail(
                "datasets upload",
                format!("missing required file: {}", p.display()),
                FailureClass::Bug,
                EXIT_CONFIG,
            )
            .emit(fmt);
        }
    }

    let corpus_bytes = match std::fs::read(&corpus_path) {
        Ok(b) => b,
        Err(e) => {
            return CliResponse::fail(
                "datasets upload",
                format!("read corpus.jsonl: {e}"),
                FailureClass::Bug,
                EXIT_CONFIG,
            )
            .emit(fmt);
        }
    };
    let queries_bytes = match std::fs::read(&queries_path) {
        Ok(b) => b,
        Err(e) => {
            return CliResponse::fail(
                "datasets upload",
                format!("read queries.jsonl: {e}"),
                FailureClass::Bug,
                EXIT_CONFIG,
            )
            .emit(fmt);
        }
    };
    let qrels_bytes = match std::fs::read(&qrels_path) {
        Ok(b) => b,
        Err(e) => {
            return CliResponse::fail(
                "datasets upload",
                format!("read qrels.tsv: {e}"),
                FailureClass::Bug,
                EXIT_CONFIG,
            )
            .emit(fmt);
        }
    };

    let mut meta = json!({
        "name": args.name,
    });
    if let Some(d) = args.description {
        meta["description"] = Value::String(d);
    }

    if args.sample_strategy != "full" {
        let mut sample = json!({ "strategy": args.sample_strategy });
        if let Some(params_str) = args.sample_params {
            match serde_json::from_str::<Value>(&params_str) {
                Ok(v) => sample["params"] = v,
                Err(e) => {
                    return CliResponse::fail(
                        "datasets upload",
                        format!("--sample-params is not valid JSON: {e}"),
                        FailureClass::Bug,
                        EXIT_CONFIG,
                    )
                    .emit(fmt);
                }
            }
        }
        if let Some(seed) = args.sample_seed {
            sample["seed"] = json!(seed);
        }
        if let Some(ids) = args.selected_query_ids {
            let list: Vec<String> = ids.split(',').map(|s| s.trim().to_string()).collect();
            sample["selected_query_ids"] = json!(list);
        }
        if let Some(ids) = args.selected_doc_ids {
            let list: Vec<String> = ids.split(',').map(|s| s.trim().to_string()).collect();
            sample["selected_doc_ids"] = json!(list);
        }
        if let Some(r) = args.rationale {
            sample["rationale"] = Value::String(r);
        }
        meta["sample"] = sample;
    }

    match client
        .post_dataset_upload(
            "/v1/datasets/upload",
            meta,
            corpus_bytes,
            queries_bytes,
            qrels_bytes,
        )
        .await
    {
        Ok(data) => CliResponse::success("datasets upload", data).emit(fmt),
        Err(e) => request_failure("datasets upload", e).emit(fmt),
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Handlers — eval-defs + runs
// ──────────────────────────────────────────────────────────────────────────

pub async fn run_eval_defs(
    client: &EnscriveClient,
    fmt: OutputFormat,
    sub: EvalDefsSubcommand,
) -> i32 {
    match sub {
        EvalDefsSubcommand::Create(args) => handle_eval_defs_create(client, fmt, args).await,
        EvalDefsSubcommand::List => match client.get_json("/v1/eval-defs").await {
            Ok(data) => CliResponse::success("eval-defs list", data).emit(fmt),
            Err(e) => request_failure("eval-defs list", e).emit(fmt),
        },
        EvalDefsSubcommand::Get { id } => {
            let path = format!("/v1/eval-defs/{id}");
            match client.get_json(&path).await {
                Ok(data) => CliResponse::success("eval-defs get", data).emit(fmt),
                Err(e) => request_failure("eval-defs get", e).emit(fmt),
            }
        }
        EvalDefsSubcommand::Delete { id } => {
            let path = format!("/v1/eval-defs/{id}");
            match client.delete_json(&path).await {
                Ok(data) => CliResponse::success("eval-defs delete", data).emit(fmt),
                Err(e) => request_failure("eval-defs delete", e).emit(fmt),
            }
        }
        EvalDefsSubcommand::Run(args) => handle_eval_defs_run(client, fmt, args).await,
        EvalDefsSubcommand::Runs { sub } => run_eval_runs(client, fmt, sub).await,
        EvalDefsSubcommand::Publish(args) => {
            let path = format!("/v1/eval-defs/{}/publish", args.id);
            let mut body = json!({ "run_id": args.run_id });
            if let Some(n) = args.notes {
                body["reviewer_notes"] = Value::String(n);
            }
            match client.post_json(&path, body).await {
                Ok(data) => CliResponse::success("eval-defs publish", data).emit(fmt),
                Err(e) => request_failure("eval-defs publish", e).emit(fmt),
            }
        }
        EvalDefsSubcommand::Publications { id } => {
            let path = format!("/v1/eval-defs/{id}/publications");
            match client.get_json(&path).await {
                Ok(data) => CliResponse::success("eval-defs publications", data).emit(fmt),
                Err(e) => request_failure("eval-defs publications", e).emit(fmt),
            }
        }
        EvalDefsSubcommand::Unpublish { publication_id } => {
            let path = format!("/v1/eval-publications/{publication_id}");
            match client.delete_json(&path).await {
                Ok(data) => CliResponse::success("eval-defs unpublish", data).emit(fmt),
                Err(e) => request_failure("eval-defs unpublish", e).emit(fmt),
            }
        }
    }
}

async fn handle_eval_defs_create(
    client: &EnscriveClient,
    fmt: OutputFormat,
    args: EvalDefsCreateArgs,
) -> i32 {
    let mut body = json!({
        "name": args.name,
        "dataset_id": args.dataset,
        "collection_id": args.collection,
    });
    if let Some(v) = args.voice {
        body["voice_id"] = Value::String(v);
    }
    if let Some(d) = args.description {
        body["description"] = Value::String(d);
    }
    if let Some(m_str) = args.methodology {
        match serde_json::from_str::<Value>(&m_str) {
            Ok(v) => body["methodology"] = v,
            Err(e) => {
                return CliResponse::fail(
                    "eval-defs create",
                    format!("--methodology is not valid JSON: {e}"),
                    FailureClass::Bug,
                    EXIT_CONFIG,
                )
                .emit(fmt);
            }
        }
    }
    match client.post_json("/v1/eval-defs", body).await {
        Ok(data) => CliResponse::success("eval-defs create", data).emit(fmt),
        Err(e) => request_failure("eval-defs create", e).emit(fmt),
    }
}

async fn handle_eval_defs_run(
    client: &EnscriveClient,
    fmt: OutputFormat,
    args: EvalDefsRunArgs,
) -> i32 {
    let path = format!("/v1/eval-defs/{}/runs", args.id);
    let accepted = match client.post_json(&path, json!({})).await {
        Ok(v) => v,
        Err(e) => return request_failure("eval-defs run", e).emit(fmt),
    };

    if args.no_follow {
        return CliResponse::success("eval-defs run (accepted)", accepted).emit(fmt);
    }

    let run_id = accepted
        .get("run_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default();
    if run_id.is_empty() {
        return CliResponse::fail(
            "eval-defs run",
            "server accepted the run but did not return run_id; aborting polling".into(),
            FailureClass::Bug,
            EXIT_CONFIG,
        )
        .emit(fmt);
    }

    // Poll loop
    let run_path = format!("/v1/eval-runs/{run_id}");
    let started = std::time::Instant::now();
    let interval = std::time::Duration::from_secs(args.poll_secs.max(1));
    let timeout = std::time::Duration::from_secs(args.timeout_secs);

    loop {
        let run = match client.get_json(&run_path).await {
            Ok(v) => v,
            Err(e) => return request_failure("eval-defs run (poll)", e).emit(fmt),
        };
        let status = run
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        match status.as_str() {
            "completed" => return CliResponse::success("eval-defs run", run).emit(fmt),
            "failed" => {
                return CliResponse::fail(
                    "eval-defs run",
                    run.get("error_message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("run failed")
                        .to_string(),
                    FailureClass::Bug,
                    crate::output::EXIT_FAILURE,
                )
                .emit(fmt)
            }
            _ => {}
        }
        if started.elapsed() > timeout {
            return CliResponse::fail(
                "eval-defs run",
                format!("polling timed out after {}s (run still {status})", args.timeout_secs),
                FailureClass::Bug,
                crate::output::EXIT_FAILURE,
            )
            .emit(fmt);
        }
        tokio::time::sleep(interval).await;
    }
}

pub async fn run_eval_runs(
    client: &EnscriveClient,
    fmt: OutputFormat,
    sub: EvalRunsSubcommand,
) -> i32 {
    match sub {
        EvalRunsSubcommand::List { eval_id } => {
            let path = format!("/v1/eval-defs/{eval_id}/runs");
            match client.get_json(&path).await {
                Ok(data) => CliResponse::success("eval-runs list", data).emit(fmt),
                Err(e) => request_failure("eval-runs list", e).emit(fmt),
            }
        }
        EvalRunsSubcommand::Get { id } => {
            let path = format!("/v1/eval-runs/{id}");
            match client.get_json(&path).await {
                Ok(data) => CliResponse::success("eval-runs get", data).emit(fmt),
                Err(e) => request_failure("eval-runs get", e).emit(fmt),
            }
        }
        EvalRunsSubcommand::Diagnose(args) => {
            let path = format!("/v1/eval-runs/{}/queries", args.id);
            let qs = [
                ("order", args.order.clone()),
                ("limit", args.limit.to_string()),
                ("offset", args.offset.to_string()),
            ];
            match client.get_json_with_query(&path, &qs).await {
                Ok(data) => CliResponse::success("eval-runs diagnose", data).emit(fmt),
                Err(e) => request_failure("eval-runs diagnose", e).emit(fmt),
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Handlers — voice diff + cost
// ──────────────────────────────────────────────────────────────────────────

pub async fn run_voice_diff(
    client: &EnscriveClient,
    fmt: OutputFormat,
    sub: VoiceDiff2Subcommand,
) -> i32 {
    match sub {
        VoiceDiff2Subcommand::Diff(args) => {
            let path = format!("/v1/voices/{}/diff", args.id);
            let mut qs: Vec<(&str, String)> = vec![("against", args.against.to_string())];
            if let Some(from) = args.from {
                qs.push(("from", from.to_string()));
            }
            match client.get_json_with_query(&path, &qs).await {
                Ok(data) => CliResponse::success("voices diff", data).emit(fmt),
                Err(e) => request_failure("voices diff", e).emit(fmt),
            }
        }
        VoiceDiff2Subcommand::DiffCost(args) => {
            let path = format!("/v1/voices/{}/diff-cost", args.id);
            let qs = [
                ("against", args.against.to_string()),
                ("collection", args.collection.clone()),
                ("batch", args.batch.to_string()),
            ];
            match client.get_json_with_query(&path, &qs).await {
                Ok(data) => CliResponse::success("voices diff-cost", data).emit(fmt),
                Err(e) => request_failure("voices diff-cost", e).emit(fmt),
            }
        }
        VoiceDiff2Subcommand::DiffProposal(args) => {
            let bytes = match std::fs::read(&args.proposed_file) {
                Ok(b) => b,
                Err(e) => {
                    return CliResponse::fail(
                        "voices diff-proposal",
                        format!("read {}: {e}", args.proposed_file),
                        FailureClass::Bug,
                        EXIT_CONFIG,
                    )
                    .emit(fmt);
                }
            };
            let body: Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    return CliResponse::fail(
                        "voices diff-proposal",
                        format!("parse {} as JSON: {e}", args.proposed_file),
                        FailureClass::Bug,
                        EXIT_CONFIG,
                    )
                    .emit(fmt);
                }
            };
            let path = format!("/v1/voices/{}/diff-proposal", args.id);
            match client.post_json(&path, body).await {
                Ok(data) => CliResponse::success("voices diff-proposal", data).emit(fmt),
                Err(e) => request_failure("voices diff-proposal", e).emit(fmt),
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Error helpers — ENS-84: typed ApiError replaces string heuristics
// ──────────────────────────────────────────────────────────────────────────

fn request_failure(command: &'static str, e: ApiError) -> CliResponse {
    use crate::output::{EXIT_UNSUPPORTED, EXIT_PLAN_REQUIRED, EXIT_QUOTA_EXCEEDED,
                        EXIT_CONFIRMATION_REQUIRED, EXIT_LICENSE_INVALID};
    let (class, exit_code) = match &e {
        ApiError::NotYetAvailable { .. } => (FailureClass::Unsupported, EXIT_UNSUPPORTED),
        ApiError::Timeout | ApiError::Network(_) | ApiError::InvalidResponse { .. }
        | ApiError::Http4xx { .. } | ApiError::Http5xx { .. } => (FailureClass::Bug, EXIT_FAILURE),
        ApiError::ServerClassified { class, .. } => {
            let fc = match class.as_str() {
                "FAIL_BUG" => FailureClass::Bug,
                "FAIL_UNSUPPORTED" => FailureClass::Unsupported,
                "FAIL_UNSUPPORTED_IN_LOCAL_MODE" => FailureClass::UnsupportedInLocalMode,
                "FAIL_PLAN_REQUIRED" => FailureClass::PlanRequired,
                "FAIL_CONFIRMATION_REQUIRED" => FailureClass::ConfirmationRequired,
                "FAIL_QUOTA_EXCEEDED" => FailureClass::QuotaExceeded,
                "FAIL_LICENSE_INVALID" => FailureClass::LicenseInvalid,
                "FAIL_UNIMPLEMENTED" => FailureClass::Unimplemented,
                "FAIL_FALSE_CLAIM" => FailureClass::FalseClaim,
                _ => FailureClass::Bug,
            };
            let code = match fc {
                FailureClass::Unsupported | FailureClass::UnsupportedInLocalMode => EXIT_UNSUPPORTED,
                FailureClass::PlanRequired => EXIT_PLAN_REQUIRED,
                FailureClass::ConfirmationRequired => EXIT_CONFIRMATION_REQUIRED,
                FailureClass::QuotaExceeded => EXIT_QUOTA_EXCEEDED,
                FailureClass::LicenseInvalid => EXIT_LICENSE_INVALID,
                _ => EXIT_FAILURE,
            };
            (fc, code)
        }
    };
    CliResponse::fail(command, e.to_string(), class, exit_code)
}
