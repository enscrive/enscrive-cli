# `enscrive-cli` — Command ↔ `/v1` parity table

[← back to STATE-OF-CLI](./STATE-OF-CLI.md)

Reconciliation of **`v1-surface-contract.toml`** against the **actual clap
command tree and handler code** on `main`. Every row was checked against the
`/v1` path literal in the source.

**Legend (Verdict):**
- ✅ **match** — command exists, verb + path agree with the contract.
- ⚠️ **name drift** — endpoint is implemented, but the command path you type
  differs from the contract's `cli_command`.
- ⚠️ **status drift** — contract status disagrees with reality.
- ⛔ **not implemented** — contract marks `deferred`; no handler exists.

Contract totals: **101 endpoints — 89 `implemented`, 12 `deferred`, 0 `missing`**
(100 `any-mode`, 1 `managed-only`; 43 free / 57 professional / 1 enterprise).

---

## Search & embeddings

| Command (as invoked) | Verb · `/v1` path | Contract | Handler | Verdict |
|---|---|---|---|---|
| `search` | POST `/v1/search` | implemented | `main.rs:3448`, body `main.rs:2553` | ✅ |
| `embeddings query` | POST `/v1/query-embeddings` | implemented | `main.rs:3465` | ✅ |

## Corpora & staging

| Command | Verb · path | Contract | Handler | Verdict |
|---|---|---|---|---|
| `corpus list` | GET `/v1/corpora` | implemented | `main.rs:3671` | ✅ |
| `corpus create` | POST `/v1/corpora` | implemented | `main.rs:3682` (requires `--embedding-model`) | ✅ |
| `corpus get` | GET `/v1/corpora/{id}` | implemented | `main.rs:3766` | ✅ (also redundantly in skip list — F3) |
| `corpus update` | PATCH `/v1/corpora/{id}` | implemented | `main.rs:3693` (`patch_json`) | ✅ |
| `corpus delete` | DELETE `/v1/corpora/{id}` | implemented | `main.rs:3729` (confirm-gated) | ✅ |
| `corpus stats` | GET `/v1/corpora/{id}/stats` | implemented | `main.rs:3737` | ✅ |
| `corpus documents` | GET `/v1/corpora/{id}/documents` | implemented | `main.rs:3744` | ✅ |
| `corpus chunks` | GET `/v1/corpora/{id}/documents/{doc_id}/chunks` | implemented | `main.rs:3756` | ✅ |
| `corpus stage` | POST `/v1/corpora/{id}/stage` | implemented | `main.rs:3807` | ✅ |
| `corpus commit` | POST `/v1/corpora/{id}/commit` | implemented | `main.rs:3819` | ✅ |
| `corpus pending` | GET `/v1/corpora/{id}/pending` | implemented | `main.rs:3826` | ✅ |
| `corpus pending-delete` | DELETE `/v1/corpora/{id}/pending/{doc_id}` | implemented | `main.rs:3833` | ✅ |
| `corpus revert` | POST `/v1/corpora/{id}/revert` | implemented | `main.rs:3775` | ✅ (redundant skip — F3) |
| `corpus commits` | GET `/v1/corpora/{id}/commits` | implemented | `main.rs:3784` (clamps limit 1–200) | ✅ (redundant skip — F3) |
| `corpus metrics` | GET `/v1/corpora/{id}/metrics` | implemented | `main.rs:3876` | ✅ (redundant skip — F3) |
| `corpus materialize-from-dataset` | POST `/v1/corpora/materialize-from-dataset` | implemented | `main.rs:3850` | ✅ (redundant skip — F3) |
| `corpus populate-from-dataset` | POST `/v1/corpora/{id}/populate-from-dataset` | implemented | `main.rs:2441` (async job) | ✅ |

## Ingestion & segmentation

