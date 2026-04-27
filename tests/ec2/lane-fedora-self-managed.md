# EC2 Test Lane: Fedora self-managed install (Lane Fedora)

## Overview

Validates the **public install path** end-to-end on a clean Fedora EC2
instance, exactly as a real developer evaluating Enscrive would do it:

```
curl -fsSL https://install.enscrive.io/install.sh | sh
enscrive init --mode self-managed --yes
```

Confirms the manifest fetch, SHA256 verification, archive extraction,
systemd-unit registration, and dev-portal startup all work without any
internal-only access.

This test is the public counterpart of `enscrive-deploy provision` — that
tool provisions the **service plane** (used by Enscrive operators); this
test exercises the path used by **end users** running Enscrive locally
for evaluation.

## Why EC2 (not Docker)

Considered both. Pure CLI smoke (just `install.sh` + `--help`) runs fine
in a `fedora:42` container. But this lane brings up the full stack
(postgres + keycloak + qdrant + loki + 3 service binaries) under
`systemd --user` units, and systemd inside Docker is rough. EC2 is the
honest substrate; the cost (a few minutes of t3.medium) is fine for the
guardrailed cycle budget.

## Prerequisites

- Operator workstation has AWS CLI configured for the `enscrive` account.
- `session-manager-plugin` installed locally (for `aws ssm start-session`):
  `sudo dnf install -y session-manager-plugin` on Fedora.
- An IAM instance profile that grants the EC2 instance SSM access
  (e.g. `enscrive-ec2-ssm` from earlier sessions). The instance does not
  need a public IP, an SSH key, or any inbound security-group rule.
- Outbound HTTPS from the EC2 instance to:
  - `install.enscrive.io` (manifest + binaries via CloudFront)
  - `auth.docker.io`, `registry-1.docker.io` (if the keycloak/qdrant/loki
    containers are pulled at init time — check the install path).

## Operator workstation: launch + drive

EC2 launches are owned by the operator (max 5 cycles per CLAUDE.md
guardrail). Suggested commands — adapt AMI / region / instance-profile
to your account:

```bash
# 1. Launch a fresh Fedora 42 t3.medium with SSM access
INSTANCE_ID=$(aws ec2 run-instances \
  --region us-east-2 \
  --image-id ami-fedora-42-x86_64 \
  --instance-type t3.medium \
  --iam-instance-profile Name=enscrive-ec2-ssm \
  --tag-specifications 'ResourceType=instance,Tags=[
    {Key=Name,Value=enscrive-cli-fedora-smoke},
    {Key=ttl,Value=3h}]' \
  --metadata-options HttpTokens=required,HttpEndpoint=enabled \
  --query 'Instances[0].InstanceId' \
  --output text)

echo "Launched: $INSTANCE_ID"

# 2. Wait for SSM agent ready
aws ssm wait instance-information-available --filters \
  Key=InstanceIds,Values="$INSTANCE_ID" --region us-east-2

# 3. Push the test script onto the instance
TEST_SCRIPT=$(base64 -w0 < tests/ec2/lane-fedora-self-managed.sh)
aws ssm send-command --region us-east-2 \
  --instance-ids "$INSTANCE_ID" \
  --document-name AWS-RunShellScript \
  --parameters "commands=[
    \"echo $TEST_SCRIPT | base64 -d > /tmp/lane-fedora.sh\",
    \"chmod +x /tmp/lane-fedora.sh\",
    \"sudo -u fedora bash /tmp/lane-fedora.sh 2>&1 | tee /tmp/lane-fedora.log\"
  ]" \
  --output text

# 4. Tail the log (use the CommandId from step 3)
aws ssm get-command-invocation --region us-east-2 \
  --command-id <CommandId> \
  --instance-id "$INSTANCE_ID" \
  --query 'StandardOutputContent'
```

## Browser access via SSM port forwarding

The dev portal binds to `127.0.0.1:3000` on the EC2 instance. SSM forwards
that to your workstation's `13001` (avoids any clash with local services
on `:3000`):

```bash
aws ssm start-session --region us-east-2 \
  --target "$INSTANCE_ID" \
  --document-name AWS-StartPortForwardingSession \
  --parameters '{"portNumber":["3000"],"localPortNumber":["13001"]}'
```

Leave that running. In another terminal or browser:

```bash
open http://localhost:13001
```

You should see the Enscrive developer portal as a fresh-eval user would.

## Default credentials

For the self-managed dev/eval flow, Keycloak admin defaults to
`developer` / `developer` (see push-back note below — this is a
deliberate convenience for local evaluation, not a security stance).
Login at `http://localhost:13001/auth/admin` if you need to inspect realm
state.

> **Push back available.** `developer/developer` is fine for local-dev
> convenience but should be (1) printed loudly at install time so the
> user knows they exist and could change them, and (2) overrideable via
> `--admin-password=...` flag for users who'd prefer a unique password
> on day one. If a user accidentally exposes `:8080` (Keycloak) to the
> internet, well-known credentials become a finding. We accept this risk
> for *local* dev; it must NOT be the default in any deployment that
> isn't strictly self-managed-on-laptop.

## Teardown

```bash
aws ec2 terminate-instances --region us-east-2 --instance-ids "$INSTANCE_ID"
aws ec2 wait instance-terminated --region us-east-2 --instance-ids "$INSTANCE_ID"
```

## Acceptance

1. `install.sh` lands `enscrive` at `~/.local/bin/enscrive`.
2. `enscrive --version` prints a non-empty version.
3. `enscrive init --mode self-managed --yes` exits 0 within ~5 minutes.
4. `curl http://127.0.0.1:3000/healthz` on the EC2 returns 200.
5. From the workstation, `http://localhost:13001` shows the dev portal.
6. Login as `developer` / `developer` succeeds at the realm admin URL.

## Out of scope

- Cosign signature verification of `install.sh` (tracked as ENS-82
  follow-on; today's install.sh does SHA256 but not Sigstore yet).
- Managed-mode init (`enscrive init --mode managed`) — that connects to
  `api.enscrive.io` and is a different code path.
- Long-running soak (this is smoke only; teardown after pass).
