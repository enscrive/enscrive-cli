#!/usr/bin/env python3
"""parity-full — the mechanical 100%-green control for enscrive-cli ↔ /v1.

Enumerates EVERY endpoint in `v1-surface-contract.toml`, then either exercises
it through the built `enscrive` CLI against a live target (`--base-url`,
default dev.api) or accounts for it as an explicit skip-with-reason. Emits a
human table AND a machine-readable summary JSON:

    {"total": N, "passed": N, "failed": N,
     "skipped": [{"endpoint": "GET /v1/...", "reason": "..."}]}

Green (exit 0) == failed == 0 AND every non-passed endpoint is skipped WITH a
reason. Nothing silent — an implemented endpoint that is neither exercised nor
skip-listed is reported as UNPLANNED and fails the run.

Design principles:
- Safe on shared dev: create→read→(update)→delete self-contained fixtures.
  NEVER fire destructive/operator writes (tenant erase, restore, backups
  trigger, catalog-import, metering backfill, ratecard apply, wallet credit) —
  those are skipped-with-reason.
- Credentials come from the environment (reuse the existing live suites'
  mechanism): ENSCRIVE_BASE_URL, ENSCRIVE_API_KEY (tenant), and optional
  ENSCRIVE_ADMIN_API_KEY (operator reads run only when present). Keys are never
  logged or persisted.
- Offline mode (`--plan-only`) runs no CLI calls; it just prints the coverage
  map (planned/skipped/UNPLANNED) so the plan can be validated without a target.

Usage:
    python3 scripts/parity_full.py --plan-only          # coverage self-check
    ENSCRIVE_BASE_URL=https://dev.api.enscrive.io \
    ENSCRIVE_API_KEY=... [ENSCRIVE_ADMIN_API_KEY=...] \
        python3 scripts/parity_full.py --json-out parity.json
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "v1-surface-contract.toml"


# ─────────────────────────── contract parsing ───────────────────────────────

def norm_path(p: str) -> str:
    """Collapse path params so contract {id} matches step {collection} etc."""
    return re.sub(r"\{[^}]+\}", "{}", p)


def parse_contract() -> list[dict]:
    endpoints = []
    for block in re.split(r"\[\[endpoint\]\]", CONTRACT.read_text())[1:]:
        d = {}
        for m in re.finditer(r'(\w+)\s*=\s*"((?:[^"\\]|\\.)*)"', block):
            d[m.group(1)] = m.group(2)
        if "method" in d and "path" in d:
            d["key"] = f"{d['method']} {norm_path(d['path'])}"
            endpoints.append(d)
    return endpoints


# ─────────────────────── explicit skips (with reasons) ───────────────────────
# Keyed by "METHOD /normalized/path". Every non-exercised implemented endpoint
# MUST appear here (contract-deferred rows are auto-skipped from their reason).

DESTRUCTIVE = "destructive/operator write — not smoke-safe against shared dev"
NEEDS_ADMIN = "operator surface — requires ENSCRIVE_ADMIN_API_KEY (injected at live-run)"
DEEP_FIXTURE = "requires heavy embedding/ingest fixture beyond the smoke lifecycle"

SKIPS: dict[str, str] = {
    # ── destructive / operator writes (never fired on shared dev) ──
    "POST /v1/admin/tenants": DESTRUCTIVE,
    "POST /v1/admin/tenants/erase": DESTRUCTIVE,
    "POST /v1/admin/api-keys": DESTRUCTIVE,
    "POST /v1/admin/wallets/credit": DESTRUCTIVE,
    "POST /v1/admin/ratecard/apply": DESTRUCTIVE,
    "POST /v1/admin/catalog-import": DESTRUCTIVE,
    "POST /v1/admin/metering/backfill": DESTRUCTIVE,
    "POST /v1/admin/restore": DESTRUCTIVE,
    "POST /v1/admin/restore/dry-run": DESTRUCTIVE,
    "POST /v1/admin/backups": DESTRUCTIVE,
    "GET /v1/admin/backups/{}": DESTRUCTIVE,
    "POST /v1/admin/corpora/{}/reconcile": DESTRUCTIVE,
    "PATCH /v1/admin/rate-limits/{}/{}": DESTRUCTIVE,
    "POST /v1/restore": DESTRUCTIVE,
    "POST /v1/restore/dry-run": DESTRUCTIVE,
    # ── admin/operator reads without a plan step (need an admin key) ──
    # NB: audit/incidents/migrations/telemetry/ratecard-list/ratecard-show have
    # plan steps that self-skip when ENSCRIVE_ADMIN_API_KEY is absent.
    "GET /v1/admin/incidents/{}": NEEDS_ADMIN,
    "GET /v1/admin/api-rate-limits/{}": NEEDS_ADMIN,
    "PATCH /v1/admin/api-rate-limits/{}/{}": NEEDS_ADMIN,
    "DELETE /v1/admin/api-rate-limits/{}/{}": NEEDS_ADMIN,
    "GET /v1/admin/backups": NEEDS_ADMIN,
    "GET /v1/admin/export": NEEDS_ADMIN,
    "GET /v1/admin/export/embeddings": NEEDS_ADMIN,
    "GET /v1/admin/export/token-usage": NEEDS_ADMIN,
    # ── deep-fixture endpoints (need embeddings/ingested docs/campaign history) ──
    "POST /v1/corpora": "corpus family needs an embedding-provider fixture; smoke skips the embedding-backed lifecycle",
    "GET /v1/models/{}/{}": "needs a specific {provider}/{model} pair; the registry is covered by `models list`",
    "POST /v1/datasets": DEEP_FIXTURE,
    "GET /v1/corpora/{}/documents/{}/chunks": DEEP_FIXTURE,
    "POST /v1/corpora/materialize-from-dataset": DEEP_FIXTURE,
    "POST /v1/corpora/{}/populate-from-dataset": DEEP_FIXTURE,
    "POST /v1/corpora/{}/revert": DEEP_FIXTURE,
    "POST /v1/corpora/{}/commit": DEEP_FIXTURE,
    "DELETE /v1/corpora/{}/pending/{}": DEEP_FIXTURE,
    "POST /v1/corpora/{}/stage": DEEP_FIXTURE,
    "POST /v1/corpora/{}/promote": DEEP_FIXTURE,
    "GET /v1/corpora/{}/batch-sets": DEEP_FIXTURE,
    "POST /v1/ingest": DEEP_FIXTURE,
    "POST /v1/ingest-prepared": DEEP_FIXTURE,
    "POST /v1/segment-document": DEEP_FIXTURE,
    "POST /v1/preview-with-template": DEEP_FIXTURE,
    "POST /v1/query-embeddings": DEEP_FIXTURE,
    "POST /v1/search": DEEP_FIXTURE,
    "POST /v1/complete": "reasoning spend — skipped in smoke to avoid provider cost",
    "POST /v1/agents": DEEP_FIXTURE,
    "GET /v1/agents/{}": DEEP_FIXTURE,
    "DELETE /v1/agents/{}": DEEP_FIXTURE,
    "POST /v1/agents/{}/answer": "reasoning spend — skipped in smoke to avoid provider cost",
    "POST /v1/analyze-content": DEEP_FIXTURE,
    "POST /v1/voices": DEEP_FIXTURE,
    "GET /v1/voices/{}": DEEP_FIXTURE,
    "PUT /v1/voices/{}": DEEP_FIXTURE,
    "DELETE /v1/voices/{}": DEEP_FIXTURE,
    "POST /v1/voices/compare": DEEP_FIXTURE,
    "POST /v1/voices/search": DEEP_FIXTURE,
    "GET /v1/voices/{}/versions": DEEP_FIXTURE,
    "GET /v1/voices/{}/versions/{}": DEEP_FIXTURE,
    "GET /v1/voices/{}/diff": DEEP_FIXTURE,
    "GET /v1/voices/{}/diff-cost": DEEP_FIXTURE,
    "POST /v1/voices/{}/diff-proposal": DEEP_FIXTURE,
    "POST /v1/voices/{}/promote": DEEP_FIXTURE,
    "GET /v1/voices/{}/gates": DEEP_FIXTURE,
    "POST /v1/voices/{}/gates": DEEP_FIXTURE,
    "DELETE /v1/voices/{}/gates/{}": DEEP_FIXTURE,
    "GET /v1/evals/voice-status/{}": DEEP_FIXTURE,
    "GET /v1/evals/convergence": DEEP_FIXTURE,
    "GET /v1/evals/campaigns": None,  # exercised (list) — see PLAN
    "GET /v1/evals/{}": DEEP_FIXTURE,
    "POST /v1/evals/{}/promote": DEEP_FIXTURE,
    "POST /v1/evals/run-campaign": DEEP_FIXTURE,
    "POST /v1/evals/run-campaign-stream": DEEP_FIXTURE,
    "POST /v1/evals/from-url": DEEP_FIXTURE,
    "GET /v1/evals/datasets/{}": DEEP_FIXTURE,
    "PUT /v1/evals/datasets/{}": DEEP_FIXTURE,
    "DELETE /v1/evals/datasets/{}": DEEP_FIXTURE,
    "POST /v1/evals/datasets/{}/promote": DEEP_FIXTURE,
    "GET /v1/evals/datasets/{}/queries": DEEP_FIXTURE,
    "POST /v1/evals/datasets": DEEP_FIXTURE,
    "GET /v1/jobs/{}": DEEP_FIXTURE,
    "POST /v1/jobs/{}/cancel": DEEP_FIXTURE,
    "POST /v1/jobs/{}/retry": DEEP_FIXTURE,
    "POST /v1/jobs/{}/abandon": DEEP_FIXTURE,
    "GET /v1/batch-sets/{}": DEEP_FIXTURE,
    "POST /v1/batch-sets/{}/retry": DEEP_FIXTURE,
    "POST /v1/batch-sets/{}/abandon": DEEP_FIXTURE,
    "GET /v1/datasets/{}": DEEP_FIXTURE,
    "GET /v1/datasets/{}/describe": DEEP_FIXTURE,
    "DELETE /v1/datasets/{}": DEEP_FIXTURE,
    "POST /v1/datasets/upload": DEEP_FIXTURE,
    "GET /v1/eval-defs/{}": DEEP_FIXTURE,
    "DELETE /v1/eval-defs/{}": DEEP_FIXTURE,
    "POST /v1/eval-defs": DEEP_FIXTURE,
    "GET /v1/eval-defs/{}/runs": DEEP_FIXTURE,
    "POST /v1/eval-defs/{}/runs": DEEP_FIXTURE,
    "GET /v1/eval-runs/{}": DEEP_FIXTURE,
    "GET /v1/eval-runs/{}/queries": DEEP_FIXTURE,
    "POST /v1/eval-defs/{}/publish": DEEP_FIXTURE,
    "GET /v1/eval-defs/{}/publications": DEEP_FIXTURE,
    "DELETE /v1/eval-publications/{}": DEEP_FIXTURE,
    "GET /v1/logs/stream": "SSE stream — not a single-shot smoke call",
    "GET /v1/backups/{}": DEEP_FIXTURE,
    "GET /v1/corpora/{}/documents": DEEP_FIXTURE,
    "GET /v1/corpora/{}/pending": DEEP_FIXTURE,
    "GET /v1/corpora/{}/commits": DEEP_FIXTURE,
    "GET /v1/corpora/{}/metrics": DEEP_FIXTURE,
    "GET /v1/corpora/{}/stats": DEEP_FIXTURE,
    "GET /v1/corpora/{}": DEEP_FIXTURE,
    "PATCH /v1/corpora/{}": DEEP_FIXTURE,
    "DELETE /v1/corpora/{}": DEEP_FIXTURE,
}
SKIPS = {k: v for k, v in SKIPS.items() if v is not None}


# ───────────────────────────── exercise plan ────────────────────────────────
# Each step: id, argv (list; {ctx} placeholders), endpoints (list of keys it
# covers), optional capture {ctx_var: dotted.data.path}, optional admin=True.
# Steps run in order; a failed capture-producing step marks its dependents
# as blocked (reported as failed with the upstream reason).

def build_plan(ts: str) -> list[dict]:
    coll = f"parity_{ts}"
    tmpl_name = f"parity-tmpl-{ts}"
    end_t = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    start_t = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() - 3600))
    return [
        # ── discovery reads (no fixtures) ──
        {"id": "ratecard", "argv": ["ratecard", "show"], "endpoints": ["GET /v1/ratecard"]},
        {"id": "models_list", "argv": ["models", "list"], "endpoints": ["GET /v1/models"]},
        {"id": "usage", "argv": ["usage", "--start-time", start_t, "--end-time", end_t],
         "endpoints": ["GET /v1/usage"]},
        {"id": "ratelimits", "argv": ["admin", "rate-limits", "show"], "endpoints": ["GET /v1/rate-limits"]},
        {"id": "wallet_balance", "argv": ["wallet", "balance"], "endpoints": ["GET /v1/wallet/balance"]},
        {"id": "wallet_debits", "argv": ["wallet", "debits"], "endpoints": ["GET /v1/wallet/debits"]},
        {"id": "corpus_list", "argv": ["corpus", "list"], "endpoints": ["GET /v1/corpora"]},
        {"id": "voices_list", "argv": ["voices", "list"], "endpoints": ["GET /v1/voices"]},
        {"id": "agents_list", "argv": ["agents", "list"], "endpoints": ["GET /v1/agents"]},
        {"id": "jobs_list", "argv": ["jobs", "list"], "endpoints": ["GET /v1/jobs"]},
        {"id": "datasets_list", "argv": ["datasets", "list"], "endpoints": ["GET /v1/datasets"]},
        {"id": "evaldefs_list", "argv": ["eval-defs", "list"], "endpoints": ["GET /v1/eval-defs"]},
        {"id": "campaigns_list", "argv": ["evals", "campaigns", "list"], "endpoints": ["GET /v1/evals/campaigns"]},
        {"id": "evals_datasets_list", "argv": ["evals", "datasets", "list"], "endpoints": ["GET /v1/evals/datasets"]},
        {"id": "segtmpl_list", "argv": ["segmentation-templates", "list"], "endpoints": ["GET /v1/segmentation-templates"]},
        {"id": "logs_search",
         "argv": ["logs", "search", "--start-time", start_t, "--end-time", end_t, "--limit", "1"],
         "endpoints": ["POST /v1/logs/search"]},
        {"id": "logs_metrics",
         "argv": ["logs", "metrics", "--start-time", start_t, "--end-time", end_t],
         "endpoints": ["POST /v1/logs/metrics"]},
        {"id": "revisions_list", "argv": ["revisions", "list"], "endpoints": ["GET /v1/backups"]},
        {"id": "preview_chunking",
         "argv": ["preview-chunking", "--content", "The quick brown fox. It jumped over the lazy dog."],
         "endpoints": ["POST /v1/preview-chunking"]},
        # ── records: fully self-contained lifecycle (no embeddings) ──
        {"id": "rec_coll_create",
         "argv": ["records", "collections", "create", "--collection", coll, "--indexed-field", "author:string"],
         "endpoints": ["POST /v1/records/collections"]},
        {"id": "rec_coll_list", "argv": ["records", "collections", "list"],
         "endpoints": ["GET /v1/records/collections"]},
        {"id": "rec_put",
         "argv": ["records", "put", coll, "--id", "r1", "--json", '{"author":"ada","body":"hello"}'],
         "endpoints": ["POST /v1/records/{}"], "needs": ["rec_coll_create"]},
        {"id": "rec_query",
         "argv": ["records", "query", coll, "--filter", "author:eq:ada"],
         "endpoints": ["POST /v1/records/{}/query"], "needs": ["rec_coll_create"]},
        {"id": "rec_get", "argv": ["records", "get", coll, "r1"],
         "endpoints": ["GET /v1/records/{}/{}"], "needs": ["rec_put"]},
        {"id": "rec_coll_update",
         "argv": ["records", "collections", "update", coll, "--indexed-field", "author:string", "--indexed-field", "score:number"],
         "endpoints": ["PUT /v1/records/collections/{}"], "needs": ["rec_coll_create"]},
        {"id": "rec_del", "argv": ["records", "delete", coll, "r1"],
         "endpoints": ["DELETE /v1/records/{}/{}"], "needs": ["rec_put"]},
        {"id": "rec_coll_del", "argv": ["records", "collections", "delete", coll],
         "endpoints": ["DELETE /v1/records/collections/{}"], "needs": ["rec_coll_create"]},
        # ── segmentation-templates: self-contained lifecycle ──
        {"id": "segtmpl_create",
         "argv": ["segmentation-templates", "create", "--name", tmpl_name,
                  "--slug", f"parity-tmpl-{ts}", "--system-prompt", "Segment the document.",
                  "--defaults", '{"min_segment_length":100,"max_segment_length":1000}'],
         "endpoints": ["POST /v1/segmentation-templates"], "capture": {"tmpl_id": "id"}},
        {"id": "segtmpl_get", "argv": ["segmentation-templates", "get", "<tmpl_id>"],
         "endpoints": ["GET /v1/segmentation-templates/{}"], "needs": ["segtmpl_create"]},
        {"id": "segtmpl_update",
         "argv": ["segmentation-templates", "update", "<tmpl_id>", "--name", f"{tmpl_name}-v2"],
         "endpoints": ["PUT /v1/segmentation-templates/{}"], "needs": ["segtmpl_create"]},
        {"id": "segtmpl_clone", "argv": ["segmentation-templates", "clone", "<tmpl_id>"],
         "endpoints": ["POST /v1/segmentation-templates/{}/clone"], "needs": ["segtmpl_create"],
         "capture": {"tmpl_clone_id": "id"}},
        {"id": "segtmpl_del", "argv": ["segmentation-templates", "delete", "<tmpl_id>"],
         "endpoints": ["DELETE /v1/segmentation-templates/{}"], "needs": ["segtmpl_create"]},
        {"id": "segtmpl_clone_del", "argv": ["segmentation-templates", "delete", "<tmpl_clone_id>"],
         "endpoints": [], "needs": ["segtmpl_clone"]},
        # ── admin/operator reads (only when admin key present) ──
        {"id": "admin_audit", "argv": ["admin", "audit", "list"], "endpoints": ["GET /v1/admin/audit"], "admin": True},
        {"id": "admin_incidents", "argv": ["admin", "incidents", "list"], "endpoints": ["GET /v1/admin/incidents"], "admin": True},
        {"id": "admin_migrations", "argv": ["admin", "migrations", "status"], "endpoints": ["GET /v1/admin/migrations"], "admin": True},
        {"id": "admin_telemetry", "argv": ["admin", "telemetry", "stats"], "endpoints": ["GET /v1/admin/telemetry/stats"], "admin": True},
        {"id": "admin_ratecard_list", "argv": ["admin", "ratecard", "list"], "endpoints": ["GET /v1/admin/ratecard/list"], "admin": True},
        {"id": "admin_ratecard_show", "argv": ["admin", "ratecard", "show"], "endpoints": ["GET /v1/admin/ratecard/show"], "admin": True},
    ]


# ─────────────────────────────── runner ─────────────────────────────────────

def dotted(data, path: str):
    cur = data
    for part in path.split("."):
        if isinstance(cur, list):
            cur = cur[int(part)]
        elif isinstance(cur, dict):
            cur = cur.get(part)
        else:
            return None
    return cur


def run_cli(base_url: str, api_key: str, argv: list[str], timeout: int = 60):
    cmd = ["enscrive", "--output", "json", "--endpoint", base_url, "--api-key", api_key, *argv]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return False, None, "timeout"
    body = None
    try:
        body = json.loads(proc.stdout)
    except (json.JSONDecodeError, ValueError):
        pass
    ok = bool(body and body.get("ok")) and proc.returncode == 0
    err = None if ok else ((body or {}).get("error") or proc.stderr.strip() or f"exit {proc.returncode}")
    return ok, (body or {}).get("data"), err


def main() -> int:
    ap = argparse.ArgumentParser(description="parity-full — 100%-green control for enscrive-cli ↔ /v1")
    ap.add_argument("--base-url", default=os.environ.get("ENSCRIVE_BASE_URL", "https://dev.api.enscrive.io"))
    ap.add_argument("--json-out", help="write summary JSON to this path")
    ap.add_argument("--plan-only", action="store_true", help="no CLI calls; print coverage map + exit")
    ap.add_argument("--cli-bin", default="enscrive", help="path to the enscrive binary (default: PATH lookup)")
    args = ap.parse_args()
    if args.cli_bin != "enscrive":
        os.environ["PATH"] = f"{Path(args.cli_bin).resolve().parent}:{os.environ.get('PATH','')}"

    endpoints = parse_contract()
    implemented = [e for e in endpoints if e["status"] == "implemented"]
    deferred = [e for e in endpoints if e["status"] == "deferred"]
    ts = time.strftime("%Y%m%d%H%M%S")
    plan = build_plan(ts)
    capture_vars = {v for step in plan for v in step.get("capture", {})}

    planned_keys = {k for step in plan for k in step["endpoints"]}
    impl_keys = {e["key"] for e in implemented}

    # Coverage self-check: every implemented endpoint must be planned or skipped.
    unplanned = sorted(impl_keys - planned_keys - set(SKIPS))
    # A skip that isn't a real implemented endpoint is dead config (warn).
    stale_skips = sorted(set(SKIPS) - impl_keys - {e["key"] for e in deferred})

    if args.plan_only:
        print(f"contract: {len(endpoints)} endpoints — {len(implemented)} implemented, {len(deferred)} deferred")
        print(f"planned (exercised): {len(planned_keys & impl_keys)}  skipped: {len(impl_keys & set(SKIPS))}")
        if unplanned:
            print(f"\nUNPLANNED implemented endpoints ({len(unplanned)}) — must be planned or skip-listed:")
            for k in unplanned:
                print(f"  {k}")
        if stale_skips:
            print(f"\nstale SKIPS (not an implemented/deferred endpoint) ({len(stale_skips)}):")
            for k in stale_skips:
                print(f"  {k}")
        print("\nplan-only OK" if not unplanned else "\nplan-only INCOMPLETE", file=sys.stderr)
        return 0 if not unplanned else 3

    api_key = os.environ.get("ENSCRIVE_API_KEY")
    admin_key = os.environ.get("ENSCRIVE_ADMIN_API_KEY")
    if not api_key:
        print("ENSCRIVE_API_KEY is required for a live run (or use --plan-only)", file=sys.stderr)
        return 2

    # Secret hygiene: the injected keys must NEVER surface in any emitted
    # artifact (stdout or --json-out). Scrub them from every captured error
    # string, and assert on the serialized output before we print/write it.
    secrets = sorted({s for s in (api_key, admin_key) if s}, key=len, reverse=True)

    def scrub(text):
        if not text:
            return text
        for s in secrets:
            text = text.replace(s, "***REDACTED***")
        return text

    results: dict[str, dict] = {}  # endpoint key -> {status, reason}
    ctx: dict[str, str] = {}
    step_ok: dict[str, bool] = {}

    for step in plan:
        needs = step.get("needs", [])
        blocked = next((n for n in needs if not step_ok.get(n)), None)
        is_admin = step.get("admin", False)
        key_for = api_key
        if is_admin:
            if not admin_key:
                for k in step["endpoints"]:
                    results[k] = {"status": "skipped", "reason": NEEDS_ADMIN}
                step_ok[step["id"]] = False
                continue
            key_for = admin_key
        if blocked:
            for k in step["endpoints"]:
                results[k] = {"status": "failed", "reason": f"blocked: upstream step '{blocked}' failed"}
            step_ok[step["id"]] = False
            continue
        # Sentinel substitution (<var>) — NOT str.format, so literal JSON braces
        # in an argv (e.g. --json '{"a":1}') pass through untouched.
        argv = []
        for a in step["argv"]:
            for var, val in ctx.items():
                a = a.replace(f"<{var}>", val)
            argv.append(a)
        leftover = next((m.group(1) for a in argv for m in [re.search(r"<([a-z_]+)>", a)]
                         if m and m.group(1) in capture_vars), None)
        if leftover:
            for k in step["endpoints"]:
                results[k] = {"status": "failed", "reason": f"missing capture <{leftover}>"}
            step_ok[step["id"]] = False
            continue
        ok, data, err = run_cli(args.base_url, key_for, argv)
        step_ok[step["id"]] = ok
        if ok and "capture" in step and data is not None:
            for var, path in step["capture"].items():
                val = dotted(data, path)
                if val is not None:
                    ctx[var] = str(val)
        for k in step["endpoints"]:
            results[k] = {"status": "passed" if ok else "failed", "reason": None if ok else scrub(err)}

    # Fold in skips + deferred + unplanned.
    for k, reason in SKIPS.items():
        if k in impl_keys:
            results.setdefault(k, {"status": "skipped", "reason": reason})
    for e in deferred:
        results.setdefault(e["key"], {"status": "skipped", "reason": e.get("reason", "deferred (no reason)")})
    for k in unplanned:
        results.setdefault(k, {"status": "failed", "reason": "UNPLANNED — neither exercised nor skip-listed"})

    passed = sorted(k for k, r in results.items() if r["status"] == "passed")
    failed = sorted(k for k, r in results.items() if r["status"] == "failed")
    skipped = sorted((k for k, r in results.items() if r["status"] == "skipped"))
    total = len(endpoints)

    summary = {
        "total": total,
        "passed": len(passed),
        "failed": len(failed),
        "skipped": [{"endpoint": k, "reason": results[k]["reason"]} for k in skipped],
    }

    # Secret-scan assert (parity with the campaign runner's bundle guard): the
    # operator/tenant keys must not appear in anything we emit. Fail loud.
    emitted = json.dumps(summary) + "".join(str(r.get("reason")) for r in results.values())
    leaked = [s for s in secrets if s in emitted]
    if leaked:
        print("FATAL: secret material detected in parity-full output — aborting emit",
              file=sys.stderr)
        return 2

    # Human table.
    print(f"\nparity-full vs {args.base_url}")
    print(f"contract {total} endpoints — {len(implemented)} implemented / {len(deferred)} deferred")
    print(f"  passed  {len(passed)}")
    print(f"  failed  {len(failed)}")
    print(f"  skipped {len(skipped)} (deferred + destructive/operator + deep-fixture)")
    if failed:
        print("\nFAILURES:")
        for k in failed:
            print(f"  ✗ {k}  —  {results[k]['reason']}")
    print("\nPASSED:")
    for k in passed:
        print(f"  ✓ {k}")

    if args.json_out:
        Path(args.json_out).write_text(json.dumps(summary, indent=2))
        print(f"\nsummary JSON → {args.json_out}")
    else:
        print("\n" + json.dumps(summary))

    green = len(failed) == 0 and not unplanned
    print("\nVERDICT: GREEN" if green else "\nVERDICT: RED", file=sys.stderr)
    return 0 if green else 1


if __name__ == "__main__":
    sys.exit(main())
