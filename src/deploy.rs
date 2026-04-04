use clap::ValueEnum;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const DEPLOY_PROFILE_VERSION: u32 = 2;
const LEGACY_LOCAL_DEPLOY_ENDPOINT: &str = "http://127.0.0.1:3000";
const DEFAULT_BOOTSTRAP_BUNDLE_SECRET_KEY: &str = "ENSCRIVE_BOOTSTRAP_BUNDLE";

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployTarget {
    Dev,
    Stage,
    Us,
    Eu,
    Ap,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeploySecretsSource {
    Prompt,
    Env,
    Esm,
}

#[derive(Debug, Clone)]
pub struct DeployInitOptions {
    pub target: Option<DeployTarget>,
    pub profile_name: Option<String>,
    pub secrets_source: Option<DeploySecretsSource>,
    pub endpoint_override: Option<String>,
    pub set_default: bool,
}

#[derive(Debug, Clone)]
pub struct DeployStatusOptions {
    pub profile_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeployBootstrapOptions {
    pub profile_name: Option<String>,
    pub endpoint_override: Option<String>,
    pub bundle_path: Option<String>,
    pub bundle_secret_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct DeployProfilesFile {
    version: u32,
    default_profile: Option<String>,
    profiles: BTreeMap<String, DeployProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct DeployProfile {
    target: String,
    aws_region: String,
    secrets_source: String,
    #[serde(default = "default_legacy_deploy_endpoint")]
    endpoint: String,
    esm: Option<EsmProfile>,
    bootstrap: Option<DeployBootstrapState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EsmProfile {
    binary: String,
    workdir: Option<String>,
    vault_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct DeployBootstrapState {
    tenant_id: String,
    tenant_name: String,
    environment_id: String,
    environment_name: String,
    user_id: Option<String>,
    platform_admin_key_id: Option<String>,
    platform_admin_api_key: Option<String>,
    operator_key_id: Option<String>,
    operator_api_key: Option<String>,
    consumed_nonce: String,
    stack_id: String,
    bootstrapped_at: String,
    bundle_source: String,
}

#[derive(Debug, Clone)]
struct CliHome {
    config_root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct EsmDiscovery {
    binary: String,
    workdir: Option<String>,
    vault_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignedBootstrapConsumeRequest {
    bundle_toml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignedBootstrapConsumeResponse {
    tenant_id: String,
    tenant_name: String,
    environment_id: String,
    environment_name: String,
    user_id: Option<String>,
    platform_admin_key_id: Option<String>,
    platform_admin_key: Option<String>,
    operator_key_id: Option<String>,
    operator_key: Option<String>,
    created_tenant: bool,
    created_environment: bool,
    created_user: bool,
    consumed_nonce: String,
    stack_id: String,
}

fn default_legacy_deploy_endpoint() -> String {
    LEGACY_LOCAL_DEPLOY_ENDPOINT.to_string()
}

pub async fn init(opts: DeployInitOptions) -> Result<Value, String> {
    let mut profiles = load_deploy_profiles()?;
    let target = match opts.target {
        Some(target) => target,
        None => prompt_target()?,
    };
    let profile_name = opts
        .profile_name
        .unwrap_or_else(|| deploy_target_name(target).to_string());
    let configured_endpoint = opts
        .endpoint_override
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_managed_endpoint(target).to_string());
    let detected_esm = discover_esm();
    let secrets_source = resolve_secrets_source(opts.secrets_source, detected_esm.as_ref())?;

    let esm = if secrets_source == DeploySecretsSource::Esm {
        let discovered = detected_esm.ok_or_else(|| {
            "ESM was selected but no local ESM binary/vault was detected".to_string()
        })?;

        if env::var("ESM_MASTER_KEY")
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        {
            // already available for this process
        } else if interactive_prompts_available() {
            let master_key = prompt_secret_line("ESM master key")?;
            if !master_key.is_empty() {
                unsafe {
                    env::set_var("ESM_MASTER_KEY", master_key);
                }
            }
        }

        Some(EsmProfile {
            binary: discovered.binary,
            workdir: discovered.workdir,
            vault_path: discovered.vault_path,
        })
    } else {
        None
    };

    let profile = DeployProfile {
        target: deploy_target_name(target).to_string(),
        aws_region: deploy_target_region(target).to_string(),
        secrets_source: deploy_secrets_source_name(secrets_source).to_string(),
        endpoint: configured_endpoint.clone(),
        esm,
        bootstrap: None,
    };

    profiles.version = DEPLOY_PROFILE_VERSION;
    profiles
        .profiles
        .insert(profile_name.clone(), profile.clone());
    if opts.set_default || profiles.default_profile.is_none() {
        profiles.default_profile = Some(profile_name.clone());
    }
    save_deploy_profiles(&profiles)?;

    Ok(json!({
        "profile": profile_name,
        "target": profile.target,
        "aws_region": profile.aws_region,
        "secrets_source": profile.secrets_source,
        "endpoint": configured_endpoint,
        "default_profile": profiles.default_profile,
        "esm": profile.esm,
        "note": "deploy profiles are operator-facing and separate from customer init/self-managed profiles"
    }))
}

pub async fn status(opts: DeployStatusOptions) -> Result<Value, String> {
    let profiles = load_deploy_profiles()?;
    let (selected_name, profile) = resolve_selected_profile(&profiles, opts.profile_name)?;
    let detected_esm = discover_esm();
    let endpoint_state = profile_endpoint_state(&profile);

    Ok(json!({
        "profile": selected_name,
        "target": profile.target,
        "aws_region": profile.aws_region,
        "secrets_source": profile.secrets_source,
        "endpoint": endpoint_state.effective,
        "stored_endpoint": endpoint_state.stored,
        "endpoint_origin": endpoint_state.origin,
        "esm_configured": profile.esm,
        "esm_detected_now": detected_esm,
        "esm_master_key_present": env::var("ESM_MASTER_KEY").ok().map(|value| !value.trim().is_empty()).unwrap_or(false),
        "bootstrap": profile.bootstrap.as_ref().map(|bootstrap| json!({
            "tenant_id": bootstrap.tenant_id,
            "tenant_name": bootstrap.tenant_name,
            "environment_id": bootstrap.environment_id,
            "environment_name": bootstrap.environment_name,
            "user_id": bootstrap.user_id,
            "stack_id": bootstrap.stack_id,
            "consumed_nonce": bootstrap.consumed_nonce,
            "platform_admin_key_id": bootstrap.platform_admin_key_id,
            "platform_admin_api_key_present": bootstrap.platform_admin_api_key.as_ref().map(|value| !value.trim().is_empty()).unwrap_or(false),
            "operator_key_id": bootstrap.operator_key_id,
            "operator_api_key_present": bootstrap.operator_api_key.as_ref().map(|value| !value.trim().is_empty()).unwrap_or(false),
            "bootstrapped_at": bootstrap.bootstrapped_at,
            "bundle_source": bootstrap.bundle_source,
        })),
    }))
}

pub async fn bootstrap(opts: DeployBootstrapOptions) -> Result<Value, String> {
    let mut profiles = load_deploy_profiles()?;
    let (selected_name, mut profile) = resolve_selected_profile(&profiles, opts.profile_name)?;
    let endpoint_state = profile_endpoint_state(&profile);
    let endpoint_override_used = opts
        .endpoint_override
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    let request_endpoint = opts
        .endpoint_override
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| endpoint_state.effective.clone());

    let (bundle_toml, bundle_source) = resolve_bootstrap_bundle(
        &profile,
        opts.bundle_path.as_deref(),
        opts.bundle_secret_key.as_deref(),
    )?;

    let response = consume_signed_bootstrap(&request_endpoint, &bundle_toml).await?;
    let bootstrapped_at = chrono::Utc::now().to_rfc3339();

    let persist_endpoint = if endpoint_override_used {
        endpoint_state.effective.clone()
    } else {
        request_endpoint.clone()
    };

    profile.endpoint = persist_endpoint.clone();
    profile.bootstrap = Some(DeployBootstrapState {
        tenant_id: response.tenant_id.clone(),
        tenant_name: response.tenant_name.clone(),
        environment_id: response.environment_id.clone(),
        environment_name: response.environment_name.clone(),
        user_id: response.user_id.clone(),
        platform_admin_key_id: response.platform_admin_key_id.clone(),
        platform_admin_api_key: response.platform_admin_key.clone(),
        operator_key_id: response.operator_key_id.clone(),
        operator_api_key: response.operator_key.clone(),
        consumed_nonce: response.consumed_nonce.clone(),
        stack_id: response.stack_id.clone(),
        bootstrapped_at: bootstrapped_at.clone(),
        bundle_source: bundle_source.clone(),
    });

    profiles.version = DEPLOY_PROFILE_VERSION;
    profiles
        .profiles
        .insert(selected_name.clone(), profile.clone());
    save_deploy_profiles(&profiles)?;

    Ok(json!({
        "profile": selected_name,
        "target": profile.target,
        "aws_region": profile.aws_region,
        "endpoint": request_endpoint,
        "stored_endpoint": persist_endpoint,
        "endpoint_override_used": endpoint_override_used,
        "bundle_source": bundle_source,
        "tenant_id": response.tenant_id,
        "tenant_name": response.tenant_name,
        "environment_id": response.environment_id,
        "environment_name": response.environment_name,
        "user_id": response.user_id,
        "platform_admin_key_id": response.platform_admin_key_id,
        "platform_admin_api_key": response.platform_admin_key,
        "operator_key_id": response.operator_key_id,
        "operator_api_key": response.operator_key,
        "created_tenant": response.created_tenant,
        "created_environment": response.created_environment,
        "created_user": response.created_user,
        "consumed_nonce": response.consumed_nonce,
        "stack_id": response.stack_id,
        "bootstrapped_at": bootstrapped_at,
    }))
}

fn load_deploy_profiles() -> Result<DeployProfilesFile, String> {
    let home = cli_home()?;
    let path = home.config_root.join("deploy-profiles.toml");
    if !path.exists() {
        return Ok(DeployProfilesFile {
            version: DEPLOY_PROFILE_VERSION,
            default_profile: None,
            profiles: BTreeMap::new(),
        });
    }

    let content =
        fs::read_to_string(&path).map_err(|e| format!("read '{}': {e}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(DeployProfilesFile {
            version: DEPLOY_PROFILE_VERSION,
            default_profile: None,
            profiles: BTreeMap::new(),
        });
    }

    toml::from_str(&content).map_err(|e| format!("parse '{}': {e}", path.display()))
}

fn save_deploy_profiles(profiles: &DeployProfilesFile) -> Result<(), String> {
    let home = cli_home()?;
    fs::create_dir_all(&home.config_root).map_err(|e| format!("create config root: {e}"))?;
    let path = home.config_root.join("deploy-profiles.toml");
    let content = toml::to_string_pretty(profiles)
        .map_err(|e| format!("serialize deploy-profiles.toml: {e}"))?;
    fs::write(&path, content).map_err(|e| format!("write '{}': {e}", path.display()))
}

fn cli_home() -> Result<CliHome, String> {
    let home_dir = env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let config_root = env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(&home_dir).join(".config"))
        .join("enscrive");
    Ok(CliHome { config_root })
}

fn resolve_selected_profile(
    profiles: &DeployProfilesFile,
    profile_name: Option<String>,
) -> Result<(String, DeployProfile), String> {
    let selected_name = profile_name
        .or_else(|| env::var("ENSCRIVE_DEPLOY_PROFILE").ok())
        .or_else(|| profiles.default_profile.clone())
        .ok_or_else(|| {
            "no deploy profile configured; run `enscrive deploy init` first".to_string()
        })?;
    let profile = profiles
        .profiles
        .get(&selected_name)
        .cloned()
        .ok_or_else(|| format!("deploy profile '{}' not found", selected_name))?;
    Ok((selected_name, profile))
}

fn deploy_target_name(target: DeployTarget) -> &'static str {
    match target {
        DeployTarget::Dev => "dev",
        DeployTarget::Stage => "stage",
        DeployTarget::Us => "us",
        DeployTarget::Eu => "eu",
        DeployTarget::Ap => "ap",
    }
}

fn deploy_target_region(target: DeployTarget) -> &'static str {
    match target {
        DeployTarget::Dev | DeployTarget::Stage | DeployTarget::Us => "us-east-2",
        DeployTarget::Eu => "eu-central-1",
        DeployTarget::Ap => "ap-southeast-1",
    }
}

fn default_managed_endpoint(target: DeployTarget) -> &'static str {
    match target {
        DeployTarget::Dev => "https://dev.api.enscrive.io",
        DeployTarget::Stage => "https://stage.api.enscrive.io",
        DeployTarget::Us => "https://us.api.enscrive.io",
        DeployTarget::Eu => "https://eu.api.enscrive.io",
        DeployTarget::Ap => "https://ap.api.enscrive.io",
    }
}

fn default_managed_endpoint_for_target_name(target: &str) -> Option<&'static str> {
    match target.trim().to_ascii_lowercase().as_str() {
        "dev" => Some(default_managed_endpoint(DeployTarget::Dev)),
        "stage" => Some(default_managed_endpoint(DeployTarget::Stage)),
        "us" => Some(default_managed_endpoint(DeployTarget::Us)),
        "eu" => Some(default_managed_endpoint(DeployTarget::Eu)),
        "ap" => Some(default_managed_endpoint(DeployTarget::Ap)),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct ProfileEndpointState {
    stored: String,
    effective: String,
    origin: &'static str,
}

fn profile_endpoint_state(profile: &DeployProfile) -> ProfileEndpointState {
    let stored = profile.endpoint.trim().to_string();

    if stored.is_empty() {
        let fallback = default_managed_endpoint_for_target_name(&profile.target)
            .unwrap_or(LEGACY_LOCAL_DEPLOY_ENDPOINT);
        return ProfileEndpointState {
            stored,
            effective: fallback.to_string(),
            origin: "target_default",
        };
    }

    if stored == LEGACY_LOCAL_DEPLOY_ENDPOINT {
        if let Some(fallback) = default_managed_endpoint_for_target_name(&profile.target) {
            return ProfileEndpointState {
                stored,
                effective: fallback.to_string(),
                origin: "target_default_from_legacy_local",
            };
        }
    }

    ProfileEndpointState {
        stored: stored.clone(),
        effective: stored,
        origin: "profile",
    }
}

fn deploy_secrets_source_name(source: DeploySecretsSource) -> &'static str {
    match source {
        DeploySecretsSource::Prompt => "prompt",
        DeploySecretsSource::Env => "env",
        DeploySecretsSource::Esm => "esm",
    }
}

fn prompt_target() -> Result<DeployTarget, String> {
    let raw = prompt_line("Deploy target (dev/stage/us/eu/ap)", Some("dev"))?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "dev" => Ok(DeployTarget::Dev),
        "stage" => Ok(DeployTarget::Stage),
        "us" => Ok(DeployTarget::Us),
        "eu" => Ok(DeployTarget::Eu),
        "ap" => Ok(DeployTarget::Ap),
        other => Err(format!(
            "invalid deploy target '{}': expected dev, stage, us, eu, or ap",
            other
        )),
    }
}

fn resolve_bootstrap_bundle(
    profile: &DeployProfile,
    explicit_bundle_path: Option<&str>,
    explicit_bundle_secret_key: Option<&str>,
) -> Result<(String, String), String> {
    if let Some(path) = explicit_bundle_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let bundle = fs::read_to_string(path)
            .map_err(|e| format!("read bootstrap bundle '{}': {e}", path))?;
        return Ok((bundle, path.to_string()));
    }

    let default_secret_key = explicit_bundle_secret_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_BOOTSTRAP_BUNDLE_SECRET_KEY);

    if profile.secrets_source == deploy_secrets_source_name(DeploySecretsSource::Esm) {
        let esm = profile.esm.as_ref().ok_or_else(|| {
            "deploy profile expects ESM but has no ESM configuration; rerun `enscrive deploy init`"
                .to_string()
        })?;

        match load_bundle_from_esm(esm, default_secret_key) {
            Ok(bundle) => {
                return Ok((
                    bundle,
                    format!(
                        "esm:{}:{}",
                        esm.workdir.as_deref().unwrap_or("."),
                        default_secret_key
                    ),
                ));
            }
            Err(esm_error) => {
                if let Some(workdir) = esm.workdir.as_deref() {
                    let fallback_path = PathBuf::from(workdir).join("bootstrap.bundle.toml");
                    if fallback_path.exists() {
                        let bundle = fs::read_to_string(&fallback_path).map_err(|e| {
                            format!(
                                "read bootstrap bundle fallback '{}': {e}",
                                fallback_path.display()
                            )
                        })?;
                        return Ok((bundle, fallback_path.display().to_string()));
                    }
                }
                return Err(format!(
                    "failed to load bootstrap bundle from ESM secret '{}': {}",
                    default_secret_key, esm_error
                ));
            }
        }
    }

    Err(
        "no bootstrap bundle available; pass --bundle-path or configure an ESM-backed deploy profile"
            .to_string(),
    )
}

