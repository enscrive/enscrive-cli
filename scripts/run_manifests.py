#!/usr/bin/env python3

import argparse
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from collections import Counter, defaultdict
from pathlib import Path

import yaml

ENV_PATTERN = re.compile(r"\$\{([A-Z0-9_]+)\}")
EXPORT_PATTERN = re.compile(r'^export\s+([A-Z0-9_]+)="(.*)"$')
MANIFEST_SUFFIXES = {".json", ".yaml", ".yml"}
EMBEDDING_PROVIDER_KEY_ENV = "ENSCRIVE_EMBEDDING_PROVIDER_KEY"
EMBEDDING_PROVIDER_KEY_HEADER = "X-Embedding-Provider-Key"

SCRIPT_PATH = Path(__file__).resolve()
CLI_ROOT = SCRIPT_PATH.parents[1]
REPO_ROOT = SCRIPT_PATH.parents[2]


def load_manifest(path: Path) -> dict:
    with path.open() as f:
        if path.suffix.lower() in {".yaml", ".yml"}:
            manifest = yaml.safe_load(f)
        else:
            manifest = json.load(f)

    if not isinstance(manifest, dict):
        raise RuntimeError(f"manifest {path} must decode to an object")

    manifest = resolve_env(manifest)
    manifest["_manifest_path"] = str(path)
    return manifest


def load_env_file(path: Path):
    with path.open() as f:
        for raw_line in f:
            line = raw_line.strip()
            if not line:
                continue
            match = EXPORT_PATTERN.match(line)
            if not match:
                raise RuntimeError(f"invalid export line in {path}: {line}")
            key, value = match.groups()
            os.environ[key] = value.replace('\\"', '"').replace("\\\\", "\\")


def resolve_env(value):
    if isinstance(value, dict):
        return {resolve_env(k): resolve_env(v) for k, v in value.items()}
    if isinstance(value, list):
        return [resolve_env(v) for v in value]
    if isinstance(value, str):
        full_match = ENV_PATTERN.fullmatch(value)
        if full_match:
            name = full_match.group(1)
            env_value = os.environ.get(name)
            if env_value is None:
                raise RuntimeError(f"missing required environment variable: {name}")
            try:
                return json.loads(env_value)
            except json.JSONDecodeError:
                return env_value

        def replace(match):
            name = match.group(1)
            env_value = os.environ.get(name)
            if env_value is None:
                raise RuntimeError(f"missing required environment variable: {name}")
            return env_value

        return ENV_PATTERN.sub(replace, value)
    return value


def build_request_headers(api_key: str, spec: dict, include_content_type: bool) -> dict[str, str]:
    headers = {"X-API-Key": api_key}
    embedding_provider_key = os.environ.get(EMBEDDING_PROVIDER_KEY_ENV, "").strip()
    if embedding_provider_key:
        headers[EMBEDDING_PROVIDER_KEY_HEADER] = embedding_provider_key
    if spec.get("accept"):
        headers["Accept"] = spec["accept"]
    if include_content_type:
        headers["Content-Type"] = "application/json"
    for name, value in spec.get("headers", {}).items():
        headers[name] = str(value)
    return headers


def json_path_exists(data, path: str) -> bool:
    try:
        json_path_get(data, path)
        return True
    except (AssertionError, IndexError, KeyError, TypeError, ValueError):
        return False


def json_path_get(data, path: str):
    if not path:
        return data

    current = data
    for part in path.split("."):
        if isinstance(current, list):
            try:
                index = int(part)
            except ValueError as exc:
                raise AssertionError(
                    f"list path segment {part!r} is not a valid integer index in {path!r}"
                ) from exc
            try:
                current = current[index]
            except IndexError as exc:
                raise AssertionError(
                    f"list index {index} out of range while resolving {path!r}"
                ) from exc
            continue

        if not isinstance(current, dict):
            raise AssertionError(
                f"cannot resolve {path!r}: encountered non-object value {current!r}"
            )

        if part not in current:
            raise AssertionError(f"path {path!r} is missing key {part!r}")
        current = current[part]
    return current


def type_matches(actual, expected_type: str) -> bool:
    mapping = {
        "array": list,
        "boolean": bool,
        "null": type(None),
        "number": (int, float),
        "object": dict,
        "string": str,
    }
    if expected_type not in mapping:
        raise AssertionError(f"unsupported json check type {expected_type!r}")
    if expected_type == "number" and isinstance(actual, bool):
        return False
    return isinstance(actual, mapping[expected_type])


def length_of(value):
    try:
        return len(value)
    except TypeError as exc:
        raise AssertionError(f"value {value!r} does not have a length") from exc


