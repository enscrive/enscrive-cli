#!/usr/bin/env bash
# Lane: Fedora self-managed install smoke test.
#
# Validates the public install path end-to-end on a fresh Fedora EC2
# instance. Mirrors what a developer evaluating Enscrive will run on
# their workstation:
#
#   1. curl install.enscrive.io/install.sh | sh        — fetch CLI
#   2. enscrive --version                              — binary works
#   3. enscrive init --mode self-managed --yes         — fetch services + start stack
#   4. curl http://127.0.0.1:3000/healthz              — developer portal reachable
#
# Run ON the EC2 instance (not on the operator workstation).
# Operator workstation drives this via SSM port-forwarding — see
# tests/ec2/lane-fedora-self-managed.md.
set -euo pipefail

INSTALL_URL="${INSTALL_URL:-https://install.enscrive.io/install.sh}"
DEV_PORT="${DEV_PORT:-3000}"

step() { echo; echo "=== $* ==="; }
fail() { echo "FAIL: $*" >&2; exit 1; }

step "Step 0: Confirm we are on a clean Fedora"
[ -f /etc/fedora-release ] || fail "this script targets Fedora; got $(uname -a)"
cat /etc/fedora-release

step "Step 1: Fetch and run install.sh"
# The script lands enscrive in ~/.local/bin and is idempotent; rerunning is
# safe. Use a fresh shell so PATH-update prints don't get swallowed.
curl -fsSL "$INSTALL_URL" | sh
export PATH="$HOME/.local/bin:$PATH"
command -v enscrive >/dev/null || fail "enscrive not on PATH after install"

step "Step 2: enscrive --version"
enscrive --version
enscrive --help | head -5

step "Step 3: enscrive init --mode self-managed"
# --yes runs non-interactively (accepts default ports, default Keycloak
# admin credentials, default install prefix). The init command:
#   - reads the manifest at install.enscrive.io/releases/dev/latest.json
#   - downloads + SHA256-verifies enscrive-developer.tar.gz (and observe/embed)
#   - extracts to ~/.local/share/enscrive/services/
#   - writes systemd --user units to ~/.config/systemd/user/
#   - starts the stack (postgres + keycloak + qdrant + loki + observe + embed + developer)
enscrive init --mode self-managed --yes

step "Step 4: Wait for developer portal"
for i in $(seq 1 60); do
  if curl -fsS -o /dev/null "http://127.0.0.1:${DEV_PORT}/healthz"; then
    echo "developer portal up after ${i}s"
    break
  fi
  sleep 2
  [ "$i" = "60" ] && fail "developer portal didn't come up on :${DEV_PORT} within 120s"
done

step "Step 5: Probe each service"
echo "Developer portal /healthz:"
curl -fsS "http://127.0.0.1:${DEV_PORT}/healthz" || fail "developer /healthz unreachable"

echo "Observe /health:"
curl -fsS "http://127.0.0.1:19090/health" || echo "  (observe not on default port — non-fatal for smoke)"

echo "Embed /v1/health:"
curl -fsS "http://127.0.0.1:18080/v1/health" || echo "  (embed not on default port — non-fatal for smoke)"

step "Smoke test complete — stack is up on this EC2 instance."
echo
echo "From the operator workstation, port-forward via SSM:"
echo "  aws ssm start-session --target <instance-id> \\"
echo "    --document-name AWS-StartPortForwardingSession \\"
echo "    --parameters '{\"portNumber\":[\"${DEV_PORT}\"],\"localPortNumber\":[\"13001\"]}' \\"
echo "    --region <region>"
echo
echo "Then open http://localhost:13001 in a browser on the workstation."
