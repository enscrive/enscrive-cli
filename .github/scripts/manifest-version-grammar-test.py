#!/usr/bin/env python3
#
# ENS-7414: self-test for the "Validate version grammar" step in
# .github/workflows/manifest.yml. It loads the workflow, extracts that
# step's run script and executes it with VERSION set to each case, so the
# shipped check is what gets tested (not a copy of its regex).
#
# Run: python3 .github/scripts/manifest-version-grammar-test.py
# Exit 0 = all pass; non-zero = one or more failures.

import os
import subprocess
import sys

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
WORKFLOW = os.path.join(REPO, ".github", "workflows", "manifest.yml")
STEP_NAME = "Validate version grammar"

ACCEPT = [
    "v20261016-t5",
    "v20261016-t5b",
    "v20261017-t5f1",
    "v20261017-t5f1b",
    "v20261016-1234",
    "v20261016-1234-abc1234",
    "v20261016-1234-abcdef0123",
]

REJECT = [
    "v20261016-t5.1",
    "v20261016-t5-1",
    "v20261016-t5ff1",
    "",
    "v20261016-t5;rm",
]

FAILS = 0


def find_step(workflow):
    jobs = workflow["jobs"]
    first_job_name = next(iter(jobs))
    first_steps = jobs[first_job_name].get("steps", [])
    matches = [
        (job, step)
        for job in jobs.values()
        for step in job.get("steps", [])
        if step.get("name") == STEP_NAME
    ]
    return first_job_name, first_steps, matches


def main():
    global FAILS
    with open(WORKFLOW, encoding="utf-8") as fh:
        workflow = yaml.safe_load(fh)

    first_job_name, first_steps, matches = find_step(workflow)
    if len(matches) != 1:
        print(f"FAIL - expected exactly one step named '{STEP_NAME}', found {len(matches)}")
        return 1
    _, step = matches[0]

    if not first_steps or first_steps[0].get("name") != STEP_NAME:
        print(f"FAIL - '{STEP_NAME}' is not the first step of first job '{first_job_name}'")
        FAILS += 1
    else:
        print(f"ok   - '{STEP_NAME}' is the first step of first job '{first_job_name}'")

    script = step.get("run", "")
    if "${{" in script:
        print("FAIL - step run script interpolates a GitHub expression")
        FAILS += 1
    else:
        print("ok   - step run script has no GitHub expression interpolation")

    cases = [(v, True) for v in ACCEPT] + [(v, False) for v in REJECT]
    for version, should_accept in cases:
        env = {"PATH": os.environ.get("PATH", ""), "VERSION": version}
        proc = subprocess.run(
            ["bash", "-c", script],
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )
        accepted = proc.returncode == 0
        label = repr(version)
        if accepted != should_accept:
            print(f"FAIL - {label}: expected {'accept' if should_accept else 'reject'}, "
                  f"exit={proc.returncode}")
            print(f"       stdout: {proc.stdout!r}")
            print(f"       stderr: {proc.stderr!r}")
            FAILS += 1
        elif not should_accept and "does not match the train-cut grammar" not in proc.stdout:
            print(f"FAIL - {label}: rejected without the expected ::error:: message")
            print(f"       stdout: {proc.stdout!r}")
            FAILS += 1
        else:
            print(f"ok   - {label}: {'accept' if should_accept else 'reject'}")

    print()
    if FAILS == 0:
        print("ALL PASS")
        return 0
    print(f"{FAILS} FAILURE(S)")
    return 1


if __name__ == "__main__":
    sys.exit(main())