def assert_json_check(label: str, payload, check: dict):
    path = check.get("path", "")
    op = check.get("op", "eq")

    if op == "exists":
        if not json_path_exists(payload, path):
            raise AssertionError(f"{label}: expected path {path!r} to exist")
        return

    if op == "not_exists":
        if json_path_exists(payload, path):
            raise AssertionError(f"{label}: expected path {path!r} to be absent")
        return

    actual = json_path_get(payload, path)
    expected = check.get("value")

    if op in {"eq", "=="}:
        if actual != expected:
            raise AssertionError(
                f"{label}: expected {path} == {expected!r}, got {actual!r}"
            )
        return

    if op in {"ne", "!="}:
        if actual == expected:
            raise AssertionError(
                f"{label}: expected {path} != {expected!r}, got {actual!r}"
            )
        return

    if op in {"gt", ">"}:
        if actual <= expected:
            raise AssertionError(
                f"{label}: expected {path} > {expected!r}, got {actual!r}"
            )
        return

    if op in {"gte", ">="}:
        if actual < expected:
            raise AssertionError(
                f"{label}: expected {path} >= {expected!r}, got {actual!r}"
            )
        return

    if op in {"lt", "<"}:
        if actual >= expected:
            raise AssertionError(
                f"{label}: expected {path} < {expected!r}, got {actual!r}"
            )
        return

    if op in {"lte", "<="}:
        if actual > expected:
            raise AssertionError(
                f"{label}: expected {path} <= {expected!r}, got {actual!r}"
            )
        return

    if op == "in":
        if actual not in expected:
            raise AssertionError(
                f"{label}: expected {path} value {actual!r} to be in {expected!r}"
            )
        return

    if op == "not_in":
        if actual in expected:
            raise AssertionError(
                f"{label}: expected {path} value {actual!r} to not be in {expected!r}"
            )
        return

    if op == "contains":
        if expected not in actual:
            raise AssertionError(
                f"{label}: expected {path} to contain {expected!r}, got {actual!r}"
            )
        return

    if op == "not_contains":
        if expected in actual:
            raise AssertionError(
                f"{label}: expected {path} to not contain {expected!r}, got {actual!r}"
            )
        return

    if op == "regex":
        if not re.search(expected, str(actual)):
            raise AssertionError(
                f"{label}: expected {path} to match /{expected}/, got {actual!r}"
            )
        return

    if op == "type":
        if not type_matches(actual, expected):
            raise AssertionError(
                f"{label}: expected {path} to be {expected!r}, got {type(actual).__name__}"
            )
        return

    if op == "length_eq":
        actual_length = length_of(actual)
        if actual_length != expected:
            raise AssertionError(
                f"{label}: expected len({path}) == {expected!r}, got {actual_length!r}"
            )
        return

    if op == "length_gte":
        actual_length = length_of(actual)
        if actual_length < expected:
            raise AssertionError(
                f"{label}: expected len({path}) >= {expected!r}, got {actual_length!r}"
            )
        return

    if op == "length_lte":
        actual_length = length_of(actual)
        if actual_length > expected:
            raise AssertionError(
                f"{label}: expected len({path}) <= {expected!r}, got {actual_length!r}"
            )
        return

    if op == "empty":
        actual_length = length_of(actual)
        if actual_length != 0:
            raise AssertionError(f"{label}: expected {path} to be empty, got {actual!r}")
        return

    if op == "not_empty":
        actual_length = length_of(actual)
        if actual_length == 0:
            raise AssertionError(f"{label}: expected {path} to be non-empty")
        return

    if op == "truthy":
        if not actual:
            raise AssertionError(f"{label}: expected {path} to be truthy, got {actual!r}")
        return

    if op == "falsy":
        if actual:
            raise AssertionError(f"{label}: expected {path} to be falsy, got {actual!r}")
        return

    raise AssertionError(f"{label}: unsupported json check op {op!r}")