fn resolve_secrets_source(
    explicit: Option<DeploySecretsSource>,
    esm: Option<&EsmDiscovery>,
) -> Result<DeploySecretsSource, String> {
    if let Some(explicit) = explicit {
        return Ok(explicit);
    }

    if !interactive_prompts_available() {
        return Ok(if esm.is_some() {
            DeploySecretsSource::Esm
        } else {
            DeploySecretsSource::Env
        });
    }

    let default = if esm.is_some() { "esm" } else { "env" };
    let raw = prompt_line("Secrets source for deploy (esm/env/prompt)", Some(default))?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "" if esm.is_some() => Ok(DeploySecretsSource::Esm),
        "" => Ok(DeploySecretsSource::Env),
        "esm" => Ok(DeploySecretsSource::Esm),
        "env" => Ok(DeploySecretsSource::Env),
        "prompt" => Ok(DeploySecretsSource::Prompt),
        other => Err(format!(
            "invalid secrets source '{}': expected esm, env, or prompt",
            other
        )),
    }
}

fn ensure_esm_master_key_if_needed() -> Result<(), String> {
    let already_present = env::var("ESM_MASTER_KEY")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if already_present {
        return Ok(());
    }

    if !interactive_prompts_available() {
        return Ok(());
    }

    let master_key = prompt_secret_line("ESM master key")?;
    if !master_key.is_empty() {
        unsafe {
            env::set_var("ESM_MASTER_KEY", master_key);
        }
    }
    Ok(())
}

