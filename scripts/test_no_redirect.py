#!/usr/bin/env python3
"""Tests for no_redirect.py and its two callers (ENS-6483 KEYREDIRECT fix
round 3, item 3): run_manifests.py's run_api and
bootstrap_current_truth_fixture.py's request(). A real http.server origin
answers with a 307 pointing at a second, real listener, which must see
ZERO connections — the redirect target answers normally rather than
hanging, so an unprotected implementation would show up as a real hit,
not a test timeout.

    python3 -m unittest discover -s scripts -p test_no_redirect.py -v
"""
from __future__ import annotations

import threading
import time
import unittest
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import bootstrap_current_truth_fixture as bctf
import no_redirect
import run_manifests as rm

no_redirect.install()


def _origin(status: int, location: str | None = None) -> ThreadingHTTPServer:
    """A real loopback server that answers every request with a fixed
    status, and a Location header only when one is given."""

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_a):
            pass

        def _redirect(self):
            self.send_response(status)
            if location is not None:
                self.send_header("Location", location)
            self.send_header("Content-Length", "0")
            self.end_headers()

        do_GET = _redirect
        do_POST = _redirect

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def _attacker():
    """The redirect target. ANSWERS normally (200) rather than hanging —
    a vulnerable (redirect-following) caller gets a real response, not a
    stuck connection, so the test fails on the hit-count assertion, not a
    timeout. Returns (server, hits-list)."""
    hits = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_a):
            pass

        def _record(self):
            hits.append(self.path)
            self.send_response(200)
            self.send_header("Content-Length", "0")
            self.end_headers()

        do_GET = _record
        do_POST = _record

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd, hits


class NoRedirectHandlerTests(unittest.TestCase):
    """The shared handler installed by no_redirect.install(), exercised
    directly over plain urllib.request.urlopen()."""

    def test_refuses_a_307_and_the_target_sees_zero_connections(self):
        attacker, hits = _attacker()
        attacker_host = f"127.0.0.1:{attacker.server_address[1]}"
        origin = _origin(307, f"http://{attacker_host}/steal")
        try:
            origin_base = f"http://127.0.0.1:{origin.server_address[1]}"
            with self.assertRaises(urllib.error.HTTPError) as ctx:
                urllib.request.urlopen(origin_base + "/x")
            self.assertEqual(ctx.exception.code, 307)
            body = ctx.exception.read().decode()
            self.assertIn("307", body)
            self.assertIn("127.0.0.1", body)
        finally:
            origin.shutdown()

        time.sleep(0.2)
        attacker.shutdown()
        self.assertEqual(hits, [], "the redirect target must never be contacted")

    def test_a_redirect_to_a_key_shaped_host_is_redacted(self):
        # No real attacker listener needed here: the Location target
        # (a fake .example domain) is never contacted regardless — this
        # test is only about the message content.
        origin = _origin(
            302,
            "https://enscrive_a1b2c3d4_thisisafakethirtytwocharkey1234.example/steal",
        )
        try:
            origin_base = f"http://127.0.0.1:{origin.server_address[1]}"
            with self.assertRaises(urllib.error.HTTPError) as ctx:
                urllib.request.urlopen(origin_base + "/x")
            body = ctx.exception.read().decode()
            self.assertIn("redacted host", body)
            self.assertNotIn("thisisafakethirtytwocharkey1234", body)
        finally:
            origin.shutdown()

    def test_ordinary_response_still_succeeds(self):
        origin = _origin(200)
        try:
            origin_base = f"http://127.0.0.1:{origin.server_address[1]}"
            with urllib.request.urlopen(origin_base + "/x") as resp:
                self.assertEqual(resp.status, 200)
        finally:
            origin.shutdown()


class RunManifestsRunApiTests(unittest.TestCase):
    """run_manifests.py::run_api — the urllib path (no `max_time_secs`,
    which shells out to curl without `-L` instead and is already safe)."""

    def test_refuses_a_redirect_and_the_target_sees_zero_connections(self):
        attacker, hits = _attacker()
        attacker_host = f"127.0.0.1:{attacker.server_address[1]}"
        origin = _origin(307, f"http://{attacker_host}/steal")
        try:
            origin_base = f"http://127.0.0.1:{origin.server_address[1]}"
            with self.assertRaises(AssertionError) as ctx:
                rm.run_api(
                    origin_base,
                    "fixture-only-key",
                    {"path": "/v1/whatever", "method": "GET", "expect_status": 200},
                )
            self.assertIn("307", str(ctx.exception))
        finally:
            origin.shutdown()

        time.sleep(0.2)
        attacker.shutdown()
        self.assertEqual(hits, [], "the redirect target must never be contacted")


class BootstrapFixtureRequestTests(unittest.TestCase):
    """bootstrap_current_truth_fixture.py::request()."""

    def test_refuses_a_redirect_and_the_target_sees_zero_connections(self):
        attacker, hits = _attacker()
        attacker_host = f"127.0.0.1:{attacker.server_address[1]}"
        origin = _origin(307, f"http://{attacker_host}/steal")
        try:
            origin_base = f"http://127.0.0.1:{origin.server_address[1]}"
            with self.assertRaises(RuntimeError) as ctx:
                bctf.request(origin_base, "fixture-only-key", "GET", "/v1/whatever")
            # item 4: must surface the NoRedirect message (status >= 300
            # now raises), never an opaque JSONDecodeError.
            self.assertIn("307", str(ctx.exception))
            self.assertNotIn("JSONDecodeError", str(ctx.exception))
        finally:
            origin.shutdown()

        time.sleep(0.2)
        attacker.shutdown()
        self.assertEqual(hits, [], "the redirect target must never be contacted")


if __name__ == "__main__":
    unittest.main()
