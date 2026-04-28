#!/usr/bin/env bash
# DinD entrypoint. Starts the inner dockerd in the background, waits for it
# to be ready, then either runs a passed command or drops to interactive shell.
set -euo pipefail

# Start dockerd. --iptables=true is required for the inner network to NAT.
# Logs go to a file so we don't drown the test output but are inspectable.
mkdir -p /var/log/dockerd
nohup dockerd \
    --host=unix:///var/run/docker.sock \
    --storage-driver=overlay2 \
    --iptables=true \
    > /var/log/dockerd/dockerd.log 2>&1 &
DOCKERD_PID=$!

# Wait for dockerd readiness (up to ~30 s).
for i in $(seq 1 30); do
    if docker info >/dev/null 2>&1; then
        echo "[entrypoint] inner dockerd ready (pid ${DOCKERD_PID})"
        break
    fi
    sleep 1
    [ "$i" = "30" ] && {
        echo "[entrypoint] inner dockerd failed to start within 30s" >&2
        tail -50 /var/log/dockerd/dockerd.log >&2
        exit 1
    }
done

# Drop to the test user for whatever follows. If the caller passed a command
# via CMD or `docker run ... <cmd>`, run it as tester.
if [ "$#" -eq 0 ] || { [ "$#" -eq 1 ] && [ "$1" = "bash" ]; }; then
    exec sudo -u tester -i bash
else
    exec sudo -u tester -i "$@"
fi
