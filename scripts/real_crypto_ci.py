#!/usr/bin/env python3
"""Closed-vocabulary projection of the existing real-cosign fixture; no raw logs."""
import hashlib
import json
import pathlib
import re
import sys

CASES = ("positive-first", "wrong-pin", "missing-bundle", "changed-manifest",
         "corrupt-bundle", "wrong-identity", "wrong-issuer", "positive-last")

def classify(raw):
    text = raw.decode("utf-8", errors="replace").lower()
    if any(token in text for token in (
        "i/o timeout", "context deadline exceeded", "no such host",
        "connection refused", "tls handshake timeout", "network is unreachable",
    )):
        return "infrastructure_failure"
    if any(token in text for token in (
        "no matching certificate identity", "none of the expected identities matched",
        "certificate identity does not match",
    )):
        return "signer_identity_mismatch"
    if any(token in text for token in (
        "unexpected end of json input", "unexpected eof", "proto: syntax error",
    )):
        return "malformed_bundle"
    # sigstore-go v1.2.0 compatVerifier deliberately discards individual verifier
    # errors. Require BOTH fixed diagnostics from that exact verification loop,
    # not merely an ambiguous "no compatible verifier" failure.
    if ("failed to verify signature with default verifier, trying compatibility verifier" in text
            and "could not verify message: no compatible verifier found" in text):
        return "signature_or_digest_mismatch"
    if any(token in text for token in (
        "invalid signature", "unable to verify signature",
        "artifact digest does not match", "artifact digest mismatch",
        "message digest does not match",
        "artifact digest does not match message digest",
        "artifact does not match digest",
    )):
        return "signature_or_digest_mismatch"
    return "unclassified"

def self_test():
    assert classify(b"invalid signature when validating ASN.1 encoded signature") == "signature_or_digest_mismatch"
    assert classify(b"no matching certificate identity found") == "signer_identity_mismatch"
    assert classify(b"unexpected end of JSON input") == "malformed_bundle"
    assert classify(b"artifact does not match digest") == "signature_or_digest_mismatch"
    assert classify(b"artifact digest does not match message digest") == "signature_or_digest_mismatch"
    assert classify(b"invalid signature; context deadline exceeded") == "infrastructure_failure"
    assert classify(b"unexpected successful process output") == "unclassified"
    assert classify(b"no compatible verifier found") == "unclassified"
    assert classify(b"Failed to verify signature with default verifier, trying compatibility verifier\\nError: could not verify message: no compatible verifier found") == "signature_or_digest_mismatch"

def main(root, test_report, source_head, test_exit):
    if not re.fullmatch(r"[0-9a-f]{40}", source_head):
        raise ValueError("invalid exact source identity")
    matrix = json.loads((root / "matrix.json").read_text())
    report = test_report.read_text(errors="replace")
    selected = re.findall(r"^test result: (.*)$", report, re.M)
    suite_ok = any(re.fullmatch(
        r"ok[.] 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in .+",
        row) for row in selected)
    rows = []
    for name in CASES:
        record = json.loads((root / (name + ".json")).read_text())
        raw = (root / (name + ".diagnostic.bin")).read_bytes()
        if len(raw) > 65536:
            raise ValueError("diagnostic boundary exceeded")
        if hashlib.sha256(raw).hexdigest() != record["diagnostic_sha256"]:
            raise ValueError("diagnostic identity mismatch")
        result = record["result"]
        expected = {
            "positive-first": ("Ok", None), "positive-last": ("Ok", None),
            "wrong-pin": ("Err", "artifact trust: manifest digest mismatch"),
            "missing-bundle": ("Err", "artifact trust: artifact file unavailable"),
            "changed-manifest": ("Err", "artifact trust: signature rejected"),
            "corrupt-bundle": ("Err", "artifact trust: signature rejected"),
            "wrong-identity": ("Err", "artifact trust: signature rejected"),
            "wrong-issuer": ("Err", "artifact trust: signature rejected"),
        }[name]
        outcome_ok = result == {expected[0]: expected[1]}
        if name.startswith("positive-"):
            category, reason_ok = "verified_signature", outcome_ok and bool(raw)
        elif name in ("wrong-pin", "missing-bundle"):
            category, reason_ok = "rejected_before_verifier", outcome_ok and not raw
        else:
            category = classify(raw)
            wanted = ("signature_or_digest_mismatch" if name == "changed-manifest"
                      else "malformed_bundle" if name == "corrupt-bundle"
                      else "signer_identity_mismatch")
            reason_ok = outcome_ok and category == wanted
        rows.append({"case": name, "category": category, "accepted": reason_ok,
                     "diagnostic_sha256": record["diagnostic_sha256"],
                     "diagnostic_bytes": len(raw)})
    passed = (test_exit == 0 and suite_ok and matrix.get("cases") == 8
              and matrix.get("execution_assertions_passed") is True
              and matrix.get("component_installation") is False
              and all(row["accepted"] for row in rows))
    receipt = {"source_head": source_head, "cases": rows, "passed": passed,
               "actual_selected_test_passed": suite_ok, "test_exit": test_exit,
               "inputs": json.loads((root / "inputs.json").read_text()),
               "raw_diagnostics_exposed": False, "component_installation": False}
    (root / "crypto-ci-receipt.json").write_text(json.dumps(receipt, sort_keys=True) + "\n")
    for row in rows:
        level = "notice" if row["accepted"] else "error"
        print(f"::{level}::Real cosign case {row['case']}: {row['category']}; accepted={row['accepted']}")
    if not passed:
        raise ValueError("real signature proof incomplete")

if __name__ == "__main__":
    try:
        self_test()
        if sys.argv[1:] != ["--self-test"]:
            main(pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3], int(sys.argv[4]))
    except (OSError, ValueError, KeyError, TypeError):
        print("::error::Real cosign proof failed closed; use safe case metadata, not raw logs.")
        raise SystemExit(1)
