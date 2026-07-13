#!/bin/sh
# ENS-81 CLI-REL-010: enscrive CLI installer
#
# Public install command:
#   curl -fsSL https://install.enscrive.io/install.sh | sh
#
# install.enscrive.io is a public alias on CloudFront distribution
# EWE9BH1POOS0A fronting s3://enscrive-install-artifacts-dev. WAF was
# dropped 2026-04-27; the path is open from anywhere on Earth.
#
# Re-publish after edits:
#   aws s3 cp installer/install.sh s3://enscrive-install-artifacts-dev/install.sh \
#     --content-type 'text/x-shellscript; charset=utf-8' \
#     --cache-control 'public, max-age=300'
#   aws cloudfront create-invalidation --distribution-id EWE9BH1POOS0A --paths '/install.sh'
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
#   --insecure            Skip cosign verification (DANGEROUS — see below, ENS-82/ENS-1014)
#
# cosign verification is MANDATORY by default (ENS-82 keyless signing via
# Fulcio/Rekor is live platform-wide; every release publishes a .bundle).
# install.enscrive.io has no WAF (see above) — verification is the only
# thing standing between a tampered/unsigned binary and an unattended
# `curl | sh`. Missing cosign, a missing bundle, or a failed verification
# now HARD-FAILS the install. `--insecure` is the sole documented bypass.
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
DEFAULT_MANIFEST_URL="https://install.enscrive.io/releases/dev/latest.json"
DEFAULT_PREFIX="$HOME/.local/bin"
MANIFEST_URL="${ENSCRIVE_MANIFEST_URL:-$DEFAULT_MANIFEST_URL}"
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
  --insecure            Skip cosign bundle verification (DANGEROUS, ENS-82/ENS-1014)
  --help                Show this message

