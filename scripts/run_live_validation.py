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
BOOTSTRAP_V1 = REPO_ROOT / "enscrive-developer" / "e2e-tests" / "scripts" / "bootstrap_v1_fixture.mjs"
BOOTSTRAP_CURRENT_TRUTH = CLI_ROOT / "scripts" / "bootstrap_current_truth_fixture.py"
START_CURRENT_SERVER = REPO_ROOT / "enscrive-developer" / "scripts" / "start-current-server.sh"

MODEL_DIMENSIONS = {
    "text-embedding-3-small": 1536,
    "text-embedding-3-large": 3072,
    "text-embedding-ada-002": 1536,
    "bge-en-icl": 1024,
    "voyage-3": 1024,
    "voyage-3-lite": 512,
    "voyage-code-3": 1024,
    "bge-large-en-v1.5": 1024,
    "bge-small-en-v1.5": 384,
    "bge-base-en-v1.5": 768,
    "bge-m3": 1024,
}

CORE_MANIFESTS = [
    ".enscrive/health/live.json",
    ".enscrive/v1/corpora/list/live-fixture.json",
    ".enscrive/v1/corpora/stats/live-fixture.json",
    ".enscrive/v1/query-embeddings/corpus-model/live-bge-cpu.json",
    ".enscrive/v1/query-embeddings/invalid-voice/live-bge-cpu.json",
    ".enscrive/v1/ingest-prepared/live-bge-cpu.json",
    ".enscrive/v1/corpora/documents/live-bge-cpu.json",
    ".enscrive/v1/corpora/chunks/live-bge-cpu.json",
    ".enscrive/v1/search/basic/live-fixture.json",
    ".enscrive/v1/search/metadata-filter/live-fixture.json",
    ".enscrive/v1/search/invalid-corpus/live.json",
    ".enscrive/v1/usage/live-bge-cpu.json",
    ".enscrive/v1/logs/search/live-observe.json",
    ".enscrive/v1/logs/metrics/live-observe.json",
    ".enscrive/v1/logs/stream/live-observe.json",
    ".enscrive/v1/admin/export-embeddings/live-bge-cpu.json",
    ".enscrive/v1/admin/export-token-usage/live-bge-cpu.json",
]

