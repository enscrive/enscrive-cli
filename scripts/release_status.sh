#!/usr/bin/env bash
# release_status.sh — consolidated cross-repo release status.
#
# Reports a single dashboard for an Enscrive coordinated release tag:
#   - Per-repo release.yml workflow run (status, duration, conclusion)
#   - S3 artifact presence per service × per platform
#   - Manifest workflow run on enscrive-cli
#   - latest.json published + reachable via install.enscrive.io
#   - Cosign bundle presence (best-effort — ENS-82 still landing)
#
# Usage:
#   scripts/release_status.sh v0.1.0-beta.2 [dev]
#
# Channel defaults to `dev`. Exit codes:
#   0 — fully green (all builds done, all artifacts present, manifest reachable)
#   1 — at least one component still in_progress
#   2 — at least one component failed or is unreachable
set -euo pipefail

VERSION="${1:?usage: release_status.sh <version> [channel]}"
CHANNEL="${2:-dev}"
S3_BUCKET="${ENSCRIVE_RELEASE_BUCKET:-enscrive-install-artifacts-dev}"
AWS_REGION="${ENSCRIVE_RELEASE_REGION:-us-east-2}"
PUBLIC_HOST="${ENSCRIVE_RELEASE_HOST:-install.enscrive.io}"

# Service binaries and their archive shape.
declare -A KIND
KIND["enscrive-cli"]="binary"
KIND["enscrive-developer"]="archive"
KIND["enscrive-observe"]="binary"
KIND["enscrive-embed"]="binary"

declare -A BIN_NAME
BIN_NAME["enscrive-cli"]="enscrive"
BIN_NAME["enscrive-developer"]="enscrive-developer"
BIN_NAME["enscrive-observe"]="enscrive-observe"
BIN_NAME["enscrive-embed"]="enscrive-embed"

declare -A PLATFORMS
PLATFORMS["enscrive-cli"]="x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-apple-darwin"
PLATFORMS["enscrive-developer"]="x86_64-unknown-linux-gnu"
PLATFORMS["enscrive-observe"]="x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-apple-darwin"
PLATFORMS["enscrive-embed"]="x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-apple-darwin"

REPOS=(enscrive-cli enscrive-developer enscrive-observe enscrive-embed)

# ANSI-C quoting ($'...') so the escape byte is interpreted at parse
# time. Plain '\033...' would land as a literal backslash-zero-three-three
# in the variable, which printf can't render.
color_reset=$'\033[0m'
color_red=$'\033[31m'
color_green=$'\033[32m'
color_yellow=$'\033[33m'
color_dim=$'\033[2m'

ok()    { printf "  ${color_green}✓${color_reset} %s\n" "$*"; }
warn()  { printf "  ${color_yellow}…${color_reset} %s\n" "$*"; }
fail()  { printf "  ${color_red}✗${color_reset} %s\n" "$*"; }
hr()    { printf "${color_dim}%s${color_reset}\n" "─────────────────────────────────────────────────────"; }

OVERALL_FAIL=0
OVERALL_PENDING=0

# ---------------------------------------------------------------------------
# Per-repo release.yml workflow status
# ---------------------------------------------------------------------------
echo
echo "Release status report — ${VERSION} (channel ${CHANNEL})"
echo "Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
hr
echo
echo "GitHub Actions release.yml runs"
hr
for repo in "${REPOS[@]}"; do
  run_json=$(gh run list --repo "enscrive/${repo}" --workflow release.yml \
    --branch "${VERSION}" --limit 1 --json status,conclusion,createdAt,url,databaseId 2>/dev/null \
    || echo "[]")
  run_count=$(echo "$run_json" | jq 'length')
  if [[ "$run_count" -eq 0 ]]; then
    # Tag-triggered runs aren't on a branch in gh-api terms; fall back to
    # finding by displayTitle / headSha equality.
    run_json=$(gh run list --repo "enscrive/${repo}" --workflow release.yml \
      --limit 5 --json status,conclusion,createdAt,url,databaseId,headBranch 2>/dev/null \
      | jq --arg v "$VERSION" '[.[] | select(.headBranch == $v)] | .[0:1]')
    run_count=$(echo "$run_json" | jq 'length')
  fi
  if [[ "$run_count" -eq 0 ]]; then
    fail "${repo}: no run found for ${VERSION}"
    OVERALL_FAIL=1
    continue
  fi
  status=$(echo "$run_json" | jq -r '.[0].status')
  conclusion=$(echo "$run_json" | jq -r '.[0].conclusion // "—"')
  url=$(echo "$run_json" | jq -r '.[0].url')
  case "$status" in
    completed)
      if [[ "$conclusion" == "success" ]]; then
        ok "${repo}: success — ${url}"
      else
        fail "${repo}: ${conclusion} — ${url}"
        OVERALL_FAIL=2
      fi ;;
    in_progress|queued|requested|waiting|pending)
      warn "${repo}: ${status} — ${url}"
      OVERALL_PENDING=1 ;;
    *)
      fail "${repo}: unknown status '${status}' — ${url}"
      OVERALL_FAIL=2 ;;
  esac
done

