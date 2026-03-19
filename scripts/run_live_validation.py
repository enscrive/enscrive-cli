#!/usr/bin/env python3

import argparse
import os
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlparse, urlunparse

SCRIPT_PATH = Path(__file__).resolve()
CLI_ROOT = SCRIPT_PATH.parents[1]
REPO_ROOT = SCRIPT_PATH.parents[2]
RUN_MANIFESTS = CLI_ROOT / "scripts" / "run_manifests.py"
BOOTSTRAP_V1 = REPO_ROOT / "enscribe-developer" / "e2e-tests" / "scripts" / "bootstrap_v1_fixture.mjs"
BOOTSTRAP_CURRENT_TRUTH = CLI_ROOT / "scripts" / "bootstrap_current_truth_fixture.py"
START_CURRENT_SERVER = REPO_ROOT / "enscribe-developer" / "scripts" / "start-current-server.sh"

SUITES = {
    "current-truth-core": [
        ".enscribe/health/live.json",
        ".enscribe/v1/collections/list/live-fixture.json",
        ".enscribe/v1/collections/stats/live-fixture.json",
        ".enscribe/v1/query-embeddings/collection-model/live-bge-cpu.json",
        ".enscribe/v1/query-embeddings/invalid-voice/live-bge-cpu.json",
        ".enscribe/v1/ingest-prepared/live-bge-cpu.json",
        ".enscribe/v1/collections/documents/live-bge-cpu.json",
        ".enscribe/v1/collections/chunks/live-bge-cpu.json",
        ".enscribe/v1/search/basic/live-fixture.json",
        ".enscribe/v1/search/metadata-filter/live-fixture.json",
        ".enscribe/v1/search/invalid-collection/live.json",
        ".enscribe/v1/usage/live-bge-cpu.json",
        ".enscribe/v1/logs/search/live-observe.json",
        ".enscribe/v1/logs/metrics/live-observe.json",
        ".enscribe/v1/logs/stream/live-observe.json",
        ".enscribe/v1/admin/export-embeddings/live-bge-cpu.json",
        ".enscribe/v1/admin/export-token-usage/live-bge-cpu.json",
    ],
}