| Command | Verb · path | Contract | Handler | Verdict |
|---|---|---|---|---|
| `ingest documents` | POST `/v1/ingest` | implemented | `main.rs:3548` (async job by default) | ✅ |
| `ingest prepared` | POST `/v1/ingest-prepared` | implemented | `main.rs:3484` | ✅ |
| `segment document` | POST `/v1/segment-document` | implemented | `main.rs:3575` (`post_text`, SSE) | ✅ |
| `analyze content` | POST `/v1/analyze-content` | implemented | `main.rs:3613` | ✅ |
| _(none)_ | POST `/v1/preview-chunking` | **deferred** | — | ⛔ as designed |
| _(none)_ | POST `/v1/preview-with-template` | **deferred** | — | ⛔ as designed |

## Models

| Command | Verb · path | Contract | Handler | Verdict |
|---|---|---|---|---|
| `models list` | GET `/v1/models` | implemented | `main.rs:3629` | ✅ |
| `models show` | GET `/v1/models/{provider}/{model}` | implemented | `main.rs:3659` (URL-encodes model) | ✅ (redundant skip — F3) |

## Voices

| Command | Verb · path | Contract | Handler | Verdict |
|---|---|---|---|---|
| `voices list` | GET `/v1/voices` | implemented | `main.rs:3891` | ✅ |
| `voices get` | GET `/v1/voices/{id}` | implemented | `main.rs:3895` | ✅ |
| `voices create` | POST `/v1/voices` | implemented | `main.rs:3902` | ✅ |
| **`voices update`** | PUT `/v1/voices/{id}` (+`POST …/diff-proposal` preview) | **deferred** | `main.rs:3920` | ⚠️ **status drift (F1)** — deferred in contract but fully shipped |
| `voices delete` | DELETE `/v1/voices/{id}` | implemented | `main.rs:3990` (confirm-gated) | ✅ |
| `voices compare` | POST `/v1/voices/compare` | implemented | `main.rs:4027` | ✅ |
| `voices search` | POST `/v1/voices/search` | implemented | `main.rs:4070` | ✅ |
| `voices promote` | POST `/v1/voices/{id}/promote` | implemented | `main.rs:4032` | ✅ |
| `voices gates list` | GET `/v1/voices/{id}/gates` | implemented | `main.rs:4043` | ✅ |
| `voices gates set` | POST `/v1/voices/{id}/gates` | implemented | `main.rs:4050` | ✅ |
| `voices gates delete` | DELETE `/v1/voices/{id}/gates/{metric}` | implemented | `main.rs:4062` | ✅ |
| **`voices diff2 diff`** | GET `/v1/voices/{id}/diff` | implemented (as `voices diff`) | `evals2.rs:812` | ⚠️ **name drift (F2)** |
| **`voices diff2 diff-cost`** | GET `/v1/voices/{id}/diff-cost` | implemented (as `voices diff-cost`) | `evals2.rs:823` | ⚠️ **name drift (F2)** |
| **`voices diff2 diff-proposal`** | POST `/v1/voices/{id}/diff-proposal` | implemented (as `voices diff-proposal`) | `evals2.rs:859` | ⚠️ **name drift (F2)** |
| _(none)_ | GET `/v1/voices/{id}/versions` | **deferred** | — | ⛔ as designed |
| _(none)_ | GET `/v1/voices/{id}/versions/{version}` | **deferred** | — | ⛔ as designed |

## Segmentation templates (all deferred)

| Command | Verb · path | Contract | Verdict |
|---|---|---|---|
| _(none)_ | GET/POST `/v1/segmentation-templates` | **deferred** | ⛔ as designed |
| _(none)_ | GET/PUT/DELETE `/v1/segmentation-templates/{id}` | **deferred** | ⛔ as designed |
| _(none)_ | POST `/v1/segmentation-templates/{id}/clone` | **deferred** | ⛔ as designed |

## Evals (legacy lineage — `/v1/evals/*`)