SUITES = {
    "current-truth-core": {
        "manifests": CORE_MANIFESTS,
        "embedding_model": "text-embedding-3-small",
        "dimensions": 1536,
    },
    "bge-capability": {
        "manifests": CORE_MANIFESTS,
        "embedding_model": "bge-large-en-v1.5",
        "dimensions": 1024,
    },
    "nebius-byok": {
        "manifests": CORE_MANIFESTS
        + [".enscrive/v1/usage/live-nebius-byok.json"],
        "embedding_model": "bge-en-icl",
        "dimensions": 1024,
        "required_env": ["ENSCRIVE_EMBEDDING_PROVIDER_KEY"],
    },
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
        cwd=REPO_ROOT / "enscrive-developer",
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
        paths.extend(
            str((REPO_ROOT / rel_path).resolve())
            for rel_path in SUITES[suite]["manifests"]
        )
    paths.extend(extra_paths)
    return paths


def resolve_fixture_config(
    suites: list[str], override_model: str | None, override_dimensions: int | None
):
    suite_configs = [SUITES[suite] for suite in suites]
    models = {config["embedding_model"] for config in suite_configs}
    dimensions = {config["dimensions"] for config in suite_configs}

    suite_includes_bge = "bge-capability" in suites
    default_model = (
        os.environ.get("BGE_MODEL_NAME")
        if suite_includes_bge and os.environ.get("BGE_MODEL_NAME")
        else next(iter(models))
    )
    embedding_model = override_model or default_model
    embedding_dimensions = override_dimensions or MODEL_DIMENSIONS.get(
        embedding_model, next(iter(dimensions))
    )

    if override_model is None and len(models) > 1:
        raise RuntimeError(
            "selected suites require different fixture embedding models; "
            "pass --fixture-embedding-model explicitly"
        )
    if override_dimensions is None and len(dimensions) > 1:
        raise RuntimeError(
            "selected suites require different fixture embedding dimensions; "
            "pass --fixture-dimensions explicitly"
        )

    return embedding_model, embedding_dimensions


def validate_suite_requirements(suites: list[str], env: dict[str, str]):
    required = []
    for suite in suites:
        required.extend(SUITES[suite].get("required_env", []))
    missing = sorted({name for name in required if not env.get(name)})
    if missing:
        joined = ", ".join(missing)
        raise RuntimeError(
            f"selected suite(s) require environment variable(s): {joined}"
        )


def main():
    parser = argparse.ArgumentParser(
        description="Run live public-stack validation against enscrive-developer /v1"
    )
    parser.add_argument(
        "--base-url",
        default=os.environ.get("ENSCRIVE_BASE_URL", "http://localhost:3000"),
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
        default=os.environ.get("ENSCRIVE_FIXTURE_PREFIX", "codex-v1"),
    )
    parser.add_argument(
        "--fixture-embedding-model",
        default=os.environ.get("ENSCRIVE_FIXTURE_EMBEDDING_MODEL"),
        help="Override the fixture corpus embedding model",
    )
    parser.add_argument(
        "--fixture-dimensions",
        type=int,
        default=(
            int(os.environ["ENSCRIVE_FIXTURE_EMBEDDING_DIMENSIONS"])
            if os.environ.get("ENSCRIVE_FIXTURE_EMBEDDING_DIMENSIONS")
            else None
        ),
        help="Override the fixture corpus embedding dimensions",
    )
    parser.add_argument(
        "--health-timeout-secs",
        type=int,
        default=45,
    )
    parser.add_argument(
        "--start-server",
        action="store_true",
        help="Launch enscrive-developer via start-current-server.sh before running",
    )
    parser.add_argument(
        "--skip-fresh-fixture",
        action="store_true",
        help="Do not mint a fresh tenant/api key fixture",
    )
    parser.add_argument(
        "--skip-current-truth-prepare",
        action="store_true",
        help="Do not create a fresh corpus/document current-truth fixture",
    )
    args = parser.parse_args()

    args.base_url = canonicalize_loopback_base_url(args.base_url)
    suites = args.suite or ["current-truth-core"]
    fixture_embedding_model, fixture_dimensions = resolve_fixture_config(
        suites, args.fixture_embedding_model, args.fixture_dimensions
    )
    artifact_dir = Path(args.artifact_dir) if args.artifact_dir else REPO_ROOT / ".artifacts" / "live-validation" / timestamp()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    env_file = artifact_dir / "fixture.env"
    server_log = artifact_dir / "developer-server.log"
    fixture_screenshot = artifact_dir / "bootstrap-fixture.png"

    env = os.environ.copy()
    env.setdefault("ENSCRIVE_REPO_ROOT", str(REPO_ROOT))
    env["ENSCRIVE_BASE_URL"] = args.base_url
    env["ENSCRIVE_FIXTURE_PREFIX"] = args.prefix
    env["ENSCRIVE_FIXTURE_EMBEDDING_MODEL"] = fixture_embedding_model
    env["ENSCRIVE_FIXTURE_EMBEDDING_DIMENSIONS"] = str(fixture_dimensions)
    validate_suite_requirements(suites, env)
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
            fresh_env["ENSCRIVE_FIXTURE_OUT"] = str(env_file)
            fresh_env["ENSCRIVE_FIXTURE_SCREENSHOT"] = str(fixture_screenshot)
            run_command(
                ["node", str(BOOTSTRAP_V1)],
                REPO_ROOT / "enscrive-developer",
                fresh_env,
            )
            env.update(load_env_file(env_file))

        if not args.skip_current_truth_prepare:
            fixture_env = env.copy()
            fixture_env["ENSCRIVE_FIXTURE_OUT"] = str(env_file)
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
                    "--embedding-model",
                    fixture_embedding_model,
                    "--dimensions",
                    str(fixture_dimensions),
                ],
                CLI_ROOT,
                fixture_env,
            )
            env.update(load_env_file(env_file))

        if "ENSCRIVE_API_KEY" not in env:
            raise RuntimeError(
                "ENSCRIVE_API_KEY is missing. Use a fresh fixture or export an existing API key."
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
            env["ENSCRIVE_API_KEY"],
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
