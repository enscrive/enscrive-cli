"""Offline publication protocol tests. No S3 or signing services are contacted."""
import hashlib
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

import publish_pinset as p

FROZEN = b'{"schema_version":3,"version":"v1","signature":null,"binaries":{}}\n'
PREFIX = "pinsets/sha256/" + hashlib.sha256(FROZEN).hexdigest()
MANIFEST = PREFIX + "/manifest.json"
BUNDLE = PREFIX + "/manifest.bundle"
VERSIONED = "releases/dev/v1/manifest.json"
LATEST = "releases/dev/latest.json"


class Store:
    def __init__(self):
        self.objects = {}
        self.writes = []
        self.hook = lambda key: None
        self.serial = 0

    def seed(self, key, body):
        self.serial += 1
        self.objects[key] = (body, f'"etag-{self.serial}"')

    def get(self, key):
        return self.objects.get(key)

    def put(self, key, data, expected_etag, immutable=False):
        self.hook(key)
        old = self.get(key)
        if (old is None and expected_etag is not None) or (old is not None and old[1] != expected_etag):
            raise p.PublicationError("conditional conflict")
        self.writes.append((key, immutable))
        self.seed(key, data)


class Signer:
    def __init__(self):
        self.signs = 0
        self.verifies = 0
        self.fail = False

    def sign(self, data):
        self.signs += 1
        return b"signed:" + hashlib.sha256(data).hexdigest().encode() + b":random-new"

    def verify(self, data, bundle):
        self.verifies += 1
        if self.fail or not bundle.startswith(b"signed:" + hashlib.sha256(data).hexdigest().encode() + b":"):
            raise p.PublicationError("invalid bundle")


class Protocol(unittest.TestCase):
    def setUp(self):
        self.store, self.signer = Store(), Signer()

    def run_publish(self):
        return p.publish(self.store, self.signer, FROZEN, "dev", "v1")

    def test_fresh_and_identical_retry_reuse_stored_bundle(self):
        receipt = self.run_publish()
        self.assertTrue(receipt["stored_pair_verified"])
        first = list(self.store.writes)
        self.assertEqual(first, [(MANIFEST, True), (BUNDLE, True), (VERSIONED, False), (LATEST, False)])
        self.run_publish()
        self.assertEqual(self.store.writes, first)
        self.assertEqual(self.signer.signs, 1)

    def test_partial_manifest_recovers_missing_bundle(self):
        self.store.seed(MANIFEST, FROZEN)
        self.run_publish()
        self.assertNotIn((MANIFEST, True), self.store.writes)
        self.assertEqual(self.store.get(LATEST)[0], FROZEN)

    def test_existing_different_valid_bundle_is_reused(self):
        self.store.seed(MANIFEST, FROZEN)
        self.store.seed(BUNDLE, self.signer.sign(FROZEN).replace(b"random-new", b"previous-random"))
        self.signer.signs = 0
        self.run_publish()
        self.assertEqual(self.signer.signs, 0)

    def test_conflicting_manifest_or_bundle_never_advances(self):
        for key, data in [(MANIFEST, b"conflict"), (BUNDLE, b"wrong signer")]:
            with self.subTest(key=key):
                self.setUp()
                self.store.seed(key, data)
                with self.assertRaises(p.PublicationError):
                    self.run_publish()
                self.assertIsNone(self.store.get(LATEST))
                self.assertIsNone(self.store.get(VERSIONED))
                self.assertEqual(self.store.get(key)[0], data)

    def test_failed_verification_never_advances(self):
        self.signer.fail = True
        with self.assertRaises(p.PublicationError):
            self.run_publish()
        self.assertIsNone(self.store.get(VERSIONED))
        self.assertIsNone(self.store.get(LATEST))

    def test_concurrent_identical_manifest_and_valid_bundle(self):
        def race(key):
            if key == MANIFEST:
                self.store.seed(key, FROZEN)
            elif key == BUNDLE:
                self.store.seed(key, self.signer.sign(FROZEN) + b"other")
        self.store.hook = race
        self.run_publish()
        self.assertEqual(self.store.get(LATEST)[0], FROZEN)

    def test_pointer_conflict_preserves_competing_latest(self):
        self.store.seed(LATEST, b"previous")
        def race(key):
            if key == LATEST:
                self.store.seed(key, b"competing publication")
        self.store.hook = race
        with self.assertRaises(p.PublicationError):
            self.run_publish()
        self.assertEqual(self.store.get(LATEST)[0], b"competing publication")
        # Pair remains usable; compatibility aliases cannot commit atomically.
        self.assertEqual(self.store.get(MANIFEST)[0], FROZEN)

    def test_versioned_conflict_does_not_update_latest(self):
        def race(key):
            if key == VERSIONED:
                self.store.seed(key, b"competing")
        self.store.hook = race
        with self.assertRaises(p.PublicationError):
            self.run_publish()
        self.assertIsNone(self.store.get(LATEST))

    def test_lost_success_response_recovers_by_reading_bytes(self):
        original = self.store.put
        def uncertain(*args, **kwargs):
            original(*args, **kwargs)
            raise p.PublicationError("connection lost after write")
        self.store.put = uncertain
        self.run_publish()
        self.assertEqual(self.store.get(LATEST)[0], FROZEN)

    def test_dispatch_identity_guard(self):
        env = {"GITHUB_REF": "refs/heads/main", "GITHUB_WORKFLOW_REF": p.WORKFLOW_REF,
               "GITHUB_EVENT_NAME": "workflow_dispatch"}
        p.guard_identity(env)
        for key, bad in [("GITHUB_REF", "refs/tags/v1"), ("GITHUB_WORKFLOW_REF", p.WORKFLOW_REF.replace("manifest.yml", "release.yml")), ("GITHUB_EVENT_NAME", "push")]:
            with self.subTest(key=key), self.assertRaises(p.PublicationError):
                p.guard_identity({**env, key: bad})


