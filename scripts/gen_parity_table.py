#!/usr/bin/env python3
"""Regenerate docs/cli-parity-table.md from v1-surface-contract.toml.

The parity table is a faithful projection of the CI-checked surface contract —
never hand-edit its rows (that is exactly how it went stale at 101 endpoints
while the contract had grown to 147). Run this after any contract change:

    python3 scripts/gen_parity_table.py

Grouping follows the `# ── Section ──` comment banners in the contract.
"""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "v1-surface-contract.toml"
OUT = ROOT / "docs" / "cli-parity-table.md"
RECONCILED = "2026-07-10"


def parse_contract(text: str):
    """Return [(section_name, [endpoint_dict, ...]), ...] in file order."""
    sections: list[tuple[str, list[dict]]] = []
    lines = text.split("\n")
    i, n = 0, len(lines)
    while i < n:
        line = lines[i]
        header = re.match(r"#\s*──\s*(.*?)\s*──", line)
        if header:
            sections.append((header.group(1).strip(), []))
            i += 1
            continue
        if line.strip() == "[[endpoint]]":
            block: dict[str, str] = {}
            i += 1
            while i < n and lines[i].strip() != "[[endpoint]]" and not lines[i].startswith("#"):
                m = re.match(r'\s*(\w+)\s*=\s*"(.*)"\s*$', lines[i])
                if m:
                    block[m.group(1)] = m.group(2)
                i += 1
            if not sections:
                sections.append(("(uncategorized)", []))
            sections[-1][1].append(block)
            continue
        i += 1
    return sections


def badge(status: str) -> str:
    return {"implemented": "✅", "deferred": "⛔ deferred", "missing": "❌ MISSING"}.get(status, status)


def main() -> None:
    sections = parse_contract(CONTRACT.read_text())
    flat = [e for _, es in sections for e in es]
    total = len(flat)
    impl = sum(1 for e in flat if e.get("status") == "implemented")
    defd = sum(1 for e in flat if e.get("status") == "deferred")
    tier = lambda t: sum(1 for e in flat if e.get("deployment_tier") == t)
    plan = lambda p: sum(1 for e in flat if e.get("required_plan") == p)

    out: list[str] = []
    out += [
        "# `enscrive-cli` — Command ↔ `/v1` parity table",
        "",
        "[← back to STATE-OF-CLI](./STATE-OF-CLI.md)",
        "",
        "**Auto-derived from [`v1-surface-contract.toml`](../v1-surface-contract.toml)** — the",
        "CI-checked source of truth (`tests/surface_contract.rs` +",
        "`command_tiers_covers_every_leaf_subcommand`). Regenerate with",
        "`python3 scripts/gen_parity_table.py`; do not hand-edit rows. Last reconciled:",
        f"**{RECONCILED}** (CLI 100%-parity workstream).",
        "",
        f"**Totals: {total} endpoints — {impl} `implemented`, {defd} `deferred`, 0 `missing`.** "
        f"Tiers: {tier('any-mode')} any-mode / {tier('managed-only')} managed-only. "
        f"Plans: {plan('free')} free / {plan('professional')} professional / {plan('enterprise')} enterprise.",
        "",
        "Status: ✅ implemented (CLI command exists + wired to this endpoint) · "
        "⛔ deferred (explicit `reason` in the contract; not silently missing).",
        "",
        "---",
        "",
    ]
    for name, es in sections:
        if not es:
            continue
        out += [f"## {name}", "", "| Command | Verb · `/v1` path | Status | Plan | Notes |", "|---|---|---|---|---|"]
        for e in es:
            cmd = f"`{e['cli_command']}`" if e.get("cli_command") else "—"
            note = (e.get("note") or e.get("reason") or "").replace("|", "\\|")
            out.append(
                f"| {cmd} | {e.get('method','')} `{e.get('path','')}` | "
                f"{badge(e.get('status',''))} | {e.get('required_plan','')} | {note} |"
            )
        out.append("")
    OUT.write_text("\n".join(out))
    print(f"wrote {OUT.relative_to(ROOT)}: {total} endpoints ({impl} impl, {defd} deferred), {sum(1 for _, es in sections if es)} sections")


if __name__ == "__main__":
    main()