| Command | Verb · path | Contract | Handler | Verdict |
|---|---|---|---|---|
| `evals campaigns list` | GET `/v1/evals/campaigns` | implemented | `main.rs:4089` | ✅ |
| `evals campaigns get` | GET `/v1/evals/{id}` | implemented | `main.rs:4097` | ✅ |
| `evals run-campaign` | POST `/v1/evals/run-campaign` | implemented | `main.rs:4109` | ✅ |
| `evals run-campaign-stream` | POST `/v1/evals/run-campaign-stream` | implemented | `main.rs:4124` (`post_text`, SSE) | ✅ |
| `evals import` | POST `/v1/evals/import` | implemented | `main.rs:4148`+ | ✅ |
| `evals from-url` | POST `/v1/evals/from-url` + poll `/v1/jobs/{id}` | implemented | `main.rs:2368` | ✅ (client-invents name + uuid — F8) |
| `evals datasets list` | GET `/v1/evals/datasets` | implemented | `main.rs:4409` | ✅ |
| `evals datasets create` | POST `/v1/evals/datasets` | implemented | `main.rs:4421` | ✅ |
| `evals datasets get` | GET `/v1/evals/datasets/{id}` | implemented | `main.rs:4437` | ✅ |
| `evals datasets queries` | GET `/v1/evals/datasets/{id}/queries` | implemented | `main.rs:4444` | ✅ |
| `evals datasets update` | PUT `/v1/evals/datasets/{id}` | implemented | `main.rs:4459` | ✅ |
| `evals datasets delete` | DELETE `/v1/evals/datasets/{id}` | implemented | `main.rs:4476` | ✅ |
| `evals voice-status` | GET `/v1/evals/voice-status/{voice_id}` | implemented | `main.rs:4489` | ✅ |
| _(none)_ | GET `/v1/evals/convergence` | **deferred** | — | ⛔ as designed |

## Evals 2.0 (`/v1/datasets`, `/v1/eval-defs`, `/v1/eval-runs`)

| Command (as invoked) | Verb · path | Contract `cli_command` | Handler | Verdict |
|---|---|---|---|---|
| `datasets list` | GET `/v1/datasets` | `datasets list` | `evals2.rs:322` | ✅ (redundant skip — F3) |
| `datasets create` | POST `/v1/datasets` | `datasets create` | `evals2.rs:399` | ✅ |
| `datasets get` | GET `/v1/datasets/{id}` | `datasets get` | `evals2.rs:327` | ✅ (redundant skip — F3) |
| `datasets describe` | GET `/v1/datasets/{id}/describe` | `datasets describe` | `evals2.rs:334` | ✅ (redundant skip) |
| `datasets delete` | DELETE `/v1/datasets/{id}` | `datasets delete` | `evals2.rs:341` | ✅ (redundant skip) |
| `datasets upload` | POST `/v1/datasets/upload` | `datasets upload` | `evals2.rs:573` (multipart) | ✅ (redundant skip) |
| `eval-defs list` | GET `/v1/eval-defs` | `eval-defs list` | `evals2.rs:607` | ✅ (redundant skip) |
| `eval-defs create` | POST `/v1/eval-defs` | `eval-defs create` | `evals2.rs:685` | ✅ (redundant skip) |
| `eval-defs get` | GET `/v1/eval-defs/{id}` | `eval-defs get` | `evals2.rs:612` | ✅ (redundant skip) |
| `eval-defs delete` | DELETE `/v1/eval-defs/{id}` | `eval-defs delete` | `evals2.rs:619` | ✅ (redundant skip) |
| `eval-defs run` | POST `/v1/eval-defs/{id}/runs` | `eval-defs run` | `evals2.rs:696` (polls run) | ✅ (redundant skip) |
| `eval-defs publish` | POST `/v1/eval-defs/{id}/publish` | `eval-defs publish` | `evals2.rs:628` | ✅ (redundant skip) |
| `eval-defs publications` | GET `/v1/eval-defs/{id}/publications` | `eval-defs publications` | `evals2.rs:639` | ✅ (redundant skip) |
| `eval-defs unpublish` | DELETE `/v1/eval-publications/{id}` | `eval-defs unpublish` | `evals2.rs:646` | ✅ (redundant skip) |
| **`eval-defs runs list`** | GET `/v1/eval-defs/{id}/runs` | **`eval-runs list`** | `evals2.rs:773` | ⚠️ **name drift (F2)** |
| **`eval-defs runs get`** | GET `/v1/eval-runs/{id}` | **`eval-runs get`** | `evals2.rs:780` | ⚠️ **name drift (F2)** |
| **`eval-defs runs diagnose`** | GET `/v1/eval-runs/{id}/queries` | **`eval-runs diagnose`** | `evals2.rs:787` | ⚠️ **name drift (F2)** |

