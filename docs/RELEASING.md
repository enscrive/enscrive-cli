# Releasing Enscrive CLI

## Versioning

Enscrive CLI uses **semantic versioning** (SemVer). Release tags follow the format:

- GA: `vX.Y.Z` (e.g., `v0.1.0`, `v1.2.3`)
- Pre-release: `vX.Y.Z-beta.N` (e.g., `v0.1.0-beta.1`, `v0.1.0-beta.2`)

## Pre-release vs. Full Release

- **Pre-release** (`-beta.N`) is used for candidate builds undergoing validation. Use this when the release is ready for early testing but not yet widely promoted.
- **Full release** (`vX.Y.Z`) is production-ready and widely published.

## Release Checklist

- [ ] Classification table up to date in `v1-surface-contract.toml`
- [ ] `cargo test` passes on main
- [ ] Contract drift test passes (`cargo test --test surface_contract`)
- [ ] CI green on main
- [ ] Tag the commit (`git tag vX.Y.Z`, then `git push origin vX.Y.Z`)
- [ ] Verify GitHub Release draft created by the release workflow
- [ ] Review draft; promote to published (or mark as pre-release for beta tags)
- [ ] Update README version references (if any)
- [ ] Verify signatures post-release (see verification section)

## Signature Verification

Binary and artifact signatures are generated during the release workflow using cosign. Until ENS-82 (cosign integration) lands, users may verify authenticity via the aggregated `SHA256SUMS` file attached to each release.

Once cosign signing is in place, the verification command will be:

```bash
cosign verify-blob --certificate <url> --signature <url> --certificate-identity-regexp ...
```

See the release notes for the exact invocation at the time of release.
