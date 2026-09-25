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

import io
import urllib.error
import urllib.parse
import urllib.request

_installed = False


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """Refuse every 3xx instead of following it.

    Raises an `HTTPError` for the redirect status (so existing
    `except urllib.error.HTTPError` call sites need no changes) whose
    BODY — what every call site already reads via `e.read()` — names only
    the HTTP status and the redirect target's host. Never the full
    `Location` URL, its query string, or any header value; the original
    (discarded) response body is closed unread."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: N802
        location = headers.get("Location") or headers.get("location") or newurl
        try:
            host = urllib.parse.urlsplit(location).hostname or "an unspecified host"
        except ValueError:
            host = "an unspecified host"
        try:
            fp.close()
        except Exception:  # noqa: BLE001 — best-effort; we're already failing closed
            pass
        message = (
            f"refusing to follow redirect (HTTP {code}) to {host} — "
            "credentials are never resent to a redirect target"
        )
        raise urllib.error.HTTPError(
            req.full_url, code, message, headers, io.BytesIO(message.encode("utf-8"))
        )


def install() -> None:
    """Idempotent: make the no-redirect opener urllib's process-wide
    default. Call once, at import time, before any real network request."""
    global _installed
    if _installed:
        return
    urllib.request.install_opener(urllib.request.build_opener(NoRedirect))
    _installed = True