fn load_bundle_from_esm(esm: &EsmProfile, secret_key: &str) -> Result<String, String> {
    ensure_esm_master_key_if_needed()?;

    let mut cmd = Command::new(&esm.binary);
    cmd.arg("--vault-path")
        .arg(&esm.vault_path)
        .arg("get")
        .arg(secret_key)
        .arg("--raw");

    if let Some(workdir) = esm.workdir.as_deref() {
        cmd.current_dir(workdir);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("run esm get {}: {e}", secret_key))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if detail.is_empty() {
            format!("esm get {} exited with {}", secret_key, output.status)
        } else {
            format!("{} (exit {})", detail, output.status)
        });
    }

    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|e| format!("bootstrap bundle is not valid UTF-8: {e}"))
        .and_then(|value| {
            if value.is_empty() {
                Err(format!(
                    "ESM secret '{}' resolved to an empty bundle",
                    secret_key
                ))
            } else {
                Ok(value)
            }
        })
}

async fn consume_signed_bootstrap(
    endpoint: &str,
    bundle_toml: &str,
) -> Result<SignedBootstrapConsumeResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("build bootstrap client: {e}"))?;
    let url = format!("{}/bootstrap/consume", endpoint.trim_end_matches('/'));
    let response = client
        .post(&url)
        .json(&SignedBootstrapConsumeRequest {
            bundle_toml: bundle_toml.to_string(),
        })
        .send()
        .await
        .map_err(|e| format!("call signed bootstrap endpoint: {e}"))?;

    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "unable to read error body".to_string());
        return Err(format!(
            "bootstrap consume failed with HTTP {}: {}",
            status, body
        ));
    }

    response
        .json::<SignedBootstrapConsumeResponse>()
        .await
        .map_err(|e| format!("parse signed bootstrap response: {e}"))
}

