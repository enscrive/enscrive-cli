#!/usr/bin/env bash
# run.sh — host-side launcher for the enscrive-cli playground.
#
# Usage:
#   tests/local/run.sh           # build (if missing) + start container + run smoke
#   tests/local/run.sh shell     # build (if missing) + start container + drop to bash
#   tests/local/run.sh teardown  # stop and remove the playground container
#
# Port mapping:
#   container 3000 (developer portal) -> host 13001
#
# Why only one port:
#   Inside the playground container we run a separate dockerd. All the infra
#   services started by `enscrive init` (postgres 55432, keycloak 8180,
#   qdrant 6333, loki 3100, vector 9010, grafana 3003 if --with-grafana)
#   live on the playground's internal docker network. Your host's docker
#   daemon never sees them, so they cannot collide with the services your
#   `enscrive-deploy provision --target dev` stack already exposes.
#
#   Only the developer portal needs to be reachable from your browser, and
#   we map it to host :13001 to leave your dev portal on host :13000 alone.
set -euo pipefail

IMAGE="${PLAYGROUND_IMAGE:-enscrive-playground:latest}"
NAME="${PLAYGROUND_NAME:-enscrive-playground}"
HOST_DEV_PORT="${HOST_DEV_PORT:-13002}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Auto-pick a free host port if the requested one is busy. Walks 13002 →
# 13099 looking for a hole. Prints the chosen port so the user knows.
# Override with HOST_DEV_PORT=<n> if you want a specific port.
port_in_use() {
    ss -ltn 2>/dev/null | awk '{print $4}' | grep -qE "[:.]${1}\$"
}
if [ -z "${HOST_DEV_PORT_LOCKED:-}" ] && port_in_use "$HOST_DEV_PORT"; then
    orig="$HOST_DEV_PORT"
    for cand in $(seq 13002 13099); do
        if ! port_in_use "$cand"; then
            HOST_DEV_PORT="$cand"
            echo "Note: host port ${orig} is busy; using ${HOST_DEV_PORT} instead."
            echo "      Pin a specific port with: HOST_DEV_PORT=<n> tests/local/run.sh"
            break
        fi
    done
fi

cmd="${1:-test}"

case "$cmd" in
  teardown)
    echo "Stopping + removing $NAME ..."
    docker rm -f "$NAME" 2>/dev/null || echo "(was not running)"
    exit 0
    ;;
esac

# Build image if absent. Re-running with a fresh Dockerfile? Pass --rebuild.
if [ "${1:-}" = "--rebuild" ]; then
  shift; cmd="${1:-test}"
  echo "Rebuilding $IMAGE ..."
  docker build -t "$IMAGE" "$SCRIPT_DIR"
elif ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "Building $IMAGE (first run) ..."
  docker build -t "$IMAGE" "$SCRIPT_DIR"
fi

# If a previous container is around, replace it. Re-using the same name lets
# the founder rerun `tests/local/run.sh` repeatedly without manual cleanup.
docker rm -f "$NAME" 2>/dev/null || true

case "$cmd" in
  shell)
    echo "Starting $NAME interactively (bash)..."
    exec docker run --rm -it \
      --privileged \
      --name "$NAME" \
      -p "${HOST_DEV_PORT}:3000" \
      -e "OPENAI_API_KEY=${OPENAI_API_KEY:-}" \
      -e "ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY:-}" \
      -e "VOYAGE_API_KEY=${VOYAGE_API_KEY:-}" \
      -e "NEBIUS_API_KEY=${NEBIUS_API_KEY:-}" \
      -v "${SCRIPT_DIR}/playground.sh:/usr/local/bin/playground.sh:ro" \
      "$IMAGE" bash
    ;;
  test)
    echo "Starting $NAME and running playground.sh ..."
    docker run -d \
      --privileged \
      --name "$NAME" \
      -p "${HOST_DEV_PORT}:3000" \
      -e "OPENAI_API_KEY=${OPENAI_API_KEY:-}" \
      -e "ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY:-}" \
      -e "VOYAGE_API_KEY=${VOYAGE_API_KEY:-}" \
      -e "NEBIUS_API_KEY=${NEBIUS_API_KEY:-}" \
      -v "${SCRIPT_DIR}/playground.sh:/usr/local/bin/playground.sh:ro" \
      "$IMAGE" tail -f /dev/null
    # Run the smoke script inside the container as tester. Output to host stdout.
    docker exec -u tester "$NAME" bash -lc 'bash /usr/local/bin/playground.sh'
    rc=$?
    if [ "$rc" -eq 0 ]; then
      echo
      echo "✓ Playground is up and tested."
      echo "  Browser: http://localhost:${HOST_DEV_PORT}"
      echo "  Login:   developer / developer"
      echo
      echo "Drop into a shell inside the container with:"
      echo "  docker exec -it -u tester $NAME bash"
      echo
      echo "Tear down when finished:"
      echo "  tests/local/run.sh teardown"
    else
      echo "✗ playground.sh failed (exit ${rc}). Container is left running for inspection."
      echo "  Inspect: docker exec -it -u tester $NAME bash"
      echo "  Tear down: tests/local/run.sh teardown"
      exit "$rc"
    fi
    ;;
  *)
    echo "usage: $0 [test|shell|teardown|--rebuild]" >&2
    exit 2
    ;;
esac
