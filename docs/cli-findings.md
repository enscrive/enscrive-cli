# `enscrive-cli` — findings register

[← back to STATE-OF-CLI](./STATE-OF-CLI.md)

Every finding from the line-by-line audit, with severity and `file:line`
evidence. **This is a documentation deliverable — nothing here was fixed in
this PR.** Severity reflects risk to the V1 launch and to the contract's
trustworthiness, not urgency of a code change.

Severity key: 🔴 high · 🟠 medium · 🟡 low · ⚪ informational / by-design.

---

## A. Contract truth & drift

### F1 — 🟠 `voices update` is `deferred` in the contract but fully shipped
`v1-surface-contract.toml:351-357` marks `PUT /v1/voices/{id}` as
`status = "deferred"` (reason: "Voice update requires config schema
alignment; tracked for next voice-builder sprint"). The code implements it
end-to-end: `VoicesSubcommand::Update` (`main.rs:869`) →
`POST /v1/voices/{id}/diff-proposal` impact preview, a `--confirm-re-embed`
safety interlock for corpus-invalidating changes, then `PUT /v1/voices/{id}`
(`main.rs:3920-3989`). The test suite is structurally blind to this:
`deferred` is a valid status, and `COMMAND_TIERS` (built from *all* contract
rows regardless of status) includes `voices update`, so the leaf-coverage
test passes.
**Impact:** the single most material contract↔code lie. Anyone reading the
contract to learn the surface would believe voice update is unavailable.
**Direction:** flip the row to `implemented`.

### F2 — 🟠 Contract command names diverge from real command paths
The contract's `cli_command` strings no longer match what you type:

| Contract `cli_command` | Real command path | Evidence |
|---|---|---|
| `voices diff` | `voices diff2 diff` | `main.rs:894` (`Diff2`), `evals2.rs:267,812` |
| `voices diff-cost` | `voices diff2 diff-cost` | `evals2.rs:823` |
| `voices diff-proposal` | `voices diff2 diff-proposal` | `evals2.rs:859` |
| `eval-runs list` | `eval-defs runs list` | `evals2.rs:164,236,772` |
| `eval-runs get` | `eval-defs runs get` | `evals2.rs:779` |
| `eval-runs diagnose` | `eval-defs runs diagnose` | `evals2.rs:786` |

For `eval-runs` the divergence is **three-way**: you invoke `eval-defs runs
list`, the contract says `eval-runs list`, and the JSON envelope's `command`
field also emits `eval-runs list` (`evals2.rs:775`). These six are exactly
the live entries in `TIER_SKIP_LIST` that are *genuinely* needed (because
the names don't match `COMMAND_TIERS`), which is what flushed the drift out.
**Direction:** reconcile the contract's `cli_command` (and/or the envelope
labels) to the real invocation path.

### F3 — 🟡 `TIER_SKIP_LIST` is mostly stale / dead
`TIER_SKIP_LIST` (`main.rs:6958-7017`) is documented as "leaf commands that
legitimately have no contract row." Of its **46** entries, roughly **32 are
dead**:
- **~25 now have contract rows** — `corpus get/revert/commits/metrics/
  materialize-from-dataset`, `jobs retry/abandon`, `batch-sets list/get`,
  `admin rate-limits show/set`, `models show`, `datasets
  list/get/describe/delete/upload`, and the entire `eval-defs *` block. The
  leaf test checks `COMMAND_TIERS` **first** (`main.rs:7056`), so these skip
  entries are never reached.
- **7 name a `deploy *` subtree that no longer exists** — there is no
  `Deploy` variant in the `Commands` enum (`main.rs:72-203`) and no
  `src/deploy.rs`, so `deploy init/status/render/fetch/verify/apply/
  bootstrap` (`main.rs:6970-6976`) reference commands the binary cannot
  produce. (A dangling comment in `fetch_verify.rs:5-6` still points at the
  deleted `src/deploy.rs`.)

The genuinely-needed entries are the 8 local/operator commands
(`init/start/stop/status/health`, `license activate/status/deactivate`) and
the 6 name-drift entries from F2 (`eval-defs runs *`, `voices diff2 *`).
Internal inconsistency: `datasets create` is contracted and **not**
skip-listed, while its siblings `datasets list/get/…` are **both** contracted
and skip-listed. The "contract row pending" comments throughout are false.
**Direction:** delete the ~32 stale entries; keep the 8 local + 6 name-drift.

### F4 — 🟠 The parity test under-verifies (structural)
The contract's own header claims a CI check verifies that "every endpoint …
has status implemented/deferred" and parity is kept. In practice:
- `tests/surface_contract.rs` checks the **TOML in isolation** (no `missing`,
  reasons present, valid enums, no dup `method+path`) — it never loads the
  clap tree (`surface_contract.rs:6-256`).
- `command_tiers_covers_every_leaf_subcommand` (`main.rs:7042`) enforces only
  **CLI→contract** leaf coverage, tolerating the skip list.
Neither checks (a) that a contract `cli_command` names a command that exists
and is named correctly, nor (b) request/response **shape** fidelity, nor
(c) contract→CLI existence beyond status hygiene. This is *why* F1–F3 can
exist while CI stays green. It is also the precise answer to the founder's
open question: **parity of request logic is real; the artifact that should
prove it does not actually prove it.**
**Direction:** add a check that every `implemented` row's `cli_command`
resolves to a real clap leaf, and treat name mismatch as a failure.

---

## B. No-implicit-defaults audit

The platform principle is "every model resolution from an explicit
customer-declared source; fail loud, never a silent platform default." The
CLI honours this **where it matters most**, with a few connection/UX
exceptions.

### F7 — ⚪ Model resolution is clean (positive finding)
`corpus create` requires `--embedding-model` as a non-`Option` field
(`main.rs:830`) — the user *must* declare the model; there is no silent
default. `search`/`embeddings query`/`voices search` inject **no** model or
voice default and defer resolution to the corpus/voice server-side
(`build_search_body` `main.rs:2553`, `build_voice_search_body:2588`). This
is strong, deliberate compliance and should be called out as such.

### F5 — 🟠 Hard-coded `http://localhost:3000` base-URL fallback
`resolve_api_context` falls back to `http://localhost:3000` when no endpoint
is supplied by flag, env, or profile (`local.rs:499`); the health path
duplicates the literal (`main.rs:3437`). This is a silent default for the
**connection target** (not a model), but it is still a platform default
applied without an explicit source. Defensible for local-dev ergonomics;
worth an explicit "no endpoint configured" error in non-local contexts.

### F6 — 🟡 Empty-string API key on the health path
`enscrive health` builds its client with
`cli.api_key.clone().unwrap_or_default()` (`main.rs:3438`) — an empty key
rather than requiring one. Harmless for a health probe, but it is the one
spot that doesn't fail loud on a missing credential.

### F8 — 🟡 `evals from-url` invents non-deterministic request fields
When omitted, `dataset_name` is generated from a UTC timestamp slug and
`corpus_id` from a **random v4 UUID** (`main.rs:2376-2389`). The UUID makes
the command non-deterministic across retries (a fresh throwaway corpus each
run). Documented in the code, but it is a determinism wrinkle worth noting
against the "deterministic and AI-steerable" stance.

### F9 — ⚪ Client-injected UX defaults (catalogue)
Sent in request bodies even when the user is silent: `search limit=10`
(`main.rs:348`); `datasets create source_type="huggingface"`
(`evals2.rs:62`); `qrels_split="test"` (`evals2.rs:108`);
`sample_strategy="full"` (`evals2.rs:70`); diff-cost `batch=true`
(`evals2.rs:299`); poll timeouts (1800/3600 s) and `poll_secs=3`. These are
UX conveniences, not model resolution — but `source_type="huggingface"` is a
content-type assumption injected silently and is the closest of these to the
spirit of the no-implicit-defaults rule.

---

## C. Correctness & robustness

### F10 — 🟡 Typed error classification bypassed on text/binary methods
The structured `ApiError` taxonomy (incl. `failure_class` and pre-launch
detection) is applied only on the `*_json` and multipart client methods. The
streaming/binary methods — `get_text_with_query`, `get_bytes_with_query`,
`post_text` — return ad-hoc `String` errors (`client.rs:294,338,377,409`)
and never call `classify_error_response`. So SSE/binary commands
(`segment document`, `evals run-campaign-stream`, `logs stream`,
`export tenant`) lose the failure-class taxonomy on error, surfacing a bare
`HTTP {status}: {body}` instead. Inconsistent error contract across command
families.

### F11 — 🟡 Server `poll_url` is ignored
The async launch contract returns `{ job_id, status, poll_url }`, but
`await_and_emit` reconstructs the poll path locally as `/v1/jobs/{job_id}`
(`jobs_polling.rs:319`) and never reads `poll_url`. If the server ever
returns a poll URL that isn't `/v1/jobs/{job_id}`, the CLI silently won't
follow it.

### F12 — ⚪ Client-side plan gating is dead in production
`preflight_gate`'s plan branch (`preflight.rs:43-49`) is unreachable because
the only caller hard-wires `let cached_plan: Option<&str> = None;`
(`main.rs:3225`). Plan enforcement is therefore **server-only** today; the
client tier table only enforces the `managed-only` deployment gate. Two
TODOs (`preflight.rs:51`, `main.rs:3223`) note the intended profile-cache.
By design for now, but the `required_plan` column in the contract has no
client-side teeth.

---

## D. Security posture

### F13 — 🟠 License JWT decoded without signature or expiry verification
`decode_jwt_payload_unverified` base64url-decodes the payload only — no
signature check, no expiry check, no trust anchor (`license.rs:52-81`). The
`expires_at` claim is captured (`license.rs:44`) but never compared to a
clock. The CLI never *gates* on the license (the server is authoritative,
which is the correct division), so the practical exposure is limited to
`enscrive license status` printing attacker-controlled values from a
tampered local file. Acceptable given server authority, but the `status`
output is trust-on-faith and should say so.

### F14 — ⚪ In-CLI binary fetch verifies SHA256 only — no signature
`enscrive init` (self-managed) fetches service binaries and verifies
**SHA256 against the manifest only** (`fetch_verify.rs:197`); cosign bundle
verification is an open TODO (ENS-82, `fetch_verify.rs:22`). The manifest's
`signature`, `size_bytes`, and `compatibility.min_cli_version` fields are
parsed but **unenforced** (`fetch_verify.rs:65,101,105`). Note the
standalone `install.sh` *does* do optional cosign verification — so the
CLI-driven fetch path is weaker than the shell installer. Path-traversal in
archive extraction *is* guarded (`fetch_verify.rs:341-350`).

### F15 — ⚪ No `User-Agent` / CLI-version header on any request
`client.rs` attaches no version or UA header (`client.rs:207-213`), though
`version::VERSION_LINE` exists for `--version`. The server cannot attribute
or version-gate CLI traffic — a gap for the stated "CLI is the platform's
primary test harness" role (no version telemetry, no min-version handshake).

---

## E. Dead code, stale comments & doc inconsistencies

### F16 — 🟡 Stale test contradicts shipped behaviour
`corpus_get_unsupported_response` (`main.rs:6027-6036`) asserts the message
"GET /v1/corpora/{id} is not yet available on public /v1" — but
`corpus get` is implemented and live (`main.rs:3766`). The test only
exercises the `unsupported()` envelope helper, but its example string is now
false.

### F17 — 🟡 `release_channel.rs` is misnamed
Despite the filename it contains **platform-target-triple** logic, with no
stable/dev channel concept at all (`current_target()`, `release_channel.rs:11`).
Misleading to a reader looking for channel resolution.

### F18 — ⚪ Unconstructed error variant
`FetchError::BinaryNotInManifest` (`fetch_verify.rs:110-124`) is defined but
never constructed (lookups fail via `PlatformMissing`). Minor dead variant.
`ApiError::Http4xx.code` is extracted but read only by tests.

### F19 — 🟡 Install URL documented three different ways
*(Hostnames unified on `install.enscrive.io` in DNS refactor P2.)*
README advertises `https://install.enscrive.io/install` (`README.md:26`),
`install.sh` documents `https://install.enscrive.io/install.sh`
(`install.sh:5`) and defaults the manifest to `install.enscrive.io/…`
(`install.sh:45`), and `installer/DESIGN-DECISIONS.md` says
`install.enscrive.io/install`. `manifest.yml` notes the distro aliases
(`install.enscrive.io`, `developer.enscrive.io`) front one CloudFront
distribution — the residual `/install` vs `/install.sh` path split is
still a minor foot-gun.

### F20 — 🟡 Release ships one platform; README claims five
`release.yml` builds a **single** target (`x86_64-unknown-linux-gnu`,
Fedora-only Phase 0, `release.yml:33-39`); mac/arm64/musl are deferred.
`README.md:31` advertises five platforms. Anyone on macOS/musl following the
README install today gets nothing for those targets.

### F21 — ⚪ Founder-gate greps for files that don't exist
`.github/scripts/pr-review.sh` escalates on changes to
`src/{signature,provision,manifest,fingerprint}.rs` — none of which exist in
this repo's `src/`. Forward-looking / dead match (defensive, not harmful).

### F22 — ⚪ Docs lag the shipped pipeline
`docs/RELEASING.md` still says cosign (ENS-82) has not landed and describes a
manual tag/promote flow, while `release.yml`'s `sign` job and `install.sh`
already perform cosign verification and `tag.yml` auto-tags every merge to
`main`. `build.rs:8-10`'s module doc describes a clap-tree test that actually
lives in `src/main.rs`. The `installer/manifests/dev/latest.json` fixture is
entirely placeholder data (`REPLACE_ME_*`, `size_bytes:0`) — intentional
schema fixture, not a live manifest.

---

## F. Cross-repo items (surfaced, not actionable here)

Per scope discipline these are documented, **not** changed:

- **Contract is downstream of `enscrive-developer`'s `/v1`.** The contract
  header instructs that adding a `/v1` endpoint to `enscrive-developer`
  requires a row here. Verifying *completeness* of the mirror (no `/v1`
  endpoint missing from the contract) requires the `enscrive-developer`
  source, which is out of this repo. The drift found here (F1–F3) is all
  within this repo and fixable here.
- **Proposed ADR direction:** make `v1-surface-contract.toml` a *generated*
  or *cross-checked* artifact against the `enscrive-developer` OpenAPI/route
  table, and extend the parity test to assert `cli_command` ↔ clap-leaf
  identity (closing F4). That couples two repos and is an orchestrator-level
  decision, not a unilateral change from inside `enscrive-cli`.
