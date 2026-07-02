//! ENS-752: operator admin CLI commands.
//!
//! Thin wrappers over the `/v1/admin/*` operator surface in
//! enscrive-developer. Every endpoint here requires an API key with the
//! `Admin` capability (an operator/platform-admin scoped key, not a normal
//! tenant key) — see `admin_provisioning.rs`, `admin_audit.rs`,
//! `admin_incidents.rs`, `admin_migrations.rs`, `admin_telemetry.rs`,
//! `metering.rs`, `reconcile.rs`, and `backup_export/api.rs` in
//! enscrive-developer for the server-side handlers these wrap.
//!
//! Request/response field names are pinned to the exact server contracts
//! (read directly from the handler source, not guessed) so the CLI never
//! silently drops a field the server actually requires.

use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::client::EnscriveClient;
use crate::jobs_polling;
use crate::output::{
    CliResponse, FailureClass, OutputFormat, EXIT_CONFIG, EXIT_CONFIRMATION_REQUIRED, EXIT_FAILURE,
};

// ---------------------------------------------------------------------------
// Wallet — POST /v1/admin/wallets/credit
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AdminWalletSubcommand {
    /// Credit a tenant's wallet (operator top-up). `POST /v1/admin/wallets/credit`.
    ///
    /// Writes `wallet_transactions.transaction_type = 'admin_seed'`. Used by
    /// `enscrive-deploy` to fund administrative tenants (e.g. `enscrive-docs`)
    /// so their first ingest doesn't 402 against the entitlement gate.
    Credit(AdminWalletCreditArgs),
}

#[derive(Args)]
pub struct AdminWalletCreditArgs {
    /// Tenant UUID to credit.
    #[arg(long, required = true)]
    tenant: String,

    /// Amount to credit, in MICROS (1,000,000 micros = $1.00). Must be > 0.
    /// Sent verbatim as the server's exact-integer unit — no client-side
    /// dollar→micros float conversion (ENS-708 exact-ledger discipline).
    #[arg(long = "amount-micros", required = true)]
    amount_micros: i64,

    /// Operator-supplied justification. Required — the server rejects an
    /// empty reason (it lands on the durable admin_audit_log row).
    #[arg(long, required = true)]
    reason: String,

    /// Optional idempotency key. When supplied, a retried call with the
    /// same key collapses to a single credit instead of double-crediting.
    #[arg(long = "idempotency-key")]
    idempotency_key: Option<String>,
}

pub async fn run_wallet_credit(client: &EnscriveClient, fmt: OutputFormat, args: &AdminWalletCreditArgs) {
    let command = "admin wallet credit";

    if args.amount_micros <= 0 {
        CliResponse::fail(
            command,
            format!(
                "--amount-micros must be positive (got {}); the server refunds/adjustments use different transaction types",
                args.amount_micros
            ),
            FailureClass::Bug,
            EXIT_CONFIG,
        )
        .emit(fmt);
    }

    if args.reason.trim().is_empty() {
        CliResponse::fail(
            command,
            "--reason cannot be empty — the server requires a non-empty justification for the audit trail".to_string(),
            FailureClass::Bug,
            EXIT_CONFIG,
        )
        .emit(fmt);
    }

    let body = json!({
        "tenant_id": args.tenant,
        "amount_micros": args.amount_micros,
        "reason": args.reason,
        "idempotency_key": args.idempotency_key,
    });

    match client.post_json("/v1/admin/wallets/credit", body).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// Audit — GET /v1/admin/audit
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AdminAuditSubcommand {
    /// List durable admin audit-log entries (tenant/api-key/wallet-credit/
    /// tenant-erase mutations). `GET /v1/admin/audit`. Most-recent first.
    List(AdminAuditListArgs),
}

#[derive(Args)]
pub struct AdminAuditListArgs {
    /// ISO-8601 lower bound (inclusive) on created_at.
    #[arg(long)]
    since: Option<String>,

    /// ISO-8601 upper bound (exclusive) on created_at.
    #[arg(long)]
    until: Option<String>,

    /// Exact action-verb filter, e.g. "wallet.credit", "tenant.create",
    /// "api_key.create", "tenant.erase".
    #[arg(long)]
    action: Option<String>,

    /// Filter to audit rows recorded against this tenant (UUID).
    #[arg(long = "subject-tenant")]
    subject_tenant: Option<String>,

    /// Page size (server default 50, max 200).
    #[arg(long)]
    limit: Option<i64>,

    /// Offset for pagination (server default 0).
    #[arg(long)]
    offset: Option<i64>,
}

fn build_audit_list_query(args: &AdminAuditListArgs) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(v) = &args.since {
        query.push(("since", v.clone()));
    }
    if let Some(v) = &args.until {
        query.push(("until", v.clone()));
    }
    if let Some(v) = &args.action {
        query.push(("action", v.clone()));
    }
    if let Some(v) = &args.subject_tenant {
        query.push(("subject_tenant_id", v.clone()));
    }
    if let Some(v) = args.limit {
        query.push(("limit", v.to_string()));
    }
    if let Some(v) = args.offset {
        query.push(("offset", v.to_string()));
    }
    query
}

