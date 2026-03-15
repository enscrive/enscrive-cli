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
from pathlib import Path

ENV_PATTERN = re.compile(r"\$\{([A-Z0-9_]+)\}")


def load_manifest(path: Path) -> dict:
    with path.open() as f:
        manifest = json.load(f)
    return resolve_env(manifest)


def resolve_env(value):
    if isinstance(value, dict):
        return {k: resolve_env(v) for k, v in value.items()}
    if isinstance(value, list):
        return [resolve_env(v) for v in value]
    if isinstance(value, str):
        def replace(match):
            name = match.group(1)
            env_value = os.environ.get(name)
            if env_value is None:
                raise RuntimeError(f"missing required environment variable: {name}")
            return env_value

        return ENV_PATTERN.sub(replace, value)
    return value


def json_path_get(data, path: str):
    current = data
    for part in path.split("."):
        if isinstance(current, list):
            current = current[int(part)]
        else:
            current = current[part]
    return current


def assert_expectations(label: str, payload, spec: dict):
    for path, expected in spec.get("json_equals", {}).items():
        actual = json_path_get(payload, path)
        if actual != expected:
            raise AssertionError(
                f"{label}: expected {path} == {expected!r}, got {actual!r}"
            )

    for path, minimum in spec.get("json_min", {}).items():
        actual = json_path_get(payload, path)
        if actual < minimum:
            raise AssertionError(
                f"{label}: expected {path} >= {minimum!r}, got {actual!r}"
            )

    for needle in spec.get("body_contains", []):
        if needle not in payload:
            raise AssertionError(f"{label}: expected body to contain {needle!r}")


def run_api(base_url: str, api_key: str, spec: dict):
    url = f"{base_url.rstrip('/')}/{spec['path'].lstrip('/')}"
    if spec.get("max_time_secs") is not None:
        command = [
            "curl",
            "-sS",
            "-N",
            "--max-time",
            str(spec["max_time_secs"]),
            "-X",
            spec.get("method", "POST"),
            "-H",
            f"X-API-Key: {api_key}",
            "-w",
            "\n__STATUS__:%{http_code}\n",
        ]

        accept = spec.get("accept")
        if accept:
            command.extend(["-H", f"Accept: {accept}"])

        if "body" in spec:
            command.extend(["-H", "Content-Type: application/json"])
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
    request = urllib.request.Request(
        url,
        data=body,
        method=spec.get("method", "POST"),
        headers={
            "Content-Type": "application/json",
            "X-API-Key": api_key,
        },
    )

    if spec.get("accept"):
        request.add_header("Accept", spec["accept"])

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
    binary = cli_root / "target" / "debug" / "enscribe-cli"
    if not binary.exists():
        raise RuntimeError(
            f"CLI binary not found at {binary}. Build enscribe-CLI first."
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
            for child in sorted(path.rglob("*.json")):
                yield child
        else:
            yield path


def main():
    parser = argparse.ArgumentParser(
        description="Run public API + enscribe-CLI parity manifests"
    )
    parser.add_argument("paths", nargs="+", help="Manifest file or directory path(s)")
    parser.add_argument(
        "--base-url",
        default=os.environ.get("ENSCRIBE_BASE_URL", "http://127.0.0.1:3000"),
    )
    parser.add_argument(
        "--api-key",
        default=os.environ.get("ENSCRIBE_API_KEY"),
    )
    args = parser.parse_args()

    if not args.api_key:
        raise RuntimeError("missing ENSCRIBE_API_KEY or --api-key")

    cli_root = Path(__file__).resolve().parents[1]
    failures = []

    for manifest_path in iter_manifest_paths(args.paths):
        manifest = load_manifest(manifest_path)
        manifest_id = manifest["id"]
        try:
            for env_name in manifest.get("requires", []):
                if os.environ.get(env_name) is None:
                    raise RuntimeError(
                        f"manifest requires environment variable {env_name}"
                    )

            run_api(args.base_url, args.api_key, manifest["api"])
            run_cli(cli_root, args.base_url, args.api_key, manifest["cli"])
            print(f"PASS {manifest_id}")
        except Exception as exc:  # noqa: BLE001
            failures.append((manifest_id, str(exc)))
            print(f"FAIL {manifest_id}: {exc}")

    if failures:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