class Adapters(unittest.TestCase):
    @patch("publish_pinset.subprocess.run")
    def test_s3_conditional_arguments_and_etag_quotes(self, run):
        run.return_value = subprocess.CompletedProcess([], 0, "{}", "")
        store = p.S3("test-bucket")
        store.put("key", b"bytes", None, immutable=True)
        args = run.call_args.args[0]
        self.assertEqual(args[args.index("--if-none-match") + 1], "*")
        store.put("latest", b"bytes", '"opaque-etag"')
        args = run.call_args.args[0]
        self.assertEqual(args[args.index("--if-match") + 1], '"opaque-etag"')
        self.assertNotIn("--if-none-match", args)

    @patch("publish_pinset.subprocess.run")
    def test_read_error_is_not_missing(self, run):
        store = p.S3("test-bucket")
        run.return_value = subprocess.CompletedProcess([], 1, "", "An error occurred (NoSuchKey)")
        self.assertIsNone(store.get("key"))
        run.return_value = subprocess.CompletedProcess([], 1, "", "An error occurred (AccessDenied)")
        with self.assertRaises(p.PublicationError):
            store.get("key")

    @patch("publish_pinset.subprocess.run")
    def test_cosign_verifies_exact_bytes_identity_and_issuer(self, run):
        def inspect(args, **kwargs):
            self.assertEqual(Path(args[-1]).read_bytes(), FROZEN)
            self.assertEqual(args[args.index("--certificate-identity") + 1], p.IDENTITY)
            self.assertEqual(args[args.index("--certificate-oidc-issuer") + 1], p.ISSUER)
            self.assertNotIn("--certificate-identity-regexp", args)
            return subprocess.CompletedProcess(args, 0, "", "")
        run.side_effect = inspect
        p.Cosign().verify(FROZEN, b"bundle")


if __name__ == "__main__":
    unittest.main()
