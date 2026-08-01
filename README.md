# Enscrive CLI

Thin, honest command-line client for the Enscrive memory + search platform. Runs against a managed tenant at `api.enscrive.io`, or against a full Enscrive stack you host yourself on one machine.

One binary. Same commands either way.

> **Status — pre-launch beta.** Managed mode against `api.enscrive.io` is coming soon (the `/v1` plane is not yet serving production traffic). Self-hosted local mode is the supported path today. Binary releases will land at [github.com/enscrive/enscrive-cli/releases](https://github.com/enscrive/enscrive-cli/releases) once v0.1.0-beta.1 is cut.

---

## What you can do

- **Search + embed**: semantic search over your documents with hybrid ranking and adaptive resolution. Choose from OpenAI, Voyage, or Nebius (BGE) embedding models.
- **Structured ingestion**: chunk, segment, and index long-form content with SHA256 deduplication and change detection.
- **Voice authoring**: define and compare chunking strategies (Baseline / Story Beats / Tone Segments). Optimize voices with eval campaigns against your own datasets or HuggingFace corpora *(Professional plan)*.
- **Operational primitives**: corpora CRUD, staging/commit, background jobs, usage metering.
- **Data portability**: backup, restore, and full tenant export *(Professional plan)*.

---

## Install

### One-liner (beta channel)

```sh
curl -fsSL https://install.enscrive.io/install | sh
```

Installs the `enscrive` CLI binary to `~/.local/bin/enscrive`. No sudo required.

Supported platforms: `x86_64`/`aarch64` Linux (glibc and musl), `x86_64`/`arm64` macOS.

> **Dev channel note.** The install URL above serves the `dev` channel while production
> CloudFront (`enscrive.io/install`) is being provisioned. The production one-liner
> `curl -fsSL https://enscrive.io/install | sh` will replace this pre-GA (tracked in ENS-81).

**Options:**

```sh
# Override install directory (default: ~/.local/bin)
curl -fsSL https://install.enscrive.io/install | sh -s -- --prefix=/usr/local/bin

# Cross-machine install (override platform detection)
curl -fsSL https://install.enscrive.io/install | sh -s -- --target=aarch64-apple-darwin

# Skip cosign bundle verification (debug only)
curl -fsSL https://install.enscrive.io/install | sh -s -- --insecure
```

If `~/.local/bin` is not on your `PATH`, the installer will remind you how to add it.

### Build from source

```bash
# Prerequisites: Rust 1.85+ (rustup recommended)
git clone https://github.com/enscrive/enscrive-cli.git
cd enscrive-cli
cargo build --release
install -m 0755 target/release/enscrive ~/.local/bin/enscrive
enscrive --version
```

---

## Quickstart

### Managed (against `api.enscrive.io`)

```bash
enscrive init --mode managed \
  --api-key ens_live_XXXXXXXXXXXX \
  --endpoint https://api.enscrive.io \
  --set-default

enscrive health
enscrive corpus list
```

API keys will be available at [enscrive.io/pricing](https://enscrive.io/pricing) once the managed plane is live.

### Self-hosted local stack

You run the full stack on your machine: `enscrive-developer` + `enscrive-observe` + `enscrive-embed` + Postgres, Keycloak, Qdrant, and Loki via Docker Compose.

**Prerequisites.** Docker Engine + Docker Compose. Provider API key(s) for at least one embedding backend.

On Fedora:

```bash
sudo dnf install -y moby-engine docker-compose
sudo systemctl enable --now docker
sudo usermod -aG docker $USER  # log out/in or `newgrp docker` to take effect
```

On Debian/Ubuntu, follow [docs.docker.com/engine/install](https://docs.docker.com/engine/install/).

Then:

```bash
enscrive init --mode self-managed \
  --openai-api-key sk-... \
  --set-default

enscrive start
enscrive status
```

`init` walks you through missing provider configuration interactively if you don't pass flags. You can mix providers:

| Flag | Enables |
|---|---|
| `--openai-api-key` | OpenAI embeddings + LLM-reasoned chunking |
| `--voyage-api-key` | Voyage AI embeddings |
| `--nebius-api-key` | Nebius Token Factory embeddings (BGE) |
| `--anthropic-api-key` | Anthropic LLM-reasoned chunking |

Bring-your-own-key for embeddings works on every plan.

---

## Commands

Local stack and project setup:

```
enscrive init            Configure a managed or self-hosted profile
enscrive start           Start the local self-hosted stack
enscrive stop            Stop the local self-hosted stack
enscrive status          Resolved profile, stack health, active project
enscrive bootstrap       Re-run tenant/key bootstrap on a running stack
enscrive project init    Give this directory its own isolated memory
```

Working with memory:

```
enscrive search          Search a corpus
enscrive embeddings      Embedding primitives
enscrive ingest          Add documents to a corpus
enscrive segment         Chunk a document
enscrive analyze         Inspect content
enscrive models          List available models
enscrive corpus          Manage corpora
enscrive voices          Author and compare voices
enscrive evals           Run eval campaigns                    [Pro]
enscrive datasets        Manage datasets                       [Pro]
enscrive eval-defs       Define and run eval suites            [Pro]
enscrive jobs            Inspect background jobs
enscrive batch-sets      Inspect batch staging
enscrive logs            Stream and search logs
enscrive revisions       List point-in-time restore points
enscrive restore         Restore tenant data to a revision
enscrive backup          Operator backup surface (admin)       [Pro]
enscrive export          Data portability                      [Pro]
enscrive usage           Usage + metering
```

`enscrive <command> --help` shows the full flag set for any command. All commands support `--output json` for scripting.

---

## Configuration

Profiles live at `~/.config/enscrive/profiles.toml`. You can have multiple profiles (e.g., one per managed tenant, one for local dev) and switch between them with `enscrive init --profile NAME --set-default ...`.

Set once and forget:

```bash
# Global override on any invocation
enscrive --profile my-tenant corpus list

# Switch default
enscrive init --profile local --set-default --mode self-managed ...
```

Runtime state (Docker Compose volumes, container IDs, logs) for self-hosted profiles lives at `~/.local/share/enscrive/runtime/<profile>/`.

### The key file

`~/.config/enscrive/profiles.toml` is the **only** place the CLI stores API keys. Each profile holds its endpoint, its mode (`managed` or `local`), and its key in plaintext:

```toml
version = 1
default_profile = "local"

[profiles.local]
mode = "local"
endpoint = "http://127.0.0.1:3000"
api_key = "..."          # secret — never commit this file
```

Treat it like `~/.ssh/id_ed25519`:

```bash
chmod 600 ~/.config/enscrive/profiles.toml
```

Keys reach this file two ways — `enscrive init` (you paste one, or `enscrive start` bootstraps one for a local stack) and `enscrive project init` (which mints a per-project tenant key and stores it under a `project-<name>` profile). Nothing else writes keys, and no command ever prints one.

**Per-project markers hold no secrets.** `enscrive project init` writes a committable `.enscrive/config.toml` next to your code that references its key *by profile name*, never by value — so the marker is safe to commit while the key stays in the file above. See `.enscrive/AGENT.md`, dropped in the same directory, for how an agent uses it.

To rotate a key, re-run `enscrive init` for that profile (or `enscrive project init` in the project directory) — both overwrite the stored key in place.

---

## Search score calibration

`--score-threshold` filters results by similarity score. The single most common way to conclude "search is broken" is to set it by intuition:

```bash
# Looks reasonable. Returns nothing.
enscrive search --query "how are auth tokens minted" --score-threshold 0.9
```

Scores are cosine similarity over embedding vectors, and **genuinely relevant matches usually land near the middle of the range, not near 1.0.** A threshold of `0.8`–`0.9` filters out results you would have considered correct. There is no universal cutoff: the useful range shifts with the embedding model, your chunk sizes, and how the query is phrased.

Calibrate against your own corpus instead of guessing:

```bash
# 1. Search with NO threshold and read the scores you actually get back.
enscrive search --query "how are auth tokens minted" --limit 10 --output json \
  | jq '.data.results[] | {score, document_id}'

# 2. Find where genuinely relevant results stop and noise starts.
# 3. Set the threshold just below that, then re-check as the corpus grows.
enscrive search --query "how are auth tokens minted" --score-threshold 0.45
```

Leaving `--score-threshold` unset returns the top `--limit` matches ranked by score, which is the right default for most retrieval — including agent memory, where a weak-but-relevant hit still beats no context at all. Reach for a threshold when you specifically need "return nothing rather than something marginal".

---

## Plans

Enscrive runs on three plans:

- **Free self-hosted** — run the full stack on your machine, forever, for free, **for noncommercial dev/eval/exploration** (see [License](#license)). You pay your own embedding-provider costs. Chunking, embedding, and neural search are the core loop you get. Production or commercial self-hosted use requires a commercial license.
- **Professional** — adds first-class evals, voice-gated promotion across Dev/Staging/Prod, daily backups, structured data portability, and extended audit retention. Cloud-managed or self-hosted with a license.
- **Enterprise** — adds dedicated tenancy, SSO enforcement, BYOK encryption-at-rest, VPC peering, compliance attestations, and an uptime SLA.

Feature comparison and pricing: [enscrive.io/pricing](https://enscrive.io/pricing).

When you run a Pro-gated command on a free profile, the CLI tells you so clearly — no silent limitations.

---

## Capability tiers

**Free self-hosted** commands:
- `enscrive search`, `enscrive corpus list`, `enscrive ingest`

**Professional** commands add:
- `enscrive evals run`, `enscrive voices promote`, `enscrive backup`

**Enterprise** commands add:
- `enscrive admin rate-limits`

License activation and plan validation:
- `enscrive license activate` — activate a Pro or Enterprise license on self-hosted
- `enscrive license status` — check current plan and expiry
- `enscrive license deactivate` — deactivate the current license

Managed mode automatically derives the plan from your Enscrive account.

For a complete feature matrix: [enscrive.io/pricing](https://enscrive.io/pricing).

---

## Output contract

Every command supports a stable JSON output shape for automation:

```json
{
  "ok": true,
  "command": "search",
  "data": { "...": "..." },
  "exit_code": 0
}
```

On failure:

```json
{
  "ok": false,
  "command": "search",
  "error": "human-readable message",
  "failure_class": "FAIL_UNSUPPORTED",
  "exit_code": 2
}
```

Exit codes: `0` success, `1` bug, `2` unsupported, `3` config error, `4` plan required, `5` confirmation required.

With `--output json`, **stdout carries exactly one JSON document and nothing else** — on success and on failure alike. Human-readable prose, progress ticks, and warnings all go to stderr, so `enscrive ... --output json | jq` is always safe and never needs filtering. This is enforced by a test (`tests/json_output_purity.rs`), not just a convention.

---

## Contributing

Issues and PRs welcome at [github.com/enscrive/enscrive-cli](https://github.com/enscrive/enscrive-cli). See the issue tracker for open work.

---

## License

Enscrive CLI is licensed under the **[PolyForm Noncommercial License 1.0.0](LICENSE)**. Licensor: Enscrive.

**Free for noncommercial use only — exploratory, evaluation, and development.**
Download it, run it, and build against Enscrive as your neural backend to learn
and prototype, for free, forever.

**Production or commercial use requires a paid license:**

- **Managed** (`api.enscrive.io`): an Enscrive **subscription** with metered billing.
- **Self-managed** (CLI / self-hosted stack): an Enscrive **commercial license**.
- **Enterprise**: a **custom agreement**.

"Commercial use" includes serving end users, generating revenue, or running a
commercial workload — whether managed or self-managed. The CLI download/run path
stays frictionless and requires no signup; the *license*, not a technical lock,
bounds the free grant to noncommercial dev/eval/exploration.

Terms and pricing: [enscrive.io/pricing](https://enscrive.io/pricing). See
[LICENSE](LICENSE) for the full PolyForm Noncommercial 1.0.0 text.
