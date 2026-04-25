# EC2 Test Lane: Plan-Tier Enforcement (Lane 9)

## Overview

Lane 9 validates plan-tier enforcement across the managed Enscrive platform on a clean EC2 instance. The harness exercises the full lifecycle: free-plan rejection → professional license activation → success → network isolation → license deactivation → fallback.

## Prerequisites

- **Instance type**: Ubuntu 22.04 LTS, t3.large or equivalent
- **Tools**: bash, jq
- **Credentials**: valid `ENSCRIVE_API_KEY` and `ENSCRIVE_API_BASE`
- **Test JWT**: a valid JWT for a professional plan (in `LICENSE_JWT_PRO` env var)
- **Server state**: enscrive-developer must have server-side plan-tier enforcement live (requires backend work not yet shipped)

## Running the Test

```bash
export ENSCRIVE_API_KEY=enscrive_...
export ENSCRIVE_API_BASE=http://localhost:13000
export LICENSE_JWT_PRO=eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...
bash tests/ec2/lane9.sh
```

## Test Stages

1. **Free-plan rejection** — Command blocked with `FAIL_PLAN_REQUIRED` (exit 4)
2. **License activation** — CLI writes JWT to license storage, plan visible in `license status`
3. **Pro command success** — Same command now succeeds with professional plan
4. **Network grace period** — (manual) Simulate outbound loss; command should still succeed within 7 days
5. **Deactivation fallback** — License removed; command reverts to `FAIL_PLAN_REQUIRED`
6. **Local mode enforcement** — (manual) Switch profile to local mode; command blocked with `FAIL_UNSUPPORTED_IN_LOCAL_MODE`

## Notes

- This harness is **blocked** until enscrive-developer ships server-side `require_plan()` and `require_tier()` enforcement
- Exit code 4 maps to `FAIL_PLAN_REQUIRED`; exit code 2 maps to `FAIL_UNSUPPORTED_IN_LOCAL_MODE`
- Manual steps (4, 6) require operator intervention or script extension (iptables for network sim, profile edit for mode change)
