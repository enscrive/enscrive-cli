# Enscrive operators guide

This document is for **Enscrive-internal deployment operators**. If you're a developer using Enscrive to power your application, see [README.md](./README.md) — the `enscrive deploy` command family documented here is for operators provisioning the Enscrive platform itself onto managed hosts.

The commands themselves are part of the public `enscrive` binary; documenting them here keeps the reference physically next to the code. Running `enscrive deploy` against infrastructure you don't operate won't achieve anything useful.

---

## Deployment lifecycle

Each managed host in each region runs `enscrive-developer`, `enscrive-observe`, and `enscrive-embed` behind an nginx reverse-proxy, fronted by a regional ALB. The `enscrive deploy` command family walks an operator through:

1. **`init`** — declare a profile for a target (`dev` / `stage` / `us` / `eu` / `ap`).
2. **`fetch`** — stage the binaries + site bundle for that profile.
3. **`render`** — generate deterministic systemd units, nginx config, and env files.
4. **`apply`** — install the rendered bundle onto the target host.
5. **`verify`** — probe the managed endpoint's `/health`.
6. **`bootstrap`** — consume a signed bootstrap bundle (first bring-up only).
7. **`status`** — inspect the current profile and ESM detection state (anytime).

Profiles live at `~/.config/enscrive/deploy-profiles.toml`. Artifact + render output directories default to `./enscrive-artifacts/<profile>/` and `./enscrive-deploy/<profile>/`.

---

## Targets and their endpoints

`deploy init` maps targets to canonical managed endpoints:

| Target | Endpoint |
|---|---|
| `dev` | `https://dev.api.enscrive.io` |
| `stage` | `https://stage.api.enscrive.io` |
| `us` | `https://us.api.enscrive.io` |
| `eu` | `https://eu.api.enscrive.io` |
| `ap` | `https://ap.api.enscrive.io` |

Override with `--endpoint <url>` only for first-boot or tunneled access before the canonical endpoint serves traffic (e.g., SSH tunnel to an EC2 private IP during initial bring-up).

---

## Subcommand reference

### `enscrive deploy init`

Declare or update a deploy profile.

```bash
enscrive deploy init \
  --target stage \
  --secrets-source esm \
  --profile-name stage \
  --set-default
```

Key flags:

- `--target {dev|stage|us|eu|ap}` — required first time. Sets the canonical endpoint.
- `--profile-name` — optional name override; defaults to the target name.
- `--secrets-source {prompt|env|esm}` — where subsequent `render`/`apply` should source secrets. `esm` is the normal managed path; `prompt` asks interactively; `env` reads env vars.
- `--endpoint <url>` — non-canonical endpoint override (bring-up only).
- `--set-default` — make this the default deploy profile for subsequent commands.

ESM-backed operator profiles declare three service-scoped vault roots (`enscrive-developer`, `enscrive-observe`, `enscrive-embed`) which the CLI discovers automatically when run from the operator machine or the target host.

### `enscrive deploy fetch`

Stage the binaries + developer portal site bundle for a profile.

```bash
# Pull from the hosted release manifest (the RB-008 default for stage/us/eu/ap)
enscrive deploy fetch --profile-name stage

# Force a local build (dev target's default; escape hatch for stage/us/eu/ap)
enscrive deploy fetch --profile-name stage --source local-build
```

Key flags:

- `--source {manifest|local-build}` — where to get artifacts. Defaults per the matrix below.
- `--manifest-url <url>` — pin to an exact release manifest URL.
- `--out-dir <dir>` — override the staging directory.

Stages artifacts into:

- `./enscrive-artifacts/<profile>/bin/` — `enscrive`, `enscrive-developer`, `enscrive-observe`, `enscrive-embed`.
- `./enscrive-artifacts/<profile>/site/enscrive-developer/` — developer portal bundle.

#### Default source matrix (STAGE=PROD semantics per founder 2026-04-23)

| Target | Default `--source` | `--source local-build` allowed? |
|---|---|---|
| `dev` | `local-build` | always |
| `stage` | `manifest` | yes — `stage` may test unreleased changes when needed |
| `us` | `manifest` | requires `--allow-unreleased` |
| `eu` | `manifest` | requires `--allow-unreleased` |
| `ap` | `manifest` | requires `--allow-unreleased` |

`dev` compiles on the operator's machine by design — it exists to test unreleased changes. `stage` defaults to manifest because stage is a pre-production branch, not a scratchpad: what stage validates is what production runs. Production targets require `--allow-unreleased` as supply-chain acknowledgement that you're pushing an un-released, un-signed binary.

**Note**: the RB-008 refactor that flips these defaults is in progress (tracked as ENS-94 in the Release Binaries project). Until that lands, `fetch` defaults to `local-build` across all targets.

### `enscrive deploy render`

Generate the deterministic managed-host bundle.

```bash
enscrive deploy render \
  --profile-name stage \
  --out-dir ./enscrive-deploy/stage \
  --host-root /opt/enscrive/stage
```

Produces in the out-dir:

- Service env files for `enscrive-developer`, `enscrive-observe`, `enscrive-embed`.
- Systemd unit files for the three services.
- An nginx reverse-proxy config targeting the private developer port.
- A machine-readable `manifest.json` describing the bundle.
- A rendered `README.md` with operator next-steps.
- Service-scoped installed secret roots under `/opt/enscrive/<target>/secrets/...`.
- Runtime `ESM_BINARY` / `ESM_VAULT_PATH` wiring for services that read from ESM at runtime.

