# Local-developer playground

Reusable Docker-in-Docker (DinD) sandbox for testing the **public install flow**
end-to-end on your own laptop — without any port conflicts against your
`enscrive-deploy provision --target dev` stack.

```
tests/local/run.sh           # build (first time) + smoke test the install path
tests/local/run.sh shell     # drop into the playground for manual poking
tests/local/run.sh teardown  # stop + remove the playground container
```

After a successful run:

- Browser → `http://localhost:13001` → playground's developer portal
- Login: `developer` / `developer`

## Why DinD instead of `-v /var/run/docker.sock`

`enscrive init --mode self-managed` brings up a docker-compose stack
(postgres, keycloak, qdrant, loki, vector, grafana) **plus** the three
service binaries (developer, observe, embed). The compose services use
default ports that **collide** with your `enscrive-deploy --target dev` stack:

| Service | self-managed init default | dev-provision host | Conflict if same dockerd? |
|---|---|---|---|
| Keycloak | 8180 | 8180 | **yes** |
| Qdrant | 6333 | 6333 | **yes** |
| Loki | 3100 | 3100 | **yes** |
| Grafana | 3003 | 3003 | **yes** |

A socket-mount sibling-container approach (`-v /var/run/docker.sock:...`)
would publish those services on **your host's** docker daemon, where your
dev stack lives — instant collision. DinD gives the playground its own
internal dockerd inside its own network namespace, so all those services
use their defaults without ever touching your host's port space.

The trade-off: DinD requires `--privileged`. That's fine for a local dev
sandbox but should never be the pattern for any production-adjacent
deployment.

## Port mapping (only one is needed)

```
Playground container (own dockerd inside)
  ├─ 127.0.0.1:3000   developer portal     ───┐
  ├─ 127.0.0.1:8180   keycloak                │  All inside the
  ├─ 127.0.0.1:6333   qdrant                  ├─  container's
  ├─ 127.0.0.1:55432  postgres                │  network namespace.
  ├─ 127.0.0.1:3100   loki                    │  Invisible to your host.
  └─ 127.0.0.1:8084   observe rest         ───┘
                                                ▼
                                       host :13001 (single -p map)
                                                ▼
                                       browser → http://localhost:13001
```

If you want to inspect Keycloak admin or Grafana from your host, pop into
the container:

```
docker exec -it -u tester enscrive-playground bash
# inside, e.g.:
curl -s http://127.0.0.1:8180/realms/master | jq
```

Or open another `-p` map on the host with:

```
HOST_DEV_PORT=13001 PLAYGROUND_NAME=enscrive-playground tests/local/run.sh teardown
docker run --rm -it --privileged \
  --name enscrive-playground \
  -p 13001:3000 \
  -p 18180:8180 \
  enscrive-playground tail -f /dev/null
# then docker exec -u tester ... playground.sh
```

But for normal browser-based smoke that's overkill — just use the default `:13001`.

## What the smoke test does

`run.sh test` (the default) runs `playground.sh` inside the container as
user `tester`:

1. `curl -fsSL https://install.enscrive.io/install.sh | sh` — fetches the
   public CLI binary, SHA256-verifies against the manifest.
2. `enscrive init --mode self-managed --yes` — generates the local profile,
   downloads the three service binaries, writes docker-compose.yml.
3. `enscrive start` — brings up the docker-compose stack (postgres +
   keycloak + qdrant + loki + vector), spawns the three service binaries,
   creates the seeded `developer/developer` Keycloak end-user.
4. Probes `http://127.0.0.1:3000/healthz` until 200 (max 2 minutes).

Exit 0 means the full public install path works. Exit non-zero leaves the
container running for `docker exec`-based investigation.

## Iteration loop

Re-running `run.sh test` is cheap because:

- The image is cached; only changes to `Dockerfile` trigger a rebuild
  (use `run.sh --rebuild` if you change the Dockerfile).
- The container is recreated each run (`docker rm -f` then fresh start),
  so each test has a clean filesystem.
- The DinD inner image cache (`/var/lib/docker` inside the container)
  is anonymous — torn down with the container. If you want to reuse it
  across runs, swap the anonymous volume for a named one in `run.sh`.

Typical loop while iterating on `install.sh` or `enscrive init`:

```
# 1. push CLI changes (cut a fresh tag if needed)
# 2. wait for manifest workflow to publish
scripts/release_status.sh v0.1.0-beta.X dev

# 3. re-run the playground smoke
tests/local/run.sh test

# 4. browse the result
xdg-open http://localhost:13001
```

## When to use this vs the EC2 lane

| Purpose | Use this | Use `tests/ec2/lane-fedora-self-managed.sh` |
|---|---|---|
| Iterating on `install.sh` shell logic | ✅ fastest | overkill |
| Verifying `enscrive init` against published artifacts | ✅ | ✅ |
| Catching DNS/IPv6/MTU issues only a real public IP would expose | ❌ | ✅ |
| Pre-launch "does this actually work for a stranger" check | ❌ | ✅ |
| Daily dev iteration | ✅ free, fast | $/hour, slower |

In practice: this playground for the next 50 iterations, then one EC2
cycle for a final pre-announce sanity check.

## Limitations

- **Apple Silicon hosts**: this image is `linux/amd64` only. On M1/M2 Macs,
  Docker Desktop will run it under emulation and inner dockerd's
  performance is degraded. Acceptable for smoke; not great for soak.
- **`--privileged` flag**: DinD requires it. Acceptable for a local
  developer sandbox; should never propagate into anything that runs in
  shared infrastructure.
- **First build is slow** (~3–5 min: dnf install of docker-ce). Subsequent
  runs reuse the cached image and start in ~5 sec.
- **Inner dockerd image cache** is per-container by default (anonymous
  volume). The first `enscrive init` inside the container pulls
  postgres/keycloak/qdrant/loki/vector/grafana images, ~2 GB total. To
  persist that across teardowns, change the `VOLUME /var/lib/docker` line
  in `Dockerfile` to a named volume.
