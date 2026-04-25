#!/bin/sh
# ENS-81 CLI-REL-010: enscrive CLI installer
#
# One-liner (dev channel — active while production CloudFront is not yet provisioned):
#   curl -fsSL https://dev.enscrive.io/install | sh
#
# Production one-liner (post-GA, tracked as a sub-ticket of ENS-81):
#   curl -fsSL https://enscrive.io/install | sh
#
# TODO(team-lead): after merging this branch, publish the script to S3 / invalidate CDN:
#   aws s3 cp installer/install.sh s3://enscrive-install-artifacts-dev/install.sh
#   aws cloudfront create-invalidation --distribution-id EWE9BH1POOS0A --paths /install
#
# Design decision — CLI-only install (ENS-81 resolved):
#   install.sh fetches ONLY the `enscrive` CLI binary.  The three service binaries
#   (enscrive-developer, enscrive-observe, enscrive-embed) are NOT installed here;
#   they are fetched later by `enscrive init --mode self-managed`, which consumes
#   the cross-repo manifest published by the Factory Squad (ENS-91).
#   Governance memo: installer/DESIGN-DECISIONS.md §1.
#
# Manifest schema (ENS-91 cross-repo manifest, §2.3 of RELEASE-INDUSTRIALIZATION-2026-04-23):
#   binaries.enscrive.platforms[<target>] -> { url, sha256 }
#
# Supported flags:
#   --target=<triple>     Override platform detection (e.g. for cross-machine prep)
#   --prefix=<dir>        Override install directory (default: ~/.local/bin)
#   --manifest-url=<url>  Override manifest URL (used by smoke tests and CI)
#   --insecure            Skip cosign verification even if cosign is on PATH (ENS-82)
#
# Supported platforms (Rust target triples):
#   x86_64-unknown-linux-gnu   Linux  x86_64 (glibc — default for non-musl Linux)
#   x86_64-unknown-linux-musl  Linux  x86_64 (musl — Alpine and similar)
#   aarch64-unknown-linux-gnu  Linux  aarch64 (glibc)
#   aarch64-apple-darwin       macOS  Apple Silicon
#   x86_64-apple-darwin        macOS  Intel

set -eu

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
DEV_MANIFEST_URL="https://dev.enscrive.io/releases/dev/latest.json"
DEFAULT_PREFIX="$HOME/.local/bin"
MANIFEST_URL="${ENSCRIVE_MANIFEST_URL:-$DEV_MANIFEST_URL}"
PREFIX="${ENSCRIVE_INSTALL_PREFIX:-$DEFAULT_PREFIX}"
TARGET_OVERRIDE=""
INSECURE=0

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
for arg in "$@"; do
    case "$arg" in
        --target=*)   TARGET_OVERRIDE="${arg#--target=}" ;;
        --prefix=*)   PREFIX="${arg#--prefix=}" ;;
        --manifest-url=*) MANIFEST_URL="${arg#--manifest-url=}" ;;
        --insecure)   INSECURE=1 ;;
        --help|-h)
            cat <<'USAGE'
Usage: install.sh [OPTIONS]

Options:
  --target=<triple>     Override platform detection
  --prefix=<dir>        Install directory (default: ~/.local/bin)
  --manifest-url=<url>  Override manifest URL (for testing/CI)
  --insecure            Skip cosign bundle verification (ENS-82)
  --help                Show this message
USAGE
            exit 0
            ;;
        *)
            echo "Unknown option: $arg" >&2
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Utilities
# ---------------------------------------------------------------------------
require_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: missing required command: $1" >&2
        exit 1
    fi
}

