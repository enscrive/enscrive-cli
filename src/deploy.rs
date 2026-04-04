use clap::ValueEnum;
use flate2::read::GzDecoder;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tar::Archive;

const DEPLOY_PROFILE_VERSION: u32 = 2;
const LEGACY_LOCAL_DEPLOY_ENDPOINT: &str = "http://127.0.0.1:3000";
const DEFAULT_BOOTSTRAP_BUNDLE_SECRET_KEY: &str = "ENSCRIVE_BOOTSTRAP_BUNDLE";
const DEFAULT_MANAGED_DEVELOPER_PRIVATE_PORT: u16 = 13000;
const DEFAULT_MANAGED_OBSERVE_REST_PORT: u16 = 18084;
const DEFAULT_MANAGED_OBSERVE_GRPC_PORT: u16 = 19090;
const DEFAULT_MANAGED_EMBED_REST_PORT: u16 = 18081;
const DEFAULT_MANAGED_EMBED_GRPC_PORT: u16 = 15052;
const DEFAULT_MANAGED_EMBED_METRICS_PORT: u16 = 19000;

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

#[derive(Debug, Clone)]
pub struct DeployRenderOptions {
    pub profile_name: Option<String>,
    pub output_dir: Option<String>,
    pub host_root: Option<String>,
    pub bootstrap_public_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeployVerifyOptions {
    pub profile_name: Option<String>,
    pub endpoint_override: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeployApplyOptions {
    pub profile_name: Option<String>,
    pub render_dir: Option<String>,
    pub binary_dir: Option<String>,
    pub site_root: Option<String>,
    pub systemd_dir: Option<String>,
    pub nginx_dir: Option<String>,
    pub reload_systemd: bool,
    pub start_services: bool,
    pub reload_nginx: bool,
}

#[derive(Debug, Clone)]
pub struct DeployFetchOptions {
    pub profile_name: Option<String>,
    pub output_dir: Option<String>,
    pub manifest_url: Option<String>,
    pub source: Option<DeployFetchSource>,
    pub workspace_root: Option<String>,
    pub build_local: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployFetchSource {
    Manifest,
    LocalBuild,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RenderLayout {
    host_root: String,
    bin_dir: String,
    config_dir: String,
    runtime_dir: String,
    logs_dir: String,
    site_root: String,
    systemd_dir: String,
    nginx_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManagedPorts {
    developer_private_port: u16,
    observe_rest_port: u16,
    observe_grpc_port: u16,
    embed_rest_port: u16,
    embed_grpc_port: u16,
    embed_metrics_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RenderedArtifactPaths {
    manifest: String,
    readme: String,
    developer_env: String,
    observe_env: String,
    embed_env: String,
    developer_service: String,
    observe_service: String,
    embed_service: String,
    nginx_config: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RenderManifest {
    version: u32,
    rendered_at: String,
    profile: String,
    target: String,
    aws_region: String,
    endpoint: String,
    endpoint_origin: String,
    public_hostname: String,
    bootstrap_present: bool,
    secrets_source: String,
    layout: RenderLayout,
    ports: ManagedPorts,
    artifacts: RenderedArtifactPaths,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeployVerifyHealth {
    healthy: bool,
    degraded: bool,
    version: String,
    observe: Option<DeployVerifyServiceHealth>,
    embed: Option<DeployVerifyServiceHealth>,
    capabilities: DeployVerifyCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeployVerifyServiceHealth {
    healthy: bool,
    version: String,
    status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeployVerifyCapabilities {
    auth: bool,
    collections: bool,
    ingest: bool,
    search: bool,
    logs: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DeployApplySources {
    binary_dir: String,
    site_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleaseManifest {
    channel: String,
    version: String,
    install_endpoint: String,
    managed_endpoint: String,
    platforms: BTreeMap<String, ReleasePlatform>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleasePlatform {
    artifacts: Vec<ReleaseArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleaseArtifact {
    name: String,
    url: String,
    sha256: String,
    mode: String,
    size_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
struct LocalBuildArtifacts {
    workspace_root: PathBuf,
    cli_binary: PathBuf,
    developer_binary: PathBuf,
    observe_binary: PathBuf,
    embed_binary: PathBuf,
    developer_site_root: PathBuf,
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

pub async fn render(opts: DeployRenderOptions) -> Result<Value, String> {
    let profiles = load_deploy_profiles()?;
    let (selected_name, profile) = resolve_selected_profile(&profiles, opts.profile_name)?;
    let endpoint_state = profile_endpoint_state(&profile);
    let endpoint = endpoint_state.effective.clone();
    let public_hostname = public_hostname_from_endpoint(&endpoint)?;
    let (bootstrap_public_key, bootstrap_public_key_source) =
        resolve_bootstrap_public_key(opts.bootstrap_public_key.as_deref())?;
    let output_dir = resolve_render_output_dir(opts.output_dir.as_deref(), &selected_name)?;
    let host_root = opts
        .host_root
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("/opt/enscrive/{}", profile.target));
    let layout = render_layout(&host_root);
    let ports = managed_ports();

    fs::create_dir_all(output_dir.join("config"))
        .map_err(|e| format!("create render config dir: {e}"))?;
    fs::create_dir_all(output_dir.join("systemd"))
        .map_err(|e| format!("create render systemd dir: {e}"))?;
    fs::create_dir_all(output_dir.join("nginx"))
        .map_err(|e| format!("create render nginx dir: {e}"))?;

    let artifacts = render_artifact_paths(&public_hostname);
    let render_targets = resolve_render_artifact_paths(&output_dir, &public_hostname, &artifacts);

    let manifest = RenderManifest {
        version: 1,
        rendered_at: chrono::Utc::now().to_rfc3339(),
        profile: selected_name.clone(),
        target: profile.target.clone(),
        aws_region: profile.aws_region.clone(),
        endpoint: endpoint.clone(),
        endpoint_origin: endpoint_state.origin.to_string(),
        public_hostname: public_hostname.clone(),
        bootstrap_present: profile.bootstrap.is_some(),
        secrets_source: profile.secrets_source.clone(),
        layout: layout.clone(),
        ports: ports.clone(),
        artifacts: artifacts.clone(),
    };

    fs::write(
        &render_targets.manifest,
        serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("serialize deploy manifest: {e}"))?,
    )
    .map_err(|e| format!("write manifest: {e}"))?;
    fs::write(
        &render_targets.readme,
        render_readme(
            &selected_name,
            &profile,
            &endpoint,
            &public_hostname,
            &layout,
            &ports,
        ),
    )
    .map_err(|e| format!("write render README: {e}"))?;
    fs::write(
        &render_targets.developer_env,
        render_developer_env(
            &profile,
            &endpoint,
            &layout,
            &ports,
            bootstrap_public_key.as_deref(),
        ),
    )
    .map_err(|e| format!("write developer env: {e}"))?;
    fs::write(&render_targets.observe_env, render_observe_env(&ports))
        .map_err(|e| format!("write observe env: {e}"))?;
    fs::write(&render_targets.embed_env, render_embed_env(&ports))
        .map_err(|e| format!("write embed env: {e}"))?;
    fs::write(
        &render_targets.developer_service,
        render_developer_service(&layout),
    )
    .map_err(|e| format!("write developer service: {e}"))?;
    fs::write(
        &render_targets.observe_service,
        render_observe_service(&layout, &ports),
    )
    .map_err(|e| format!("write observe service: {e}"))?;
    fs::write(&render_targets.embed_service, render_embed_service(&layout))
        .map_err(|e| format!("write embed service: {e}"))?;
    fs::write(
        &render_targets.nginx_config,
        render_nginx_config(&public_hostname, &ports),
    )
    .map_err(|e| format!("write nginx config: {e}"))?;

    Ok(json!({
        "profile": selected_name,
        "target": profile.target,
        "aws_region": profile.aws_region,
        "endpoint": endpoint,
        "endpoint_origin": endpoint_state.origin,
        "public_hostname": public_hostname,
        "bootstrap_present": profile.bootstrap.is_some(),
        "bootstrap_trusted_public_key_present": bootstrap_public_key.is_some(),
        "bootstrap_trusted_public_key_source": bootstrap_public_key_source,
        "host_root": layout.host_root,
        "output_dir": output_dir.display().to_string(),
        "layout": layout,
        "ports": ports,
        "artifacts": render_targets,
        "note": "render produces deterministic managed-host artifacts for enscrive deploy apply on the target host"
    }))
}

pub async fn verify(opts: DeployVerifyOptions) -> Result<Value, String> {
    let profiles = load_deploy_profiles()?;
    let (selected_name, profile) = resolve_selected_profile(&profiles, opts.profile_name)?;
    let endpoint_state = profile_endpoint_state(&profile);
    let configured_endpoint = endpoint_state.effective.clone();
    let endpoint_override_used = opts
        .endpoint_override
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let request_endpoint = opts
        .endpoint_override
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| configured_endpoint.clone());
    let expected_endpoint = default_managed_endpoint_for_target_name(&profile.target)
        .unwrap_or(LEGACY_LOCAL_DEPLOY_ENDPOINT)
        .to_string();
    let public_hostname = public_hostname_from_endpoint(&request_endpoint)?;
    let health = fetch_health(&request_endpoint).await?;

    let service_healthy = health
        .observe
        .as_ref()
        .map(|svc| svc.healthy)
        .unwrap_or(false)
        && health
            .embed
            .as_ref()
            .map(|svc| svc.healthy)
            .unwrap_or(false);
    if !health.healthy || health.degraded || !service_healthy {
        let summary = serde_json::to_string(&health).unwrap_or_else(|_| "<unserializable>".into());
        return Err(format!(
            "managed endpoint reported unhealthy state at {}: {}",
            request_endpoint, summary
        ));
    }

    Ok(json!({
        "profile": selected_name,
        "target": profile.target,
        "aws_region": profile.aws_region,
        "endpoint": request_endpoint,
        "configured_endpoint": configured_endpoint,
        "expected_endpoint": expected_endpoint,
        "matches_target_default": request_endpoint == expected_endpoint,
        "endpoint_origin": endpoint_state.origin,
        "endpoint_override_used": endpoint_override_used,
        "public_hostname": public_hostname,
        "bootstrap_present": profile.bootstrap.is_some(),
        "secrets_source": profile.secrets_source,
        "health": health,
        "verified": true
    }))
}

pub async fn fetch(opts: DeployFetchOptions) -> Result<Value, String> {
    let profiles = load_deploy_profiles()?;
    let (selected_name, profile) = resolve_selected_profile(&profiles, opts.profile_name)?;
    let output_dir = resolve_fetch_output_dir(opts.output_dir.as_deref(), &selected_name)?;
    let source = resolve_fetch_source(&profile, opts.source, opts.manifest_url.as_deref());
    let build_local =
        opts.build_local || (opts.source.is_none() && source == DeployFetchSource::LocalBuild);

    let downloads_dir = output_dir.join("downloads");
    let bin_dir = output_dir.join("bin");
    let site_root = output_dir.join("site").join("enscrive-developer");
    fs::create_dir_all(&downloads_dir)
        .map_err(|e| format!("create downloads dir '{}': {e}", downloads_dir.display()))?;
    fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("create bin dir '{}': {e}", bin_dir.display()))?;

    match source {
        DeployFetchSource::Manifest => {
            let manifest_url = opts
                .manifest_url
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| default_release_manifest_url(&profile.target).to_string());
            let platform = detect_release_platform()?;
            let manifest = fetch_release_manifest(&manifest_url).await?;
            let platform_release = manifest.platforms.get(&platform).ok_or_else(|| {
                format!(
                    "release manifest '{}' does not define platform '{}'",
                    manifest_url, platform
                )
            })?;

            let mut downloaded = Vec::new();
            let mut extracted_site = false;
            for artifact in &platform_release.artifacts {
                let downloaded_path = downloads_dir.join(&artifact.name);
                download_release_artifact(artifact, &downloaded_path).await?;
                downloaded.push(downloaded_path.display().to_string());

                if artifact.name == "enscrive-developer-site.tar.gz" {
                    remove_dir_if_exists(&site_root)?;
                    fs::create_dir_all(&site_root)
                        .map_err(|e| format!("create site root '{}': {e}", site_root.display()))?;
                    extract_tar_gz(&downloaded_path, &site_root)?;
                    extracted_site = true;
                    continue;
                }

                let installed_path = bin_dir.join(&artifact.name);
                copy_file(&downloaded_path, &installed_path)?;
                if artifact.mode != "0644" {
                    set_executable(&installed_path)?;
                }
            }

            if !extracted_site || !site_root.join("pkg").is_dir() {
                return Err(
                    "release manifest did not yield an enscrive developer site bundle with pkg/; managed apply requires the site artifact"
                        .to_string(),
                );
            }

            Ok(json!({
                "profile": selected_name,
                "target": profile.target,
                "source": "manifest",
                "manifest_url": manifest_url,
                "channel": manifest.channel,
                "version": manifest.version,
                "platform": platform,
                "managed_endpoint_from_manifest": manifest.managed_endpoint,
                "profile_endpoint": profile_endpoint_state(&profile).effective,
                "out_dir": output_dir.display().to_string(),
                "binary_dir": bin_dir.display().to_string(),
                "site_root": site_root.display().to_string(),
                "downloaded_artifacts": downloaded,
                "note": "fetch downloaded and verified release artifacts from a hosted manifest; use the returned binary_dir and site_root with `enscrive deploy apply`"
            }))
        }
        DeployFetchSource::LocalBuild => {
            let platform = detect_release_platform()?;
            let artifacts = stage_local_build_artifacts(
                &bin_dir,
                &site_root,
                opts.workspace_root.as_deref(),
                build_local,
            )?;

            Ok(json!({
                "profile": selected_name,
                "target": profile.target,
                "source": "local_build",
                "platform": platform,
                "workspace_root": artifacts.workspace_root.display().to_string(),
                "build_executed": build_local,
                "profile_endpoint": profile_endpoint_state(&profile).effective,
                "out_dir": output_dir.display().to_string(),
                "binary_dir": bin_dir.display().to_string(),
                "site_root": site_root.display().to_string(),
                "staged_artifacts": [
                    artifacts.cli_binary.display().to_string(),
                    artifacts.developer_binary.display().to_string(),
                    artifacts.observe_binary.display().to_string(),
                    artifacts.embed_binary.display().to_string(),
                    artifacts.developer_site_root.display().to_string()
                ],
                "note": "fetch staged locally built Enscrive artifacts into the managed apply layout; use the returned binary_dir and site_root with `enscrive deploy apply`"
            }))
        }
    }
}

fn resolve_fetch_source(
    profile: &DeployProfile,
    explicit: Option<DeployFetchSource>,
    manifest_url: Option<&str>,
) -> DeployFetchSource {
    if let Some(explicit) = explicit {
        return explicit;
    }

    if manifest_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return DeployFetchSource::Manifest;
    }

    match profile.target.as_str() {
        "stage" | "us" | "eu" | "ap" => DeployFetchSource::LocalBuild,
        _ => DeployFetchSource::Manifest,
    }
}

pub async fn apply(opts: DeployApplyOptions) -> Result<Value, String> {
    let profiles = load_deploy_profiles()?;
    let (selected_name, profile) = resolve_selected_profile(&profiles, opts.profile_name)?;
    let render_dir = resolve_render_output_dir(opts.render_dir.as_deref(), &selected_name)?;
    let manifest_path = render_dir.join("manifest.json");
    let manifest = load_render_manifest(&manifest_path)?;
    let render_artifacts =
        resolve_render_artifact_paths(&render_dir, &manifest.public_hostname, &manifest.artifacts);
    let sources = resolve_apply_sources(
        &selected_name,
        opts.binary_dir.as_deref(),
        opts.site_root.as_deref(),
    )?;

    ensure_render_artifacts_exist(&render_artifacts)?;
    fs::create_dir_all(&manifest.layout.host_root)
        .map_err(|e| format!("create host root '{}': {e}", manifest.layout.host_root))?;
    fs::create_dir_all(&manifest.layout.bin_dir)
        .map_err(|e| format!("create bin dir '{}': {e}", manifest.layout.bin_dir))?;
    fs::create_dir_all(&manifest.layout.config_dir)
        .map_err(|e| format!("create config dir '{}': {e}", manifest.layout.config_dir))?;
    fs::create_dir_all(&manifest.layout.runtime_dir)
        .map_err(|e| format!("create runtime dir '{}': {e}", manifest.layout.runtime_dir))?;
    fs::create_dir_all(&manifest.layout.logs_dir)
        .map_err(|e| format!("create logs dir '{}': {e}", manifest.layout.logs_dir))?;
    fs::create_dir_all(&manifest.layout.site_root)
        .map_err(|e| format!("create site root '{}': {e}", manifest.layout.site_root))?;

    let installed_binaries =
        install_binaries(&manifest.layout.bin_dir, Path::new(&sources.binary_dir))?;
    let installed_site_root = install_site_bundle(
        Path::new(&manifest.layout.site_root),
        Path::new(&sources.site_root),
    )?;

    let installed_configs = install_named_files(&[
        (
            "developer.env",
            &render_artifacts.developer_env,
            &manifest.layout.config_dir,
        ),
        (
            "observe.env",
            &render_artifacts.observe_env,
            &manifest.layout.config_dir,
        ),
        (
            "embed.env",
            &render_artifacts.embed_env,
            &manifest.layout.config_dir,
        ),
    ])?;

    let systemd_dir = opts
        .systemd_dir
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/etc/systemd/system".to_string());
    let nginx_dir = opts
        .nginx_dir
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/etc/nginx/conf.d".to_string());

    fs::create_dir_all(&systemd_dir)
        .map_err(|e| format!("create systemd dir '{}': {e}", systemd_dir))?;
    fs::create_dir_all(&nginx_dir).map_err(|e| format!("create nginx dir '{}': {e}", nginx_dir))?;

    let installed_units = install_named_files(&[
        (
            "enscrive-developer.service",
            &render_artifacts.developer_service,
            &systemd_dir,
        ),
        (
            "enscrive-observe.service",
            &render_artifacts.observe_service,
            &systemd_dir,
        ),
        (
            "enscrive-embed.service",
            &render_artifacts.embed_service,
            &systemd_dir,
        ),
    ])?;
    let installed_nginx = install_named_files(&[(
        &format!("{}.conf", manifest.public_hostname),
        &render_artifacts.nginx_config,
        &nginx_dir,
    )])?;

    let mut actions_run = Vec::new();
    if opts.reload_systemd {
        run_host_command("systemctl daemon-reload", &["systemctl", "daemon-reload"])?;
        actions_run.push("systemctl daemon-reload".to_string());
    }
    if opts.start_services {
        run_host_command(
            "systemctl enable --now enscrive-developer enscrive-observe enscrive-embed",
            &[
                "systemctl",
                "enable",
                "--now",
                "enscrive-developer",
                "enscrive-observe",
                "enscrive-embed",
            ],
        )?;
        actions_run.push(
            "systemctl enable --now enscrive-developer enscrive-observe enscrive-embed".to_string(),
        );
    }
    if opts.reload_nginx {
        run_host_command("nginx -t", &["nginx", "-t"])?;
        run_host_command("systemctl reload nginx", &["systemctl", "reload", "nginx"])?;
        actions_run.push("nginx -t".to_string());
        actions_run.push("systemctl reload nginx".to_string());
    }

    Ok(json!({
        "profile": selected_name,
        "target": profile.target,
        "endpoint": manifest.endpoint,
        "host_root": manifest.layout.host_root,
        "render_dir": render_dir.display().to_string(),
        "sources": sources,
        "installed": {
            "binaries": installed_binaries,
            "site_root": installed_site_root,
            "config_files": installed_configs,
            "systemd_units": installed_units,
            "nginx_files": installed_nginx,
        },
        "actions_run": actions_run,
        "next_steps": [
            "enscrive deploy bootstrap --profile-name <profile>",
            "enscrive deploy verify --profile-name <profile>"
        ],
        "note": "apply stages the rendered bundle onto the local host; remote execution and SSH orchestration remain intentionally out of scope"
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

fn default_release_manifest_url(target: &str) -> &'static str {
    match target.trim().to_ascii_lowercase().as_str() {
        "dev" => "https://dev.enscrive.io/releases/dev/latest.json",
        "stage" => "https://stage.enscrive.io/releases/stage/latest.json",
        "us" | "eu" | "ap" => "https://enscrive.io/releases/prod/latest.json",
        _ => "https://enscrive.io/releases/prod/latest.json",
    }
}

fn resolve_fetch_output_dir(explicit: Option<&str>, profile_name: &str) -> Result<PathBuf, String> {
    if let Some(path) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    let cwd = env::current_dir().map_err(|e| format!("resolve current dir: {e}"))?;
    Ok(cwd.join("enscrive-artifacts").join(profile_name))
}

fn resolve_render_output_dir(
    explicit: Option<&str>,
    profile_name: &str,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    let cwd = env::current_dir().map_err(|e| format!("resolve current dir: {e}"))?;
    Ok(cwd.join("enscrive-deploy").join(profile_name))
}

fn public_hostname_from_endpoint(endpoint: &str) -> Result<String, String> {
    let parsed = Url::parse(endpoint).map_err(|e| format!("parse endpoint '{}': {e}", endpoint))?;
    parsed
        .host_str()
        .map(str::to_string)
        .ok_or_else(|| format!("endpoint '{}' has no public hostname", endpoint))
}

fn render_layout(host_root: &str) -> RenderLayout {
    let host_root = host_root.trim_end_matches('/').to_string();
    RenderLayout {
        bin_dir: format!("{host_root}/bin"),
        config_dir: format!("{host_root}/config"),
        runtime_dir: format!("{host_root}/runtime"),
        logs_dir: format!("{host_root}/logs"),
        site_root: format!("{host_root}/site/enscrive-developer"),
        systemd_dir: format!("{host_root}/systemd"),
        nginx_dir: format!("{host_root}/nginx"),
        host_root,
    }
}

fn managed_ports() -> ManagedPorts {
    ManagedPorts {
        developer_private_port: DEFAULT_MANAGED_DEVELOPER_PRIVATE_PORT,
        observe_rest_port: DEFAULT_MANAGED_OBSERVE_REST_PORT,
        observe_grpc_port: DEFAULT_MANAGED_OBSERVE_GRPC_PORT,
        embed_rest_port: DEFAULT_MANAGED_EMBED_REST_PORT,
        embed_grpc_port: DEFAULT_MANAGED_EMBED_GRPC_PORT,
        embed_metrics_port: DEFAULT_MANAGED_EMBED_METRICS_PORT,
    }
}

fn target_region_tag(target: &str) -> String {
    target.trim().to_ascii_uppercase()
}

fn render_readme(
    profile_name: &str,
    profile: &DeployProfile,
    endpoint: &str,
    public_hostname: &str,
    layout: &RenderLayout,
    ports: &ManagedPorts,
) -> String {
    format!(
        "# Enscrive Managed Deploy Render\n\n\
profile: {profile_name}\n\
target: {target}\n\
endpoint: {endpoint}\n\
public hostname: {public_hostname}\n\n\
This bundle is the deterministic output of `enscrive deploy render`.\n\
It does not perform host mutation by itself. Use it to stage a managed host\n\
before running `enscrive deploy bootstrap` and `enscrive deploy verify`.\n\n\
## Host Layout\n\n\
- host root: `{host_root}`\n\
- bin dir: `{bin_dir}`\n\
- config dir: `{config_dir}`\n\
- runtime dir: `{runtime_dir}`\n\
- logs dir: `{logs_dir}`\n\n\
## Private Ports\n\n\
- enscrive-developer: `{developer_port}`\n\
- enscrive-observe REST: `{observe_rest}`\n\
- enscrive-observe gRPC: `{observe_grpc}`\n\
- enscrive-embed REST: `{embed_rest}`\n\
- enscrive-embed gRPC: `{embed_grpc}`\n\
- enscrive-embed metrics: `{embed_metrics}`\n\n\
## Next Steps\n\n\
1. Copy the current `enscrive-*` binaries into `{bin_dir}`.\n\
2. Fill the generated env files under `{config_dir}` with real secrets and host values.\n\
3. Install the generated systemd units under `/etc/systemd/system/` and the generated nginx config under `/etc/nginx/conf.d/`.\n\
4. Reload systemd and nginx, then start the three Enscrive services.\n\
5. Run `enscrive deploy bootstrap --profile-name {profile_name}` against the managed endpoint once bootstrap is ready.\n\
6. Run `enscrive deploy verify --profile-name {profile_name}` to verify `/health` over the managed endpoint.\n\n\
## Notes\n\n\
- The canonical public endpoint remains `{endpoint}`.\n\
- The reverse proxy should terminate TLS on `443` for `{public_hostname}`.\n\
- This bundle uses placeholders rather than silently inventing secrets.\n",
        target = profile.target,
        host_root = layout.host_root,
        bin_dir = layout.bin_dir,
        config_dir = layout.config_dir,
        runtime_dir = layout.runtime_dir,
        logs_dir = layout.logs_dir,
        developer_port = ports.developer_private_port,
        observe_rest = ports.observe_rest_port,
        observe_grpc = ports.observe_grpc_port,
        embed_rest = ports.embed_rest_port,
        embed_grpc = ports.embed_grpc_port,
        embed_metrics = ports.embed_metrics_port,
    )
}

fn render_developer_env(
    profile: &DeployProfile,
    endpoint: &str,
    layout: &RenderLayout,
    ports: &ManagedPorts,
    bootstrap_public_key: Option<&str>,
) -> String {
    let bootstrap_public_key = bootstrap_public_key.unwrap_or("__OPTIONAL_BOOTSTRAP_PUBLIC_KEY__");
    format!(
        "ENSCRIVE_REGION={region}\n\
DEVELOPER_PORT={developer_port}\n\
DATABASE_URL=__REQUIRED__\n\
KEYCLOAK_ISSUER=__REQUIRED__\n\
KEYCLOAK_CLIENT_ID=enscrive-developer\n\
KEYCLOAK_CLIENT_SECRET=__REQUIRED__\n\
PORTAL_OIDC_REDIRECT_URI={endpoint}/auth/callback\n\
HMAC_PEPPER=__REQUIRED__\n\
AES_KEY=__REQUIRED__\n\
OBSERVE_GRPC_ADDR=http://127.0.0.1:{observe_grpc}\n\
EBA_TRUSTED_PUBLIC_KEY={bootstrap_public_key}\n\
LEPTOS_SITE_ROOT={site_root}\n\
LEPTOS_OUTPUT_NAME=enscrive-developer\n\
LEPTOS_SITE_PKG_DIR=pkg\n",
        region = target_region_tag(&profile.target),
        developer_port = ports.developer_private_port,
        observe_grpc = ports.observe_grpc_port,
        bootstrap_public_key = bootstrap_public_key,
        site_root = layout.site_root,
    )
}

fn render_observe_env(ports: &ManagedPorts) -> String {
    format!(
        "LOKI_URL=http://127.0.0.1:3100\n\
EMBED_URL=http://127.0.0.1:{embed_grpc}\n\
DATABASE_URL=__REQUIRED__\n\
LAB_SERVICE_SECRET=__REQUIRED__\n",
        embed_grpc = ports.embed_grpc_port,
    )
}

fn render_embed_env(ports: &ManagedPorts) -> String {
    format!(
        "SERVER_ADDR=127.0.0.1:{embed_grpc}\n\
REST_ADDR=127.0.0.1:{embed_rest}\n\
METRICS_PORT={embed_metrics}\n\
QDRANT_URL=http://127.0.0.1:6333\n\
QDRANT_GRPC_URL=http://127.0.0.1:6334\n\
LAB_SERVICE_SECRET=__REQUIRED__\n\
OPENAI_API_KEY=__OPTIONAL__\n\
NEBIUS_API_KEY=__OPTIONAL__\n\
VOYAGE_API_KEY=__OPTIONAL__\n\
BGE_ENDPOINT=__OPTIONAL__\n\
BGE_API_KEY=__OPTIONAL__\n\
BGE_MODEL_NAME=bge-large-en-v1.5\n",
        embed_grpc = ports.embed_grpc_port,
        embed_rest = ports.embed_rest_port,
        embed_metrics = ports.embed_metrics_port,
    )
}

fn resolve_bootstrap_public_key(
    explicit: Option<&str>,
) -> Result<(Option<String>, &'static str), String> {
    if let Some(value) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok((Some(value.to_string()), "explicit"));
    }

    if let Ok(value) = env::var("EBA_TRUSTED_PUBLIC_KEY") {
        let trimmed = value.trim().to_string();
        if !trimmed.is_empty() {
            return Ok((Some(trimmed), "env:EBA_TRUSTED_PUBLIC_KEY"));
        }
    }

    if let Ok(value) = env::var("EBA_SIGNING_SECRET") {
        if !value.trim().is_empty() {
            let derived = derive_bootstrap_public_key_from_eba()?;
            return Ok((Some(derived), "derived_from_eba"));
        }
    }

    Ok((None, "placeholder"))
}

fn derive_bootstrap_public_key_from_eba() -> Result<String, String> {
    let output = Command::new("eba")
        .arg("public-key")
        .output()
        .map_err(|e| format!("derive bootstrap public key with `eba public-key`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if detail.is_empty() {
            format!("`eba public-key` exited with {}", output.status)
        } else {
            format!("{} (exit {})", detail, output.status)
        });
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        return Err("`eba public-key` returned an empty public key".to_string());
    }
    Ok(value)
}

fn render_developer_service(layout: &RenderLayout) -> String {
    format!(
        "[Unit]\n\
Description=enscrive-developer\n\
After=network-online.target\n\
Wants=network-online.target\n\n\
[Service]\n\
Type=simple\n\
User=enscrive\n\
Group=enscrive\n\
WorkingDirectory={host_root}\n\
EnvironmentFile={config_dir}/developer.env\n\
ExecStart={bin_dir}/enscrive-developer\n\
Restart=always\n\
RestartSec=5\n\n\
[Install]\n\
WantedBy=multi-user.target\n",
        host_root = layout.host_root,
        config_dir = layout.config_dir,
        bin_dir = layout.bin_dir,
    )
}

fn render_observe_service(layout: &RenderLayout, ports: &ManagedPorts) -> String {
    format!(
        "[Unit]\n\
Description=enscrive-observe\n\
After=network-online.target\n\
Wants=network-online.target\n\n\
[Service]\n\
Type=simple\n\
User=enscrive\n\
Group=enscrive\n\
WorkingDirectory={host_root}\n\
EnvironmentFile={config_dir}/observe.env\n\
ExecStart={bin_dir}/enscrive-observe --port {rest_port} --grpc-port {grpc_port}\n\
Restart=always\n\
RestartSec=5\n\n\
[Install]\n\
WantedBy=multi-user.target\n",
        host_root = layout.host_root,
        config_dir = layout.config_dir,
        bin_dir = layout.bin_dir,
        rest_port = ports.observe_rest_port,
        grpc_port = ports.observe_grpc_port,
    )
}

fn render_embed_service(layout: &RenderLayout) -> String {
    format!(
        "[Unit]\n\
Description=enscrive-embed\n\
After=network-online.target\n\
Wants=network-online.target\n\n\
[Service]\n\
Type=simple\n\
User=enscrive\n\
Group=enscrive\n\
WorkingDirectory={host_root}\n\
EnvironmentFile={config_dir}/embed.env\n\
ExecStart={bin_dir}/enscrive-embed\n\
Restart=always\n\
RestartSec=5\n\n\
[Install]\n\
WantedBy=multi-user.target\n",
        host_root = layout.host_root,
        config_dir = layout.config_dir,
        bin_dir = layout.bin_dir,
    )
}

fn render_nginx_config(public_hostname: &str, ports: &ManagedPorts) -> String {
    format!(
        "server {{\n\
    listen 443 ssl http2;\n\
    server_name {public_hostname};\n\n\
    ssl_certificate /etc/letsencrypt/live/{public_hostname}/fullchain.pem;\n\
    ssl_certificate_key /etc/letsencrypt/live/{public_hostname}/privkey.pem;\n\n\
    location / {{\n\
        proxy_pass http://127.0.0.1:{developer_port};\n\
        proxy_http_version 1.1;\n\
        proxy_set_header Host $host;\n\
        proxy_set_header X-Forwarded-Proto https;\n\
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n\
        proxy_set_header X-Real-IP $remote_addr;\n\
    }}\n\
}}\n",
        developer_port = ports.developer_private_port,
    )
}

async fn fetch_health(endpoint: &str) -> Result<DeployVerifyHealth, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("build verify client: {e}"))?;
    let url = format!("{}/health", endpoint.trim_end_matches('/'));
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("call /health: {e}"))?;

    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "unable to read error body".to_string());
        return Err(format!(
            "verify health failed with HTTP {}: {}",
            status, body
        ));
    }

    response
        .json::<DeployVerifyHealth>()
        .await
        .map_err(|e| format!("parse /health response: {e}"))
}

async fn fetch_release_manifest(manifest_url: &str) -> Result<ReleaseManifest, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("build fetch client: {e}"))?;
    let response = client
        .get(manifest_url)
        .send()
        .await
        .map_err(|e| format!("download manifest '{}': {e}", manifest_url))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let body = response
        .bytes()
        .await
        .map_err(|e| format!("read manifest body '{}': {e}", manifest_url))?;

    if status != StatusCode::OK {
        return Err(format!(
            "fetch manifest failed with HTTP {}: {}",
            status,
            summarize_manifest_response(manifest_url, content_type.as_deref(), &body)
        ));
    }

    if looks_like_html(content_type.as_deref(), &body) {
        return Err(format!(
            "parse manifest '{}': expected JSON release manifest but received HTML; the artifact host may be misconfigured or the stage release channel may not be published yet. Use `--source local-build` for local staging or pass a known-good `--manifest-url`",
            manifest_url
        ));
    }

    serde_json::from_slice::<ReleaseManifest>(&body).map_err(|e| {
        format!(
            "parse manifest '{}': {}. Body summary: {}",
            manifest_url,
            e,
            summarize_manifest_response(manifest_url, content_type.as_deref(), &body)
        )
    })
}

fn detect_release_platform() -> Result<String, String> {
    let os = match env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        other => return Err(format!("unsupported operating system '{}'", other)),
    };
    let arch = match env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(format!("unsupported architecture '{}'", other)),
    };
    Ok(format!("{os}-{arch}"))
}

fn looks_like_html(content_type: Option<&str>, body: &[u8]) -> bool {
    if content_type
        .map(|value| value.to_ascii_lowercase().contains("text/html"))
        .unwrap_or(false)
    {
        return true;
    }

    let snippet = String::from_utf8_lossy(body);
    let trimmed = snippet.trim_start();
    trimmed.starts_with("<!DOCTYPE html")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<HTML")
}

fn summarize_manifest_response(
    _manifest_url: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> String {
    if looks_like_html(content_type, body) {
        return format!(
            "received HTML instead of a JSON manifest{}",
            content_type
                .map(|value| format!(" (content-type {})", value))
                .unwrap_or_default()
        );
    }

    let snippet = String::from_utf8_lossy(body);
    let mut summary = snippet.trim().replace(char::is_control, " ");
    if summary.len() > 240 {
        summary.truncate(240);
        summary.push_str("...");
    }
    if summary.is_empty() {
        "empty response body".to_string()
    } else {
        summary
    }
}

fn stage_local_build_artifacts(
    bin_dir: &Path,
    site_root: &Path,
    workspace_root_override: Option<&str>,
    build_local: bool,
) -> Result<LocalBuildArtifacts, String> {
    let mut artifacts = resolve_local_build_artifacts(workspace_root_override)?;
    if build_local {
        build_local_workspace(&artifacts.workspace_root)?;
        artifacts = resolve_local_build_artifacts(Some(
            artifacts
                .workspace_root
                .to_str()
                .ok_or_else(|| "workspace root is not valid UTF-8".to_string())?,
        ))?;
    }

    remove_dir_if_exists(bin_dir)?;
    fs::create_dir_all(bin_dir)
        .map_err(|e| format!("create local-build bin dir '{}': {e}", bin_dir.display()))?;
    stage_binary(&artifacts.cli_binary, &bin_dir.join("enscrive"))?;
    stage_binary(
        &artifacts.developer_binary,
        &bin_dir.join("enscrive-developer"),
    )?;
    stage_binary(&artifacts.observe_binary, &bin_dir.join("enscrive-observe"))?;
    stage_binary(&artifacts.embed_binary, &bin_dir.join("enscrive-embed"))?;

    remove_dir_if_exists(site_root)?;
    copy_dir_recursive(&artifacts.developer_site_root, site_root)?;
    if !site_root.join("pkg").is_dir() {
        return Err(format!(
            "staged site root '{}' is missing pkg/ after copy",
            site_root.display()
        ));
    }

    Ok(artifacts)
}

fn stage_binary(source: &Path, destination: &Path) -> Result<(), String> {
    copy_file(source, destination)?;
    set_executable(destination)
}

fn resolve_local_build_artifacts(
    workspace_root_override: Option<&str>,
) -> Result<LocalBuildArtifacts, String> {
    let workspace_root = resolve_workspace_root(workspace_root_override)?;
    let cli_binary = workspace_root
        .join("enscrive-cli")
        .join("target")
        .join("release")
        .join("enscrive");
    let developer_binary = workspace_root
        .join("enscrive-developer")
        .join("target")
        .join("release")
        .join("enscrive-developer");
    let observe_binary = workspace_root
        .join("enscrive-observe")
        .join("target")
        .join("release")
        .join("enscrive-observe");
    let embed_binary = [
        workspace_root
            .join("enscrive-embed")
            .join("embed-svc")
            .join("target")
            .join("release")
            .join("enscrive-embed"),
        workspace_root
            .join("enscrive-embed")
            .join("target")
            .join("release")
            .join("enscrive-embed"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .unwrap_or_else(|| {
        workspace_root
            .join("enscrive-embed")
            .join("embed-svc")
            .join("target")
            .join("release")
            .join("enscrive-embed")
    });
    let developer_site_root = workspace_root
        .join("enscrive-developer")
        .join("target")
        .join("site");

    for (label, path) in [
        ("enscrive CLI binary", &cli_binary),
        ("enscrive-developer binary", &developer_binary),
        ("enscrive-observe binary", &observe_binary),
        ("enscrive-embed binary", &embed_binary),
    ] {
        if !path.is_file() {
            return Err(format!(
                "{} not found at '{}'; build local artifacts first or rerun with `--build`",
                label,
                path.display()
            ));
        }
    }

    if !developer_site_root.join("pkg").is_dir() {
        return Err(format!(
            "enscrive-developer site bundle not found at '{}'; run `cargo leptos build --release` in enscrive-developer or rerun with `--build`",
            developer_site_root.display()
        ));
    }

    Ok(LocalBuildArtifacts {
        workspace_root,
        cli_binary,
        developer_binary,
        observe_binary,
        embed_binary,
        developer_site_root,
    })
}

fn resolve_workspace_root(explicit: Option<&str>) -> Result<PathBuf, String> {
    if let Some(path) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        let root = PathBuf::from(path);
        ensure_workspace_root(&root)?;
        return Ok(root);
    }

    discover_workspace_root().ok_or_else(|| {
        "unable to discover Enscrive workspace root; pass --workspace-root pointing at the directory containing enscrive-cli, enscrive-developer, enscrive-observe, and enscrive-embed".to_string()
    })
}

fn discover_workspace_root() -> Option<PathBuf> {
    let mut current = env::current_dir().ok()?;
    loop {
        if ensure_workspace_root(&current).is_ok() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

fn ensure_workspace_root(root: &Path) -> Result<(), String> {
    for child in [
        root.join("enscrive-cli"),
        root.join("enscrive-developer"),
        root.join("enscrive-observe"),
        root.join("enscrive-embed"),
    ] {
        if !child.is_dir() {
            return Err(format!(
                "workspace root '{}' is missing '{}'",
                root.display(),
                child.display()
            ));
        }
    }
    Ok(())
}

fn build_local_workspace(workspace_root: &Path) -> Result<(), String> {
    let cli_manifest = workspace_root.join("enscrive-cli").join("Cargo.toml");
    run_command_in_dir(
        workspace_root,
        "build enscrive CLI",
        "cargo",
        &[
            "build",
            "--release",
            "--manifest-path",
            cli_manifest
                .to_str()
                .ok_or_else(|| "CLI manifest path is not valid UTF-8".to_string())?,
            "--bin",
            "enscrive",
        ],
    )?;

    let observe_manifest = workspace_root.join("enscrive-observe").join("Cargo.toml");
    run_command_in_dir(
        workspace_root,
        "build enscrive-observe",
        "cargo",
        &[
            "build",
            "--release",
            "--manifest-path",
            observe_manifest
                .to_str()
                .ok_or_else(|| "observe manifest path is not valid UTF-8".to_string())?,
            "--bin",
            "enscrive-observe",
        ],
    )?;

    let embed_manifest = workspace_root
        .join("enscrive-embed")
        .join("embed-svc")
        .join("Cargo.toml");
    run_command_in_dir(
        workspace_root,
        "build enscrive-embed",
        "cargo",
        &[
            "build",
            "--release",
            "--manifest-path",
            embed_manifest
                .to_str()
                .ok_or_else(|| "embed manifest path is not valid UTF-8".to_string())?,
            "--bin",
            "enscrive-embed",
        ],
    )?;

    let developer_dir = workspace_root.join("enscrive-developer");
    run_developer_site_build(&developer_dir)?;

    Ok(())
}

fn ensure_leptos_server_binary_compat_path(developer_dir: &Path) -> Result<(), String> {
    let release_dir = developer_dir.join("target").join("release");
    let source = release_dir.join("enscrive-developer");
    let destination = release_dir.join("server");
    if !source.is_file() {
        return Err(format!(
            "enscrive-developer server binary not found at '{}'",
            source.display()
        ));
    }

    copy_file(&source, &destination)?;
    set_executable(&destination)
}

fn run_developer_site_build(developer_dir: &Path) -> Result<(), String> {
    match run_command_in_dir(
        developer_dir,
        "build enscrive-developer and site bundle",
        "cargo",
        &["leptos", "build", "--release"],
    ) {
        Ok(()) => Ok(()),
        Err(error) if error.contains("target/release/server") => {
            ensure_leptos_server_binary_compat_path(developer_dir)?;
            run_command_in_dir(
                developer_dir,
                "build enscrive-developer and site bundle",
                "cargo",
                &["leptos", "build", "--release"],
            )
        }
        Err(error) => Err(error),
    }
}

async fn download_release_artifact(
    artifact: &ReleaseArtifact,
    destination: &Path,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("build download client: {e}"))?;
    let response = client
        .get(&artifact.url)
        .send()
        .await
        .map_err(|e| format!("download artifact '{}': {e}", artifact.url))?;

    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "unable to read error body".to_string());
        return Err(format!(
            "download artifact '{}' failed with HTTP {}: {}",
            artifact.url, status, body
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("read artifact '{}': {e}", artifact.url))?;
    let actual_sha = format!("{:x}", Sha256::digest(&bytes));
    if actual_sha != artifact.sha256.to_ascii_lowercase() {
        return Err(format!(
            "sha256 mismatch for '{}': expected {}, got {}",
            artifact.name, artifact.sha256, actual_sha
        ));
    }

    let parent = destination.parent().ok_or_else(|| {
        format!(
            "destination '{}' has no parent directory",
            destination.display()
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("create destination dir '{}': {e}", parent.display()))?;
    fs::write(destination, &bytes)
        .map_err(|e| format!("write artifact '{}': {e}", destination.display()))?;
    Ok(())
}

fn extract_tar_gz(archive_path: &Path, destination: &Path) -> Result<(), String> {
    let file = fs::File::open(archive_path)
        .map_err(|e| format!("open archive '{}': {e}", archive_path.display()))?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    archive
        .unpack(destination)
        .map_err(|e| format!("extract archive '{}': {e}", archive_path.display()))
}

fn load_render_manifest(path: &Path) -> Result<RenderManifest, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("read render manifest '{}': {e}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("parse render manifest '{}': {e}", path.display()))
}

fn render_artifact_paths(public_hostname: &str) -> RenderedArtifactPaths {
    RenderedArtifactPaths {
        manifest: "manifest.json".to_string(),
        readme: "README.md".to_string(),
        developer_env: PathBuf::from("config")
            .join("developer.env")
            .display()
            .to_string(),
        observe_env: PathBuf::from("config")
            .join("observe.env")
            .display()
            .to_string(),
        embed_env: PathBuf::from("config")
            .join("embed.env")
            .display()
            .to_string(),
        developer_service: PathBuf::from("systemd")
            .join("enscrive-developer.service")
            .display()
            .to_string(),
        observe_service: PathBuf::from("systemd")
            .join("enscrive-observe.service")
            .display()
            .to_string(),
        embed_service: PathBuf::from("systemd")
            .join("enscrive-embed.service")
            .display()
            .to_string(),
        nginx_config: PathBuf::from("nginx")
            .join(format!("{public_hostname}.conf"))
            .display()
            .to_string(),
    }
}

fn resolve_render_artifact_paths(
    render_dir: &Path,
    public_hostname: &str,
    artifacts: &RenderedArtifactPaths,
) -> RenderedArtifactPaths {
    let expected = render_artifact_paths(public_hostname);

    RenderedArtifactPaths {
        manifest: resolve_render_artifact_path(render_dir, &artifacts.manifest, &expected.manifest),
        readme: resolve_render_artifact_path(render_dir, &artifacts.readme, &expected.readme),
        developer_env: resolve_render_artifact_path(
            render_dir,
            &artifacts.developer_env,
            &expected.developer_env,
        ),
        observe_env: resolve_render_artifact_path(
            render_dir,
            &artifacts.observe_env,
            &expected.observe_env,
        ),
        embed_env: resolve_render_artifact_path(render_dir, &artifacts.embed_env, &expected.embed_env),
        developer_service: resolve_render_artifact_path(
            render_dir,
            &artifacts.developer_service,
            &expected.developer_service,
        ),
        observe_service: resolve_render_artifact_path(
            render_dir,
            &artifacts.observe_service,
            &expected.observe_service,
        ),
        embed_service: resolve_render_artifact_path(
            render_dir,
            &artifacts.embed_service,
            &expected.embed_service,
        ),
        nginx_config: resolve_render_artifact_path(
            render_dir,
            &artifacts.nginx_config,
            &expected.nginx_config,
        ),
    }
}

fn resolve_render_artifact_path(render_dir: &Path, configured: &str, expected_relative: &str) -> String {
    let configured_path = Path::new(configured);
    if configured_path.is_absolute() {
        if configured_path.exists() {
            configured.to_string()
        } else {
            render_dir.join(expected_relative).display().to_string()
        }
    } else {
        render_dir.join(configured_path).display().to_string()
    }
}

fn ensure_render_artifacts_exist(artifacts: &RenderedArtifactPaths) -> Result<(), String> {
    for path in [
        artifacts.developer_env.as_str(),
        artifacts.observe_env.as_str(),
        artifacts.embed_env.as_str(),
        artifacts.developer_service.as_str(),
        artifacts.observe_service.as_str(),
        artifacts.embed_service.as_str(),
        artifacts.nginx_config.as_str(),
    ] {
        if !Path::new(path).is_file() {
            return Err(format!(
                "rendered artifact '{}' is missing; rerun `enscrive deploy render` first",
                path
            ));
        }
    }
    Ok(())
}

fn resolve_apply_sources(
    profile_name: &str,
    binary_dir_override: Option<&str>,
    site_root_override: Option<&str>,
) -> Result<DeployApplySources, String> {
    let binary_dir = if let Some(path) = binary_dir_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        PathBuf::from(path)
    } else {
        discover_fetched_binary_dir(profile_name).unwrap_or(discover_binary_dir()?)
    };
    let site_root = if let Some(path) = site_root_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        PathBuf::from(path)
    } else {
        discover_fetched_site_root(profile_name).unwrap_or(discover_site_root()?)
    };

    if !binary_dir.is_dir() {
        return Err(format!(
            "binary source dir '{}' does not exist",
            binary_dir.display()
        ));
    }
    if !site_root.join("pkg").is_dir() {
        return Err(format!(
            "site source '{}' is missing a pkg/ directory",
            site_root.display()
        ));
    }

    Ok(DeployApplySources {
        binary_dir: binary_dir.display().to_string(),
        site_root: site_root.display().to_string(),
    })
}

fn discover_fetched_binary_dir(profile_name: &str) -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?;
    let candidate = cwd
        .join("enscrive-artifacts")
        .join(profile_name)
        .join("bin");
    let required = [
        candidate.join("enscrive-developer"),
        candidate.join("enscrive-observe"),
        candidate.join("enscrive-embed"),
    ];
    if required.iter().all(|path| path.is_file()) {
        Some(candidate)
    } else {
        None
    }
}

fn discover_fetched_site_root(profile_name: &str) -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?;
    let candidate = cwd
        .join("enscrive-artifacts")
        .join(profile_name)
        .join("site")
        .join("enscrive-developer");
    if candidate.join("pkg").is_dir() {
        Some(candidate)
    } else {
        None
    }
}

fn discover_binary_dir() -> Result<PathBuf, String> {
    let current_exe = env::current_exe().map_err(|e| format!("resolve current executable: {e}"))?;
    if let Some(parent) = current_exe.parent() {
        let candidates = [
            parent.join("enscrive-developer"),
            parent.join("enscrive-observe"),
            parent.join("enscrive-embed"),
        ];
        if candidates.iter().all(|candidate| candidate.is_file()) {
            return Ok(parent.to_path_buf());
        }
    }

    let path_var = env::var_os("PATH").ok_or_else(|| "PATH is not set".to_string())?;
    for dir in env::split_paths(&path_var) {
        let candidates = [
            dir.join("enscrive-developer"),
            dir.join("enscrive-observe"),
            dir.join("enscrive-embed"),
        ];
        if candidates.iter().all(|candidate| candidate.is_file()) {
            return Ok(dir);
        }
    }

    Err(
        "unable to discover enscrive service binaries; pass --binary-dir with enscrive-developer, enscrive-observe, and enscrive-embed"
            .to_string(),
    )
}

fn discover_site_root() -> Result<PathBuf, String> {
    let home = env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let xdg_data_home = env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(home).join(".local").join("share"));
    let installed = xdg_data_home
        .join("enscrive")
        .join("site")
        .join("enscrive-developer");
    if installed.join("pkg").is_dir() {
        return Ok(installed);
    }

    let cwd = env::current_dir().map_err(|e| format!("resolve current dir: {e}"))?;
    let repo_candidate = cwd.join("enscrive-developer").join("target").join("site");
    if repo_candidate.join("pkg").is_dir() {
        return Ok(repo_candidate);
    }

    Err(
        "unable to discover the enscrive developer site bundle; pass --site-root with a directory containing pkg/"
            .to_string(),
    )
}

fn install_binaries(destination_dir: &str, source_dir: &Path) -> Result<Vec<String>, String> {
    let mut installed = Vec::new();
    for binary_name in ["enscrive-developer", "enscrive-observe", "enscrive-embed"] {
        let source = source_dir.join(binary_name);
        if !source.is_file() {
            return Err(format!(
                "required binary '{}' is missing from '{}'",
                binary_name,
                source_dir.display()
            ));
        }
        let destination = Path::new(destination_dir).join(binary_name);
        copy_file(&source, &destination)?;
        set_executable(&destination)?;
        installed.push(destination.display().to_string());
    }
    Ok(installed)
}

fn install_site_bundle(destination_root: &Path, source_root: &Path) -> Result<String, String> {
    remove_dir_if_exists(destination_root)?;
    copy_dir_recursive(source_root, destination_root)?;
    Ok(destination_root.display().to_string())
}

fn install_named_files(files: &[(&str, &str, &str)]) -> Result<Vec<String>, String> {
    let mut installed = Vec::new();
    for (name, source, destination_dir) in files {
        let destination = Path::new(destination_dir).join(name);
        copy_file(Path::new(source), &destination)?;
        installed.push(destination.display().to_string());
    }
    Ok(installed)
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), String> {
    let parent = destination.parent().ok_or_else(|| {
        format!(
            "destination '{}' has no parent directory",
            destination.display()
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("create destination dir '{}': {e}", parent.display()))?;
    fs::copy(source, destination).map_err(|e| {
        format!(
            "copy '{}' to '{}': {e}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|e| format!("create directory '{}': {e}", destination.display()))?;
    for entry in
        fs::read_dir(source).map_err(|e| format!("read directory '{}': {e}", source.display()))?
    {
        let entry =
            entry.map_err(|e| format!("read directory entry '{}': {e}", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|e| format!("read file type '{}': {e}", source_path.display()))?
            .is_dir()
        {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else {
            copy_file(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|e| format!("remove directory '{}': {e}", path.display()))?;
    }
    Ok(())
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        let metadata = fs::metadata(path).map_err(|e| format!("stat '{}': {e}", path.display()))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .map_err(|e| format!("set permissions '{}': {e}", path.display()))?;
    }
    Ok(())
}

fn run_host_command(label: &str, args: &[&str]) -> Result<(), String> {
    let (program, rest) = args
        .split_first()
        .ok_or_else(|| format!("invalid host command '{}'", label))?;
    let output = Command::new(program)
        .args(rest)
        .output()
        .map_err(|e| format!("run '{}': {e}", label))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if detail.is_empty() {
            format!("'{}' exited with {}", label, output.status)
        } else {
            format!("{} (exit {})", detail, output.status)
        });
    }
    Ok(())
}

fn run_command_in_dir(
    working_dir: &Path,
    label: &str,
    program: &str,
    args: &[&str],
) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(working_dir)
        .output()
        .map_err(|e| format!("run '{}': {e}", label))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if detail.is_empty() {
            format!("'{}' exited with {}", label, output.status)
        } else {
            format!("{} (exit {})", detail, output.status)
        });
    }
    Ok(())
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
    use std::collections::HashMap;
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

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn spawn_static_server(responses: HashMap<String, (String, Vec<u8>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..16 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buffer = [0_u8; 8192];
                let Ok(size) = stream.read(&mut buffer) else {
                    continue;
                };
                let request = String::from_utf8_lossy(&buffer[..size]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (status_line, content_type, body) =
                    if let Some((content_type, body)) = responses.get(path) {
                        ("HTTP/1.1 200 OK", content_type.as_str(), body.clone())
                    } else {
                        ("HTTP/1.1 404 Not Found", "text/plain", b"missing".to_vec())
                    };
                let header = format!(
                    "{status_line}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn deploy_render_writes_stage_bundle() {
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

        let output_dir = temp.path().join("rendered");
        let result = render(DeployRenderOptions {
            profile_name: Some("stage".to_string()),
            output_dir: Some(output_dir.display().to_string()),
            host_root: None,
            bootstrap_public_key: Some("test-bootstrap-public-key".to_string()),
        })
        .await
        .unwrap();

        assert_eq!(
            result["public_hostname"].as_str(),
            Some("stage.api.enscrive.io")
        );
        assert!(output_dir.join("manifest.json").exists());
        assert!(output_dir.join("config").join("developer.env").exists());
        assert!(
            output_dir
                .join("systemd")
                .join("enscrive-developer.service")
                .exists()
        );

        let developer_env =
            fs::read_to_string(output_dir.join("config").join("developer.env")).unwrap();
        assert!(
            developer_env
                .contains("PORTAL_OIDC_REDIRECT_URI=https://stage.api.enscrive.io/auth/callback")
        );
        assert!(developer_env.contains("DEVELOPER_PORT=13000"));
        assert!(developer_env.contains("EBA_TRUSTED_PUBLIC_KEY=test-bootstrap-public-key"));
        assert_eq!(
            result["bootstrap_trusted_public_key_present"].as_bool(),
            Some(true)
        );
        assert_eq!(
            result["bootstrap_trusted_public_key_source"].as_str(),
            Some("explicit")
        );
    }

    #[tokio::test]
    async fn deploy_verify_checks_health() {
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

        let endpoint = spawn_json_server(
            json!({
                "healthy": true,
                "degraded": false,
                "version": "test",
                "observe": {"healthy": true, "version": "test", "status": "ok"},
                "embed": {"healthy": true, "version": "test", "status": "ok"},
                "capabilities": {
                    "auth": true,
                    "collections": true,
                    "ingest": true,
                    "search": true,
                    "logs": true
                }
            })
            .to_string(),
        );

        let result = verify(DeployVerifyOptions {
            profile_name: Some("stage".to_string()),
            endpoint_override: Some(endpoint),
        })
        .await
        .unwrap();

        assert_eq!(result["verified"].as_bool(), Some(true));
        assert_eq!(result["health"]["healthy"].as_bool(), Some(true));
        assert_eq!(result["matches_target_default"].as_bool(), Some(false));
    }

    #[tokio::test]
    async fn deploy_apply_installs_rendered_bundle() {
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

        let host_root = temp.path().join("host");
        let render_dir = temp.path().join("rendered");
        render(DeployRenderOptions {
            profile_name: Some("stage".to_string()),
            output_dir: Some(render_dir.display().to_string()),
            host_root: Some(host_root.display().to_string()),
            bootstrap_public_key: None,
        })
        .await
        .unwrap();

        let binary_dir = temp.path().join("bin-src");
        fs::create_dir_all(&binary_dir).unwrap();
        for binary_name in ["enscrive-developer", "enscrive-observe", "enscrive-embed"] {
            fs::write(binary_dir.join(binary_name), format!("{binary_name}-bytes")).unwrap();
        }

        let site_root = temp.path().join("site-src");
        fs::create_dir_all(site_root.join("pkg")).unwrap();
        fs::write(
            site_root.join("pkg").join("enscrive-developer.css"),
            "body{}",
        )
        .unwrap();

        let systemd_dir = temp.path().join("systemd");
        let nginx_dir = temp.path().join("nginx");

        let result = apply(DeployApplyOptions {
            profile_name: Some("stage".to_string()),
            render_dir: Some(render_dir.display().to_string()),
            binary_dir: Some(binary_dir.display().to_string()),
            site_root: Some(site_root.display().to_string()),
            systemd_dir: Some(systemd_dir.display().to_string()),
            nginx_dir: Some(nginx_dir.display().to_string()),
            reload_systemd: false,
            start_services: false,
            reload_nginx: false,
        })
        .await
        .unwrap();

        assert_eq!(result["profile"].as_str(), Some("stage"));
        assert!(host_root.join("bin").join("enscrive-developer").exists());
        assert!(
            host_root
                .join("site")
                .join("enscrive-developer")
                .join("pkg")
                .join("enscrive-developer.css")
                .exists()
        );
        assert!(systemd_dir.join("enscrive-developer.service").exists());
        assert!(nginx_dir.join("stage.api.enscrive.io.conf").exists());
    }

    #[tokio::test]
    async fn deploy_apply_accepts_portable_copy_of_absolute_manifest() {
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

        let host_root = temp.path().join("host");
        let operator_render_dir = temp.path().join("operator-rendered");
        render(DeployRenderOptions {
            profile_name: Some("stage".to_string()),
            output_dir: Some(operator_render_dir.display().to_string()),
            host_root: Some(host_root.display().to_string()),
            bootstrap_public_key: None,
        })
        .await
        .unwrap();

        let host_render_dir = temp.path().join("host-rendered");
        copy_dir_recursive(&operator_render_dir, &host_render_dir).unwrap();

        let mut manifest = load_render_manifest(&host_render_dir.join("manifest.json")).unwrap();
        manifest.artifacts = RenderedArtifactPaths {
            manifest: operator_render_dir.join("manifest.json").display().to_string(),
            readme: operator_render_dir.join("README.md").display().to_string(),
            developer_env: operator_render_dir
                .join("config")
                .join("missing-developer.env")
                .display()
                .to_string(),
            observe_env: operator_render_dir
                .join("config")
                .join("missing-observe.env")
                .display()
                .to_string(),
            embed_env: operator_render_dir
                .join("config")
                .join("missing-embed.env")
                .display()
                .to_string(),
            developer_service: operator_render_dir
                .join("systemd")
                .join("missing-enscrive-developer.service")
                .display()
                .to_string(),
            observe_service: operator_render_dir
                .join("systemd")
                .join("missing-enscrive-observe.service")
                .display()
                .to_string(),
            embed_service: operator_render_dir
                .join("systemd")
                .join("missing-enscrive-embed.service")
                .display()
                .to_string(),
            nginx_config: operator_render_dir
                .join("nginx")
                .join("missing-stage.api.enscrive.io.conf")
                .display()
                .to_string(),
        };
        fs::write(
            host_render_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let binary_dir = temp.path().join("bin-src");
        fs::create_dir_all(&binary_dir).unwrap();
        for binary_name in ["enscrive-developer", "enscrive-observe", "enscrive-embed"] {
            fs::write(binary_dir.join(binary_name), format!("{binary_name}-bytes")).unwrap();
        }

        let site_root = temp.path().join("site-src");
        fs::create_dir_all(site_root.join("pkg")).unwrap();
        fs::write(
            site_root.join("pkg").join("enscrive-developer.css"),
            "body{}",
        )
        .unwrap();

        let systemd_dir = temp.path().join("systemd");
        let nginx_dir = temp.path().join("nginx");

        let result = apply(DeployApplyOptions {
            profile_name: Some("stage".to_string()),
            render_dir: Some(host_render_dir.display().to_string()),
            binary_dir: Some(binary_dir.display().to_string()),
            site_root: Some(site_root.display().to_string()),
            systemd_dir: Some(systemd_dir.display().to_string()),
            nginx_dir: Some(nginx_dir.display().to_string()),
            reload_systemd: false,
            start_services: false,
            reload_nginx: false,
        })
        .await
        .unwrap();

        assert_eq!(result["profile"].as_str(), Some("stage"));
        assert!(systemd_dir.join("enscrive-developer.service").exists());
        assert!(nginx_dir.join("stage.api.enscrive.io.conf").exists());
    }

    #[tokio::test]
    async fn deploy_fetch_downloads_and_extracts_release_artifacts() {
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

        let cli_bytes = b"cli".to_vec();
        let developer_bytes = b"developer".to_vec();
        let observe_bytes = b"observe".to_vec();
        let embed_bytes = b"embed".to_vec();

        let site_dir = temp.path().join("site-src");
        fs::create_dir_all(site_dir.join("pkg")).unwrap();
        fs::write(
            site_dir.join("pkg").join("enscrive-developer.css"),
            "body{}",
        )
        .unwrap();
        let site_archive = temp.path().join("enscrive-developer-site.tar.gz");
        {
            let tar_gz = fs::File::create(&site_archive).unwrap();
            let encoder = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            archive
                .append_dir_all(".", &site_dir)
                .expect("append site dir");
            archive.into_inner().unwrap().finish().unwrap();
        }
        let site_bytes = fs::read(&site_archive).unwrap();

        let mut responses: HashMap<String, (String, Vec<u8>)> = HashMap::new();
        responses.insert(
            "/releases/stage/0.0.0-stage/linux-x86_64/enscrive".into(),
            ("application/octet-stream".into(), cli_bytes.clone()),
        );
        responses.insert(
            "/releases/stage/0.0.0-stage/linux-x86_64/enscrive-developer".into(),
            ("application/octet-stream".into(), developer_bytes.clone()),
        );
        responses.insert(
            "/releases/stage/0.0.0-stage/linux-x86_64/enscrive-observe".into(),
            ("application/octet-stream".into(), observe_bytes.clone()),
        );
        responses.insert(
            "/releases/stage/0.0.0-stage/linux-x86_64/enscrive-embed".into(),
            ("application/octet-stream".into(), embed_bytes.clone()),
        );
        responses.insert(
            "/releases/stage/0.0.0-stage/linux-x86_64/enscrive-developer-site.tar.gz".into(),
            ("application/gzip".into(), site_bytes.clone()),
        );

        let server = spawn_static_server(responses.clone());
        let manifest = json!({
            "channel": "stage",
            "version": "0.0.0-stage",
            "install_endpoint": format!("{server}/install"),
            "managed_endpoint": "https://stage.api.enscrive.io",
            "platforms": {
                "linux-x86_64": {
                    "artifacts": [
                        {
                            "name": "enscrive",
                            "url": format!("{server}/releases/stage/0.0.0-stage/linux-x86_64/enscrive"),
                            "sha256": sha256_hex(&cli_bytes),
                            "mode": "0755",
                            "size_bytes": cli_bytes.len()
                        },
                        {
                            "name": "enscrive-developer",
                            "url": format!("{server}/releases/stage/0.0.0-stage/linux-x86_64/enscrive-developer"),
                            "sha256": sha256_hex(&developer_bytes),
                            "mode": "0755",
                            "size_bytes": developer_bytes.len()
                        },
                        {
                            "name": "enscrive-observe",
                            "url": format!("{server}/releases/stage/0.0.0-stage/linux-x86_64/enscrive-observe"),
                            "sha256": sha256_hex(&observe_bytes),
                            "mode": "0755",
                            "size_bytes": observe_bytes.len()
                        },
                        {
                            "name": "enscrive-embed",
                            "url": format!("{server}/releases/stage/0.0.0-stage/linux-x86_64/enscrive-embed"),
                            "sha256": sha256_hex(&embed_bytes),
                            "mode": "0755",
                            "size_bytes": embed_bytes.len()
                        },
                        {
                            "name": "enscrive-developer-site.tar.gz",
                            "url": format!("{server}/releases/stage/0.0.0-stage/linux-x86_64/enscrive-developer-site.tar.gz"),
                            "sha256": sha256_hex(&site_bytes),
                            "mode": "0644",
                            "size_bytes": site_bytes.len()
                        }
                    ]
                }
            }
        })
        .to_string();
        let manifest_server = spawn_static_server(HashMap::from([(
            "/latest.json".into(),
            ("application/json".into(), manifest.into_bytes()),
        )]));

        let output_dir = temp.path().join("fetched");
        let result = fetch(DeployFetchOptions {
            profile_name: Some("stage".to_string()),
            output_dir: Some(output_dir.display().to_string()),
            manifest_url: Some(format!("{manifest_server}/latest.json")),
            source: Some(DeployFetchSource::Manifest),
            workspace_root: None,
            build_local: false,
        })
        .await
        .unwrap();

        assert_eq!(result["channel"].as_str(), Some("stage"));
        assert!(output_dir.join("bin").join("enscrive-developer").exists());
        assert!(
            output_dir
                .join("site")
                .join("enscrive-developer")
                .join("pkg")
                .join("enscrive-developer.css")
                .exists()
        );
    }

    #[tokio::test]
    async fn deploy_fetch_stages_local_build_workspace_artifacts() {
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

        let workspace_root = temp.path().join("workspace");
        fs::create_dir_all(
            workspace_root
                .join("enscrive-cli")
                .join("target")
                .join("release"),
        )
        .unwrap();
        fs::create_dir_all(
            workspace_root
                .join("enscrive-developer")
                .join("target")
                .join("release"),
        )
        .unwrap();
        fs::create_dir_all(
            workspace_root
                .join("enscrive-developer")
                .join("target")
                .join("site")
                .join("pkg"),
        )
        .unwrap();
        fs::create_dir_all(
            workspace_root
                .join("enscrive-observe")
                .join("target")
                .join("release"),
        )
        .unwrap();
        fs::create_dir_all(
            workspace_root
                .join("enscrive-embed")
                .join("embed-svc")
                .join("target")
                .join("release"),
        )
        .unwrap();

        fs::write(
            workspace_root
                .join("enscrive-cli")
                .join("target")
                .join("release")
                .join("enscrive"),
            "cli",
        )
        .unwrap();
        fs::write(
            workspace_root
                .join("enscrive-developer")
                .join("target")
                .join("release")
                .join("enscrive-developer"),
            "developer",
        )
        .unwrap();
        fs::write(
            workspace_root
                .join("enscrive-developer")
                .join("target")
                .join("site")
                .join("pkg")
                .join("enscrive-developer.css"),
            "body{}",
        )
        .unwrap();
        fs::write(
            workspace_root
                .join("enscrive-observe")
                .join("target")
                .join("release")
                .join("enscrive-observe"),
            "observe",
        )
        .unwrap();
        fs::write(
            workspace_root
                .join("enscrive-embed")
                .join("embed-svc")
                .join("target")
                .join("release")
                .join("enscrive-embed"),
            "embed",
        )
        .unwrap();

        let output_dir = temp.path().join("local-build-staged");
        let result = fetch(DeployFetchOptions {
            profile_name: Some("stage".to_string()),
            output_dir: Some(output_dir.display().to_string()),
            manifest_url: None,
            source: Some(DeployFetchSource::LocalBuild),
            workspace_root: Some(workspace_root.display().to_string()),
            build_local: false,
        })
        .await
        .unwrap();

        let workspace_root_str = workspace_root.display().to_string();
        assert_eq!(result["source"].as_str(), Some("local_build"));
        assert_eq!(
            result["workspace_root"].as_str(),
            Some(workspace_root_str.as_str())
        );
        assert!(output_dir.join("bin").join("enscrive").exists());
        assert!(output_dir.join("bin").join("enscrive-developer").exists());
        assert!(output_dir.join("bin").join("enscrive-observe").exists());
        assert!(output_dir.join("bin").join("enscrive-embed").exists());
        assert!(
            output_dir
                .join("site")
                .join("enscrive-developer")
                .join("pkg")
                .join("enscrive-developer.css")
                .exists()
        );
    }

    #[tokio::test]
    async fn deploy_fetch_reports_html_manifest_misconfiguration_cleanly() {
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

        let manifest_server = spawn_static_server(HashMap::from([(
            "/latest.json".into(),
            (
                "text/html".into(),
                b"<!DOCTYPE html><html>wrong</html>".to_vec(),
            ),
        )]));

        let error = fetch(DeployFetchOptions {
            profile_name: Some("stage".to_string()),
            output_dir: Some(temp.path().join("bad-fetch").display().to_string()),
            manifest_url: Some(format!("{manifest_server}/latest.json")),
            source: Some(DeployFetchSource::Manifest),
            workspace_root: None,
            build_local: false,
        })
        .await
        .unwrap_err();

        assert!(error.contains("expected JSON release manifest but received HTML"));
        assert!(error.contains("--source local-build"));
    }

    #[test]
    fn ensure_leptos_server_binary_compat_path_materializes_server_alias() {
        let temp = TempDir::new().unwrap();
        let developer_dir = temp.path().join("enscrive-developer");
        let release_dir = developer_dir.join("target").join("release");
        fs::create_dir_all(&release_dir).unwrap();
        let source = release_dir.join("enscrive-developer");
        fs::write(&source, b"developer-binary").unwrap();

        ensure_leptos_server_binary_compat_path(&developer_dir).unwrap();

        let destination = release_dir.join("server");
        assert!(destination.is_file());
        assert_eq!(fs::read(destination).unwrap(), b"developer-binary");
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
