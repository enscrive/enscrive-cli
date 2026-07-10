# `enscrive-cli` — Command ↔ `/v1` parity table

[← back to STATE-OF-CLI](./STATE-OF-CLI.md)

**Auto-derived from [`v1-surface-contract.toml`](../v1-surface-contract.toml)** — the
CI-checked source of truth (`tests/surface_contract.rs` +
`command_tiers_covers_every_leaf_subcommand`). Regenerate with
`python3 scripts/gen_parity_table.py`; do not hand-edit rows. Last reconciled:
**2026-07-10** (CLI 100%-parity workstream).

**Totals: 147 endpoints — 141 `implemented`, 6 `deferred`, 0 `missing`.** Tiers: 142 any-mode / 5 managed-only. Plans: 66 free / 61 professional / 20 enterprise.

Status: ✅ implemented (CLI command exists + wired to this endpoint) · ⛔ deferred (explicit `reason` in the contract; not silently missing).

---

## Search

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `search` | POST `/v1/search` | ✅ | free |  |
| `embeddings query` | POST `/v1/query-embeddings` | ✅ | free |  |

## Corpora

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `corpus list` | GET `/v1/corpora` | ✅ | free |  |
| `corpus create` | POST `/v1/corpora` | ✅ | free |  |
| `corpus get` | GET `/v1/corpora/{id}` | ✅ | free | J-004c — detail route exposing a single corpus |
| `corpus update` | PATCH `/v1/corpora/{id}` | ✅ | free |  |
| `corpus delete` | DELETE `/v1/corpora/{id}` | ✅ | free |  |
| `corpus stats` | GET `/v1/corpora/{id}/stats` | ✅ | free |  |
| `corpus documents` | GET `/v1/corpora/{id}/documents` | ✅ | free |  |
| `corpus chunks` | GET `/v1/corpora/{id}/documents/{doc_id}/chunks` | ✅ | free |  |

## Corpus Staging

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `corpus stage` | POST `/v1/corpora/{id}/stage` | ✅ | free | P0 gap closure — client method added this session |
| `corpus commit` | POST `/v1/corpora/{id}/commit` | ✅ | free | P0 gap closure — client method added this session |
| `corpus pending` | GET `/v1/corpora/{id}/pending` | ✅ | free | P0 gap closure — client method added this session |
| `corpus pending-delete` | DELETE `/v1/corpora/{id}/pending/{doc_id}` | ✅ | free | P0 gap closure — client method added this session |
| `corpus revert` | POST `/v1/corpora/{id}/revert` | ✅ | free | J-004c — revert uncommitted pending changes |
| `corpus commits` | GET `/v1/corpora/{id}/commits` | ✅ | free | J-004c — commit history for the corpus |
| `corpus metrics` | GET `/v1/corpora/{id}/metrics` | ✅ | free | J-020 — vector-space metrics (density, diversity, drift) |
| `corpus materialize-from-dataset` | POST `/v1/corpora/materialize-from-dataset` | ✅ | free | ENS-104 — materialize a corpus from a dataset URL (combines steps 2+4 of the 5-step eval workflow; conflates create + populate) |
| `corpus populate-from-dataset` | POST `/v1/corpora/{id}/populate-from-dataset` | ✅ | free | ENS-133 (parent ENS-132) — Step-4 primitive of the canonical 5-step eval workflow. Populates an existing empty corpus with a dataset's corpus, chunked by the supplied voice. Returns 409 if corpus already has documents. |
| `corpus promote` | POST `/v1/corpora/{id}/promote` | ✅ | professional | WS-45 / ADR CORPUS-ENV-PROMOTION-2026-06-27 — promote a corpus into another environment; MultiEnv (Pro+) entitlement; target env must belong to the same tenant and differ from the source (400 on same-env) |

## Ingestion

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `ingest documents` | POST `/v1/ingest` | ✅ | free | P0 gap closure — client method added this session |
| `ingest vectors` | POST `/v1/ingest-vectors` | ⛔ deferred | enterprise | Admin + ENSCRIVE_VECTOR_PASSTHROUGH_ENABLED test-gated operator surface; driven by the LD-0 load driver, not the CLI (founder-ratified deferred 2026-07-10). No CLI command by design. |
| `ingest prepared` | POST `/v1/ingest-prepared` | ✅ | free |  |
| `segment document` | POST `/v1/segment-document` | ✅ | free |  |
| `preview-chunking` | POST `/v1/preview-chunking` | ✅ | free | ENS-752 — local-only preview, no ingest/storage/metering |
| `preview-with-template` | POST `/v1/preview-with-template` | ✅ | professional | ENS-752 |