cosign bundle verification is mandatory by default. Missing cosign, a
missing/undownloadable .bundle, or a failed verification will abort the
install with a non-zero exit code. Pass --insecure only if you explicitly
accept the risk of installing an unverified binary.
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

    # Prefer python3 (reliable JSON parsing); then jq; then a brace-aware awk
    # scoped to the binaries.enscrive sub-object specifically.
    #
    # CRITICAL: the awk fallback MUST be scope-aware. The cross-repo manifest
    # contains four `binaries.<repo>.platforms.<target>` blocks (enscrive,
    # enscrive-developer, enscrive-observe, enscrive-embed) — each uses the
    # same target triple keys. Earlier this fallback matched the first
    # occurrence of the target string in the file and silently installed the
    # wrong binary (whichever repo happened to appear first in the manifest's
    # JSON), so tracking depth via brace count is required.
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
    elif command -v jq >/dev/null 2>&1; then
        jq -er --arg target "$_target" --arg field "$_field" \
            '.binaries.enscrive.platforms[$target][$field]' "$_manifest"
    else
        # Brace-aware awk fallback. Enters enscrive scope on `"enscrive":`
        # (rejecting `"enscrive-developer":` etc. via the trailing `\":`
        # marker plus a length check), counts depth across braces, and only
        # extracts the target's field while inside that scope at depth >= 1.
        awk -v target="$_target" -v field="$_field" '
            BEGIN { in_enscrive = 0; depth = 0; in_target = 0 }
            {
                # Detect entry into the binaries.enscrive object. Match the
                # exact key "enscrive": (NOT enscrive-developer, etc.).
                if (!in_enscrive && match($0, /"enscrive"[[:space:]]*:[[:space:]]*\{/)) {
                    in_enscrive = 1
                    depth = 1
                    next
                }
                if (in_enscrive) {
                    # Count brace depth changes on this line.
                    line = $0
                    n_open = gsub(/\{/, "{", line)
                    n_close = gsub(/\}/, "}", line)
                    depth += n_open - n_close
                    if (depth <= 0) { in_enscrive = 0; in_target = 0; next }

                    # Inside enscrive scope. Watch for the target key.
                    if (match($0, "\"" target "\"[[:space:]]*:[[:space:]]*\\{")) {
                        in_target = 1
                        next
                    }
                    if (in_target && match($0, "\"" field "\"[[:space:]]*:[[:space:]]*\"[^\"]+\"")) {
                        n = split($0, parts, "\"")
                        for (i = 1; i <= n; i++) {
                            if (parts[i] == field) {
                                print parts[i + 2]
                                exit
                            }
                        }
                    }
                    # Leaving the target sub-object on a closing brace at
                    # the right depth: rough heuristic. Once the field is
                    # found and printed the script exits; otherwise the
                    # in_target flag persists until the enscrive-scope
                    # closing brace drops depth back to 0.
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
# cosign bundle verification (MANDATORY, ENS-82 — fail-closed, ENS-1014)
#
# ENS-82 keyless signing (Fulcio/Rekor, no COSIGN_* key material) is live
# platform-wide: every release publishes a .bundle alongside its binary
# (.github/workflows/release.yml, "cosign keyless signing" job). This
# installer is served from install.enscrive.io, a CloudFront distribution
# with no WAF (see file header) — cosign verification is the only barrier
# between a tampered/unsigned binary and an unattended `curl | sh`.
#
# Missing cosign, a missing/undownloadable bundle, or a failed verification
# now HARD-FAILS the install (non-zero exit, nothing is installed). The only
# bypass is the documented --insecure flag, which prints a loud warning.
# ---------------------------------------------------------------------------
BUNDLE_URL="${BINARY_URL}.bundle"
BUNDLE_TMP="$TMPDIR_WORK/enscrive.bundle"

if [ "$INSECURE" -eq 1 ]; then
    echo "" >&2
    echo "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!" >&2
    echo "!! WARNING: --insecure passed. Skipping cosign signature verification. !!" >&2
    echo "!! The binary about to be installed has NOT been cryptographically     !!" >&2
    echo "!! verified against enscrive's release identity (github.com/enscrive/  !!" >&2
    echo "!! enscrive-cli via Fulcio/Rekor keyless signing). Only proceed if you  !!" >&2
    echo "!! understand and accept the supply-chain risk.                        !!" >&2
    echo "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!" >&2
    echo "" >&2
else
    if ! command -v cosign >/dev/null 2>&1; then
        echo "" >&2
        echo "error: cosign is required to verify this binary's signature but was not found on PATH." >&2
        echo "" >&2
        echo "  ENS-82 keyless signing is live for every enscrive-cli release; this" >&2
        echo "  installer fails closed rather than install an unverified binary from" >&2
        echo "  a distribution with no WAF (see install.sh header)." >&2
        echo "" >&2
        echo "  Install cosign, then re-run this installer:" >&2
        echo "    https://docs.sigstore.dev/cosign/system_config/installation/" >&2
        echo "    macOS (Homebrew):  brew install cosign" >&2
        echo "    Linux:             see the sigstore install docs above for your distro" >&2
        echo "" >&2
        echo "  If you explicitly accept the risk of installing an UNVERIFIED binary," >&2
        echo "  re-run with --insecure. This is NOT recommended." >&2
        echo "" >&2
        exit 1
    fi

    echo "Fetching cosign bundle ..."
    if ! curl -fsSL --max-time 10 "$BUNDLE_URL" -o "$BUNDLE_TMP"; then
        echo "" >&2
        echo "error: failed to download cosign bundle: $BUNDLE_URL" >&2
        echo "" >&2
        echo "  ENS-82 keyless signing is live; every release is expected to publish a" >&2
        echo "  .bundle alongside its binary. A missing/unreachable bundle means either" >&2
        echo "  the release was not signed or the download path is compromised — this" >&2
        echo "  installer fails closed rather than install an unverified binary." >&2
        echo "" >&2
        echo "  If you explicitly accept the risk of installing an UNVERIFIED binary," >&2
        echo "  re-run with --insecure. This is NOT recommended." >&2
        echo "" >&2
        exit 1
    fi

    echo "Verifying cosign bundle ..."
    if ! cosign verify-blob \
        --bundle "$BUNDLE_TMP" \
        --certificate-identity-regexp "https://github.com/enscrive/enscrive-cli/.*" \
        --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
        "$BINARY_TMP"; then
        echo "" >&2
        echo "error: cosign bundle verification FAILED." >&2
        echo "" >&2
        echo "  The downloaded binary does not match a valid signature from" >&2
        echo "  github.com/enscrive/enscrive-cli's release workflow. This can mean the" >&2
        echo "  binary was tampered with in transit or at rest. Refusing to install." >&2
        echo "" >&2
        echo "  Do not bypass this with --insecure unless you fully understand and" >&2
        echo "  accept the risk of installing a binary that failed verification." >&2
        echo "" >&2
        exit 1
    fi
    echo "cosign: bundle verified OK."
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
