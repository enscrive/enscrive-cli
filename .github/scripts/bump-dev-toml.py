#!/usr/bin/env python3
#
# Bumps the [overrides] pin for one roster component in
# enscrive-deploy/channels/dev.toml, called from release-orchestrator.yml's
# "Update channels/dev.toml override" step (working-directory: _deploy).
#
# Monotonic guard: release-published events can arrive out-of-order (a
# slower CI run for an OLDER tag lands after a faster run for a NEWER tag).
# The version scheme (vYYYYMMDD-HHMM, UTC) is lexically comparable, so a
# plain string compare tells us whether the incoming version is actually
# newer. A pin is never allowed to regress backward.
#
# Extracted from an inline workflow heredoc (2026-07-10) so the guard logic
# is unit-testable; see bump-dev-toml-test.py.
#
# ADR: enscrive-governance/plans/PER-COMPONENT-VERSION-PINNING-2026-05-27/ADR.md
# Linear: ENS-538

import os
import re
import sys


def bump(content, roster_key, version):
    """Return (new_content, message). new_content == content means no-op."""

    # Idempotency: if the exact pin already exists, no-op.
    already = re.search(
        rf'^[ \t]*{re.escape(roster_key)}[ \t]*=[ \t]*"{re.escape(version)}"[ \t]*$',
        content, re.MULTILINE,
    )
    if already:
        return content, f"NO_OP: {roster_key} already pinned to {version}"

    # Update existing override line, or insert into [overrides] table.
    existing = re.search(
        rf'^([ \t]*){re.escape(roster_key)}[ \t]*=[ \t]*"([^"]*)"[ \t]*$',
        content, re.MULTILINE,
    )
    if existing:
        current_version = existing.group(2)
        if version < current_version:
            return content, (
                f"NOT bumping {roster_key}: incoming {version} < current "
                f"{current_version} — out-of-order release-published, "
                "keeping newer pin"
            )
        new_content = re.sub(
            rf'^([ \t]*){re.escape(roster_key)}[ \t]*=[ \t]*"[^"]*"[ \t]*$',
            rf'\g<1>{roster_key} = "{version}"',
            content, count=1, flags=re.MULTILINE,
        )
        if new_content == content:
            raise ValueError("planned an edit but content unchanged")
        return new_content, f'WROTE: {roster_key} = "{version}"'

    overrides_header = re.search(r'(^\[overrides\][ \t]*$)', content, re.MULTILINE)
    if not overrides_header:
        raise ValueError("[overrides] section not found in channels/dev.toml")
    # Insert immediately after the [overrides] header line.
    new_content = (
        content[:overrides_header.end()] +
        f'\n{roster_key} = "{version}"' +
        content[overrides_header.end():]
    )
    if new_content == content:
        raise ValueError("planned an edit but content unchanged")
    return new_content, f'WROTE: {roster_key} = "{version}"'


def main():
    path = "channels/dev.toml"
    roster_key = os.environ["ROSTER_KEY"]
    version = os.environ["VERSION"]

    with open(path, "r") as f:
        content = f.read()

    try:
        new_content, message = bump(content, roster_key, version)
    except ValueError as exc:
        sys.stderr.write(f"ERROR: {exc}\n")
        sys.exit(1)

    print(message)
    if new_content == content:
        return

    with open(path, "w") as f:
        f.write(new_content)


if __name__ == "__main__":
    main()