## Jobs

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `jobs list` | GET `/v1/jobs` | ✅ | free | P0 gap closure — client method added this session |
| `jobs get` | GET `/v1/jobs/{id}` | ✅ | free | P0 gap closure — client method added this session |
| `jobs cancel` | POST `/v1/jobs/{id}/cancel` | ✅ | free | P0 gap closure — client method added this session |
| `jobs retry` | POST `/v1/jobs/{id}/retry` | ✅ | free | J-024 Unit 3 Concern 6 — requeue a failed job |
| `jobs abandon` | POST `/v1/jobs/{id}/abandon` | ✅ | free | J-024 Unit 3 Concern 6 — mark a failed job as abandoned |

## Batch Sets

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `batch-sets list` | GET `/v1/corpora/{id}/batch-sets` | ✅ | free | J-024 Unit 5 — list batch sets for a corpus |
| `batch-sets get` | GET `/v1/batch-sets/{id}` | ✅ | free | J-024 Unit 5 — batch-set detail |
| `batch-sets retry` | POST `/v1/batch-sets/{id}/retry` | ✅ | free | retry a failed_recoverable batch-set (invalid in other states) |
| `batch-sets abandon` | POST `/v1/batch-sets/{id}/abandon` | ✅ | free | abandon a batch-set (invalid for terminal committed/abandoned states) |

## Content Analysis

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `analyze content` | POST `/v1/analyze-content` | ✅ | free |  |

## Models

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `models list` | GET `/v1/models` | ✅ | free |  |
| `models show` | GET `/v1/models/{provider}/{model}` | ✅ | free |  |

## Voices

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `voices list` | GET `/v1/voices` | ✅ | free |  |
| `voices create` | POST `/v1/voices` | ✅ | free |  |
| `voices get` | GET `/v1/voices/{id}` | ✅ | free |  |
| `voices update` | PUT `/v1/voices/{id}` | ✅ | free | PUT /v1/voices/{id} — live with a corpus-invalidation confirm gate (--confirm-re-embed). Contract-truth fix 2026-07-10: was mislabeled deferred; the command has shipped. |
| `voices delete` | DELETE `/v1/voices/{id}` | ✅ | free |  |
| `voices compare` | POST `/v1/voices/compare` | ✅ | free |  |
| `voices search` | POST `/v1/voices/search` | ✅ | free |  |

## Voice Versions

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `voices versions list` | GET `/v1/voices/{id}/versions` | ⛔ deferred | professional | Voice version history is a post-launch audit feature; requires version-aware voice model |
| `voices versions get` | GET `/v1/voices/{id}/versions/{version}` | ⛔ deferred | professional | Voice version history is a post-launch audit feature; requires version-aware voice model |

## Voice Promotion & Gates

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `voices promote` | POST `/v1/voices/{id}/promote` | ✅ | professional |  |
| `voices gates list` | GET `/v1/voices/{id}/gates` | ✅ | professional |  |
| `voices gates set` | POST `/v1/voices/{id}/gates` | ✅ | professional |  |
| `voices gates delete` | DELETE `/v1/voices/{id}/gates/{metric}` | ✅ | professional |  |

## Voice Diff & Cost (EV-011 / EV-012)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `voices diff` | GET `/v1/voices/{id}/diff` | ✅ | professional | EV-011 — diff a voice against an earlier version |
| `voices diff-cost` | GET `/v1/voices/{id}/diff-cost` | ✅ | professional | EV-012 — estimate money + time cost of applying the diff |
| `voices diff-proposal` | POST `/v1/voices/{id}/diff-proposal` | ✅ | professional | EV-011 — diff live voice against a proposed config |

## Segmentation Templates

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `segmentation-templates list` | GET `/v1/segmentation-templates` | ✅ | professional | ENS-752 |
| `segmentation-templates create` | POST `/v1/segmentation-templates` | ✅ | professional | ENS-752 |
| `segmentation-templates get` | GET `/v1/segmentation-templates/{id}` | ✅ | professional | ENS-752 |
| `segmentation-templates update` | PUT `/v1/segmentation-templates/{id}` | ✅ | professional | ENS-752 |
| `segmentation-templates delete` | DELETE `/v1/segmentation-templates/{id}` | ✅ | professional | ENS-752 |
| `segmentation-templates clone` | POST `/v1/segmentation-templates/{id}/clone` | ✅ | professional | ENS-752 |

