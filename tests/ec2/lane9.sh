#!/usr/bin/env bash
# Lane 9: plan-tier enforcement on a fresh EC2 t3.large.
# BLOCKED until enscrive-developer ships server-side require_plan() / require_tier().
# Run with: ENSCRIVE_API_KEY=... ENSCRIVE_API_BASE=... bash tests/ec2/lane9.sh
set -euo pipefail

ENSCRIVE_BIN="${ENSCRIVE_BIN:-./target/release/enscrive}"
API_KEY="${ENSCRIVE_API_KEY:?required}"
API_BASE="${ENSCRIVE_API_BASE:?required}"
LICENSE_JWT_PRO="${LICENSE_JWT_PRO:?test license JWT required}"

step() { echo; echo "=== $* ==="; }
expect_exit() { local want="$1" got="$2" ctx="$3"; [ "$got" = "$want" ] || { echo "FAIL $ctx: want exit $want, got $got"; exit 1; }; }

step "Step 1: Free-plan rejection on Pro command"
set +e; "$ENSCRIVE_BIN" --endpoint "$API_BASE" --api-key "$API_KEY" voices promote --voice-id v1 --target-environment-id stage; ec=$?; set -e
expect_exit 4 "$ec" "step 1 (FAIL_PLAN_REQUIRED expected, exit 4)"

step "Step 2: Activate Pro license"
"$ENSCRIVE_BIN" license activate "$LICENSE_JWT_PRO"
"$ENSCRIVE_BIN" license status --output json | jq -e '.data.claims_as_written.plan == "professional"'

step "Step 3: Pro command now succeeds"
"$ENSCRIVE_BIN" --endpoint "$API_BASE" --api-key "$API_KEY" voices promote --voice-id v1 --target-environment-id stage

step "Step 4: Network loss — within 7-day grace"
# Block outbound to api.enscrive.io / api base via iptables (requires root)
echo "(manual step: simulate network loss; rerun voices promote and assert success)"

step "Step 5: license deactivate → fall back to FAIL_PLAN_REQUIRED"
"$ENSCRIVE_BIN" license deactivate
set +e; "$ENSCRIVE_BIN" --endpoint "$API_BASE" --api-key "$API_KEY" voices promote --voice-id v1 --target-environment-id stage; ec=$?; set -e
expect_exit 4 "$ec" "step 5 (FAIL_PLAN_REQUIRED post-deactivate)"

step "Step 6: voices promote in local mode rejected regardless of license"
# Configure profile mode=local with cached_plan=enterprise (force unsupported-in-local)
echo "(manual step: enscrive-deploy or profile edit to local mode, assert FAIL_UNSUPPORTED_IN_LOCAL_MODE)"

echo "Lane 9: PASS"