fn discover_esm() -> Option<EsmDiscovery> {
    let vault_workdir = env::var("ESM_VAULT_PATH").ok();

    if let Ok(binary) = env::var("ESM_BINARY") {
        let workdir = vault_workdir.clone();
        let vault_path = vault_path_for(workdir.as_deref());
        if Path::new(&binary).exists() && Path::new(&vault_path).exists() {
            return Some(EsmDiscovery {
                binary,
                workdir,
                vault_path,
            });
        }
    }

    if let Some(binary) = which_in_path("esm") {
        let workdir = vault_workdir.clone().or_else(find_vault_workdir);
        let vault_path = vault_path_for(workdir.as_deref());
        if Path::new(&vault_path).exists() {
            return Some(EsmDiscovery {
                binary: binary.display().to_string(),
                workdir,
                vault_path,
            });
        }
    }

    for candidate in ["./esm", "../esm", "/usr/local/bin/esm"] {
        if Path::new(candidate).exists() {
            let workdir = vault_workdir.clone().or_else(find_vault_workdir);
            let vault_path = vault_path_for(workdir.as_deref());
            if Path::new(&vault_path).exists() {
                return Some(EsmDiscovery {
                    binary: candidate.to_string(),
                    workdir,
                    vault_path,
                });
            }
        }
    }

    None
}