# ---------------------------------------------------------------------------
# Platform detection → Rust target triple
#
# Maps `uname -sm` output to the canonical Rust target triple:
#   Linux x86_64  → x86_64-unknown-linux-gnu  (or musl on Alpine)
#   Linux aarch64 → aarch64-unknown-linux-gnu
#   Darwin arm64  → aarch64-apple-darwin
#   Darwin x86_64 → x86_64-apple-darwin
# ---------------------------------------------------------------------------
detect_target() {
    _os="$(uname -s)"
    _arch="$(uname -m)"

    case "$_os" in
        Linux)
            case "$_arch" in
                x86_64|amd64)
                    # Detect musl: Alpine and similar distros install the musl dynamic linker
                    # at /lib/ld-musl-x86_64.so.1 (or similar).
                    if ls /lib/ld-musl-* >/dev/null 2>&1; then
                        printf "x86_64-unknown-linux-musl"
                    else
                        printf "x86_64-unknown-linux-gnu"
                    fi
                    ;;
                aarch64|arm64)
                    # aarch64 musl is less common; default to gnu for now.
                    printf "aarch64-unknown-linux-gnu"
                    ;;
                *)
                    echo "error: unsupported Linux architecture: $_arch" >&2
                    exit 1
                    ;;
            esac
            ;;
        Darwin)
            case "$_arch" in
                arm64|aarch64) printf "aarch64-apple-darwin" ;;
                x86_64)        printf "x86_64-apple-darwin" ;;
                *)
                    echo "error: unsupported macOS architecture: $_arch" >&2
                    exit 1
                    ;;
            esac
            ;;
        *)
            echo "error: unsupported operating system: $_os" >&2
            exit 1
            ;;
    esac
}

# ---------------------------------------------------------------------------
# SHA256 verification (pure POSIX sh — no Python required)
# ---------------------------------------------------------------------------
sha256_file() {
    _path="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$_path" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$_path" | cut -d' ' -f1
    else
        echo "error: neither sha256sum nor shasum found; cannot verify download" >&2
        exit 1
    fi
}

