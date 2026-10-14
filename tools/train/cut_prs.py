#!/usr/bin/env python3
"""List every PR that shipped between two release cuts.

Compares per-component release tags (from two cut manifests, i.e. latest.json
or a dev.toml pin set) using the `gh` CLI, and prints JSON:

  {"cut": cur, "prev": prev,
   "components": {name: {"prev", "cur", "repo", "prs": [...], "unattributed": [...]}},
   "cut_prs": ["repo#N", ...], "errors": []}

Plan rule 12: enumerate by tool, never by hand. PR numbers come from
`commits/<sha>/pulls`; squash subjects are never the source.
An unreadable compare (e.g. a missing tag) is an ERROR for that component:
it is recorded in "errors" and the exit status is non-zero. A component whose
tag did not change contributes nothing.

Usage:
  cut_prs.py --prev <latest.json|pinset.toml> --cur <latest.json|pinset.toml>
             [--deployer PREV CUR] --json
"""
import argparse
import json
import re
import subprocess
import sys
import tomllib

OWNER = "enscrive"

# Binary / pin name -> repository (each verified with `gh repo view`).
REPO_OF = {
    "enscrive-developer": "enscrive-developer",
    "enscrive-embed": "enscrive-embed",
    "enscrive-observe": "enscrive-observe",
    "enscrive-metering-sentinel": "enscrive-metering-sentinel",
    "enscrive": "enscrive-cli",
    "enscrive-cli": "enscrive-cli",  # dev.toml pin name for the `enscrive` binary
    "esm": "enscrive-secrets-manager",
    "enscrive-docs": "enscrive-docs",
    "eba": "enscrive-bootstrap-authority",
    "enscrive-probe": "enscrive-control-tower-tui",
    "enscrive-deploy": "enscrive-deploy",
}
ALIASES = {"enscrive-cli": "enscrive"}  # normalise pin names to manifest names
DEPLOYER = "enscrive-deploy"
ENTRY_RE = re.compile(r"[a-z0-9-]{1,64}#[0-9]{1,9}")  # == ^...$ via fullmatch
MAX_ENTRIES = 1024
DEFAULT_BRANCHES =("main", "master")


def load_cut(path):
    """Return (version, {component: tag}) from a latest.json or a dev.toml pin set."""
    if path.endswith(".toml") or "dev.toml" in path.rsplit("/", 1)[-1]:
        with open(path, "rb") as f:
            d = tomllib.load(f)
        tags = {ALIASES.get(k, k): v for k, v in d.get("overrides", {}).items()}
        return d.get("platform_version", "?"), tags
    with open(path) as f:
        d = json.load(f)
    tags = {k: v["source_version"] for k, v in d["binaries"].items()}
    return d["version"], tags


def gh_api(path):
    """Call `gh api` and return parsed JSON; raise RuntimeError on failure."""
    p = subprocess.run(["gh", "api", path], capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(f"gh api {path}: {p.stderr.strip()[:300]}")
    return json.loads(p.stdout)


def compare_commits(repo, prev, cur, gh):
    """All commit shas in prev...cur (paged, completeness-checked)."""
    shas, page, total = [], 1, None
    while True:
        d = gh(f"repos/{OWNER}/{repo}/compare/{prev}...{cur}?per_page=100&page={page}")
        total = d.get("total_commits", total)
        got = d.get("commits", [])
        shas += [c["sha"] for c in got]
        if len(got) < 100:
            break
        page += 1
    if total is not None and len(shas) != total:
        raise RuntimeError(f"{repo} compare {prev}...{cur}: got {len(shas)} of {total} commits")
    return shas


def prs_of_commit(repo, sha, gh):
    pulls = gh(f"repos/{OWNER}/{repo}/commits/{sha}/pulls")
    return [
        p["number"]
        for p in pulls
        if p.get("merged_at") and p.get("base", {}).get("ref") in DEFAULT_BRANCHES
    ]


def component_prs(repo, prev, cur, gh):
    prs, unattributed = set(), []
    for sha in compare_commits(repo, prev, cur, gh):
        found = prs_of_commit(repo, sha, gh)
        if found:
            prs.update(found)
        else:
            unattributed.append(sha[:8])
    return sorted(prs), unattributed


def build(prev_cut, cur_cut, deployer, gh):
    prev_ver, prev_tags = prev_cut
    cur_ver, cur_tags = cur_cut
    if deployer:
        prev_tags = dict(prev_tags, **{DEPLOYER: deployer[0]})
        cur_tags = dict(cur_tags, **{DEPLOYER: deployer[1]})
    out = {"cut": cur_ver, "prev": prev_ver, "components": {}, "cut_prs": [], "errors": []}
    allp = set()
    for name in sorted(set(prev_tags) | set(cur_tags)):
        p, c = prev_tags.get(name), cur_tags.get(name)
        if p == c:
            continue
        repo = REPO_OF.get(name)
        if repo is None:
            out["errors"].append(f"{name}: no repo mapping")
            continue
        if p is None or c is None:
            out["errors"].append(f"{name}: tag present on one side only (prev={p}, cur={c})")
            continue
        try:
            prs, unattr = component_prs(repo, p, c, gh)
        except RuntimeError as e:
            out["errors"].append(f"{name}: {e}")
            continue
        out["components"][name] = {
            "prev": p, "cur": c, "repo": repo, "prs": prs, "unattributed": unattr,
        }
        allp.update(f"{repo}#{n}" for n in prs)
    out["cut_prs"] = sorted(allp, key=lambda s: (s.split("#")[0], int(s.split("#")[1])))
    out["errors"] += validate_cut_prs(out["cut_prs"])
    return out


def validate_cut_prs(entries):
    """Tower constraints: exact grammar per entry, at most MAX_ENTRIES entries."""
    errs = [f"cut_prs entry violates grammar: {e!r}" for e in entries if not ENTRY_RE.fullmatch(e)]
    if len(entries) > MAX_ENTRIES:
        errs.append(f"cut_prs has {len(entries)} entries, cap is {MAX_ENTRIES}")
    return errs


def main(argv=None, gh=gh_api):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--prev", required=True)
    ap.add_argument("--cur", required=True)
    ap.add_argument("--deployer", nargs=2, metavar=("PREV", "CUR"))
    ap.add_argument("--json", action="store_true", help="print JSON (the only output mode)")
    a = ap.parse_args(argv)
    out = build(load_cut(a.prev), load_cut(a.cur), a.deployer, gh)
    if out["errors"]:
        out["cut_prs"] = []  # never emit an invalid or truncated list
    print(json.dumps(out, indent=2))
    for e in out["errors"]:
        print(f"ERROR {e}", file=sys.stderr)
    return 1 if out["errors"] else 0


if __name__ == "__main__":
    sys.exit(main())
