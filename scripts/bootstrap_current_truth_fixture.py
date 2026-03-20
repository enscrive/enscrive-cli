#!/usr/bin/env python3

import argparse
import json
import os
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlparse, urlunparse

SCRIPT_PATH = Path(__file__).resolve()
REPO_ROOT = SCRIPT_PATH.parents[2]


def iso_now():
    return (
        datetime.now(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
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


def escape_export(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def append_exports(out_path: Path | None, values: dict[str, str]):
    lines = [
        f'export {key}="{escape_export(value)}"' for key, value in values.items()
    ]
    text = "\n".join(lines) + "\n"
    if out_path:
        out_path.parent.mkdir(parents=True, exist_ok=True)
        mode = "a" if out_path.exists() else "w"
        with out_path.open(mode) as f:
            f.write(text)
    print(text, end="")


def default_dimensions_for_model(model: str) -> int | None:
    return {
        "text-embedding-3-small": 1536,
        "text-embedding-3-large": 3072,
        "text-embedding-ada-002": 1536,
        "voyage-3": 1024,
        "voyage-3-lite": 512,
        "voyage-code-3": 1024,
        "bge-large-en-v1.5": 1024,
        "bge-small-en-v1.5": 384,
        "bge-base-en-v1.5": 768,
        "bge-m3": 1024,
    }.get(model)


def request(base_url: str, api_key: str, method: str, path: str, body=None):
    payload = None
    headers = {
        "X-API-Key": api_key,
        "Accept": "application/json",
    }
    if body is not None:
        headers["Content-Type"] = "application/json"
        payload = json.dumps(body).encode("utf-8")

    req = urllib.request.Request(
        f"{base_url.rstrip('/')}/{path.lstrip('/')}",
        data=payload,
        method=method,
        headers=headers,
    )
    try:
        with urllib.request.urlopen(req) as response:
            status = response.status
            text = response.read().decode("utf-8")
    except urllib.error.HTTPError as exc:
        status = exc.code
        text = exc.read().decode("utf-8")

    if status >= 400:
        raise RuntimeError(f"{method} {path} returned {status}: {text}")

    if not text:
        return None
    return json.loads(text)


def wait_for_search_hit(base_url: str, api_key: str, collection_id: str, document_id: str, query: str, metadata_key: str, metadata_value: str, timeout_secs: int):
    deadline = time.time() + timeout_secs
    last_payload = None
    while time.time() < deadline:
        payload = request(
            base_url,
            api_key,
            "POST",
            "/v1/search",
            {
                "query": query,
                "collection_id": collection_id,
                "limit": 5,
                "include_vectors": False,
                "filters": {
                    "metadata": {
                        metadata_key: metadata_value,
                    }
                },
            },
        )
        last_payload = payload
        if any(item.get("document_id") == document_id for item in payload.get("results", [])):
            return payload
        time.sleep(1)

    raise RuntimeError(
        "fixture document did not appear in search results before timeout: "
        f"{json.dumps(last_payload, indent=2)}"
    )


def main():
    parser = argparse.ArgumentParser(
        description="Create a fresh current-truth fixture on public /v1"
    )
    parser.add_argument(
        "--base-url",
        default=os.environ.get("ENSCRIBE_BASE_URL", "http://127.0.0.1:3000"),
    )
    parser.add_argument(
        "--api-key",
        default=os.environ.get("ENSCRIBE_API_KEY"),
    )
    parser.add_argument(
        "--prefix",
        default=os.environ.get("ENSCRIBE_FIXTURE_PREFIX", "codex-v1"),
    )
    parser.add_argument(
        "--out",
        help="Append shell exports to this file",
    )
    parser.add_argument(
        "--embedding-model",
        default=os.environ.get(
            "ENSCRIBE_FIXTURE_EMBEDDING_MODEL", "text-embedding-3-small"
        ),
    )
    parser.add_argument(
        "--dimensions",
        type=int,
        default=(
            int(os.environ["ENSCRIBE_FIXTURE_EMBEDDING_DIMENSIONS"])
            if os.environ.get("ENSCRIBE_FIXTURE_EMBEDDING_DIMENSIONS")
            else None
        ),
    )
    parser.add_argument(
        "--search-timeout-secs",
        type=int,
        default=20,
    )
    args = parser.parse_args()

    if not args.api_key:
        raise RuntimeError("missing ENSCRIBE_API_KEY or --api-key")

    if args.dimensions is None:
        args.dimensions = default_dimensions_for_model(args.embedding_model)
    if args.dimensions is None:
        raise RuntimeError(
            f"unknown dimensions for embedding model '{args.embedding_model}'; pass --dimensions"
        )

    os.environ.setdefault("ENSCRIBE_REPO_ROOT", str(REPO_ROOT))
    args.base_url = canonicalize_loopback_base_url(args.base_url)

    suffix = str(int(time.time()))
    collection_name = f"{args.prefix}-current-truth-{suffix}"
    document_id = f"{args.prefix}-current-truth-doc-{suffix}"
    search_query = f"{args.prefix} mars basalt telemetry fixture"
    metadata_key = "fixture_case"
    metadata_value = f"{args.prefix}-metadata-{suffix}"
    logs_start = iso_now()

    collection = request(
        args.base_url,
        args.api_key,
        "POST",
        "/v1/collections",
        {
            "name": collection_name,
            "description": "Codex live validation current-truth fixture",
            "embedding_model": args.embedding_model,
            "dimensions": args.dimensions,
            "default_voice_id": None,
        },
    )
    collection_id = collection["id"]

    request(
        args.base_url,
        args.api_key,
        "POST",
        "/v1/ingest-prepared",
        {
            "collection_id": collection_id,
            "document_id": document_id,
            "segments": [
                {
                    "content": (
                        "Codex current-truth fixture document about mars basalt telemetry, "
                        "sample return logistics, and regression-proof search coverage."
                    ),
                    "label": "fixture",
                    "confidence": 0.99,
                    "reasoning": "Unique search document for live public-stack validation.",
                    "start_paragraph": 0,
                    "end_paragraph": 0,
                    "metadata": {
                        metadata_key: metadata_value,
                        "fixture": "current-truth",
                        "source": "codex",
                    },
                }
            ],
            "voice_id": None,
        },
    )

    wait_for_search_hit(
        args.base_url,
        args.api_key,
        collection_id,
        document_id,
        search_query,
        metadata_key,
        metadata_value,
        args.search_timeout_secs,
    )

    request(
        args.base_url,
        args.api_key,
        "POST",
        "/v1/query-embeddings",
        {
            "texts": [search_query],
            "collection_id": collection_id,
        },
    )

    time.sleep(2)
    logs_end = iso_now()

    append_exports(
        Path(args.out) if args.out else None,
        {
            "ENSCRIBE_REPO_ROOT": os.environ["ENSCRIBE_REPO_ROOT"],
            "ENSCRIBE_COLLECTION_ID": collection_id,
            "ENSCRIBE_COLLECTION_NAME": collection_name,
            "ENSCRIBE_COLLECTION_EMBEDDING_MODEL": args.embedding_model,
            "ENSCRIBE_COLLECTION_EMBEDDING_DIMENSIONS": str(args.dimensions),
            "ENSCRIBE_EXPECT_DOCUMENT_ID": document_id,
            "ENSCRIBE_SEARCH_QUERY": search_query,
            "ENSCRIBE_METADATA_KEY": metadata_key,
            "ENSCRIBE_METADATA_VALUE": metadata_value,
            "ENSCRIBE_LOGS_START_TIME": logs_start,
            "ENSCRIBE_LOGS_END_TIME": logs_end,
        },
    )


if __name__ == "__main__":
    main()