# ---------------------------------------------------------------------------
# JSON field extraction (pure awk — no jq/python required)
# Reads the manifest JSON and extracts url/sha256 for the given binary+target.
#
# Expected manifest shape (ENS-91 schema, §2.3 RELEASE-INDUSTRIALIZATION):
# {
#   "binaries": {
#     "enscrive": {
#       "platforms": {
#         "x86_64-unknown-linux-gnu": { "url": "...", "sha256": "..." }
#       }
#     }
#   }
# }
# ---------------------------------------------------------------------------
extract_field() {
    _manifest="$1"   # path to manifest JSON file
    _target="$2"     # target triple string
    _field="$3"      # "url" or "sha256"

    # Use Python if available for reliable JSON parsing; fall back to awk grep.
    if command -v python3 >/dev/null 2>&1; then
        python3 - "$_manifest" "$_target" "$_field" <<'PY'
import json, sys, pathlib
manifest = json.loads(pathlib.Path(sys.argv[1]).read_text())
target   = sys.argv[2]
field    = sys.argv[3]
try:
    entry = manifest["binaries"]["enscrive"]["platforms"][target]
    print(entry[field])
except KeyError as e:
    print(f"error: manifest key not found: {e}", file=sys.stderr)
    sys.exit(1)
PY
    else
        # Minimal awk fallback: works for well-formatted single-line-value JSON.
        awk -v target="$_target" -v field="$_field" '
            /\"'"$_target"'"/ { in_target=1 }
            in_target && /\"'"$_field"'"/ {
                match($0, /": *"([^"]+)"/, a)
                # awk match with 3-arg not portable; use split instead
                n = split($0, parts, "\"")
                for (i=1;i<=n;i++) {
                    if (parts[i] == field) { print parts[i+2]; exit }
                }
            }
        ' "$_manifest"
    fi
}

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------
require_cmd curl

# ---------------------------------------------------------------------------
# Resolve target triple
# ---------------------------------------------------------------------------
if [ -n "$TARGET_OVERRIDE" ]; then
    TARGET="$TARGET_OVERRIDE"
    echo "Target override: $TARGET"
else
    TARGET="$(detect_target)"
    echo "Detected platform: $TARGET"
fi

# ---------------------------------------------------------------------------
# Temporary workspace
# ---------------------------------------------------------------------------
TMPDIR_WORK="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_WORK"' EXIT HUP INT TERM

MANIFEST_PATH="$TMPDIR_WORK/latest.json"
BINARY_TMP="$TMPDIR_WORK/enscrive"

# ---------------------------------------------------------------------------
# Fetch manifest
# ---------------------------------------------------------------------------
echo "Fetching manifest from $MANIFEST_URL ..."
curl -fsSL "$MANIFEST_URL" -o "$MANIFEST_PATH"

# ---------------------------------------------------------------------------
# Parse url + sha256 from manifest
# ---------------------------------------------------------------------------
BINARY_URL="$(extract_field "$MANIFEST_PATH" "$TARGET" "url")"
EXPECTED_SHA="$(extract_field "$MANIFEST_PATH" "$TARGET" "sha256")"

if [ -z "$BINARY_URL" ] || [ "$BINARY_URL" = "error:*" ]; then
    echo "error: could not find binary URL for target '$TARGET' in manifest" >&2
    echo "       manifest: $MANIFEST_URL" >&2
    exit 1
fi

if [ -z "$EXPECTED_SHA" ]; then
    echo "error: could not find sha256 for target '$TARGET' in manifest" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Download binary
# ---------------------------------------------------------------------------
echo "Downloading enscrive ($TARGET) ..."
curl -fSL --retry 3 --retry-delay 2 --max-time 0 \
    --progress-bar \
    "$BINARY_URL" -o "$BINARY_TMP"

# ---------------------------------------------------------------------------
# SHA256 verification (mandatory)
# ---------------------------------------------------------------------------
echo "Verifying SHA256 ..."
ACTUAL_SHA="$(sha256_file "$BINARY_TMP")"
if [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
    echo "error: SHA256 mismatch" >&2
    echo "  expected: $EXPECTED_SHA" >&2
    echo "  actual:   $ACTUAL_SHA" >&2
    exit 1
fi
echo "SHA256 OK."

# ---------------------------------------------------------------------------
# cosign bundle verification (optional, ENS-82)
#
# ENS-82 (cosign signing workflow) will publish a .bundle file alongside each
# binary once the signing workflow is dispatched.  We check for the bundle and
# verify if cosign is on PATH.  If the bundle is absent or cosign is not
# installed we warn but do not fail — this is expected during beta until ENS-82
# cuts its first signed release.
#
# Pass --insecure to unconditionally skip cosign verification.
# ---------------------------------------------------------------------------
BUNDLE_URL="${BINARY_URL}.bundle"
BUNDLE_TMP="$TMPDIR_WORK/enscrive.bundle"

if [ "$INSECURE" -eq 1 ]; then
    echo "cosign: skipped (--insecure)"
elif ! command -v cosign >/dev/null 2>&1; then
    echo "cosign: not on PATH — skipping bundle verification (install cosign for supply-chain verification)"
else
    echo "Fetching cosign bundle ..."
    if curl -fsSL --max-time 10 "$BUNDLE_URL" -o "$BUNDLE_TMP" 2>/dev/null; then
        echo "Verifying cosign bundle ..."
        if cosign verify-blob \
            --bundle "$BUNDLE_TMP" \
            --certificate-identity-regexp "https://github.com/enscrive/enscrive-cli/.*" \
            --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
            "$BINARY_TMP"; then
            echo "cosign: bundle verified OK."
        else
            echo "error: cosign bundle verification failed" >&2
            exit 1
        fi
    else
        echo "cosign: bundle not yet published for this release — skipping (expected pre-ENS-82)"
    fi
fi

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------
mkdir -p "$PREFIX"
INSTALL_PATH="$PREFIX/enscrive"
cp "$BINARY_TMP" "$INSTALL_PATH"
chmod 0755 "$INSTALL_PATH"

echo ""
echo "Installed: $INSTALL_PATH"
echo ""

# Check whether PREFIX is on PATH and advise if not.
case ":$PATH:" in
    *":$PREFIX:"*)
        echo "enscrive is ready.  Run: enscrive --version"
        ;;
    *)
        echo "NOTE: $PREFIX is not on your PATH."
        echo "Add it to your shell profile, then run 'enscrive --version':"
        echo ""
        echo "  For bash:   echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc"
        echo "  For zsh:    echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
        echo ""
        ;;
esac

echo "Next steps:"
echo "  enscrive init --mode managed      # connect to api.enscrive.io"
echo "  enscrive init --mode self-managed # fetch service stack + run locally"
echo ""
echo "Docs: https://docs.enscrive.io"