# ---------------------------------------------------------------------------
# S3 artifact presence per service × per platform
# ---------------------------------------------------------------------------
echo
echo "S3 artifact presence (s3://${S3_BUCKET}/releases/${CHANNEL}/${VERSION}/)"
hr
TOTAL_EXPECTED=0
TOTAL_PRESENT=0
for repo in "${REPOS[@]}"; do
  bin="${BIN_NAME[$repo]}"
  kind="${KIND[$repo]}"
  suffix="$bin"
  [[ "$kind" == "archive" ]] && suffix="${bin}.tar.gz"
  for triple in ${PLATFORMS[$repo]}; do
    TOTAL_EXPECTED=$((TOTAL_EXPECTED + 1))
    key="releases/${CHANNEL}/${VERSION}/${triple}/${suffix}"
    if aws s3api head-object --bucket "$S3_BUCKET" --key "$key" --region "$AWS_REGION" \
        >/dev/null 2>&1; then
      size=$(aws s3api head-object --bucket "$S3_BUCKET" --key "$key" --region "$AWS_REGION" \
        --query ContentLength --output text 2>/dev/null)
      ok "${repo} / ${triple} (${size} bytes)"
      TOTAL_PRESENT=$((TOTAL_PRESENT + 1))
    else
      warn "${repo} / ${triple} not yet uploaded"
      OVERALL_PENDING=1
    fi
  done
done
echo
printf "  %d / %d expected artifacts present\n" "$TOTAL_PRESENT" "$TOTAL_EXPECTED"

# ---------------------------------------------------------------------------
# Manifest workflow + latest.json
# ---------------------------------------------------------------------------
echo
echo "Manifest workflow (enscrive-cli .github/workflows/manifest.yml)"
hr
manifest_run=$(gh run list --repo enscrive/enscrive-cli --workflow manifest.yml \
  --limit 5 --json status,conclusion,createdAt,url 2>/dev/null \
  | jq '.[0]')
if [[ -n "$manifest_run" && "$manifest_run" != "null" ]]; then
  m_status=$(echo "$manifest_run" | jq -r '.status')
  m_conclusion=$(echo "$manifest_run" | jq -r '.conclusion // "—"')
  m_url=$(echo "$manifest_run" | jq -r '.url')
  case "$m_status" in
    completed)
      [[ "$m_conclusion" == "success" ]] \
        && ok  "most recent run: success — ${m_url}" \
        || { fail "most recent run: ${m_conclusion} — ${m_url}"; OVERALL_FAIL=2; }
      ;;
    *)
      warn "most recent run: ${m_status} — ${m_url}"
      OVERALL_PENDING=1 ;;
  esac
else
  warn "no manifest workflow run found (re-dispatch with: gh workflow run manifest.yml --repo enscrive/enscrive-cli -f version=${VERSION})"
fi

manifest_url="https://${PUBLIC_HOST}/releases/${CHANNEL}/${VERSION}/manifest.json"
latest_url="https://${PUBLIC_HOST}/releases/${CHANNEL}/latest.json"
echo
echo "Public manifest URLs"
hr
for url in "$manifest_url" "$latest_url"; do
  # Drop -f so curl returns 0 on 4xx/5xx and we read the http_code from
  # -w cleanly. With -f, curl exits non-zero and the `||` fallback
  # concatenates "000" onto the actual code (e.g. "403000").
  http=$(curl -sSL -o /tmp/manifest-probe.json -w "%{http_code}" "$url" 2>/dev/null || echo "000")
  if [[ "$http" == "200" ]]; then
    schema=$(jq -r '.schema_version // "?"' /tmp/manifest-probe.json 2>/dev/null)
    mver=$(jq -r '.version // "?"' /tmp/manifest-probe.json 2>/dev/null)
    ok  "${url} → 200 (schema=${schema}, version=${mver})"
  elif [[ "$http" == "404" || "$http" == "403" ]]; then
    # 403 from CloudFront/OAC means the S3 key is missing — same
    # operational meaning as 404. Both are PENDING during a release
    # in flight, not failures.
    warn "${url} → HTTP ${http} (not yet published)"
    OVERALL_PENDING=1
  else
    fail "${url} → HTTP ${http}"
    OVERALL_FAIL=2
  fi
done

# ---------------------------------------------------------------------------
# Cosign bundles (best-effort)
# ---------------------------------------------------------------------------
echo
echo "Cosign bundles (best-effort; ENS-82 may not have shipped yet)"
hr
for repo in "${REPOS[@]}"; do
  bin="${BIN_NAME[$repo]}"
  kind="${KIND[$repo]}"
  for triple in ${PLATFORMS[$repo]}; do
    suffix="$bin"
    [[ "$kind" == "archive" ]] && suffix="${bin}.tar.gz"
    bundle_key="releases/${CHANNEL}/${VERSION}/${triple}/${suffix}.bundle"
    if aws s3api head-object --bucket "$S3_BUCKET" --key "$bundle_key" --region "$AWS_REGION" \
        >/dev/null 2>&1; then
      ok "${repo} / ${triple} bundle present"
    fi
  done
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo
hr
if [[ "$OVERALL_FAIL" -eq 2 ]]; then
  printf "${color_red}OVERALL: FAIL${color_reset} — at least one component is broken or unreachable\n"
  exit 2
elif [[ "$OVERALL_PENDING" -eq 1 ]]; then
  printf "${color_yellow}OVERALL: PENDING${color_reset} — at least one component is still in flight\n"
  exit 1
else
  printf "${color_green}OVERALL: GREEN${color_reset} — release ${VERSION} is fully published\n"
  exit 0
fi