Render is service-scoped: `developer.env` from the developer vault, `observe.env` from the observe vault, `embed.env` from the embed vault. Secrets stay partitioned.

Key flags:

- `--host-root <path>` — where artifacts land on the target host. Default `/opt/enscrive/<profile>/`.
- `--eba-trusted-public-key <key>` — bootstrap public key baked into `developer.env`. Auto-populated from render-time context when present.

### `enscrive deploy apply`

Install the rendered bundle onto the host. **Run on the target host**, not the operator machine.

```bash
enscrive deploy apply \
  --profile-name stage \
  --render-dir ./enscrive-deploy/stage \
  --binary-dir ./enscrive-artifacts/stage/bin \
  --site-root ./enscrive-artifacts/stage/site/enscrive-developer \
  --reload-systemd \
  --start-services \
  --reload-nginx
```

By default `apply` only installs files. Opt in to reconciliation flags explicitly:

- `--reload-systemd` — `systemctl daemon-reload` after writing units.
- `--start-services` — `systemctl enable --now` the three services.
- `--reload-nginx` — reload nginx configuration.

Stages onto the managed host:

- `enscrive-developer`, `enscrive-observe`, `enscrive-embed` into `bin/`.
- `esm` into `bin/`.
- Developer portal site bundle into `site/`.
- Discovered vault roots (developer, observe, embed) into `secrets/`.
- Rendered env files into `config/`.
- Systemd units + nginx config into operator-selected destinations.

Config hydration is service-scoped — each service reads its own env file, no cross-contamination.

### `enscrive deploy verify`

Probe the managed endpoint's `/health` (see `CLI-REL-015` follow-up — this may move to `/v1/health` for stricter readiness).

```bash
enscrive deploy verify --profile-name stage
```

Fails explicitly if the managed stack is unhealthy or degraded.

### `enscrive deploy bootstrap`

Consume a signed bootstrap bundle and persist the returned `platform_admin` and `operator` keys for steady-state operator use.

```bash
# ESM-backed operator profile (normal managed path)
enscrive deploy bootstrap \
  --profile-name stage \
  --bundle-secret-key ENSCRIVE_BOOTSTRAP_BUNDLE

# Fallback: plain file
enscrive deploy bootstrap \
  --profile-name stage \
  --bundle-path ./bootstrap.bundle.toml
```

For a fresh bring-up before the canonical endpoint is serving, use `--endpoint <tunnel-url>` to point at a private IP or SSH tunnel. The override is not persisted — steady-state operator use stays on the canonical endpoint once bootstrap succeeds.

For ESM-backed profiles, bootstrap tries `esm get --raw <key>` first, then falls back to `<vault-workdir>/bootstrap.bundle.toml` if present.

### `enscrive deploy status`

Inspect the current deploy profile, including ESM detection state:

```bash
enscrive deploy status
enscrive deploy status --profile-name stage --output json
```

No side effects — use anytime.

---

## Secrets: source modes

`deploy init --secrets-source` accepts three values:

| Mode | Behavior |
|---|---|
| `esm` | Read from the ESM vault (the normal managed-host path). CLI discovers developer/observe/embed vault roots automatically. |
| `env` | Read from environment variables on the operator or target machine. |
| `prompt` | Ask interactively at `render` time. Useful for dev/scratch. |

Production (us/eu/ap) should always use `esm`.

---

## First bring-up (bootstrap) flow

For a fresh regional deployment — first time `us.api.enscrive.io` is brought up, for example:

```bash
# 1. On the operator machine: declare the profile
enscrive deploy init --target us --secrets-source esm --profile-name us --set-default

# 2. Stage binaries (once RB-008 lands, this defaults to manifest)
enscrive deploy fetch --profile-name us

# 3. Render the host bundle
enscrive deploy render --profile-name us --out-dir ./enscrive-deploy/us --host-root /opt/enscrive/us

# 4. (On the target host, SSH'd in) Apply
enscrive deploy apply --profile-name us \
  --render-dir ./enscrive-deploy/us \
  --binary-dir ./enscrive-artifacts/us/bin \
  --site-root ./enscrive-artifacts/us/site/enscrive-developer \
  --reload-systemd --start-services --reload-nginx

# 5. Consume the signed bootstrap bundle (private IP override for first contact)
enscrive deploy bootstrap --profile-name us \
  --endpoint http://10.0.1.42:3000 \
  --bundle-secret-key ENSCRIVE_BOOTSTRAP_BUNDLE

# 6. Verify
enscrive deploy verify --profile-name us
```

Once DNS + TLS for `us.api.enscrive.io` are serving, drop the `--endpoint` override; `verify` and subsequent commands use the canonical endpoint from the profile.

---

## Steady-state operations

After first bring-up:

- **Patch deploy** (binary refresh, same rendered config): `fetch` → `apply` with `--start-services` to restart.
- **Config change**: `render` → `apply` with the appropriate reload flags.
- **Full redeploy**: `fetch` → `render` → `apply` with all reload flags.
- **Health check**: `verify` whenever.

Production SLA: use `fetch --source manifest` (the default post-RB-008). Never `--source local-build` on us/eu/ap without `--allow-unreleased` + a clear reason — the acknowledgement gate exists because pushing an unsigned binary to prod bypasses supply-chain provenance.

---

## Links

- [README.md](./README.md) — public user-facing CLI docs.
- Release Binaries project in Linear — end-to-end pipeline that produces the manifests this command family consumes.
- `enscrive deploy <subcommand> --help` — authoritative current-flag reference.
