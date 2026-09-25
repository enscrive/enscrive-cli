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

`install()` replaces urllib's process-wide default opener with one that
refuses every redirect outright.
"""
from __future__ import annotations

import email.message
import io
import urllib.error
import urllib.parse
import urllib.request

_installed = False


def redact_if_secret_like(host: str) -> str:
    """If `host` matches a known API-key shape, return a fixed placeholder
    instead of the real string — parity with
    `client.rs::redact_if_secret_like`'s key-shape rule. `host` is
    extracted from a 3xx response's `Location` header, which is
    attacker-controlled.

    Deliberately NOT a generic "long token-shaped label" rule — that
    would also redact ordinary long hostnames (AWS ELB names, other long
    service hosts). Unlike the Rust client, no single "live credential"
    value is threaded through here (this opener is process-global, shared
    by every caller, not tied to one client's key) — this rule is the key
    shapes only."""
    return (
        "<redacted host>"
        if any(label.startswith(("enscrive_", "sk-")) for label in host.split("."))
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
        host = redact_if_secret_like(host)
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


def install() -> None:
    """Idempotent: make the no-redirect opener urllib's process-wide
    default. Call once, at import time, before any real network request."""
    global _installed
    if _installed:
        return
    urllib.request.install_opener(urllib.request.build_opener(NoRedirect))
    _installed = True
