# Aggregate pin-set publication (ENS-5787)

The `manifest.yml` workflow signs the exact schema-3 JSON bytes with cosign,
then publishes:

- `pinsets/sha256/<manifest-byte-sha256>/manifest.json`
- `pinsets/sha256/<manifest-byte-sha256>/manifest.bundle`

The signer identity is exactly
`https://github.com/enscrive/enscrive-cli/.github/workflows/manifest.yml@refs/heads/main`,
issued by `https://token.actions.githubusercontent.com`. Both the workflow and
publisher reject a dispatch from another ref. The cosign installer uses the
existing release workflow's verified cache and pinned binary digests.

Manifest and bundle objects use conditional creation. An occupied manifest must
match exactly. An occupied bundle must verify against those bytes and the pinned
identity; randomized signature bundles are not compared byte-for-byte. A missing
bundle after a partial upload can be recovered without replacing the manifest.
A failed write is reconciled by reading stored bytes, including when a response
was lost after a successful write. Conflicting or unverifiable objects stop
publication. The stored pair is verified before discovery changes.

`releases/<channel>/<version>/manifest.json` and
`releases/<channel>/latest.json` remain schema-compatible discovery copies. Both
are mutable and individually updated with ETag compare-and-swap (or conditional
creation when absent). The versioned alias is updated first; a later latest
conflict can therefore leave only the versioned alias advanced. The workflow
fails in that case, preserves the signed pair, and does not overwrite the
competing latest value. There is no atomic transaction across these aliases.
Same-channel workflows serialize without cancelling an active publisher; this
is not a guarantee of FIFO publication or newest-source freshness.

Each publisher invocation reads the generated manifest once and uses those
frozen bytes throughout. A complete workflow rerun regenerates `released_at`
and may select newer channel pins, producing a new pin-set. It is not a retry
of the same logical aggregate. Conditional creation and bundle reuse apply
when exact manifest bytes are reused.

Successful publication records the digest, object keys and fixed signing
identity in a workflow artifact. The S3 publisher must have read/write access
to the pin-set prefix; consumers need read access. Missing-key reads must be
distinguishable from access errors. No IAM or CDN policy changes are included.

This implements append-only behavior under the publication protocol, not S3
Object Lock or protection against deletion by an authorized principal. It does
not change component signatures, install verification, provisioning, campaign
certification, or historical unsigned manifests. Aggregate consumer enforcement
is a separate change. A real first dev publication must verify the stored bundle
with cosign and the exact identity above before enabling consumer requirements.

Offline verification:

```sh
python3 -m unittest discover -s scripts -p test_publish_pinset.py -v
```