pub async fn run_audit_list(client: &EnscriveClient, fmt: OutputFormat, args: &AdminAuditListArgs) {
    let command = "admin audit list";
    let query = build_audit_list_query(args);
    match client.get_json_with_query("/v1/admin/audit", &query).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// Incidents — GET /v1/admin/incidents, GET /v1/admin/incidents/{id}
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AdminIncidentsSubcommand {
    /// List incidents (admin-scoped, cross-tenant). `GET /v1/admin/incidents`.
    List(AdminIncidentsListArgs),

    /// Get one incident's full detail (includes `body`).
    /// `GET /v1/admin/incidents/{id}`.
    Get(AdminIncidentsGetArgs),
}

#[derive(Args)]
pub struct AdminIncidentsListArgs {
    /// ISO-8601 lower bound (inclusive) on created_at.
    #[arg(long)]
    since: Option<String>,

    /// ISO-8601 upper bound (exclusive) on created_at.
    #[arg(long)]
    until: Option<String>,

    /// Severity filter: critical | high | medium | low.
    #[arg(long)]
    severity: Option<String>,

    /// Source prefix match (server does `LIKE '<source>%'`).
    #[arg(long)]
    source: Option<String>,

    /// Filter to a specific tenant UUID.
    #[arg(long)]
    tenant: Option<String>,

    /// Page size (server default 50, max 200).
    #[arg(long)]
    limit: Option<i64>,

    /// Offset for pagination (server default 0).
    #[arg(long)]
    offset: Option<i64>,
}

#[derive(Args)]
pub struct AdminIncidentsGetArgs {
    /// Incident UUID.
    id: String,
}

fn build_incidents_list_query(args: &AdminIncidentsListArgs) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(v) = &args.since {
        query.push(("since", v.clone()));
    }
    if let Some(v) = &args.until {
        query.push(("until", v.clone()));
    }
    if let Some(v) = &args.severity {
        query.push(("severity", v.clone()));
    }
    if let Some(v) = &args.source {
        query.push(("source", v.clone()));
    }
    if let Some(v) = &args.tenant {
        query.push(("tenant_id", v.clone()));
    }
    if let Some(v) = args.limit {
        query.push(("limit", v.to_string()));
    }
    if let Some(v) = args.offset {
        query.push(("offset", v.to_string()));
    }
    query
}

