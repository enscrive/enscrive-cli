# Enscribe CLI

Enscribe CLI is the thin command-line client and validation harness for the public `enscribe-developer /v1` API.

It is intentionally not a direct client for `enscribe-observe`, `enscribe-embed`, or portal-only endpoints.

## Role In The Platform

```text
enscribe-CLI -> enscribe-developer /v1 -> enscribe-observe -> enscribe-embed
```

The CLI exists to:

- exercise the real public API
- validate current truth and current honesty
- provide a scriptable entry point for smoke tests and manifests
- expose unsupported public capabilities explicitly instead of hiding them

## Current Capability Snapshot

The CLI currently includes namespaces for:

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

It now covers the major current public `/v1` surface, including tenant export and backup/restore validation paths.

## Current Honest Caveats

The CLI should be described carefully:

- it is already good as a contract-truth harness
- it is not yet a fully polished day-to-day developer UX tool
- human output is still fairly raw
- streaming behavior is still thinner than an ideal interactive CLI experience
- some unsupported/error classification logic remains heuristic rather than perfectly typed

So the honest positioning is:

- strong validation harness now
- stronger developer product later

## Canonical Platform Docs

The canonical current-state platform docs live in the top-level `ENSCRIBE-IO` repo:

- [Formal Documentation Index](https://github.com/chrisroge/ENSCRIBE-IO/blob/main/ENSCRIBE-FORMAL-DOCUMENTATION-INDEX.md)
- [Platform Capability And Remaining Gaps](https://github.com/chrisroge/ENSCRIBE-IO/blob/main/ENSCRIBE-PLATFORM-CAPABILITY-AND-REMAINING-GAPS-2026-03-15.md)
- [API Gap Closure Control](https://github.com/chrisroge/ENSCRIBE-IO/blob/main/ENSCRIBE-API-GAP-CLOSURE-CONTROL.md)
- [CLI Validation Strategy](https://github.com/chrisroge/ENSCRIBE-IO/blob/main/ENSCRIBE-CLI-VALIDATION-STRATEGY-2026-03-14.md)
- [Gap Closure Tracker](https://github.com/chrisroge/ENSCRIBE-IO/blob/main/MAJOR-PROJECTS/CLOSING-ALL-API-GAPS/TRACKER.md)

## Local Use

```bash
cargo run -- --help
```

Manifest runner:

```bash
python3 scripts/run_manifests.py --help
```

This README is the companion current-state entry point for the CLI repo. The top-level `ENSCRIBE-IO` docs remain canonical for platform truth.
