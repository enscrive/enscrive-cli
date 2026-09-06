#!/usr/bin/env python3
"""ENS-5787: publish a verified, content-addressed aggregate before discovery.

Only publication-protocol immutability is claimed: this does not configure S3
Object Lock or certify a deployment. A regenerated manifest is a new pin-set;
this process freezes one byte string for all signing and upload attempts.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

IDENTITY = "https://github.com/enscrive/enscrive-cli/.github/workflows/manifest.yml@refs/heads/main"
ISSUER = "https://token.actions.githubusercontent.com"
WORKFLOW_REF = "enscrive/enscrive-cli/.github/workflows/manifest.yml@refs/heads/main"


class PublicationError(RuntimeError):
    pass


def guard_identity(env):
    if (env.get("GITHUB_REF") != "refs/heads/main"
            or env.get("GITHUB_WORKFLOW_REF") != WORKFLOW_REF
            or env.get("GITHUB_EVENT_NAME") != "workflow_dispatch"):
        raise PublicationError("pin-set publication requires manifest.yml workflow_dispatch on main")


class S3:
    def __init__(self, bucket):
        self.bucket = bucket

    def command(self, *args):
        return subprocess.run(
            ["aws", "s3api", *args, "--bucket", self.bucket, "--output", "json"],
            capture_output=True, text=True, check=False,
        )

    def get(self, key):
        with tempfile.TemporaryDirectory(prefix="pinset-get-") as tmp:
            dest = Path(tmp) / "object"
            result = self.command("get-object", "--key", key, str(dest))
            if result.returncode:
                # AccessDenied, transport errors, etc. are NOT absence.
                if re.search(r"\((NoSuchKey|404)\)", result.stderr):
                    return None
                raise PublicationError(f"S3 read failed for {key}: {result.stderr.strip()}")
            metadata = json.loads(result.stdout)
            return dest.read_bytes(), metadata["ETag"]

    def put(self, key, data, expected_etag, immutable=False):
        with tempfile.TemporaryDirectory(prefix="pinset-put-") as tmp:
            source = Path(tmp) / "object"
            source.write_bytes(data)
            condition = ["--if-none-match", "*"] if expected_etag is None else ["--if-match", expected_etag]
            result = self.command(
                "put-object", "--key", key, "--body", str(source),
                "--content-type", "application/json", "--cache-control",
                "public, max-age=31536000, immutable" if immutable else "public, max-age=300",
                *condition,
            )
            if result.returncode:
                raise PublicationError(f"S3 conditional write failed for {key}: {result.stderr.strip()}")


class Cosign:
    def verify(self, manifest, bundle):
        with tempfile.TemporaryDirectory(prefix="pinset-verify-") as tmp:
            path = Path(tmp) / "manifest.json"
            sidecar = Path(tmp) / "manifest.bundle"
            path.write_bytes(manifest)
            sidecar.write_bytes(bundle)
            result = subprocess.run([
                "cosign", "verify-blob", "--bundle", str(sidecar),
                "--certificate-identity", IDENTITY,
                "--certificate-oidc-issuer", ISSUER, str(path),
            ], capture_output=True, text=True, check=False)
            if result.returncode:
                raise PublicationError(f"aggregate signature verification failed: {result.stderr.strip()}")

    def sign(self, manifest):
        with tempfile.TemporaryDirectory(prefix="pinset-sign-") as tmp:
            path = Path(tmp) / "manifest.json"
            sidecar = Path(tmp) / "manifest.bundle"
            path.write_bytes(manifest)
            # Inherit stdout/stderr so OIDC/transparency diagnostics stay visible.
            subprocess.run(["cosign", "sign-blob", "--yes", "--bundle", str(sidecar), str(path)], check=True)
            bundle = sidecar.read_bytes()
            self.verify(manifest, bundle)
            return bundle


def create_manifest(store, key, frozen):
    existing = store.get(key)
    if existing is None:
        try:
            store.put(key, frozen, None, immutable=True)
        except PublicationError:
            # A competing identical writer, or a lost success response, is safe
            # only after reading and checking what actually landed.
            existing = store.get(key)
            if existing is None or existing[0] != frozen:
                raise
        existing = store.get(key)
    if existing is None or existing[0] != frozen:
        raise PublicationError(f"occupied content-addressed manifest conflicts: {key}")


def publish(store, signer, frozen, channel, version):
    if channel not in {"dev", "stage", "prod"}:
        raise PublicationError("unknown release channel")
    if not re.fullmatch(r"v[A-Za-z0-9][A-Za-z0-9._-]*", version):
        raise PublicationError("invalid release version path segment")
    parsed = json.loads(frozen)
    if parsed.get("schema_version") != 3 or parsed.get("version") != version:
        raise PublicationError("manifest schema/version does not match publication")
    digest = hashlib.sha256(frozen).hexdigest()
    prefix = f"pinsets/sha256/{digest}"
    manifest_key, bundle_key = f"{prefix}/manifest.json", f"{prefix}/manifest.bundle"
    pointers = [f"releases/{channel}/{version}/manifest.json", f"releases/{channel}/latest.json"]
    # Snapshot BOTH before any publication. These are compatibility discovery
    # copies, not immutable evidence. CAS detects changes during this attempt.
    previous = {key: store.get(key) for key in pointers}
    create_manifest(store, manifest_key, frozen)
    existing = store.get(bundle_key)
    if existing is None:
        generated = signer.sign(frozen)
        try:
            store.put(bundle_key, generated, None, immutable=True)
        except PublicationError:
            existing = store.get(bundle_key)
            if existing is None:
                raise
        existing = store.get(bundle_key)
    if existing is None:
        raise PublicationError("published bundle is absent")
    # Never compare randomized bundle bytes. Verify and reuse the occupied one.
    signer.verify(frozen, existing[0])
    stored_manifest = store.get(manifest_key)
    if stored_manifest is None or stored_manifest[0] != frozen:
        raise PublicationError("stored manifest changed before discovery update")
    signer.verify(stored_manifest[0], existing[0])
    for key in pointers:
        before = previous[key]
        if before is not None and before[0] == frozen:
            # Still check now: another publisher may have changed this pointer
            # while this attempt was verifying the pair.
            current = store.get(key)
            if current is not None and current[0] == frozen:
                continue
            raise PublicationError(f"discovery pointer changed during publication: {key}")
        try:
            store.put(key, frozen, None if before is None else before[1])
        except PublicationError:
            current = store.get(key)
            if current is None or current[0] != frozen:
                raise
    return {"manifest_sha256": digest, "manifest_key": manifest_key,
            "bundle_key": bundle_key, "signer": IDENTITY, "issuer": ISSUER,
            "stored_pair_verified": True, "discovery_updated": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--bucket", required=True)
    parser.add_argument("--channel", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    guard_identity(os.environ)
    frozen = args.manifest.read_bytes()
    receipt = publish(S3(args.bucket), Cosign(), frozen, args.channel, args.version)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