## Jobs & batch-sets

| Command | Verb · path | Contract | Handler | Verdict |
|---|---|---|---|---|
| `jobs list` | GET `/v1/jobs` | implemented | `main.rs:4725` | ✅ |
| `jobs get` | GET `/v1/jobs/{id}` | implemented | `main.rs:4731` | ✅ |
| `jobs cancel` | POST `/v1/jobs/{id}/cancel` | implemented | `main.rs:4738` | ✅ |
| `jobs retry` | POST `/v1/jobs/{id}/retry` | implemented | `main.rs:4746` | ✅ (redundant skip — F3) |
| `jobs abandon` | POST `/v1/jobs/{id}/abandon` | implemented | `main.rs:4753` | ✅ (redundant skip — F3) |
| `batch-sets list` | GET `/v1/corpora/{id}/batch-sets` | implemented | `main.rs:4773` | ✅ (redundant skip — F3) |
| `batch-sets get` | GET `/v1/batch-sets/{id}` | implemented | `main.rs:4780` | ✅ (redundant skip — F3) |

## Logs, usage, backup, export, admin

| Command | Verb · path | Contract | Handler | Verdict |
|---|---|---|---|---|
| `logs stream` | GET `/v1/logs/stream` | implemented | `main.rs:4645` (`get_text_with_query`, SSE) | ✅ |
| `logs search` | POST `/v1/logs/search` | implemented | `main.rs:4666` | ✅ |
| `logs metrics` | POST `/v1/logs/metrics` | implemented | `main.rs:4673` | ✅ |
| `usage` | GET `/v1/usage` | implemented | `main.rs:4684` | ✅ |
| `backup create` | POST `/v1/admin/backups` | implemented | `main.rs:4501` | ✅ |
| `backup list` | GET `/v1/admin/backups` | implemented | `main.rs:4509` | ✅ |
| `backup get` | GET `/v1/admin/backups/{backup_id}` | implemented | `main.rs:4517` | ✅ |
| `backup restore` | POST `/v1/admin/restore` | implemented | `main.rs:4559` (confirm-gated) | ✅ |
| `backup dry-run` | POST `/v1/admin/restore/dry-run` | implemented | `main.rs:4570` | ✅ |
| `export tenant` | GET `/v1/admin/export` | implemented | `main.rs:4583` (`get_bytes_with_query`) | ✅ |
| `export embeddings` | GET `/v1/admin/export/embeddings` | implemented | `main.rs:4617` | ✅ |
| `export token-usage` | GET `/v1/admin/export/token-usage` | implemented | `main.rs:4629` | ✅ |
| `admin rate-limits show` | GET `/v1/rate-limits` | implemented (free) | `main.rs:4796` | ✅ (redundant skip — F3) |
| `admin rate-limits set` | PATCH `/v1/admin/rate-limits/{tenant}/{provider}` | implemented (**managed-only, enterprise**) | `main.rs:4804` | ✅ (redundant skip — F3) |

## Local / operator (no `/v1` endpoint — correctly skip-listed)

| Command | Effect | Source |
|---|---|---|
| `init` | Create/update a managed or self-managed profile; self-managed fetches service binaries from the release manifest | `local.rs:511` (`init_managed`), self-managed init flow |
| `start` / `stop` / `status` | Drive the local self-managed stack | `local.rs` |
| `health` | Probe **`/v1/health`** (not `/health`) to answer "can I use the API?" | `main.rs:3439` |
| `license activate` / `status` / `deactivate` | Write/read/remove the license JWT locally; **server** verifies | `license.rs`, dispatch ~`main.rs:4887` |

> `enscrive health` is the one local command that touches the network; it
> deliberately probes `/v1/health` rather than the edge `/health`, because
> the edge can answer `200 {"phase":"pre-launch"}` while `/v1/*` returns 503
> (`main.rs:3426-3439`). It also tolerates an empty API key here
> (`unwrap_or_default()`, `main.rs:3438` — see F6).