def timestamp():
    return (
        datetime.now(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
        .replace(":", "")
    )


def canonicalize_loopback_base_url(base_url: str) -> str:
    parsed = urlparse(base_url)
    if parsed.hostname != "127.0.0.1":
        return base_url
    return urlunparse(
        (
            parsed.scheme,
            f"localhost:{parsed.port}" if parsed.port else "localhost",
            parsed.path,
            parsed.params,
            parsed.query,
            parsed.fragment,
        )
    )


def load_env_file(path: Path):
    values = {}
    if not path.exists():
        return values
    with path.open() as f:
        for raw_line in f:
            line = raw_line.strip()
            if not line:
                continue
            if not line.startswith("export ") or "=" not in line:
                raise RuntimeError(f"invalid export line in {path}: {line}")
            key, raw_value = line[len("export ") :].split("=", 1)
            value = raw_value.strip()
            if value.startswith('"') and value.endswith('"'):
                value = value[1:-1]
            value = value.replace('\\"', '"').replace("\\\\", "\\")
            values[key] = value
    return values


def wait_for_health(base_url: str, timeout_secs: int):
    deadline = time.time() + timeout_secs
    last_error = None
    while time.time() < deadline:
        request = urllib.request.Request(
            f"{base_url.rstrip('/')}/health",
            method="GET",
            headers={"Accept": "application/json"},
        )
        try:
            with urllib.request.urlopen(request) as response:
                if response.status == 200:
                    return
        except urllib.error.URLError as exc:
            last_error = exc
        time.sleep(1)
    raise RuntimeError(f"health check failed for {base_url}: {last_error}")


def start_server(log_path: Path, env: dict[str, str]):
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log_file = log_path.open("w")
    proc = subprocess.Popen(
        [str(START_CURRENT_SERVER)],
        cwd=REPO_ROOT / "enscribe-developer",
        env=env,
        stdout=log_file,
        stderr=subprocess.STDOUT,
        text=True,
        preexec_fn=os.setsid,
    )
    return proc, log_file


def stop_server(proc, log_file):
    if proc.poll() is None:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait(timeout=5)
    log_file.close()


def run_command(command, cwd: Path, env: dict[str, str]):
    subprocess.run(command, cwd=cwd, env=env, check=True)


def resolve_manifest_paths(suites: list[str], extra_paths: list[str]):
    paths = []
    for suite in suites:
        paths.extend(str((REPO_ROOT / rel_path).resolve()) for rel_path in SUITES[suite])
    paths.extend(extra_paths)
    return paths


def main():
    parser = argparse.ArgumentParser(
        description="Run live public-stack validation against enscribe-developer /v1"
    )
    parser.add_argument(
        "--base-url",
        default=os.environ.get("ENSCRIBE_BASE_URL", "http://localhost:3000"),
    )
    parser.add_argument(
        "--suite",
        action="append",
        choices=sorted(SUITES.keys()),
        default=[],
        help="Named manifest suite to run",
    )
    parser.add_argument(
        "--manifests",
        nargs="*",
        default=[],
        help="Additional manifest paths relative to repo root or absolute",
    )
    parser.add_argument(
        "--artifact-dir",
        help="Directory for env files and logs",
    )
    parser.add_argument(
        "--prefix",
        default=os.environ.get("ENSCRIBE_FIXTURE_PREFIX", "codex-v1"),
    )
    parser.add_argument(
        "--health-timeout-secs",
        type=int,
        default=45,
    )
    parser.add_argument(
        "--start-server",
        action="store_true",
        help="Launch enscribe-developer via start-current-server.sh before running",
    )
    parser.add_argument(
        "--skip-fresh-fixture",
        action="store_true",
        help="Do not mint a fresh tenant/api key fixture",
    )
    parser.add_argument(
        "--skip-current-truth-prepare",
        action="store_true",
        help="Do not create a fresh collection/document current-truth fixture",
    )
    args = parser.parse_args()

    args.base_url = canonicalize_loopback_base_url(args.base_url)
    suites = args.suite or ["current-truth-core"]
    artifact_dir = Path(args.artifact_dir) if args.artifact_dir else REPO_ROOT / ".artifacts" / "live-validation" / timestamp()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    env_file = artifact_dir / "fixture.env"
    server_log = artifact_dir / "developer-server.log"
    fixture_screenshot = artifact_dir / "bootstrap-fixture.png"

    env = os.environ.copy()
    env.setdefault("ENSCRIBE_REPO_ROOT", str(REPO_ROOT))
    env["ENSCRIBE_BASE_URL"] = args.base_url
    env["ENSCRIBE_FIXTURE_PREFIX"] = args.prefix
    parsed_base_url = urlparse(args.base_url)
    if parsed_base_url.port:
        env.setdefault("DEVELOPER_PORT", str(parsed_base_url.port))

    server_proc = None
    server_log_file = None
    try:
        if args.start_server:
            server_proc, server_log_file = start_server(server_log, env)

        wait_for_health(args.base_url, args.health_timeout_secs)

        if not args.skip_fresh_fixture:
            fresh_env = env.copy()
            fresh_env["ENSCRIBE_FIXTURE_OUT"] = str(env_file)
            fresh_env["ENSCRIBE_FIXTURE_SCREENSHOT"] = str(fixture_screenshot)
            run_command(
                ["node", str(BOOTSTRAP_V1)],
                REPO_ROOT / "enscribe-developer",
                fresh_env,
            )
            env.update(load_env_file(env_file))

        if not args.skip_current_truth_prepare:
            fixture_env = env.copy()
            fixture_env["ENSCRIBE_FIXTURE_OUT"] = str(env_file)
            run_command(
                [
                    sys.executable,
                    str(BOOTSTRAP_CURRENT_TRUTH),
                    "--base-url",
                    args.base_url,
                    "--out",
                    str(env_file),
                    "--prefix",
                    args.prefix,
                ],
                CLI_ROOT,
                fixture_env,
            )
            env.update(load_env_file(env_file))

        if "ENSCRIBE_API_KEY" not in env:
            raise RuntimeError(
                "ENSCRIBE_API_KEY is missing. Use a fresh fixture or export an existing API key."
            )

        manifest_paths = []
        for raw_path in args.manifests:
            if raw_path.startswith("/"):
                manifest_paths.append(raw_path)
            else:
                manifest_paths.append(str((REPO_ROOT / raw_path).resolve()))
        manifest_paths = resolve_manifest_paths(suites, manifest_paths)

        command = [
            sys.executable,
            str(RUN_MANIFESTS),
            "--base-url",
            args.base_url,
            "--api-key",
            env["ENSCRIBE_API_KEY"],
        ]
        if env_file.exists():
            command.extend(["--env-file", str(env_file)])
        command.extend(manifest_paths)
        run_command(command, CLI_ROOT, env)
        print(f"artifacts: {artifact_dir}")
    finally:
        if server_proc is not None and server_log_file is not None:
            stop_server(server_proc, server_log_file)


if __name__ == "__main__":
    main()
