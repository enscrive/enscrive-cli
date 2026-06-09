# `enscrive-cli` — annotated flow walkthroughs

[← back to STATE-OF-CLI](./STATE-OF-CLI.md)

Four key flows traced through the code (`init` · `ingest` · `search` ·
`eval`), plus the async-job and output models that several of them share.
Every step cites the source. These describe **client behaviour**; where a
step depends on the server's response shape, that dependence is called out
(it cannot be proven from a static read — see
[STATE-OF-CLI §5](./STATE-OF-CLI.md#5-what-i-could-and-could-not-verify-honestly)).

---

## Shared substrate

### Connection resolution
Every networked command resolves a `ResolvedApiContext` via
`resolve_api_context` (`local.rs:482`):

1. endpoint = `--endpoint` / `ENSCRIVE_BASE_URL` → selected profile's
   `endpoint` → **`http://localhost:3000`** hard fallback (`local.rs:499`).
2. api_key = `--api-key` / `ENSCRIVE_API_KEY` → profile's `api_key`.

The client (`EnscriveClient::new`, `client.rs:192`) attaches **only**
`X-API-Key` and, if present, `X-Embedding-Provider-Key`
(`client.rs:207-213`) — a 120 s request timeout, rustls TLS, no retries, no
`User-Agent`/version header.

### Output / envelope model (`output.rs`)
Handlers terminate by building a `CliResponse` and calling `.emit(fmt)`
(`output.rs:114`), which `process::exit`s with a code:

| Class | Exit | Meaning |
|---|---|---|
| success | 0 | `{ok:true, command, data}` |
| `FAIL_BUG` | 1 | generic failure |
| `FAIL_UNSUPPORTED` | 2 | endpoint not on this surface |
| config | 3 | bad client input |
| `FAIL_PLAN_REQUIRED` | 4 | plan gate |
| `FAIL_CONFIRMATION_REQUIRED` | 5 | destructive op needs confirm |
| `FAIL_QUOTA_EXCEEDED` | 6 | quota |
| `FAIL_LICENSE_INVALID` | 7 | license |

`--output json` prints the whole envelope; `--output human` (the default,
`main.rs:65`) prints only `data` on success and `[CLASS] error` to stderr on
failure. There is **no** table renderer and **no** `--format` flag.

### Async-job model (`jobs_polling.rs`)
A mutation endpoint may answer **synchronously** or return a launch body
`{ job_id, status, poll_url }`. `maybe_await_async_launch`
(`jobs_polling.rs:355`) decides:

- no `job_id`, or `--async` set → emit the launch body verbatim and exit.
- otherwise → **poll to terminal**: reconstruct `poll_path =
  /v1/jobs/{job_id}` (`jobs_polling.rs:319` — note it ignores the server's
  `poll_url`, **F11**), `GET` it on exponential backoff `2s → 4 → 8 → 15`
  (`INITIAL_DELAY_SECS`/`MAX_DELAY_SECS`, `:29-30`) until a terminal status.
- terminal success = `complete|completed|succeeded`; terminal failure =
  `failed|cancelled` (`classify_status`, `:86`). Timeout or post-deadline
  poll error → `FAIL_BUG` + exit 1. Progress (`progress_percent`,
  `documents_ingested`, `sub_batches[]`) is rendered to **stderr**
  (`:379`).

This is the wire-level realization of the platform's "async-by-default
mutations, synchronous neural search" stance.

---

## Flow 1 — `init` (provision a profile)

`enscrive init --mode {managed | self-managed}` (`main.rs:75`,
`InitArgs:237`).

**Managed** (`local.rs:511`):
1. profile name ← `--profile-name` or `managed` default.
2. endpoint ← flag, else interactive prompt defaulting to
   `https://api.enscrive.io` (`local.rs:519`).
3. API key ← flag, else prompt; **rejected if empty** (`local.rs:526`).
4. profile persisted to `~/.config/enscrive/profiles.toml`; optional
   `--set-default`.

**Self-managed** (`SelfManagedInitOptions`, `local.rs:46`):
1. Resolve the **release manifest URL** — `--manifest-url` /
   `ENSCRIVE_MANIFEST_URL`, default `DEFAULT_RELEASE_MANIFEST_URL =
   https://dev.enscrive.io/releases/dev/latest.json` (`local.rs:84`).
   `file://` is accepted for offline harnesses.
2. Fetch + verify each service binary (`enscrive-developer`, `-observe`,
   `-embed`, `enscrive-docs`) from the manifest via `fetch_verify`:
   `fetch_manifest` → `fetch_and_verify`, which checks **SHA256 only**
   against the manifest and installs atomically (`<dest>.partial` →
   rename, chmod 0755) (`fetch_verify.rs:171,197`). **No signature
   verification yet** (TODO ENS-82, `fetch_verify.rs:22`) — `--force-refetch`
   bypasses the SHA fast-path. `esm` is discovered on `PATH` (not fetched),
   `--esm-bin` overrides (`local.rs:53`).
3. BYOK provider keys (`--openai-api-key`, `--voyage-api-key`,
   `--nebius-api-key`, `--anthropic-api-key`, `--bge-endpoint`/`-api-key`/
   `-model-name`) are recorded for the local stack.

> **Determinism note:** the default manifest points at the **dev** channel;
> the source comments (`InitArgs:310`, `local.rs:72`) flag ENS-81 to
> re-point at a production URL once CloudFront is provisioned.

---

## Flow 2 — `ingest documents` (async mutation)

`enscrive ingest documents --corpus-id … (--content | --content-file |
--documents-json | --documents-file) [--voice-id] [--sync] [--no-batch]
[--dry-run] [--async] [--timeout-secs 1800]` (`IngestDocumentsArgs:508`).

1. Exactly one content source (the four are mutually `conflicts_with`).
2. Body assembled and `POST /v1/ingest` (`main.rs:3548`).
3. **No model default is injected** — the embedding model is bound by the
   target corpus server-side. This is the no-implicit-defaults principle in
   action.
4. Response handed to the async-job model above: by default the CLI polls
   `/v1/jobs/{job_id}` to terminal and emits the terminal job; `--sync`
   requests a synchronous server path; `--async` returns the launch body;
   `--dry-run` previews. `--async` is `conflicts_with = "sync"`
   (`main.rs:552`).

`ingest prepared` (`IngestPreparedArgs:477`) is the pre-segmented sibling →
`POST /v1/ingest-prepared` (`main.rs:3484`) with the same `--async` /
`--timeout-secs` semantics.

---

## Flow 3 — `search` (synchronous neural search)

`enscrive search --query … [--corpus] [--limit 10] [--resolution]
[--hybrid-alpha] [--granularity] [--metadata k=v]… [filters]`
(`SearchArgs:338`).

1. `build_search_body` (`main.rs:2553`) assembles the request. Filters
   (`document_id`, `user_id`, `metadata`, `layer`, `strategy`) are nested
   under `filters` **only if at least one is present** — otherwise `filters:
   null` (clean omission, no invented filter).
2. `--metadata k=v` repeats are parsed by `parse_metadata_filters` (rejects
   non-`k=v` with "expected key=value").
3. `POST /v1/search` (`main.rs:3448`), synchronous; the response `data` is
   emitted directly.

The only client-side defaults sent are **UX** values — `limit = 10`,
`include_vectors = false`, `extended_results = false`. **No model, voice, or
resolution default is invented**; `resolution` / `hybrid_alpha` are sent
only when the user supplies them. `voices search` (`VoiceSearchArgs:1062`,
`build_voice_search_body:2588`) is the voice-scoped twin → `POST
/v1/voices/search`.

---

## Flow 4 — `eval` (the BeIR 5-step workflow, Evals 2.0)

The platform's "end-to-end Eval vision proven on BeIR" maps to five CLI
steps. Metrics are **computed server-side**; the CLI shapes requests, polls
jobs, and forwards opaque JSON — there is no client-side nDCG/recall math
anywhere (`evals2.rs`).

1. **Dataset** — register a BeIR dataset:
   - from a HuggingFace URL: `datasets create --from-url <BeIR/…>` →
     `POST /v1/datasets` (`evals2.rs:399`); a `job_id` in the response
     switches to job-polling (`evals2.rs:404`). Client default
     `source_type = "huggingface"` is injected (`evals2.rs:62`, **F9**).
   - or multipart: `datasets upload` ships `corpus.jsonl` +
     `queries.jsonl` + `qrels.tsv` (`post_dataset_upload`,
     `client.rs:430`; default `qrels_split = "test"`, `evals2.rs:108`).
2. **Empty corpus** — `corpus create --name … --embedding-model …`
   (`--embedding-model` is **required**, `main.rs:830`). The corpus's model
   binds the embedding space.
3. **Populate** — `corpus populate-from-dataset --corpus <id> --dataset-id
   <id> --voice-id <id>` → `POST /v1/corpora/{id}/populate-from-dataset`,
   an **async job** (`main.rs:2441`). The voice's `embedding_model` is
   advisory; the corpus model wins (server returns 409 if the corpus is
   non-empty). `corpus materialize-from-dataset` (`main.rs:3850`) is the
   one-shot convenience that conflates create+populate from a dataset's
   stratified subset.
4. **Voice** — `voices create --config-json '{chunking_strategy,…}'` →
   `POST /v1/voices` (`main.rs:3902`).
5. **Eval definition + run**:
   - `eval-defs create --name --dataset <id> --corpus <id> [--voice <id>]`
     → `POST /v1/eval-defs` (`evals2.rs:685`).
   - `eval-defs run --id <def>` → `POST /v1/eval-defs/{id}/runs`, then polls
     the run (`/v1/eval-runs/{run_id}`) to terminal and emits aggregate
     metrics (`evals2.rs:696,722`).
   - `eval-defs runs diagnose --id <run>` → `GET /v1/eval-runs/{id}/queries`
     for per-query, worst-first diagnostics (default `order=worst`,
     `limit=20`; `evals2.rs:787`).
   - `eval-defs publish --id <def> --run-id <run>` promotes a completed
     full-scope run to canonical (`evals2.rs:628`).

> **Command-name caveat:** step 5's run sub-tree is invoked as
> `eval-defs runs {list,get,diagnose}`, but the contract and the JSON
> envelope both call it `eval-runs …` (`evals2.rs:775`). See
> [`cli-findings.md` F2](./cli-findings.md). The **legacy** path —
> `evals from-url <BeIR>` (`main.rs:2368`) and `evals run-campaign`
> (`main.rs:4109`) — predates Evals 2.0 and coexists pending the EV-016
> unification (`evals2.rs:7`).

---

## What a static read cannot confirm

The request paths, verbs, bodies, and the async/polling/envelope behaviour
above are all verified against the source. **Not** verified (no live `/v1`
was available to this audit): that the server accepts these exact body
shapes, that response fields the handlers read (`job_id`, `status`,
`dataset_id`, `run_id`, `impact`, `changed_fields`) are actually emitted,
and the real terminal behaviour of jobs and SSE streams. Those are
integration concerns and are the correct place for a manual / harness check.