## Evals: Datasets

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `evals datasets list` | GET `/v1/evals/datasets` | ✅ | professional |  |
| `evals datasets create` | POST `/v1/evals/datasets` | ✅ | professional |  |
| `evals datasets get` | GET `/v1/evals/datasets/{id}` | ✅ | professional |  |
| `evals datasets update` | PUT `/v1/evals/datasets/{id}` | ✅ | professional |  |
| `evals datasets delete` | DELETE `/v1/evals/datasets/{id}` | ✅ | professional |  |
| `evals datasets promote` | POST `/v1/evals/datasets/{id}/promote` | ✅ | professional | promote an eval dataset into another environment; target env must belong to the same tenant and differ from the source (400 on same-env) |
| `evals datasets queries` | GET `/v1/evals/datasets/{id}/queries` | ✅ | professional |  |

## Evals: Campaigns

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `evals campaigns list` | GET `/v1/evals/campaigns` | ✅ | professional |  |
| `evals campaigns get` | GET `/v1/evals/{id}` | ✅ | professional |  |
| `evals campaigns promote` | POST `/v1/evals/{id}/promote` | ✅ | professional | promote an eval campaign into another environment; target env must belong to the same tenant and differ from the source (400 on same-env) |
| `evals run-campaign` | POST `/v1/evals/run-campaign` | ✅ | professional |  |
| `evals run-campaign-stream` | POST `/v1/evals/run-campaign-stream` | ✅ | professional |  |
| `evals import` | POST `/v1/evals/import` | ⛔ deferred | professional | Server-side BEIR importer. The CLI `evals import` command is a client-side composite (local BEIR parse -> POST /v1/evals/datasets + optional POST /v1/ingest), already covered by those endpoints' rows; the raw server importer has no direct CLI caller. Contract-truth reconcile 2026-07-10 (was mislabeled implemented against `evals import`, which does not call this path). Promote to implemented if an `evals import --server-side` variant is ever added. |

## Evals: Advanced

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `evals voice-status` | GET `/v1/evals/voice-status/{voice_id}` | ✅ | professional |  |
| `evals convergence` | GET `/v1/evals/convergence` | ⛔ deferred | professional | Convergence analysis requires multi-campaign history; deferred until eval maturity milestone |
| `evals from-url` | POST `/v1/evals/from-url` | ✅ | professional | Gap1 W-003 — wraps POST /v1/evals/from-url + polls /v1/jobs/{id} until terminal |

## Evals 2.0: Datasets (EV-013)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `datasets list` | GET `/v1/datasets` | ✅ | professional | EV-013 — Evals 2.0 dataset registry (distinct from legacy /v1/evals/datasets) |
| `datasets create` | POST `/v1/datasets` | ✅ | professional | EV-013 Sprint B — create dataset from HuggingFace BeIR URL |
| `datasets get` | GET `/v1/datasets/{id}` | ✅ | professional | EV-013 — Evals 2.0 dataset by id |
| `datasets describe` | GET `/v1/datasets/{id}/describe` | ✅ | professional | EV-013 — structured dataset summary for agents |
| `datasets delete` | DELETE `/v1/datasets/{id}` | ✅ | professional | EV-013 — archive (hard-delete) a dataset |
| `datasets expand` | POST `/v1/datasets/{id}/expand` | ⛔ deferred | professional | server-side Slice-2 stub (501). A CLI command against a 501 can't green-light; flips to in-scope when the server implements expansion (founder-ratified deferred 2026-07-10). |
| `datasets upload` | POST `/v1/datasets/upload` | ✅ | professional | EV-013 — multipart upload of BeIR-layout corpus + queries + qrels |

## Evals 2.0: Eval Definitions (EV-013)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `eval-defs list` | GET `/v1/eval-defs` | ✅ | professional | EV-013 — list eval definitions for tenant + environment |
| `eval-defs create` | POST `/v1/eval-defs` | ✅ | professional | EV-013 — create a new eval definition |
| `eval-defs get` | GET `/v1/eval-defs/{id}` | ✅ | professional | EV-013 — fetch a single eval definition |
| `eval-defs delete` | DELETE `/v1/eval-defs/{id}` | ✅ | professional | EV-013 — soft-archive an eval definition |
| `eval-defs run` | POST `/v1/eval-defs/{id}/runs` | ✅ | professional | EV-013 — trigger a run and poll until terminal |
| `eval-runs list` | GET `/v1/eval-defs/{id}/runs` | ✅ | professional | EV-013 — list all runs for an eval definition |

## Evals 2.0: Eval Runs (EV-013)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `eval-runs get` | GET `/v1/eval-runs/{id}` | ✅ | professional | EV-013 — aggregate metrics + status for a single run |
| `eval-runs diagnose` | GET `/v1/eval-runs/{id}/queries` | ✅ | professional | EV-013 — per-query details sorted worst-first (diagnose view) |