def assert_expectations(label: str, payload, spec: dict):
    if not spec:
        return

    body_text = payload if isinstance(payload, str) else json.dumps(payload, sort_keys=True)

    if not isinstance(payload, str):
        for path, expected in spec.get("json_equals", {}).items():
            actual = json_path_get(payload, path)
            if actual != expected:
                raise AssertionError(
                    f"{label}: expected {path} == {expected!r}, got {actual!r}"
                )

        for path, expected in spec.get("json_not_equals", {}).items():
            actual = json_path_get(payload, path)
            if actual == expected:
                raise AssertionError(
                    f"{label}: expected {path} != {expected!r}, got {actual!r}"
                )

        for path, minimum in spec.get("json_min", {}).items():
            actual = json_path_get(payload, path)
            if actual < minimum:
                raise AssertionError(
                    f"{label}: expected {path} >= {minimum!r}, got {actual!r}"
                )

        for path, maximum in spec.get("json_max", {}).items():
            actual = json_path_get(payload, path)
            if actual > maximum:
                raise AssertionError(
                    f"{label}: expected {path} <= {maximum!r}, got {actual!r}"
                )

        for path, expected_values in spec.get("json_in", {}).items():
            actual = json_path_get(payload, path)
            if actual not in expected_values:
                raise AssertionError(
                    f"{label}: expected {path} value {actual!r} to be in {expected_values!r}"
                )

        for check in spec.get("json_checks", []):
            assert_json_check(label, payload, check)

    for needle in spec.get("body_contains", []):
        if needle not in body_text:
            raise AssertionError(f"{label}: expected body to contain {needle!r}")

    for needle in spec.get("body_not_contains", []):
        if needle in body_text:
            raise AssertionError(f"{label}: expected body to not contain {needle!r}")

    for pattern in spec.get("body_regex", []):
        if not re.search(pattern, body_text):
            raise AssertionError(f"{label}: expected body to match /{pattern}/")


def run_api(base_url: str, api_key: str, spec: dict):
    url = f"{base_url.rstrip('/')}/{spec['path'].lstrip('/')}"
    if spec.get("max_time_secs") is not None:
        headers = build_request_headers(api_key, spec, "body" in spec)
        command = [
            "curl",
            "-sS",
            "-N",
            "--max-time",
            str(spec["max_time_secs"]),
            "-X",
            spec.get("method", "POST"),
            "-w",
            "\n__STATUS__:%{http_code}\n",
        ]
        for name, value in headers.items():
            command.extend(["-H", f"{name}: {value}"])
        if "body" in spec:
            command.extend(["--data-binary", json.dumps(spec["body"])])

        command.append(url)
        trigger = spec.get("trigger")
        if trigger:
            proc = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                time.sleep(trigger.get("delay_secs", 1))
                trigger_spec = {
                    **trigger,
                    "expect_status": trigger.get("expect_status", 200),
                }
                run_api(base_url, api_key, trigger_spec)
                stdout, stderr = proc.communicate()
            except Exception:
                proc.kill()
                proc.wait()
                raise
            proc_returncode = proc.returncode
        else:
            proc = subprocess.run(command, capture_output=True, text=True)
            stdout = proc.stdout
            stderr = proc.stderr
            proc_returncode = proc.returncode

        allowed_exit_codes = {0}
        if spec.get("allow_timeout_exit"):
            allowed_exit_codes.add(28)

        if proc_returncode not in allowed_exit_codes:
            raise AssertionError(
                f"api: curl transport failed with exit {proc_returncode}\nstdout={stdout}\nstderr={stderr}"
            )

        if "\n__STATUS__:" not in stdout:
            raise AssertionError(
                f"api: streaming response missing status marker\nstdout={stdout}\nstderr={stderr}"
            )

        text, status_text = stdout.rsplit("\n__STATUS__:", 1)
        status = int(status_text.strip().splitlines()[0])

        expected_status = spec["expect_status"]
        if status != expected_status:
            raise AssertionError(f"api: expected status {expected_status}, got {status}")

        if spec.get("expect_json"):
            payload = json.loads(text)
            assert_expectations("api", payload, spec["expect_json"])

        if spec.get("expect_text"):
            assert_expectations("api", text, spec["expect_text"])

        return

    body = None
    if "body" in spec:
        body = json.dumps(spec["body"]).encode("utf-8")
    headers = build_request_headers(api_key, spec, body is not None)
    request = urllib.request.Request(
        url,
        data=body,
        method=spec.get("method", "POST"),
        headers=headers,
    )

    try:
        with urllib.request.urlopen(request) as response:
            status = response.status
            text = response.read().decode("utf-8")
    except urllib.error.HTTPError as exc:
        status = exc.code
        text = exc.read().decode("utf-8")

    expected_status = spec["expect_status"]
    if status != expected_status:
        raise AssertionError(f"api: expected status {expected_status}, got {status}")

    if spec.get("expect_json"):
        payload = json.loads(text)
        assert_expectations("api", payload, spec["expect_json"])

    if spec.get("expect_text"):
        assert_expectations("api", text, spec["expect_text"])


