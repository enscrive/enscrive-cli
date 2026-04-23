/// Surface contract test — ensures every /v1 endpoint in enscrive-developer
/// has a corresponding CLI command entry in v1-surface-contract.toml and that
/// no entries remain in "missing" status.
use serde::Deserialize;

const CONTRACT_TOML: &str = include_str!("../v1-surface-contract.toml");

#[derive(Debug, Deserialize)]
struct Contract {
    endpoint: Vec<Endpoint>,
}

#[derive(Debug, Deserialize)]
struct Endpoint {
    method: String,
    path: String,
    cli_command: String,
    status: String,
    #[serde(default)]
    reason: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    note: Option<String>,
    /// Deployment-mode gate. Values: "any-mode" | "managed-only".
    /// "managed-only" endpoints return 501 / FAIL_UNSUPPORTED_IN_LOCAL_MODE
    /// in local deployment mode regardless of plan.
    deployment_tier: String,
    /// Minimum plan tier. Values: "free" | "professional" | "enterprise".
    /// Free-plan callers hitting a non-free endpoint get FAIL_PLAN_REQUIRED.
    required_plan: String,
}

#[test]
fn contract_toml_parses() {
    let contract: Contract =
        toml::from_str(CONTRACT_TOML).expect("v1-surface-contract.toml must parse as valid TOML");
    assert!(
        !contract.endpoint.is_empty(),
        "contract must contain at least one endpoint"
    );
}

#[test]
fn no_missing_endpoints() {
    let contract: Contract =
        toml::from_str(CONTRACT_TOML).expect("v1-surface-contract.toml must parse");

    let missing: Vec<&Endpoint> = contract
        .endpoint
        .iter()
        .filter(|e| e.status == "missing")
        .collect();

    if !missing.is_empty() {
        eprintln!("\n=== CLI Surface Contract Gaps ===\n");
        for e in &missing {
            eprintln!("  MISSING: {} {} -> `enscrive {}`", e.method, e.path, e.cli_command);
        }
        eprintln!("\n  {} endpoint(s) have no CLI implementation.\n", missing.len());
        eprintln!("  To fix: implement the CLI command or change status to");
        eprintln!("  \"deferred\" with a reason in v1-surface-contract.toml.\n");
        panic!(
            "{} endpoint(s) have status = \"missing\" in v1-surface-contract.toml",
            missing.len()
        );
    }
}

#[test]
fn deferred_endpoints_have_reasons() {
    let contract: Contract =
        toml::from_str(CONTRACT_TOML).expect("v1-surface-contract.toml must parse");

    let bad: Vec<&Endpoint> = contract
        .endpoint
        .iter()
        .filter(|e| e.status == "deferred" && e.reason.as_deref().unwrap_or("").is_empty())
        .collect();

    if !bad.is_empty() {
        eprintln!("\n=== Deferred Endpoints Without Reasons ===\n");
        for e in &bad {
            eprintln!("  DEFERRED (no reason): {} {} -> `enscrive {}`", e.method, e.path, e.cli_command);
        }
        eprintln!();
        panic!(
            "{} deferred endpoint(s) are missing a reason field",
            bad.len()
        );
    }
}

#[test]
fn valid_status_values() {
    let contract: Contract =
        toml::from_str(CONTRACT_TOML).expect("v1-surface-contract.toml must parse");

    let valid_statuses = ["implemented", "missing", "deferred"];

    let invalid: Vec<&Endpoint> = contract
        .endpoint
        .iter()
        .filter(|e| !valid_statuses.contains(&e.status.as_str()))
        .collect();

    if !invalid.is_empty() {
        eprintln!("\n=== Invalid Status Values ===\n");
        for e in &invalid {
            eprintln!("  INVALID status \"{}\": {} {}", e.status, e.method, e.path);
        }
        eprintln!();
        panic!(
            "{} endpoint(s) have invalid status values",
            invalid.len()
        );
    }
}