## Evals 2.0: Publications (EV-017)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `eval-defs publish` | POST `/v1/eval-defs/{id}/publish` | ✅ | professional | EV-017 — publish a completed full-scope run as canonical |
| `eval-defs publications` | GET `/v1/eval-defs/{id}/publications` | ✅ | professional | EV-017 — list active publications for an eval |
| `eval-defs unpublish` | DELETE `/v1/eval-publications/{id}` | ✅ | professional | EV-017 — unpublish (soft delete, audit row remains) |

## Logs & Observability

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `logs stream` | GET `/v1/logs/stream` | ✅ | free |  |
| `logs search` | POST `/v1/logs/search` | ✅ | professional |  |
| `logs metrics` | POST `/v1/logs/metrics` | ✅ | professional |  |

## Usage

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `usage` | GET `/v1/usage` | ✅ | free |  |

## Wallet (ENS-476 / ENS-750)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `wallet balance` | GET `/v1/wallet/balance` | ✅ | free | API-key mirror of the portal wallet balance read; caller's own tenant only |
| `wallet debits` | GET `/v1/wallet/debits` | ✅ | free | Debit ledger history (debit_request/debit_disk_rent); paginates via next_page_token |

## Admin: Export

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `export tenant` | GET `/v1/admin/export` | ✅ | professional |  |
| `export embeddings` | GET `/v1/admin/export/embeddings` | ✅ | professional |  |
| `export token-usage` | GET `/v1/admin/export/token-usage` | ✅ | professional |  |

## Admin: Backup & Restore

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `backup list` | GET `/v1/admin/backups` | ✅ | professional |  |
| `backup create` | POST `/v1/admin/backups` | ✅ | professional |  |
| `backup get` | GET `/v1/admin/backups/{backup_id}` | ✅ | professional |  |
| `backup restore` | POST `/v1/admin/restore` | ✅ | professional |  |
| `backup dry-run` | POST `/v1/admin/restore/dry-run` | ✅ | professional |  |

## Revisions: tenant backup evidence + restore (ENS-651 / ENS-649)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `revisions list` | GET `/v1/backups` | ✅ | free |  |
| `revisions show` | GET `/v1/backups/{backup_id}` | ✅ | free |  |
| `restore` | POST `/v1/restore` | ✅ | free |  |
| `restore` | POST `/v1/restore/dry-run` | ✅ | free | Reached via `enscrive restore --revision <id> --dry-run` |

## Admin: Rate Limits

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `admin rate-limits show` | GET `/v1/rate-limits` | ✅ | free | Caller's own tenant rate-limit view; cross-tenant admin read deferred |
| `admin rate-limits set` | PATCH `/v1/admin/rate-limits/{tenant}/{provider}` | ✅ | enterprise | Cross-tenant rate-limit override — managed-mode operator tooling only |
| `admin api-rate-limits list` | GET `/v1/admin/api-rate-limits/{tenant_id}` | ✅ | enterprise | ENS-782 — list a tenant's inbound /v1 edge rate-limit overrides. Admin capability; distinct from the provider governor above |
| `admin api-rate-limits set` | PATCH `/v1/admin/api-rate-limits/{tenant_id}/{category}` | ✅ | enterprise | ENS-782 — upsert a per-tenant, per-category inbound rate-limit override (requests_per_minute >= 1). Categories: search\|query_embeddings\|ingest\|corpus_crud\|voice_crud\|preview_chunking |
| `admin api-rate-limits delete` | DELETE `/v1/admin/api-rate-limits/{tenant_id}/{category}` | ✅ | enterprise | ENS-782 — remove a per-tenant override, reverting to service default (204; 404 when absent) |

## Admin: Rate Card

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `admin ratecard apply` | POST `/v1/admin/ratecard/apply` | ✅ | enterprise | ENS-486 / ENS-175 — parses+validates a TOML file and applies it as the new active rate card; requires Admin-capability API key |
| `admin ratecard list` | GET `/v1/admin/ratecard/list` | ✅ | enterprise | ENS-486 — rate card version history (default limit 20, max 1000); requires Admin-capability API key |
| `admin ratecard show` | GET `/v1/admin/ratecard/show` | ✅ | enterprise | ENS-486 — current or a specific rate card version; requires Admin-capability API key |

## Enscrive Agents: Complete (managed reasoning)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `complete` | POST `/v1/complete` | ✅ | free | ENS-751 — single-shot (--prompt) and agentic tool-use (--messages/--tools) reasoning; BYOK or platform-managed fallback, budget-gated + metered |