def run_cli(cli_root: Path, base_url: str, api_key: str, spec: dict):
    binary = cli_root / "target" / "debug" / "enscrive"
    if not binary.exists():
        raise RuntimeError(
            f"CLI binary not found at {binary}. Build Enscrive first."
        )

    command = [
        str(binary),
        "--endpoint",
        base_url,
        "--api-key",
        api_key,
        "--output",
        "json",
    ] + spec["args"]

    proc = subprocess.run(
        command,
        cwd=cli_root,
        capture_output=True,
        text=True,
    )

    expected_exit = spec["expect_exit"]
    if proc.returncode != expected_exit:
        raise AssertionError(
            f"cli: expected exit {expected_exit}, got {proc.returncode}\nstdout={proc.stdout}\nstderr={proc.stderr}"
        )

    stdout = proc.stdout.strip()
    if not stdout:
        raise AssertionError("cli: expected JSON output, got empty stdout")

    payload = json.loads(stdout)
    assert_expectations("cli", payload, spec.get("expect_json", {}))
    if spec.get("expect_text"):
        assert_expectations("cli", stdout, spec["expect_text"])


def iter_manifest_paths(paths):
    for raw_path in paths:
        path = Path(raw_path)
        if path.is_dir():
            for child in sorted(
                candidate
                for candidate in path.rglob("*")
                if candidate.is_file() and candidate.suffix.lower() in MANIFEST_SUFFIXES
            ):
                yield child
        else:
            yield path


def manifest_suite(manifest: dict) -> str:
    if manifest.get("suite"):
        return manifest["suite"]
    if manifest.get("expected_status") == "unsupported":
        return "current-honesty"
    if manifest.get("expected_status") == "aspirational":
        return "end-state"
    if str(manifest.get("id", "")).startswith("CT-"):
        return "current-truth"
    return "unspecified"


def manifest_expected_status(manifest: dict) -> str | None:
    if manifest.get("expected_status"):
        return manifest["expected_status"]
    suite = manifest_suite(manifest)
    if suite == "current-truth":
        return "pass"
    if suite == "current-honesty":
        return "unsupported"
    if suite == "end-state":
        return "aspirational"
    return None


def manifest_label(manifest: dict) -> str:
    suite = manifest_suite(manifest)
    expected_status = manifest_expected_status(manifest)
    if expected_status:
        return f"{manifest['id']} [{suite}/{expected_status}]"
    return f"{manifest['id']} [{suite}]"


def main():
    parser = argparse.ArgumentParser(
        description="Run public API + Enscrive parity manifests"
    )
    parser.add_argument("paths", nargs="+", help="Manifest file or directory path(s)")
    parser.add_argument(
        "--env-file",
        action="append",
        default=[],
        help="Shell export file to load before resolving manifests",
    )
    parser.add_argument(
        "--base-url",
        default=os.environ.get("ENSCRIVE_BASE_URL", "http://localhost:3000"),
    )
    parser.add_argument(
        "--api-key",
        default=os.environ.get("ENSCRIVE_API_KEY"),
    )
    parser.add_argument(
        "--suite",
        action="append",
        default=[],
        help="Only run manifests whose suite metadata matches this value",
    )
    args = parser.parse_args()

    os.environ.setdefault("ENSCRIVE_REPO_ROOT", str(REPO_ROOT))
    for raw_path in args.env_file:
        load_env_file(Path(raw_path))

    if not args.api_key:
        raise RuntimeError("missing ENSCRIVE_API_KEY or --api-key")

    failures = []
    suite_counts = defaultdict(Counter)
    selected = 0

    for manifest_path in iter_manifest_paths(args.paths):
        manifest = load_manifest(manifest_path)
        suite = manifest_suite(manifest)
        if args.suite and suite not in args.suite:
            continue

        selected += 1
        label = manifest_label(manifest)
        try:
            for env_name in manifest.get("requires", []):
                if os.environ.get(env_name) is None:
                    raise RuntimeError(
                        f"manifest requires environment variable {env_name}"
                    )

            if "api" not in manifest and "cli" not in manifest:
                raise RuntimeError("manifest must define at least one of api or cli")

            if manifest.get("api"):
                run_api(args.base_url, args.api_key, manifest["api"])
            if manifest.get("cli"):
                run_cli(CLI_ROOT, args.base_url, args.api_key, manifest["cli"])

            suite_counts[suite]["pass"] += 1
            print(f"PASS {label}")
        except Exception as exc:  # noqa: BLE001
            suite_counts[suite]["fail"] += 1
            failures.append((manifest["id"], str(exc)))
            print(f"FAIL {label}: {exc}")

    if selected == 0:
        raise RuntimeError("no manifests matched the requested paths/suite filters")

    total_pass = sum(counter["pass"] for counter in suite_counts.values())
    total_fail = sum(counter["fail"] for counter in suite_counts.values())
    print(f"SUMMARY pass={total_pass} fail={total_fail} selected={selected}")
    for suite in sorted(suite_counts):
        counts = suite_counts[suite]
        print(f"SUITE {suite} pass={counts['pass']} fail={counts['fail']}")

    if failures:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