pub async fn run_incidents_list(client: &EnscriveClient, fmt: OutputFormat, args: &AdminIncidentsListArgs) {
    let command = "admin incidents list";
    let query = build_incidents_list_query(args);
    match client.get_json_with_query("/v1/admin/incidents", &query).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

pub async fn run_incidents_get(client: &EnscriveClient, fmt: OutputFormat, args: &AdminIncidentsGetArgs) {
    let command = "admin incidents get";
    let path = format!("/v1/admin/incidents/{}", args.id);
    match client.get_json(&path).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// Migrations — GET /v1/admin/migrations
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AdminMigrationsSubcommand {
    /// Report applied vs pending vs failed sqlx migrations.
    /// `GET /v1/admin/migrations`.
    Status,
}

pub async fn run_migrations_status(client: &EnscriveClient, fmt: OutputFormat) {
    let command = "admin migrations status";
    match client.get_json("/v1/admin/migrations").await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// Telemetry — GET /v1/admin/telemetry/stats
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AdminTelemetrySubcommand {
    /// Aggregate-only wallet + incident + six-sigma counters for the whole
    /// stack. `GET /v1/admin/telemetry/stats`.
    Stats,
}

pub async fn run_telemetry_stats(client: &EnscriveClient, fmt: OutputFormat) {
    let command = "admin telemetry stats";
    match client.get_json("/v1/admin/telemetry/stats").await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// Metering — POST /v1/admin/metering/backfill
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AdminMeteringSubcommand {
    /// One-shot Loki -> metering_events backfill. `POST
    /// /v1/admin/metering/backfill`. The handler reads its parameters from
    /// the URL query string (not a JSON body) — this command sends them
    /// that way.
    Backfill(AdminMeteringBackfillArgs),
}

#[derive(Args)]
pub struct AdminMeteringBackfillArgs {
    /// RFC3339 inclusive lower bound for occurred_at.
    #[arg(long, required = true)]
    start: String,

    /// RFC3339 exclusive upper bound for occurred_at.
    #[arg(long, required = true)]
    end: String,

    /// Optional single-tenant scope (UUID). Omit to scan every tenant.
    #[arg(long)]
    tenant: Option<String>,

    /// Scan + synthesize rows but skip the INSERT (counts still reflect
    /// what would have been written).
    #[arg(long = "dry-run", default_value_t = false)]
    dry_run: bool,
}

pub async fn run_metering_backfill(client: &EnscriveClient, fmt: OutputFormat, args: &AdminMeteringBackfillArgs) {
    let command = "admin metering backfill";

    let mut query: Vec<(&'static str, String)> = vec![
        ("start", args.start.clone()),
        ("end", args.end.clone()),
    ];
    if args.dry_run {
        query.push(("dry_run", "true".to_string()));
    }
    if let Some(v) = &args.tenant {
        query.push(("tenant_id", v.clone()));
    }

    match client
        .post_json_with_query("/v1/admin/metering/backfill", &query)
        .await
    {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// Tenants — POST /v1/admin/tenants, POST /v1/admin/tenants/erase
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AdminTenantsSubcommand {
    /// Create (or idempotently return) an administrative tenant.
    /// `POST /v1/admin/tenants`.
    Create(AdminTenantsCreateArgs),

    /// PERMANENTLY erase a tenant's backups (GDPR Article 17): tombstones
    /// the tenant, deletes catalog artifacts + ledger rows, and erases
    /// substrate-side backups. Irreversible. `POST /v1/admin/tenants/erase`.
    Erase(AdminTenantsEraseArgs),
}

#[derive(Args)]
pub struct AdminTenantsCreateArgs {
    /// Tenant name. Idempotent — re-calling with the same name returns the
    /// existing administrative tenant (`was_created: false` in the response).
    #[arg(long, required = true)]
    name: String,
}

pub async fn run_tenants_create(client: &EnscriveClient, fmt: OutputFormat, args: &AdminTenantsCreateArgs) {
    let command = "admin tenants create";
    let body = json!({ "name": args.name });
    match client.post_json("/v1/admin/tenants", body).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

#[derive(Args)]
pub struct AdminTenantsEraseArgs {
    /// Tenant UUID to erase. DESTRUCTIVE and IRREVERSIBLE.
    #[arg(long, required = true)]
    tenant: String,

    /// Must exactly repeat --tenant's value. Defense-in-depth client-side
    /// check mirroring the same field the server itself requires (the
    /// server's own confirm gate is authoritative; this just fails fast
    /// before a network round-trip on a typo).
    #[arg(long, required = true)]
    confirm: String,

    /// Operator justification; lands on the tombstone row. Server default:
    /// "erasure_requested" when omitted.
    #[arg(long)]
    reason: Option<String>,
}

/// Pure defense-in-depth check: `--confirm` must exactly repeat the target
/// id before an irreversible/cross-tenant admin mutation is sent. The
/// server enforces the identical invariant (same field name); this just
/// fails fast client-side on a typo before a network round-trip.
fn require_confirm_matches(confirm: &str, target: &str) -> Result<(), String> {
    if confirm != target {
        Err(format!(
            "--confirm ({confirm}) must exactly match the target id ({target})"
        ))
    } else {
        Ok(())
    }
}

/// Shared exit path for a failed `require_confirm_matches` check: emits and
/// exits (never returns). Keeps the fail-construction identical across every
/// confirm-gated admin mutation.
fn fail_confirm_mismatch(command: &str, err: String, fmt: OutputFormat) -> ! {
    CliResponse::fail(command, err, FailureClass::ConfirmationRequired, EXIT_CONFIRMATION_REQUIRED)
        .emit(fmt);
}

pub async fn run_tenants_erase(client: &EnscriveClient, fmt: OutputFormat, args: &AdminTenantsEraseArgs) {
    let command = "admin tenants erase";

    if let Err(e) = require_confirm_matches(&args.confirm, &args.tenant) {
        fail_confirm_mismatch(
            command,
            format!("{e}; refusing to send an irreversible tenant erasure with a mismatched confirmation"),
            fmt,
        );
    }

    let body = json!({
        "tenant_id": args.tenant,
        "confirm": args.confirm,
        "reason": args.reason,
    });

    match client.post_json("/v1/admin/tenants/erase", body).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// API keys — POST /v1/admin/api-keys
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AdminApiKeysSubcommand {
    /// Mint an API key for a (tenant, environment) pair. Cross-tenant —
    /// the caller's Admin capability is the auth gate, the target is named
    /// explicitly in the body. `POST /v1/admin/api-keys`.
    Create(AdminApiKeysCreateArgs),
}

#[derive(Args)]
pub struct AdminApiKeysCreateArgs {
    /// Target tenant UUID.
    #[arg(long, required = true)]
    tenant: String,

    /// Target environment UUID. Must belong to --tenant (the server
    /// pre-validates this and 400s otherwise).
    #[arg(long = "environment", required = true)]
    environment: String,

    /// Human-readable label for the minted key.
    #[arg(long, required = true)]
    label: String,

    /// Key scope: tenant | operator | platform_admin. Server default: tenant.
    #[arg(long)]
    scope: Option<String>,

    /// Comma-separated capability list (e.g. "search,records,admin"). Empty
    /// means "apply the default capabilities for the scope" server-side.
    #[arg(long, value_delimiter = ',')]
    capabilities: Vec<String>,

    /// Atomically revoke prior active keys on (tenant, label) before
    /// minting the new one. Used to rotate a sidecar key without
    /// accumulating dead rows.
    #[arg(long = "revoke-existing-with-label", default_value_t = false)]
    revoke_existing_with_label: bool,
}

pub async fn run_api_keys_create(client: &EnscriveClient, fmt: OutputFormat, args: &AdminApiKeysCreateArgs) {
    let command = "admin api-keys create";
    let body = json!({
        "tenant_id": args.tenant,
        "environment_id": args.environment,
        "label": args.label,
        "scope": args.scope,
        "capabilities": args.capabilities,
        "revoke_existing_with_label": args.revoke_existing_with_label,
    });
    match client.post_json("/v1/admin/api-keys", body).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// Catalog import — POST /v1/admin/catalog-import
// ---------------------------------------------------------------------------

/// NOTE: the ticket's suggested shape (`--file <PATH>`) does not match the
/// real handler (`backup_export::api::import_catalog`), which takes no file
/// path at all — the artifact is located server-side from the
/// `catalog_exports` ledger (or `--ts` to pin a specific generation). Args
/// below match the actual `CatalogImportRequest` wire shape.
#[derive(Args)]
pub struct AdminCatalogImportArgs {
    /// Tenant UUID whose catalog to import.
    #[arg(long, required = true)]
    tenant: String,

    /// Must exactly repeat --tenant's value (server confirm-gate; same
    /// field name the server itself uses).
    #[arg(long, required = true)]
    confirm: String,

    /// Artifact generation (compact UTC timestamp, e.g. `20260610T120000Z`).
    /// Omit to use the latest `catalog_exports` ledger row for the tenant.
    #[arg(long)]
    ts: Option<String>,

    /// DISASTER-RECOVERY ONLY: import an artifact with no matching
    /// `catalog_exports` ledger row (integrity then rests on the manifest
    /// alone). Use only when the ledger itself is gone.
    #[arg(long = "allow-unledgered", default_value_t = false)]
    allow_unledgered: bool,
}

pub async fn run_catalog_import(client: &EnscriveClient, fmt: OutputFormat, args: &AdminCatalogImportArgs) {
    let command = "admin catalog-import";

    if let Err(e) = require_confirm_matches(&args.confirm, &args.tenant) {
        fail_confirm_mismatch(command, e, fmt);
    }

    let body = json!({
        "tenant_id": args.tenant,
        "ts": args.ts,
        "confirm": args.confirm,
        "allow_unledgered": args.allow_unledgered,
    });

    match client.post_json("/v1/admin/catalog-import", body).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// Corpora reconcile — POST /v1/admin/corpora/{id}/reconcile (always-async)
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum AdminCorporaSubcommand {
    /// Repair a corpus whose catalog is complete but whose substrate
    /// (vector storage) is partial, by re-embedding exactly the missing
    /// chunks. ALWAYS-ASYNC: `POST /v1/admin/corpora/{id}/reconcile` returns
    /// 202 + a job id; this command polls to terminal by default.
    Reconcile(AdminCorporaReconcileArgs),
}

#[derive(Args)]
pub struct AdminCorporaReconcileArgs {
    /// Corpus UUID to reconcile.
    corpus_id: String,

    /// Return immediately with the launch response instead of polling to
    /// terminal status. The job continues server-side; poll with
    /// `enscrive jobs get --id <job_id>`.
    #[arg(long = "async", default_value_t = false)]
    r#async: bool,

    /// Poll timeout for the wait path. Ignored when --async is set.
    #[arg(long = "timeout-secs", default_value_t = 1800)]
    timeout_secs: u64,
}

pub async fn run_corpora_reconcile(client: &EnscriveClient, fmt: OutputFormat, args: &AdminCorporaReconcileArgs) {
    let command = "admin corpora reconcile";
    let path = format!("/v1/admin/corpora/{}/reconcile", args.corpus_id);

    let launch = match client.post_json(&path, json!({})).await {
        Ok(v) => v,
        Err(e) => crate::request_failure(command, e).emit(fmt),
    };

    let job_id = match launch.get("job_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => CliResponse::fail(
            command,
            format!("launch response missing job_id: {launch}"),
            FailureClass::FalseClaim,
            EXIT_FAILURE,
        )
        .emit(fmt),
    };

    if args.r#async {
        CliResponse::success(command, launch).emit(fmt);
    }

    jobs_polling::await_and_emit_launch_job(client, command, launch, &job_id, args.timeout_secs, fmt).await
}

// ---------------------------------------------------------------------------
// Tests — pure logic only (query builders, confirm gate). CLI parse tests
// and process-exiting paths live in main.rs's test module.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn confirm_matches_accepts_exact_match() {
        assert!(require_confirm_matches("t-1", "t-1").is_ok());
    }

    #[test]
    fn confirm_matches_rejects_mismatch() {
        let err = require_confirm_matches("t-2", "t-1").expect_err("mismatch must be rejected");
        assert!(err.contains("t-2"));
        assert!(err.contains("t-1"));
    }

    #[test]
    fn audit_list_query_empty_when_no_filters() {
        let args = AdminAuditListArgs {
            since: None,
            until: None,
            action: None,
            subject_tenant: None,
            limit: None,
            offset: None,
        };
        assert!(build_audit_list_query(&args).is_empty());
    }

    #[test]
    fn audit_list_query_includes_requested_filters() {
        let args = AdminAuditListArgs {
            since: Some("2026-01-01T00:00:00Z".to_string()),
            until: Some("2026-02-01T00:00:00Z".to_string()),
            action: Some("wallet.credit".to_string()),
            subject_tenant: Some("11111111-1111-1111-1111-111111111111".to_string()),
            limit: Some(25),
            offset: Some(10),
        };
        let query = build_audit_list_query(&args);
        assert_eq!(
            query,
            vec![
                ("since", "2026-01-01T00:00:00Z".to_string()),
                ("until", "2026-02-01T00:00:00Z".to_string()),
                ("action", "wallet.credit".to_string()),
                (
                    "subject_tenant_id",
                    "11111111-1111-1111-1111-111111111111".to_string()
                ),
                ("limit", "25".to_string()),
                ("offset", "10".to_string()),
            ]
        );
    }

    // -- clap parse tests: pin flag names / positional wiring -------------

    #[test]
    fn parse_admin_wallet_credit() {
        let args = crate::Cli::parse_from([
            "enscrive",
            "admin",
            "wallet",
            "credit",
            "--tenant",
            "11111111-1111-1111-1111-111111111111",
            "--amount-micros",
            "5000000",
            "--reason",
            "seed docs sidecar",
            "--idempotency-key",
            "deploy-run-42",
        ]);
        match args.command {
            crate::Commands::Admin {
                sub: crate::AdminSubcommand::Wallet {
                    sub: AdminWalletSubcommand::Credit(AdminWalletCreditArgs {
                        tenant,
                        amount_micros,
                        reason,
                        idempotency_key,
                    }),
                },
            } => {
                assert_eq!(tenant, "11111111-1111-1111-1111-111111111111");
                assert_eq!(amount_micros, 5_000_000);
                assert_eq!(reason, "seed docs sidecar");
                assert_eq!(idempotency_key.as_deref(), Some("deploy-run-42"));
            }
            _ => panic!("expected admin wallet credit"),
        }
    }

    #[test]
    fn parse_admin_tenants_erase() {
        let args = crate::Cli::parse_from([
            "enscrive",
            "admin",
            "tenants",
            "erase",
            "--tenant",
            "t-1",
            "--confirm",
            "t-1",
            "--reason",
            "gdpr request",
        ]);
        match args.command {
            crate::Commands::Admin {
                sub: crate::AdminSubcommand::Tenants {
                    sub: AdminTenantsSubcommand::Erase(AdminTenantsEraseArgs {
                        tenant,
                        confirm,
                        reason,
                    }),
                },
            } => {
                assert_eq!(tenant, "t-1");
                assert_eq!(confirm, "t-1");
                assert_eq!(reason.as_deref(), Some("gdpr request"));
            }
            _ => panic!("expected admin tenants erase"),
        }
    }

    #[test]
    fn parse_admin_api_keys_create_with_capabilities() {
        let args = crate::Cli::parse_from([
            "enscrive",
            "admin",
            "api-keys",
            "create",
            "--tenant",
            "t-1",
            "--environment",
            "e-1",
            "--label",
            "docs-sidecar",
            "--capabilities",
            "search,records",
            "--revoke-existing-with-label",
        ]);
        match args.command {
            crate::Commands::Admin {
                sub: crate::AdminSubcommand::ApiKeys {
                    sub: AdminApiKeysSubcommand::Create(AdminApiKeysCreateArgs {
                        tenant,
                        environment,
                        label,
                        scope,
                        capabilities,
                        revoke_existing_with_label,
                    }),
                },
            } => {
                assert_eq!(tenant, "t-1");
                assert_eq!(environment, "e-1");
                assert_eq!(label, "docs-sidecar");
                assert_eq!(scope, None);
                assert_eq!(capabilities, vec!["search".to_string(), "records".to_string()]);
                assert!(revoke_existing_with_label);
            }
            _ => panic!("expected admin api-keys create"),
        }
    }

    #[test]
    fn parse_admin_metering_backfill_dry_run() {
        let args = crate::Cli::parse_from([
            "enscrive",
            "admin",
            "metering",
            "backfill",
            "--start",
            "2026-01-01T00:00:00Z",
            "--end",
            "2026-01-02T00:00:00Z",
            "--dry-run",
        ]);
        match args.command {
            crate::Commands::Admin {
                sub: crate::AdminSubcommand::Metering {
                    sub: AdminMeteringSubcommand::Backfill(AdminMeteringBackfillArgs {
                        start,
                        end,
                        tenant,
                        dry_run,
                    }),
                },
            } => {
                assert_eq!(start, "2026-01-01T00:00:00Z");
                assert_eq!(end, "2026-01-02T00:00:00Z");
                assert_eq!(tenant, None);
                assert!(dry_run);
            }
            _ => panic!("expected admin metering backfill"),
        }
    }

    #[test]
    fn parse_admin_corpora_reconcile_positional_and_async() {
        let args = crate::Cli::parse_from([
            "enscrive",
            "admin",
            "corpora",
            "reconcile",
            "78c512ef-0000-0000-0000-000000000000",
            "--async",
        ]);
        match args.command {
            crate::Commands::Admin {
                sub: crate::AdminSubcommand::Corpora {
                    sub: AdminCorporaSubcommand::Reconcile(AdminCorporaReconcileArgs {
                        corpus_id,
                        r#async,
                        timeout_secs,
                    }),
                },
            } => {
                assert_eq!(corpus_id, "78c512ef-0000-0000-0000-000000000000");
                assert!(r#async);
                assert_eq!(timeout_secs, 1800);
            }
            _ => panic!("expected admin corpora reconcile"),
        }
    }

    #[test]
    fn parse_admin_catalog_import() {
        let args = crate::Cli::parse_from([
            "enscrive",
            "admin",
            "catalog-import",
            "--tenant",
            "t-1",
            "--confirm",
            "t-1",
            "--ts",
            "20260610T120000Z",
            "--allow-unledgered",
        ]);
        match args.command {
            crate::Commands::Admin {
                sub: crate::AdminSubcommand::CatalogImport(AdminCatalogImportArgs {
                    tenant,
                    confirm,
                    ts,
                    allow_unledgered,
                }),
            } => {
                assert_eq!(tenant, "t-1");
                assert_eq!(confirm, "t-1");
                assert_eq!(ts.as_deref(), Some("20260610T120000Z"));
                assert!(allow_unledgered);
            }
            _ => panic!("expected admin catalog-import"),
        }
    }

    #[test]
    fn incidents_list_query_includes_requested_filters() {
        let args = AdminIncidentsListArgs {
            since: None,
            until: None,
            severity: Some("critical".to_string()),
            source: Some("embed".to_string()),
            tenant: Some("22222222-2222-2222-2222-222222222222".to_string()),
            limit: Some(50),
            offset: None,
        };
        let query = build_incidents_list_query(&args);
        assert_eq!(
            query,
            vec![
                ("severity", "critical".to_string()),
                ("source", "embed".to_string()),
                (
                    "tenant_id",
                    "22222222-2222-2222-2222-222222222222".to_string()
                ),
                ("limit", "50".to_string()),
            ]
        );
    }
}