## Records: bounded structural store (Slice 1)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `records collections create` | POST `/v1/records/collections` | ✅ | free | Records Slice 1 — create a collection with declared indexed_fields (name:type). embed=true returns 501 (Slice 2) |
| `records collections list` | GET `/v1/records/collections` | ✅ | free | Records Slice 1 — list collections for the tenant |
| `records collections update` | PUT `/v1/records/collections/{collection}` | ✅ | free | Records Slice 1 — whole-set replace of a collection's indexed_fields (not a merge) |
| `records collections delete` | DELETE `/v1/records/collections/{collection}` | ✅ | free | Records Slice 1 — delete a collection; cascade-deletes its records (204 No Content) |
| `records put` | POST `/v1/records/{collection}` | ✅ | free | Records Slice 1 — upsert a record by id (201 created / 200 replaced); last-write-wins, no versioning |
| `records query` | POST `/v1/records/{collection}/query` | ✅ | free | Records Slice 1 — typed filters (field:op:value), sort (field:dir), keyset cursor; vector_query returns 501 (Slice 2). --query-json escape hatch |
| `records get` | GET `/v1/records/{collection}/{id}` | ✅ | free | Records Slice 1 — fetch a single record by id |
| `records delete` | DELETE `/v1/records/{collection}/{id}` | ✅ | free | Records Slice 1 — delete a single record by id (200 with {deleted:true,...}) |

## Rate Card

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `ratecard show` | GET `/v1/ratecard` | ✅ | free | ENS-751 — public, no API-key auth; supports historical lookup via ?at=<RFC3339> |

## Admin: Operator Ops (ENS-752)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `admin wallet credit` | POST `/v1/admin/wallets/credit` | ✅ | enterprise | ENS-752 — operator wallet top-up; writes transaction_type='admin_seed' |
| `admin audit list` | GET `/v1/admin/audit` | ✅ | enterprise | ENS-752 / ENS-250 / WS-16 — durable admin_audit_log read surface |
| `admin incidents list` | GET `/v1/admin/incidents` | ✅ | enterprise | ENS-752 / ENS-298 Phase 3 |
| `admin incidents get` | GET `/v1/admin/incidents/{id}` | ✅ | enterprise | ENS-752 / ENS-298 Phase 3 |
| `admin migrations status` | GET `/v1/admin/migrations` | ✅ | enterprise | ENS-752 / ENS-229 — applied vs pending vs failed sqlx migrations |
| `admin telemetry stats` | GET `/v1/admin/telemetry/stats` | ✅ | enterprise | ENS-752 — aggregate-only wallet + incident + six-sigma stack counters |
| `admin metering backfill` | POST `/v1/admin/metering/backfill` | ✅ | enterprise | ENS-752 / Pillar 2 M3.2-5 — one-shot Loki -> metering_events backfill; infra-internal, kept managed-only |
| `admin tenants create` | POST `/v1/admin/tenants` | ✅ | enterprise | ENS-752 — idempotent on tenant name (migration 056 partial unique index) |
| `admin tenants erase` | POST `/v1/admin/tenants/erase` | ✅ | enterprise | ENS-752 / ENS-652 / ENS-659 — DESTRUCTIVE + IRREVERSIBLE tenant backup erasure (GDPR Article 17); confirm-gated client- and server-side |
| `admin api-keys create` | POST `/v1/admin/api-keys` | ✅ | enterprise | ENS-752 — cross-tenant API-key minting |
| `admin catalog-import` | POST `/v1/admin/catalog-import` | ✅ | enterprise | ENS-752 / ENS-648 — confirm-gated catalog restore from a backup artifact; checksum-verified before any write |
| `admin corpora reconcile` | POST `/v1/admin/corpora/{id}/reconcile` | ✅ | enterprise | ENS-752 / ENS-660 — always-async catalog/substrate repair |

## Enscrive Agents: persistent agents (ENS-783)

| Command | Verb · `/v1` path | Status | Plan | Notes |
|---|---|---|---|---|
| `agents create` | POST `/v1/agents` | ✅ | free | ENS-783 PR-5 — create a persistent agent bound to a corpus |
| `agents list` | GET `/v1/agents` | ✅ | free | ENS-783 PR-5 |
| `agents get` | GET `/v1/agents/{id}` | ✅ | free | ENS-783 PR-5 |
| `agents delete` | DELETE `/v1/agents/{id}` | ✅ | free | ENS-783 PR-5 |
| `agents answer` | POST `/v1/agents/{id}/answer` | ✅ | free | ENS-783 PR-5 — retrieval + managed reasoning against the agent's bound corpus; thin passthrough, no local scoring |
