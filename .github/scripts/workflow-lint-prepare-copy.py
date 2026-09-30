#!/usr/bin/env python3
"""
Copies every *.yml/*.yaml file from SRC_DIR into DST_DIR (flat, same
basenames), commenting out `queue: max` lines along the way -- but ONLY
lines proven, by parsing the file as YAML, to be a legitimate top-level
`concurrency.queue: max` or job-level `jobs.<id>.concurrency.queue: max`
(where `concurrency` is a mapping). Any workflow file where the set of
textual "queue: max" lines does not exactly match the set of such
legitimate occurrences (e.g. a stray/invalid `queue: max` placed directly
under a job, which GitHub rejects as an unknown key) makes this script
fail outright, naming the offending file, instead of silently blanking
something that might be a real bug.

Why any of this exists at all: GitHub shipped the `queue` sub-key of
`concurrency` on 2026-05-07
(https://github.blog/changelog/2026-05-07-github-actions-concurrency-groups-now-allow-larger-queues/),
after actionlint v1.7.12 (2026-03-30) was released, so actionlint does
not yet recognize the key and flags `queue: max` as an unexpected key.
`queue: max` in the real workflow files (see release-orchestrator.yml)
is correct and stays as-is -- this script only ever touches the
throwaway copy that .github/workflows/workflow-lint.yml lints.

The replacement keeps the line count and indentation identical (a
comment replaces the line in place; nothing is inserted or deleted), so
actionlint's reported line numbers still match the real files.

See tests/fixtures/workflow-lint/bad-queue-value.yml (queue: bogus,
inside concurrency -- must still fail actionlint) and
tests/fixtures/workflow-lint/bad-queue-max-placement.yml (queue: max,
NOT inside concurrency -- must make THIS script fail) for the regression
tests.

Usage: workflow-lint-prepare-copy.py <src-dir> <dst-dir>
"""
import glob
import os
import re
import sys

try:
    import yaml
except ImportError:
    print("PyYAML is required (import yaml failed)", file=sys.stderr)
    sys.exit(2)

QUEUE_MAX_TEXT = re.compile(r"^(\s*)queue:\s*max\s*$")

COMMENT_SUFFIX = (
    "# workflow-lint: commented out only in this throwaway diagnostic copy "
    "-- see .github/scripts/workflow-lint-prepare-copy.py"
)


def mapping_queue_max_line(node):
    """node: a yaml Node (expected to be the value of a `concurrency` key).
    Returns the 1-indexed line of `queue: max` if node is a MappingNode
    containing queue: max, else None."""
    if not isinstance(node, yaml.MappingNode):
        return None
    for key_node, value_node in node.value:
        if (
            isinstance(key_node, yaml.ScalarNode)
            and key_node.value == "queue"
            and isinstance(value_node, yaml.ScalarNode)
            and value_node.value == "max"
        ):
            return key_node.start_mark.line + 1
    return None


def legitimate_queue_max_lines(text):
    """Returns the set of 1-indexed line numbers of `queue: max` that sit
    directly under a top-level `concurrency:` mapping or a
    `jobs.<id>.concurrency:` mapping."""
    root = yaml.compose(text, Loader=yaml.SafeLoader)
    lines = set()
    if not isinstance(root, yaml.MappingNode):
        return lines

    for key_node, value_node in root.value:
        if not isinstance(key_node, yaml.ScalarNode):
            continue
        if key_node.value == "concurrency":
            line = mapping_queue_max_line(value_node)
            if line is not None:
                lines.add(line)
        elif key_node.value == "jobs" and isinstance(value_node, yaml.MappingNode):
            for _job_key, job_node in value_node.value:
                if not isinstance(job_node, yaml.MappingNode):
                    continue
                for jk, jv in job_node.value:
                    if isinstance(jk, yaml.ScalarNode) and jk.value == "concurrency":
                        line = mapping_queue_max_line(jv)
                        if line is not None:
                            lines.add(line)
    return lines


def textual_queue_max_lines(lines):
    """lines: list of raw lines (no trailing newline). Returns the set of
    1-indexed line numbers that read (ignoring surrounding whitespace)
    exactly `queue: max`."""
    found = set()
    for i, line in enumerate(lines, start=1):
        if QUEUE_MAX_TEXT.match(line):
            found.add(i)
    return found


def prepare_file(src_path, dst_path):
    with open(src_path, "r", encoding="utf-8") as fh:
        text = fh.read()
    lines = text.splitlines()

    try:
        legit = legitimate_queue_max_lines(text)
    except yaml.YAMLError as exc:
        print(f"::error::{src_path}: failed to parse as YAML: {exc}", file=sys.stderr)
        return False

    textual = textual_queue_max_lines(lines)

    if legit != textual:
        only_textual = sorted(textual - legit)
        only_legit = sorted(legit - textual)
        print(
            f"::error::{src_path}: 'queue: max' text line(s) and legitimate "
            "concurrency.queue occurrences do not match 1:1. "
            f"Text-only line(s) (not a top-level or job-level "
            f"concurrency.queue -- GitHub rejects this): {only_textual or 'none'}. "
            f"Structure-only line(s) (a legitimate concurrency.queue: max "
            f"whose source text doesn't read exactly 'queue: max', e.g. "
            f"quoted): {only_legit or 'none'}. Refusing to touch this file.",
            file=sys.stderr,
        )
        return False

    out_lines = list(lines)
    for line_no in legit:
        original = out_lines[line_no - 1]
        indent = original[: len(original) - len(original.lstrip())]
        out_lines[line_no - 1] = f"{indent}# queue: max  {COMMENT_SUFFIX}"

    with open(dst_path, "w", encoding="utf-8") as fh:
        fh.write("\n".join(out_lines))
        if text.endswith("\n"):
            fh.write("\n")

    return True


def main(argv):
    if len(argv) != 3:
        print(f"usage: {argv[0]} <src-dir> <dst-dir>", file=sys.stderr)
        return 2

    src, dst = argv[1], argv[2]
    os.makedirs(dst, exist_ok=True)

    files = sorted(glob.glob(os.path.join(src, "*.yml"))) + sorted(
        glob.glob(os.path.join(src, "*.yaml"))
    )
    if not files:
        print(
            f"::error::no workflow files found under {src} (checked *.yml and *.yaml)",
            file=sys.stderr,
        )
        return 1

    ok = True
    for f in files:
        base = os.path.basename(f)
        if not prepare_file(f, os.path.join(dst, base)):
            ok = False

    if not ok:
        return 1

    print(f"Prepared {len(files)} workflow file(s) in {dst}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
