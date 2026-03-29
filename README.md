# Enscrive CLI

Enscrive CLI is the thin command-line client and validation harness for the public `enscrive-developer /v1` API.

It is intentionally not a direct client for `enscrive-observe`, `enscrive-embed`, or portal-only endpoints.

## Role In The Platform

```text
enscrive -> enscrive-developer /v1 -> enscrive-observe -> enscrive-embed
```

The CLI exists to:

- exercise the real public API
- validate current truth and current honesty
- provide a scriptable entry point for smoke tests and manifests
- expose unsupported public capabilities explicitly instead of hiding them
- bootstrap the emerging local Enscrive self-managed lane

## Current Capability Snapshot

The CLI currently includes namespaces for:

- health
- search
- embeddings
- ingest
- segmentation
- content analysis
- collections
- voices
- evals
- logs
- backup
- export
- usage

It now covers the major current public `/v1` surface, including `health`, tenant export, embedding export, token-usage export, and backup/restore validation paths.

## Current Honest Caveats

The CLI should be described carefully:

- it is already good as a contract-truth harness
- it now includes first-pass local lifecycle commands (`init`, `start`, `stop`, `status`)
- it is not yet a fully polished day-to-day developer UX tool
- human output is still fairly raw
- streaming behavior is still thinner than an ideal interactive CLI experience
- some unsupported/error classification logic remains heuristic rather than perfectly typed
- local first-tenant/API-key bootstrap is not yet automatic
- `enscrive-embed` still expects `OPENAI_API_KEY` at startup even when the developer may only want BGE/Voyage

So the honest positioning is:

- strong validation harness now
- early local/self-managed product scaffolding now
- stronger developer product later

## Canonical Platform Docs

The canonical current-state platform docs live in the top-level `ENSCRIVE-IO` repo:

- [Formal Documentation Index](https://github.com/chrisroge/ENSCRIVE-IO/blob/main/ENSCRIVE-FORMAL-DOCUMENTATION-INDEX.md)
- [Platform Capability And Remaining Gaps](https://github.com/chrisroge/ENSCRIVE-IO/blob/main/ENSCRIVE-PLATFORM-CAPABILITY-AND-REMAINING-GAPS-2026-03-15.md)
- [API Gap Closure Control](https://github.com/chrisroge/ENSCRIVE-IO/blob/main/ENSCRIVE-API-GAP-CLOSURE-CONTROL.md)
- [CLI Validation Strategy](https://github.com/chrisroge/ENSCRIVE-IO/blob/main/ENSCRIVE-CLI-VALIDATION-STRATEGY-2026-03-14.md)
- [Gap Closure Tracker](https://github.com/chrisroge/ENSCRIVE-IO/blob/main/MAJOR-PROJECTS/CLOSING-ALL-API-GAPS/TRACKER.md)

## Local Use

```bash
cargo run --bin enscrive -- --help
```

## Early Profile And Local Stack Commands

The CLI now supports named profiles in `~/.config/enscrive/profiles.toml`.

Managed profile:

```bash
enscrive init --mode managed --api-key ens_live_... --endpoint https://api.enscrive.io --set-default
```

Self-managed profile:

```bash
enscrive init --mode self-managed \
  --openai-api-key sk-... \
  --bge-endpoint http://192.168.68.135:8088 \
  --set-default
```

This generates local runtime/config files under:

- `~/.config/enscrive/profiles/<profile>/`
- `~/.local/share/enscrive/runtime/<profile>/`

Local lifecycle commands:

```bash
enscrive start
enscrive status
enscrive stop
```

What the current self-managed slice does:

- generates local config/env/runtime files
- creates a local Docker Compose infra definition for PostgreSQL, Keycloak, Qdrant, and Loki
- bootstraps a default local Keycloak realm/client/developer user
- starts local `enscrive-developer`, `enscrive-observe`, and `enscrive-embed` binaries if they are available
- seeds a default local tenant/environment and captures the first local API key into the active profile

What it does not yet do:

- install binaries for you from `curl -L https://enscrive.io/install | sh`
- remove the current `OPENAI_API_KEY` startup assumption from `enscrive-embed`

For now, the intended flow is:

1. `enscrive init --mode self-managed`
2. `enscrive start`
3. sign in to the local portal with the bootstrapped developer credentials from the `start` output
4. use the CLI immediately against the local profile; the first API key is already persisted

Manifest runner:

```bash
python3 scripts/run_manifests.py --help
```

The manifest runner supports both JSON and YAML manifests, suite metadata
(`current-truth`, `current-honesty`, `end-state`), and richer threshold/check
assertions for dataset-oriented validation.

Current-truth fixture bootstrap:

```bash
python3 scripts/bootstrap_current_truth_fixture.py --help
```

Live validation orchestrator:

```bash
python3 scripts/run_live_validation.py --help
```

The orchestrator supports provider-specific suite presets. For example, the
default `current-truth-core` lane uses an OpenAI-backed fixture collection,
while `bge-capability` mints a BGE-backed fixture collection and exercises the
same public `/v1` surface against that provider. The canonical local BGE lane
is now `bge-large-en-v1.5`; use overrides only when you are intentionally
comparing alternate BGE models:

```bash
python3 scripts/run_live_validation.py --suite bge-capability
```

For a protected self-hosted BGE deployment, `embed-svc` must be started with
`BGE_ENDPOINT` and, if required by the upstream service, `BGE_API_KEY`.
For single-model BGE endpoints, set `BGE_MODEL_NAME` as well so the stack
fails cleanly on model mismatch instead of pretending the endpoint supports the
entire `bge-*` family.

Eval campaign commands also expose campaign-level match semantics. For
segmented content, use `--match-mode document_prefix` so any matching chunk of
the expected document can satisfy relevance during scoring. Dataset-backed
campaigns can now omit `--queries-json` / `--queries-file`; the CLI will send
`queries: []` and let the stored dataset drive execution.

This README is the companion current-state entry point for the CLI repo. The top-level `ENSCRIVE-IO` docs remain canonical for platform truth.
