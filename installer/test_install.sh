#!/bin/sh
# ENS-81: smoke test for installer/install.sh
#
# Sets up a fake file:// manifest pointing at a local fixture binary,
# runs install.sh with --manifest-url and --prefix overrides,
# then asserts the binary lands at the expected path with mode 0755
# and a matching SHA256.
#
# Returns 0 on success, non-zero on failure.
# Cleans up after itself unconditionally.

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INSTALL_SH="$SCRIPT_DIR/install.sh"

# ---------------------------------------------------------------------------
# Resolve fixture binary
# Use the enscrive release build from the parent enscrive-cli tree if it
# exists; otherwise fall back to any small executable we can find.
# ---------------------------------------------------------------------------
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
FIXTURE_BIN=""

for candidate in \
    "$REPO_ROOT/target/release/enscrive" \
    "$REPO_ROOT/target/debug/enscrive" \
    "$(command -v sh 2>/dev/null)"; do
    if [ -f "$candidate" ] && [ -x "$candidate" ]; then
        FIXTURE_BIN="$candidate"
        break
    fi
done

if [ -z "$FIXTURE_BIN" ]; then
    echo "SKIP: no fixture binary found (build enscrive first)" >&2
    exit 0
fi

echo "Using fixture binary: $FIXTURE_BIN"

# ---------------------------------------------------------------------------
# Compute SHA256 of fixture
# ---------------------------------------------------------------------------
if command -v sha256sum >/dev/null 2>&1; then
    FIXTURE_SHA="$(sha256sum "$FIXTURE_BIN" | cut -d' ' -f1)"
elif command -v shasum >/dev/null 2>&1; then
    FIXTURE_SHA="$(shasum -a 256 "$FIXTURE_BIN" | cut -d' ' -f1)"
else
    echo "SKIP: no sha256sum/shasum found" >&2
    exit 0
fi

echo "Fixture SHA256: $FIXTURE_SHA"

# ---------------------------------------------------------------------------
# Build a minimal ENS-91 manifest pointing at the fixture via file://
# ---------------------------------------------------------------------------
SMOKE_DIR="$(mktemp -d)"
FIXTURE_SIZE="$(wc -c < "$FIXTURE_BIN" | tr -d ' ')"

# Detect the target triple that install.sh will auto-detect on this machine
# by calling the same logic inline.
_os="$(uname -s)"
_arch="$(uname -m)"
case "$_os" in
    Linux)
        case "$_arch" in
            x86_64|amd64)
                if ls /lib/ld-musl-* >/dev/null 2>&1; then
                    LOCAL_TARGET="x86_64-unknown-linux-musl"
                else
                    LOCAL_TARGET="x86_64-unknown-linux-gnu"
                fi
                ;;
            aarch64|arm64) LOCAL_TARGET="aarch64-unknown-linux-gnu" ;;
            *) LOCAL_TARGET="x86_64-unknown-linux-gnu" ;;
        esac
        ;;
    Darwin)
        case "$_arch" in
            arm64|aarch64) LOCAL_TARGET="aarch64-apple-darwin" ;;
            *)             LOCAL_TARGET="x86_64-apple-darwin" ;;
        esac
        ;;
    *) LOCAL_TARGET="x86_64-unknown-linux-gnu" ;;
esac

echo "Local target triple: $LOCAL_TARGET"

FIXTURE_FILE_URL="file://$FIXTURE_BIN"

cat > "$SMOKE_DIR/latest.json" <<JSON
{
  "schema_version": 1,
  "version": "0.0.0-smoke",
  "released_at": "2026-04-24T00:00:00Z",
  "channel": "dev",
  "binaries": {
    "enscrive": {
      "source_version": "0.0.0-smoke",
      "platforms": {
        "$LOCAL_TARGET": {
          "url": "$FIXTURE_FILE_URL",
          "sha256": "$FIXTURE_SHA",
          "size_bytes": $FIXTURE_SIZE
        }
      }
    }
  },
  "compatibility": {
    "min_cli_version": "0.0.0-smoke"
  },
  "signature": null
}
JSON

MANIFEST_FILE_URL="file://$SMOKE_DIR/latest.json"
INSTALL_PREFIX="$SMOKE_DIR/install-$$"

trap 'rm -rf "$SMOKE_DIR"' EXIT HUP INT TERM

echo ""
echo "Running install.sh ..."
echo "  --manifest-url=$MANIFEST_FILE_URL"
echo "  --prefix=$INSTALL_PREFIX"
echo "  --insecure"
echo ""

sh "$INSTALL_SH" \
    --manifest-url="$MANIFEST_FILE_URL" \
    --prefix="$INSTALL_PREFIX" \
    --insecure

# ---------------------------------------------------------------------------
# Assertions
# ---------------------------------------------------------------------------
INSTALLED_BIN="$INSTALL_PREFIX/enscrive"

if [ ! -f "$INSTALLED_BIN" ]; then
    echo "FAIL: binary not found at $INSTALLED_BIN" >&2
    exit 1
fi

# Mode check (octal 755)
FILE_MODE="$(stat -c '%a' "$INSTALLED_BIN" 2>/dev/null || stat -f '%OLp' "$INSTALLED_BIN" 2>/dev/null)"
if [ "$FILE_MODE" != "755" ]; then
    echo "FAIL: expected mode 755, got $FILE_MODE" >&2
    exit 1
fi

# SHA256 check
if command -v sha256sum >/dev/null 2>&1; then
    INSTALLED_SHA="$(sha256sum "$INSTALLED_BIN" | cut -d' ' -f1)"
elif command -v shasum >/dev/null 2>&1; then
    INSTALLED_SHA="$(shasum -a 256 "$INSTALLED_BIN" | cut -d' ' -f1)"
fi

if [ "$INSTALLED_SHA" != "$FIXTURE_SHA" ]; then
    echo "FAIL: SHA256 mismatch on installed binary" >&2
    echo "  expected: $FIXTURE_SHA" >&2
    echo "  actual:   $INSTALLED_SHA" >&2
    exit 1
fi

echo ""
echo "PASS: binary installed at $INSTALLED_BIN with mode 755 and correct SHA256."
exit 0
