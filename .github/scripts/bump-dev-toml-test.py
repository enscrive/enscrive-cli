#!/usr/bin/env python3
#
# Unit tests for the release-orchestrator dev.toml pin-bump generator
# (bump-dev-toml.py), in particular the monotonic guard added 2026-07-10
# after an out-of-order release-published pair (v20260710-1836 then a
# late-arriving v20260710-1831) regressed the developer pin backward and
# would have shipped a stale binary on the next `provision`.
#
# Pure-function tests against bump(): no filesystem, no subprocess.
#
# Run: python3 .github/scripts/bump-dev-toml-test.py
# Exit 0 = all pass; non-zero = one or more failures.

import importlib.util
import os
import sys

# bump-dev-toml.py has a hyphenated filename (matches this repo's script
# naming convention), so it can't be `import`ed by name -- load it by path.
_here = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "bump_dev_toml", os.path.join(_here, "bump-dev-toml.py")
)
_bump_dev_toml = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_bump_dev_toml)
bump = _bump_dev_toml.bump

FAILS = 0


def check(desc, expected_content, expected_msg_prefix, content, roster_key, version):
    global FAILS
    new_content, message = bump(content, roster_key, version)
    ok = new_content == expected_content and message.startswith(expected_msg_prefix)
    if ok:
        print(f"ok   - {desc}")
    else:
        print(f"FAIL - {desc}")
        print(f"       expected content: {expected_content!r}")
        print(f"       got content:      {new_content!r}")
        print(f"       expected msg prefix: {expected_msg_prefix!r}")
        print(f"       got msg:              {message!r}")
        FAILS += 1


BASE = '[overrides]\nenscrive-developer = "v20260710-1831"\nenscrive-cli = "v20260701-0900"\n'

# --- newer -> writes ---------------------------------------------------
check(
    "newer version overwrites current pin",
    '[overrides]\nenscrive-developer = "v20260710-1836"\nenscrive-cli = "v20260701-0900"\n',
    "WROTE:",
    BASE, "enscrive-developer", "v20260710-1836",
)

# --- older -> skips (the regression this guard exists to prevent) ------
check(
    "older version (out-of-order release-published) is skipped, content untouched",
    BASE,
    "NOT bumping",
    BASE, "enscrive-developer", "v20260710-1801",
)

# --- equal -> no-op (pre-existing idempotency path) ---------------------
check(
    "equal version is a pure no-op",
    BASE,
    "NO_OP:",
    BASE, "enscrive-developer", "v20260710-1831",
)

# --- absent current -> always writes (new roster key) -------------------
check(
    "absent current pin always writes (new roster component)",
    '[overrides]\nenscrive-observe = "v20260710-1200"\nenscrive-developer = "v20260710-1831"\nenscrive-cli = "v20260701-0900"\n',
    "WROTE:",
    BASE, "enscrive-observe", "v20260710-1200",
)

# --- real-world regression scenario, replayed as two calls in arrival order
# First the NEWER release lands (fast CI run), then the OLDER release's
# release-published event arrives late (slow CI run). The end state must
# still be the newer pin.
after_first = bump(BASE, "enscrive-developer", "v20260710-1836")[0]
after_second, second_msg = bump(after_first, "enscrive-developer", "v20260710-1831")
if after_second == after_first and second_msg.startswith("NOT bumping"):
    print("ok   - replay: newer-then-late-older leaves the newer pin standing")
else:
    print("FAIL - replay: newer-then-late-older did not preserve the newer pin")
    print(f"       got content: {after_second!r}")
    print(f"       got msg:     {second_msg!r}")
    FAILS += 1

print()
if FAILS == 0:
    print("ALL PASS")
    sys.exit(0)
else:
    print(f"{FAILS} FAILURE(S)")
    sys.exit(1)