#[test]
fn no_duplicate_endpoints() {
    let contract: Contract =
        toml::from_str(CONTRACT_TOML).expect("v1-surface-contract.toml must parse");

    let mut seen = std::collections::HashSet::new();
    let mut dupes = Vec::new();

    for e in &contract.endpoint {
        let key = format!("{} {}", e.method, e.path);
        if !seen.insert(key.clone()) {
            dupes.push(key);
        }
    }

    if !dupes.is_empty() {
        eprintln!("\n=== Duplicate Endpoints ===\n");
        for d in &dupes {
            eprintln!("  DUPLICATE: {}", d);
        }
        eprintln!();
        panic!("{} duplicate endpoint(s) found", dupes.len());
    }
}

#[test]
fn valid_deployment_tier_values() {
    let contract: Contract =
        toml::from_str(CONTRACT_TOML).expect("v1-surface-contract.toml must parse");

    let valid = ["any-mode", "managed-only"];
    let invalid: Vec<&Endpoint> = contract
        .endpoint
        .iter()
        .filter(|e| !valid.contains(&e.deployment_tier.as_str()))
        .collect();

    if !invalid.is_empty() {
        eprintln!("\n=== Invalid deployment_tier values ===\n");
        for e in &invalid {
            eprintln!(
                "  INVALID deployment_tier \"{}\": {} {}",
                e.deployment_tier, e.method, e.path
            );
        }
        eprintln!();
        panic!(
            "{} endpoint(s) have invalid deployment_tier values (must be any-mode or managed-only)",
            invalid.len()
        );
    }
}

#[test]
fn valid_required_plan_values() {
    let contract: Contract =
        toml::from_str(CONTRACT_TOML).expect("v1-surface-contract.toml must parse");

    let valid = ["free", "professional", "enterprise"];
    let invalid: Vec<&Endpoint> = contract
        .endpoint
        .iter()
        .filter(|e| !valid.contains(&e.required_plan.as_str()))
        .collect();

    if !invalid.is_empty() {
        eprintln!("\n=== Invalid required_plan values ===\n");
        for e in &invalid {
            eprintln!(
                "  INVALID required_plan \"{}\": {} {}",
                e.required_plan, e.method, e.path
            );
        }
        eprintln!();
        panic!(
            "{} endpoint(s) have invalid required_plan values (must be free, professional, or enterprise)",
            invalid.len()
        );
    }
}

#[test]
fn managed_only_endpoints_imply_non_free_plan() {
    // A managed-only endpoint should never be free-plan: if it's only available
    // in managed mode (i.e., against api.enscrive.io), the user is necessarily
    // on a paid plan. This catches classification mistakes.
    let contract: Contract =
        toml::from_str(CONTRACT_TOML).expect("v1-surface-contract.toml must parse");

    let bad: Vec<&Endpoint> = contract
        .endpoint
        .iter()
        .filter(|e| e.deployment_tier == "managed-only" && e.required_plan == "free")
        .collect();

    if !bad.is_empty() {
        eprintln!("\n=== managed-only + free endpoints (classification conflict) ===\n");
        for e in &bad {
            eprintln!(
                "  CONFLICT: {} {} -> `enscrive {}`",
                e.method, e.path, e.cli_command
            );
        }
        eprintln!();
        panic!(
            "{} endpoint(s) are managed-only but free-plan — review classification",
            bad.len()
        );
    }
}

#[test]
fn contract_coverage_report() {
    let contract: Contract =
        toml::from_str(CONTRACT_TOML).expect("v1-surface-contract.toml must parse");

    let total = contract.endpoint.len();
    let implemented = contract.endpoint.iter().filter(|e| e.status == "implemented").count();
    let deferred = contract.endpoint.iter().filter(|e| e.status == "deferred").count();
    let missing = contract.endpoint.iter().filter(|e| e.status == "missing").count();

    eprintln!("\n=== CLI Surface Contract Coverage ===\n");
    eprintln!("  Total endpoints:  {}", total);
    eprintln!("  Implemented:      {} ({:.0}%)", implemented, (implemented as f64 / total as f64) * 100.0);
    eprintln!("  Deferred:         {}", deferred);
    eprintln!("  Missing:          {}", missing);

    if deferred > 0 {
        eprintln!("\n  Deferred endpoints:");
        for e in contract.endpoint.iter().filter(|e| e.status == "deferred") {
            eprintln!("    {} {} -> `enscrive {}`", e.method, e.path, e.cli_command);
            if let Some(reason) = &e.reason {
                eprintln!("      reason: {}", reason);
            }
        }
    }
    eprintln!();
}
