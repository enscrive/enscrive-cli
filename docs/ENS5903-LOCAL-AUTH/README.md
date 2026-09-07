# ENS5903: prepared Observe credentials for local stacks

Self-managed init, reinit and start now require a prepared ESM executable, master-key file and the three existing service vaults. The CLI validates the complete credential inventory before local mutation or service start, then supplies each child only its authorized Observe credential fields. Missing, malformed, mismatched or ambiguous inputs fail explicitly; credentials are not generated or repaired by a fallback. Public API request behavior is unchanged.

The implementation follows the ratified Observe caller-authority CLI decision (governance cfa16f1) and CLI CI fixture decision (2beea13). Delivery remains on the coordinated authentication **merge/release hold** until the orchestration gate is satisfied. A draft PR and passing local tests do not establish rollout readiness.

## Validation and evidence

Local proof passed **62 named Rust cases**: 52 existing/local cases, six loader/process cases, three integration cases and one ignored test explicitly executed with the previously signature-verified ESM artifact. This covers prepared-file/PID refusal, full credential inventory, recipient overlays, protected files, bounded child capture and actual disposable synthetic ESM parser/synthesis behavior. The native version smoke also passed. Build and runtime each stayed within 3 GiB with no swap/OOM and verified owned cleanup.

A separate bounded run passed **17 synthetic Python CI-helper controls**, including stalled download-worker phases, incorrect bytes and file identity, timeout/output limits, selector setup/close failures and process cleanup. That run used no network download, Cargo, ESM or provider. It validates the helper's synthetic seams, not the hosted CI campaign.

The required test job preserves ordinary tests and adds an exact frozen 61-case ordinary subset plus the one actual-ESM case, using fixed size/hash trust in a previously signature-verified artifact. The reviewed helper limits download and child execution and verifies owned file identity before execution. Hosted CI results remain pending at delivery; these workflow changes have not yet proved a hosted download or hosted ESM run.

The paid Docker/Podman onboarding matrix and its assertions remain present, but the entire job now requires a manual workflow dispatch with `run_paid_onboarding: true` (default false). No paid campaign was run or claimed by this work. A separately authorized coherent service/credential campaign is still required for full onboarding.

Retained evidence identifiers:

- Product source inventory: `a316060296c9f545e32f79ff511f889ed16da71c4293e29a42f7c6948e4900be`.
- Successful build receipt: `1dca5d929ebdbb19ab134ca75b2fd2136444d27fcb3228c1d394504d5e5717c9`.
- Successful runtime receipt: `e4d43b3f89b1474e1cca1a53c4e6ea947d4410fd9110e0094e1166e99814438e`.
- Closed 182-artifact product index: `6d5afeecacd669799eba409deeb64ac86008030957035aee7705c5030b9ea92b`.
- CI candidate inventory: `0841f3fee4b329ddd1c1ac920cca01a0137ddbecb3d4d00a4ddbdc7e669677eb`.
- Independent helper result review: `71095a2c3f86f3ba3fce4014fd3e4d24df6b7800ebf61004f363ae67e3c1b8d8`.

The first compile failed lint, and the first runtime failed seven legacy fixture assertions that incorrectly treated the unrelated `OBSERVE_GRPC_ADDR` as a protected credential. Those runs and approved corrections remain retained. The passing result uses the corrected exact-ten-name fixture; product runtime semantics were not weakened. This proof does not establish full-stack interoperability, provider delivery, recall, accounting, disaster recovery or production readiness.
