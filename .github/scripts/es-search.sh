#!/usr/bin/env bash
#
# Shared `enscrive search` wrapper for the clean-room gate.
#
# WHY THIS EXISTS
# ---------------
# Under `--output json` the CLI writes its FAILURE envelope to STDOUT, the
# same stream as a successful response (this is deliberate — see
# tests/json_output_purity.rs). So the obvious shape:
#
#     enscrive search ... --output json > /tmp/out.json
#
# swallows the error into the file, and `set -e` then kills the step with
# NOTHING on the console. That happened once — the isolation step in PR #58
# — and cost a full CI round-trip to diagnose a plain HTTP 500.
#
# Routing every search in the gate through here makes that structurally
# impossible to repeat: the response is always dumped, stderr is always
# surfaced, and the exit code is always reported.
#
# CONTRACT
# --------
#   es-search.sh <outfile> [enscrive search args...]
#
# `--output json` is appended for you; do not pass it.
#
# This script ALWAYS EXITS 0. That is intentional. The caller must assert
# on the envelope — `.ok == true` — rather than relying on process exit,
# which is the discipline that keeps NEGATIVE assertions honest: "the
# document is absent" is satisfied for free by an error payload, an empty
# body, or a truncated file, so absence only means something once the
# caller has established the search actually ran.
#
set -uo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: es-search.sh <outfile> [enscrive search args...]" >&2
  exit 2
fi

out="$1"
shift

err="$(mktemp)"
enscrive search "$@" --output json > "$out" 2> "$err"
rc=$?

echo "--- enscrive search $* -> exit ${rc}"
if ! jq . "$out" 2>/dev/null; then
  echo "(response body is not parseable JSON:)"
  cat "$out" 2>/dev/null || echo "(no response body at all)"
fi
if [ -s "$err" ]; then
  echo "--- stderr:"
  cat "$err"
fi
rm -f "$err"

exit 0
