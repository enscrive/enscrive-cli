#!/usr/bin/env bash
# playground.sh — full smoke of the public install + self-managed init flow.
# Runs INSIDE the playground container as user `tester`. Mirrors
# tests/ec2/lane-fedora-self-managed.sh but doesn't assume EC2 metadata.
#
# Steps:
#   1. curl install.enscrive.io/install.sh | sh   — fetch enscrive CLI
#   2. enscrive init --mode self-managed --yes    — fetch services + write profile
#   3. enscrive start                             — bring up infra (compose) + bins
#   4. probe http://127.0.0.1:3000/health        — confirm dev portal up
#
# Exit codes: 0 = green; non-zero = failure with the failing step in the log.
set -euo pipefail

INSTALL_URL="${INSTALL_URL:-https://install.enscrive.io/install.sh}"
DEV_PORT="${DEV_PORT:-3000}"

step() { echo; echo "=== $* ==="; }
fail() { echo "FAIL: $*" >&2; exit 1; }

step "Step 0: Sanity"
[ -f /etc/fedora-release ] || fail "this script targets Fedora; got $(uname -a)"
cat /etc/fedora-release
docker info >/dev/null || fail "inner dockerd unreachable from tester user"
echo "  dockerd reachable as $(whoami)"

step "Step 1: Fetch and run install.sh"
curl -fsSL "$INSTALL_URL" | sh
export PATH="$HOME/.local/bin:$PATH"
command -v enscrive >/dev/null || fail "enscrive not on PATH after install"
enscrive --version

step "Step 2: enscrive init --mode self-managed"
# init is non-interactive when --mode is supplied; there is no --yes flag.
# It requires at least one embedding-provider key so the local embed
# service has something to call. Pass the operator's real OPENAI_API_KEY
# through the container if set, otherwise use a placeholder so the
# install/init/start plumbing still exercises end-to-end (the embed
# service will boot but report degraded — fine for install-path smoke).
OPENAI_KEY="${OPENAI_API_KEY:-sk-placeholder-for-playground-smoke}"
enscrive init --mode self-managed --openai-api-key "$OPENAI_KEY"

step "Step 3: enscrive start"
enscrive start

step "Step 4: Wait for developer portal"
for i in $(seq 1 60); do
  if curl -fsS -o /dev/null "http://127.0.0.1:${DEV_PORT}/health"; then
    echo "  developer portal up after ${i}s"
    break
  fi
  sleep 2
  [ "$i" = "60" ] && fail "developer portal didn't come up on :${DEV_PORT} within 120s"
done

step "Smoke complete"
echo
echo "Stack is running inside this playground container."
echo "  developer portal: http://127.0.0.1:${DEV_PORT} (mapped to host :13001)"
echo "  log in: developer / developer"
echo
echo "Other services (Keycloak, Qdrant, Loki, etc.) are reachable inside the"
echo "container at their default ports. Inspect with:"
echo "  docker exec -it enscrive-playground curl -s http://127.0.0.1:8180/realms/master"
echo "  docker exec -it enscrive-playground enscrive status"
