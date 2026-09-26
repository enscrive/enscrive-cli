#!/usr/bin/env bash
# Copies every *.yml/*.yaml file from SRC_DIR into DST_DIR (flat, same
# basenames), commenting out any line that is EXACTLY `queue: max` (any
# leading/trailing whitespace) along the way.
#
# Why: GitHub shipped the `queue` sub-key of `concurrency` on 2026-05-07
# (https://github.blog/changelog/2026-05-07-github-actions-concurrency-groups-now-allow-larger-queues/),
# after actionlint v1.7.12 (2026-03-30) was released, so actionlint does
# not yet recognize the key and flags `queue: max` as an unexpected key.
# `queue: max` in the real workflow files is correct and stays as-is --
# this script only ever touches the throwaway copy that
# .github/workflows/workflow-lint.yml lints, and only ever touches lines
# that are EXACTLY "queue: max". Any other queue value (a typo, or a
# genuinely invalid one) is left untouched and must still fail actionlint
# -- see tests/fixtures/workflow-lint/bad-queue-value.yml, which is the
# regression test proving that.
#
# The replacement keeps the line count identical (a comment replaces the
# line in place; nothing is inserted or deleted), so actionlint's
# reported line numbers still match the real files.
#
# Usage: workflow-lint-prepare-copy.sh <src-dir> <dst-dir>
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <src-dir> <dst-dir>" >&2
  exit 2
fi

src="$1"
dst="$2"

mkdir -p "$dst"

shopt -s nullglob
files=("$src"/*.yml "$src"/*.yaml)
if [ "${#files[@]}" -eq 0 ]; then
  echo "::error::no workflow files found under $src (checked *.yml and *.yaml)" >&2
  exit 1
fi

for f in "${files[@]}"; do
  base="$(basename "$f")"
  sed -E 's|^([[:space:]]*)queue:[[:space:]]*max[[:space:]]*$|\1# queue: max  # workflow-lint: commented out only in this throwaway diagnostic copy -- see .github/scripts/workflow-lint-prepare-copy.sh|' \
    "$f" > "$dst/$base"
done

echo "Prepared ${#files[@]} workflow file(s) in $dst"
