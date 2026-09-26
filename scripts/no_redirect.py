#!/usr/bin/env python3
"""Refuse every HTTP redirect on urllib requests that carry an Enscrive
credential header (ENS-6483 KEYREDIRECT).

urllib's default opener follows up to 10 redirects (301/302/303/307/308)
and resends every header set via `Request.add_header`/the `headers=` dict
to wherever `Location` points, including across hosts. Unlike
`Authorization`, urllib does not special-case or strip `X-API-Key` /
`X-Embedding-Provider-Key` on a cross-host hop, so a redirect from a
misconfigured or MITM'd `--base-url` would resend the key to whatever host
`Location` names.

`urlopen()` is a drop-in replacement for `urllib.request.urlopen()` that
sends the request through a MODULE-LEVEL opener built once here, not
urllib's process-wide default. Each script that authenticates with
`X-API-Key` calls `no_redirect.urlopen(...)` instead of
`urllib.request.urlopen(...)` at its own send sites. This (rather than a
single `install()` mutating the global default) is deliberate: a global
opener can't be probed per script — removing the protection from one
script would leave every OTHER script's tests still green, since they'd
all still route through whatever the last `install()` call configured.
Routing each script's own calls through its own reference to `_OPENER`
makes each script's protection independently testable, with a plain
`urllib.request.urlopen` (or a client built with the library default) as
a valid negative control that DOES follow.
"""
from __future__ import annotations

import email.message
import io
import urllib.error
import urllib.parse
import urllib.request


def credential_header_values(req) -> list:
    """Every value of a header on `req` whose name looks like an API-key
    credential (`X-API-Key`, `X-Embedding-Provider-Key`, or any other
    `*-Key` header) — read directly off the request that is actually
    being sent, not a hardcoded, easily-stale header-name list. urllib
    normalizes header names via `str.capitalize()` (e.g. `X-API-Key` is
    stored as `X-api-key`), so this matches case-insensitively."""
    return [
        value
        for name, value in req.header_items()
        if name.lower() == "x-api-key" or name.lower().endswith("-key")
    ]


def redact_if_secret_like(host: str, credentials=()) -> str:
    """If `host` contains any of `credentials` verbatim (case-insensitively
    — an attacker-controlled `Location` host is not guaranteed to
    preserve the credential's original casing), or matches a known
    API-key shape, return a fixed placeholder instead of the real string
    — parity with `client.rs::redact_if_secret_like`. `host` is extracted
    from a 3xx response's `Location` header, which is attacker-controlled.

    Deliberately NOT a generic "long token-shaped label" rule for the key
    shape — that would also redact ordinary long hostnames (AWS ELB
    names, other long service hosts)."""
    host_lower = host.lower()
    if any(c and c.lower() in host_lower for c in credentials):
        return "<redacted host>"
    return (
        "<redacted host>"
        if any(label.startswith(("enscrive_", "sk-")) for label in host_lower.split("."))
        else host
    )


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """Refuse every 3xx instead of following it.

    Raises an `HTTPError` for the redirect status (so existing
    `except urllib.error.HTTPError` call sites need no changes) whose
    BODY — what every call site already reads via `e.read()` — names only
    the HTTP status and the redirect target's host. Never the full
    `Location` URL, its query string, or any header value; the original
    (discarded) response body is closed unread.

    The raised `HTTPError` itself is built with EMPTY `headers` and `url`
    — never the real ones. `exc.headers` would otherwise still carry the
    origin's raw, unredacted `Location` (query string included) for any
    caller that inspects it directly instead of `exc.read()`; `exc.url`
    serves no purpose here a caller should ever need. Call sites must use
    `exc.code` and `exc.read()` only — never `exc.msg`, `exc.headers` or
    `exc.url`, all of which this raises as empty/safe placeholders on
    purpose, not as an oversight."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: N802
        location = headers.get("Location") or headers.get("location") or newurl
        try:
            host = urllib.parse.urlsplit(location).hostname or "an unspecified host"
        except ValueError:
            host = "an unspecified host"
        host = redact_if_secret_like(host, credential_header_values(req))
        try:
            fp.close()
        except Exception:  # noqa: BLE001 — best-effort; we're already failing closed
            pass
        message = (
            f"refusing to follow redirect (HTTP {code}) to {host} — "
            "credentials are never resent to a redirect target"
        )
        raise urllib.error.HTTPError(
            "", code, message, email.message.Message(), io.BytesIO(message.encode("utf-8"))
        )


_OPENER = urllib.request.build_opener(NoRedirect)


def urlopen(request, timeout=None):
    """Drop-in replacement for `urllib.request.urlopen(request,
    timeout=timeout)`: same return value and exceptions, except a 3xx is
    refused (see `NoRedirect`) instead of followed."""
    return _OPENER.open(request, timeout=timeout)