fn vault_path_for(workdir: Option<&str>) -> String {
    match workdir {
        Some(path) => PathBuf::from(path)
            .join(".esm")
            .join("secrets.esm")
            .display()
            .to_string(),
        None => PathBuf::from(".esm")
            .join("secrets.esm")
            .display()
            .to_string(),
    }
}

fn find_vault_workdir() -> Option<String> {
    let mut current = env::current_dir().ok()?;
    loop {
        let candidate = current.join(".esm").join("secrets.esm");
        if candidate.exists() {
            return Some(current.display().to_string());
        }
        if !current.pop() {
            break;
        }
    }
    None
}

fn which_in_path(binary_name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(binary_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn interactive_prompts_available() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn prompt_line(label: &str, default: Option<&str>) -> Result<String, String> {
    print!(
        "{}{}: ",
        label,
        default
            .map(|value| format!(" [{}]", value))
            .unwrap_or_default()
    );
    io::stdout()
        .flush()
        .map_err(|e| format!("flush stdout: {e}"))?;
    let mut buf = String::new();
    io::stdin()
        .read_line(&mut buf)
        .map_err(|e| format!("read input: {e}"))?;
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        Ok(default.unwrap_or_default().to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn prompt_secret_line(label: &str) -> Result<String, String> {
    print!("{label}: ");
    io::stdout()
        .flush()
        .map_err(|e| format!("flush stdout: {e}"))?;
    rpassword::read_password()
        .map(|value| value.trim().to_string())
        .map_err(|e| format!("read secret input: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use tempfile::TempDir;

    fn set_xdg(temp: &TempDir) {
        unsafe {
            env::set_var("XDG_CONFIG_HOME", temp.path().join("config"));
        }
    }

    #[tokio::test]
    async fn deploy_init_writes_profile() {
        let _guard = crate::test_support::lock_env();
        let temp = TempDir::new().unwrap();
        set_xdg(&temp);
        unsafe {
            env::remove_var("ESM_BINARY");
            env::remove_var("ESM_VAULT_PATH");
        }

        let result = init(DeployInitOptions {
            target: Some(DeployTarget::Stage),
            profile_name: Some("stage".to_string()),
            secrets_source: Some(DeploySecretsSource::Env),
            endpoint_override: None,
            set_default: true,
        })
        .await
        .unwrap();

        assert_eq!(result["profile"].as_str(), Some("stage"));
        assert_eq!(result["target"].as_str(), Some("stage"));
        assert_eq!(result["aws_region"].as_str(), Some("us-east-2"));
        assert_eq!(result["secrets_source"].as_str(), Some("env"));
        assert_eq!(
            result["endpoint"].as_str(),
            Some("https://stage.api.enscrive.io")
        );

        let profiles = load_deploy_profiles().unwrap();
        assert_eq!(profiles.default_profile.as_deref(), Some("stage"));
        assert_eq!(
            profiles
                .profiles
                .get("stage")
                .map(|profile| profile.target.as_str()),
            Some("stage")
        );
        assert_eq!(
            profiles
                .profiles
                .get("stage")
                .map(|profile| profile.endpoint.as_str()),
            Some("https://stage.api.enscrive.io")
        );
    }

    #[test]
    fn resolve_secrets_source_prefers_explicit_value() {
        let resolved = resolve_secrets_source(Some(DeploySecretsSource::Prompt), None).unwrap();
        assert_eq!(resolved, DeploySecretsSource::Prompt);
    }

    fn spawn_json_server(body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0_u8; 8192];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn deploy_bootstrap_persists_returned_authority() {
        let _guard = crate::test_support::lock_env();
        let temp = TempDir::new().unwrap();
        set_xdg(&temp);
        unsafe {
            env::remove_var("ESM_BINARY");
            env::remove_var("ESM_VAULT_PATH");
        }

        init(DeployInitOptions {
            target: Some(DeployTarget::Stage),
            profile_name: Some("stage".to_string()),
            secrets_source: Some(DeploySecretsSource::Env),
            endpoint_override: None,
            set_default: true,
        })
        .await
        .unwrap();

        let bundle_path = temp.path().join("bootstrap.bundle.toml");
        fs::write(&bundle_path, "version = 1\nnonce = \"n\"\n").unwrap();

        let endpoint = spawn_json_server(
            json!({
                "tenant_id": "tenant-1",
                "tenant_name": "Stage Tenant",
                "environment_id": "env-1",
                "environment_name": "stage",
                "user_id": "user-1",
                "platform_admin_key_id": "pak-1",
                "platform_admin_key": "ens_pa_123",
                "operator_key_id": "opk-1",
                "operator_key": "ens_op_456",
                "created_tenant": true,
                "created_environment": true,
                "created_user": true,
                "consumed_nonce": "nonce-1",
                "stack_id": "us-east-2-0"
            })
            .to_string(),
        );

        let result = bootstrap(DeployBootstrapOptions {
            profile_name: Some("stage".to_string()),
            endpoint_override: Some(endpoint),
            bundle_path: Some(bundle_path.display().to_string()),
            bundle_secret_key: None,
        })
        .await
        .unwrap();

        assert_eq!(result["tenant_id"].as_str(), Some("tenant-1"));
        assert_eq!(result["operator_api_key"].as_str(), Some("ens_op_456"));
        assert_eq!(
            result["platform_admin_api_key"].as_str(),
            Some("ens_pa_123")
        );

        let profiles = load_deploy_profiles().unwrap();
        let profile = profiles.profiles.get("stage").unwrap();
        let bootstrap = profile.bootstrap.as_ref().unwrap();
        assert_eq!(bootstrap.stack_id, "us-east-2-0");
        assert_eq!(bootstrap.operator_key_id.as_deref(), Some("opk-1"));
        assert_eq!(bootstrap.operator_api_key.as_deref(), Some("ens_op_456"));
        assert_eq!(profile.endpoint, "https://stage.api.enscrive.io");
    }

    #[test]
    fn profile_endpoint_state_upgrades_legacy_local_to_target_default() {
        let state = profile_endpoint_state(&DeployProfile {
            target: "stage".to_string(),
            aws_region: "us-east-2".to_string(),
            secrets_source: "esm".to_string(),
            endpoint: LEGACY_LOCAL_DEPLOY_ENDPOINT.to_string(),
            esm: None,
            bootstrap: None,
        });

        assert_eq!(state.effective, "https://stage.api.enscrive.io");
        assert_eq!(state.origin, "target_default_from_legacy_local");
        assert_eq!(state.stored, LEGACY_LOCAL_DEPLOY_ENDPOINT);
    }
}
