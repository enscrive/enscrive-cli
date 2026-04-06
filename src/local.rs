use rand::{Rng, distributions::Alphanumeric};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, IsTerminal, Write};
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::ValueEnum;

const DEFAULT_LOCAL_PROFILE: &str = "local";
const DEFAULT_MANAGED_PROFILE: &str = "managed";
const DEFAULT_LEPTOS_OUTPUT_NAME: &str = "enscrive-developer";
const INSTALLED_DEVELOPER_SITE_SUBDIR: &str = "site/enscrive-developer";
const PROFILE_VERSION: u32 = 1;
const FEDORA_DOCKER_HINT: &str = "On Fedora: `sudo dnf install -y moby-engine docker-compose && sudo systemctl enable --now docker && sudo usermod -aG docker $USER`, then re-login or run `newgrp docker` before retrying.";
const LOCAL_POSTGRES_DATABASES: [&str; 4] = [
    "enscrive_developer",
    "enscrive_keycloak",
    "enscrive_observe",
    "enscrive_embed_backup",
];

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InitMode {
    Managed,
    SelfManaged,
}

#[derive(Debug, Clone)]
pub struct ManagedInitOptions {
    pub profile_name: Option<String>,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub set_default: bool,
}

#[derive(Debug, Clone)]
pub struct SelfManagedInitOptions {
    pub profile_name: Option<String>,
    pub with_grafana: bool,
    pub developer_port: Option<u16>,
    pub developer_bin: Option<String>,
    pub observe_bin: Option<String>,
    pub embed_bin: Option<String>,
    pub openai_api_key: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub voyage_api_key: Option<String>,
    pub nebius_api_key: Option<String>,
    pub bge_endpoint: Option<String>,
    pub bge_api_key: Option<String>,
    pub bge_model_name: Option<String>,
    pub set_default: bool,
}

#[derive(Debug, Clone)]
pub struct StartOptions {
    pub profile_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StopOptions {
    pub profile_name: Option<String>,
    pub remove_infra: bool,
}

#[derive(Debug, Clone)]
pub struct StatusOptions {
    pub profile_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedApiContext {
    pub endpoint: String,
    pub api_key: Option<String>,
    pub profile_name: Option<String>,
    pub profile_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct ProfilesFile {
    version: u32,
    default_profile: Option<String>,
    profiles: BTreeMap<String, StoredProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredProfile {
    mode: String,
    endpoint: String,
    api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local: Option<LocalProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalProfile {
    deployment_mode: String,
    runtime_dir: String,
    config_dir: String,
    compose_file: String,
    infra_env_file: String,
    developer_env_file: String,
    observe_env_file: String,
    embed_env_file: String,
    log_dir: String,
    docker_project: String,
    binaries: LocalBinaries,
    ports: LocalPorts,
    features: LocalFeatures,
    keycloak: LocalKeycloak,
    #[serde(default)]
    bootstrap: LocalBootstrap,
    providers: LocalProviders,
}

#[derive(Debug, Clone, Copy)]
enum ComposeBinary {
    DockerPlugin,
    DockerCompose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalBinaries {
    developer: String,
    observe: String,
    embed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalPorts {
    developer: u16,
    observe_rest: u16,
    observe_grpc: u16,
    embed_rest: u16,
    embed_grpc: u16,
    embed_metrics: u16,
    postgres: u16,
    qdrant_http: u16,
    qdrant_grpc: u16,
    keycloak: u16,
    loki: u16,
    grafana: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalFeatures {
    with_grafana: bool,
    local_bge_management: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalKeycloak {
    realm: String,
    client_id: String,
    client_secret: String,
    admin_username: String,
    admin_password: String,
    developer_username: String,
    developer_password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LocalBootstrap {
    secret: String,
    developer_email: String,
    developer_name: String,
    tenant_name: String,
    environment_name: String,
    api_key_label: String,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    environment_id: Option<String>,
    #[serde(default)]
    api_key_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
struct LocalProviders {
    credentials: LocalProviderCredentials,
    embedding: LocalEmbeddingProviders,
    llm_inference: LocalLlmInferenceProviders,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LocalProviderCredentials {
    #[serde(skip_serializing_if = "Option::is_none")]
    openai_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    anthropic_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    voyage_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nebius_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bge_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bge_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bge_model_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LocalEmbeddingProviders {
    #[serde(default)]
    openai: bool,
    #[serde(default)]
    voyage: bool,
    #[serde(default)]
    nebius: bool,
    #[serde(default)]
    bge: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LocalLlmInferenceProviders {
    #[serde(default)]
    openai: bool,
    #[serde(default)]
    anthropic: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct LocalProvidersSerde {
    #[serde(skip_serializing_if = "Option::is_none")]
    credentials: Option<LocalProviderCredentials>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embedding: Option<LocalEmbeddingProviders>,
    #[serde(skip_serializing_if = "Option::is_none")]
    llm_inference: Option<LocalLlmInferenceProviders>,
    #[serde(skip_serializing_if = "Option::is_none")]
    openai_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    anthropic_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    voyage_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nebius_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bge_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bge_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bge_model_name: Option<String>,
}

impl<'de> Deserialize<'de> for LocalProviders {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = LocalProvidersSerde::deserialize(deserializer)?;
        Ok(raw.into())
    }
}

impl From<LocalProvidersSerde> for LocalProviders {
    fn from(raw: LocalProvidersSerde) -> Self {
        let legacy_openai = raw.openai_api_key.is_some();
        let legacy_anthropic = raw.anthropic_api_key.is_some();
        let legacy_voyage = raw.voyage_api_key.is_some();
        let legacy_nebius = raw.nebius_api_key.is_some();
        let legacy_bge = raw.bge_endpoint.is_some();

        let mut credentials = raw.credentials.unwrap_or_default();
        credentials.openai_api_key = credentials
            .openai_api_key
            .or_else(|| normalize_optional(raw.openai_api_key));
        credentials.anthropic_api_key = credentials
            .anthropic_api_key
            .or_else(|| normalize_optional(raw.anthropic_api_key));
        credentials.voyage_api_key = credentials
            .voyage_api_key
            .or_else(|| normalize_optional(raw.voyage_api_key));
        credentials.nebius_api_key = credentials
            .nebius_api_key
            .or_else(|| normalize_optional(raw.nebius_api_key));
        credentials.bge_endpoint = credentials
            .bge_endpoint
            .or_else(|| normalize_optional(raw.bge_endpoint));
        credentials.bge_api_key = credentials
            .bge_api_key
            .or_else(|| normalize_optional(raw.bge_api_key));
        credentials.bge_model_name = credentials
            .bge_model_name
            .or_else(|| normalize_optional(raw.bge_model_name));

        let mut embedding = raw.embedding.unwrap_or_default();
        let mut llm_inference = raw.llm_inference.unwrap_or_default();

        if credentials.openai_api_key.is_some() {
            embedding.openai = embedding.openai || legacy_openai;
            llm_inference.openai = llm_inference.openai || legacy_openai;
        }
        if credentials.anthropic_api_key.is_some() {
            llm_inference.anthropic = llm_inference.anthropic || legacy_anthropic;
        }
        if credentials.voyage_api_key.is_some() {
            embedding.voyage = embedding.voyage || legacy_voyage;
        }
        if credentials.nebius_api_key.is_some() {
            embedding.nebius = embedding.nebius || legacy_nebius;
        }
        if credentials.bge_endpoint.is_some() {
            embedding.bge = embedding.bge || legacy_bge;
        }

        Self {
            credentials,
            embedding,
            llm_inference,
        }
    }
}

impl LocalEmbeddingProviders {
    fn any_enabled(&self) -> bool {
        self.openai || self.voyage || self.nebius || self.bge
    }
}

impl LocalLlmInferenceProviders {
    fn any_enabled(&self) -> bool {
        self.openai || self.anthropic
    }
}

impl LocalProviders {
    fn embedding_openai_api_key(&self) -> Option<String> {
        if self.embedding.openai {
            self.credentials.openai_api_key.clone()
        } else {
            None
        }
    }

    fn embedding_voyage_api_key(&self) -> Option<String> {
        if self.embedding.voyage {
            self.credentials.voyage_api_key.clone()
        } else {
            None
        }
    }

    fn embedding_nebius_api_key(&self) -> Option<String> {
        if self.embedding.nebius {
            self.credentials.nebius_api_key.clone()
        } else {
            None
        }
    }

    fn embedding_bge_endpoint(&self) -> Option<String> {
        if self.embedding.bge {
            self.credentials.bge_endpoint.clone()
        } else {
            None
        }
    }

    fn embedding_bge_api_key(&self) -> Option<String> {
        if self.embedding.bge {
            self.credentials.bge_api_key.clone()
        } else {
            None
        }
    }

    fn embedding_bge_model_name(&self) -> Option<String> {
        if self.embedding.bge {
            self.credentials.bge_model_name.clone()
        } else {
            None
        }
    }

    fn llm_inference_openai_api_key(&self) -> Option<String> {
        if self.llm_inference.openai {
            self.credentials.openai_api_key.clone()
        } else {
            None
        }
    }

    fn llm_inference_anthropic_api_key(&self) -> Option<String> {
        if self.llm_inference.anthropic {
            self.credentials.anthropic_api_key.clone()
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
struct CliHome {
    config_root: PathBuf,
    data_root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct LocalKeycloakUser {
    subject: String,
    email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalBootstrapRequest {
    secret: String,
    developer_subject: String,
    developer_email: String,
    developer_name: String,
    tenant_name: String,
    environment_name: String,
    api_key_label: String,
    issue_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalBootstrapResponse {
    user_id: String,
    tenant_id: String,
    tenant_name: String,
    environment_id: String,
    environment_name: String,
    api_key_id: Option<String>,
    api_key: Option<String>,
    created_tenant: bool,
    created_environment: bool,
    issued_api_key: bool,
}

pub fn resolve_api_context(
    profile_name: Option<&str>,
    endpoint_override: Option<String>,
    api_key_override: Option<String>,
) -> Result<ResolvedApiContext, String> {
    let profiles = load_profiles()?;
    let selected_name = selected_profile_name(profile_name, &profiles);
    let selected_profile = selected_name
        .as_ref()
        .and_then(|name| profiles.profiles.get(name).cloned());

    let endpoint = endpoint_override
        .or_else(|| {
            selected_profile
                .as_ref()
                .map(|profile| profile.endpoint.clone())
        })
        .unwrap_or_else(|| "http://localhost:3000".to_string());
    let api_key =
        api_key_override.or_else(|| selected_profile.as_ref().and_then(|p| p.api_key.clone()));

    Ok(ResolvedApiContext {
        endpoint,
        api_key,
        profile_name: selected_name,
        profile_mode: selected_profile.map(|profile| profile.mode),
    })
}

pub async fn init_managed(opts: ManagedInitOptions) -> Result<Value, String> {
    let mut profiles = load_profiles()?;
    let profile_name = opts
        .profile_name
        .unwrap_or_else(|| DEFAULT_MANAGED_PROFILE.to_string());

    let endpoint = match opts.endpoint {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => prompt_line("Managed endpoint", Some("https://api.enscrive.io"))?,
    };
    let api_key = match opts.api_key {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => prompt_line("Managed API key", None)?,
    };

    if api_key.trim().is_empty() {
        return Err("managed init requires a non-empty API key".to_string());
    }

    profiles.version = PROFILE_VERSION;
    profiles.profiles.insert(
        profile_name.clone(),
        StoredProfile {
            mode: "managed".to_string(),
            endpoint: endpoint.clone(),
            api_key: Some(api_key),
            local: None,
        },
    );
    if opts.set_default || profiles.default_profile.is_none() {
        profiles.default_profile = Some(profile_name.clone());
    }
    save_profiles(&profiles)?;

    Ok(json!({
        "profile": profile_name,
        "mode": "managed",
        "endpoint": endpoint,
        "default_profile": profiles.default_profile,
    }))
}

pub async fn init_self_managed(opts: SelfManagedInitOptions) -> Result<Value, String> {
    let mut profiles = load_profiles()?;
    let home = cli_home()?;
    fs::create_dir_all(&home.config_root).map_err(|e| format!("create config root: {e}"))?;
    fs::create_dir_all(&home.data_root).map_err(|e| format!("create data root: {e}"))?;

    let profile_name = opts
        .profile_name
        .clone()
        .unwrap_or_else(|| DEFAULT_LOCAL_PROFILE.to_string());
    let runtime_dir = home.data_root.join("runtime").join(&profile_name);
    let config_dir = home.config_root.join("profiles").join(&profile_name);
    let log_dir = runtime_dir.join("logs");
    let data_dir = runtime_dir.join("data");
    let infra_dir = runtime_dir.join("infra");
    let infra_env_path = config_dir.join("infra.env");
    let developer_env_path = config_dir.join("developer.env");
    let observe_env_path = config_dir.join("observe.env");
    let embed_env_path = config_dir.join("embed.env");
    let existing_local = profiles
        .profiles
        .get(&profile_name)
        .and_then(|profile| profile.local.as_ref())
        .cloned();

    fs::create_dir_all(&runtime_dir).map_err(|e| format!("create runtime dir: {e}"))?;
    fs::create_dir_all(&config_dir).map_err(|e| format!("create profile config dir: {e}"))?;
    fs::create_dir_all(&log_dir).map_err(|e| format!("create log dir: {e}"))?;
    fs::create_dir_all(&data_dir).map_err(|e| format!("create data dir: {e}"))?;
    fs::create_dir_all(&infra_dir).map_err(|e| format!("create infra dir: {e}"))?;
    prepare_local_data_dirs(&data_dir)?;

    let ports = LocalPorts {
        developer: opts.developer_port.unwrap_or(3000),
        observe_rest: 8084,
        observe_grpc: 9090,
        embed_rest: 8081,
        embed_grpc: 50052,
        embed_metrics: 9000,
        postgres: 55432,
        qdrant_http: 6333,
        qdrant_grpc: 6334,
        keycloak: 8180,
        loki: 3100,
        grafana: opts.with_grafana.then_some(3003),
    };

    let providers = resolve_local_provider_config(existing_local.as_ref(), &opts)?;

    let binaries = LocalBinaries {
        developer: opts.developer_bin.unwrap_or_else(|| {
            discover_binary("enscrive-developer")
                .unwrap_or_else(|| "enscrive-developer".to_string())
        }),
        observe: opts.observe_bin.unwrap_or_else(|| {
            discover_binary("enscrive-observe").unwrap_or_else(|| "enscrive-observe".to_string())
        }),
        embed: opts.embed_bin.unwrap_or_else(|| {
            discover_binary("enscrive-embed").unwrap_or_else(|| "enscrive-embed".to_string())
        }),
    };

    let lab_secret = read_env_value(&observe_env_path, "LAB_SERVICE_SECRET")
        .or_else(|| read_env_value(&embed_env_path, "LAB_SERVICE_SECRET"))
        .unwrap_or_else(|| generate_secret(48));
    let local_bootstrap_secret = existing_local
        .as_ref()
        .map(|local| local.bootstrap.secret.clone())
        .filter(|value| !value.trim().is_empty())
        .or_else(|| read_env_value(&developer_env_path, "LOCAL_BOOTSTRAP_SECRET"))
        .unwrap_or_else(|| generate_secret(48));
    let hmac_pepper =
        read_env_value(&developer_env_path, "HMAC_PEPPER").unwrap_or_else(|| generate_secret(48));
    let aes_key = read_env_value(&developer_env_path, "AES_KEY")
        .filter(|value| is_valid_aes_key(value))
        .unwrap_or_else(|| generate_hex_secret(32));
    let qdrant_api_key = read_env_value(&infra_env_path, "QDRANT_API_KEY")
        .or_else(|| read_env_value(&embed_env_path, "QDRANT_API_KEY"))
        .unwrap_or_else(|| generate_secret(48));
    let postgres_password = read_env_value(&infra_env_path, "POSTGRES_PASSWORD")
        .or_else(|| database_url_password(&developer_env_path))
        .or_else(|| database_url_password(&observe_env_path))
        .unwrap_or_else(|| generate_secret(24));
    let docker_project = format!("enscrive-local-{}", sanitize_name(&profile_name));

    let keycloak = existing_local
        .as_ref()
        .map(|local| local.keycloak.clone())
        .unwrap_or_else(|| LocalKeycloak {
            realm: "enscrive".to_string(),
            client_id: "enscrive-developer".to_string(),
            client_secret: generate_secret(32),
            admin_username: "admin".to_string(),
            admin_password: generate_secret(24),
            developer_username: "developer".to_string(),
            developer_password: generate_secret(24),
        });

    let local = LocalProfile {
        deployment_mode: "local".to_string(),
        runtime_dir: runtime_dir.display().to_string(),
        config_dir: config_dir.display().to_string(),
        compose_file: config_dir.join("docker-compose.yml").display().to_string(),
        infra_env_file: infra_env_path.display().to_string(),
        developer_env_file: developer_env_path.display().to_string(),
        observe_env_file: observe_env_path.display().to_string(),
        embed_env_file: embed_env_path.display().to_string(),
        log_dir: log_dir.display().to_string(),
        docker_project,
        binaries,
        ports,
        features: LocalFeatures {
            with_grafana: opts.with_grafana,
            local_bge_management: false,
        },
        keycloak,
        bootstrap: existing_local
            .as_ref()
            .map(|local| {
                let mut bootstrap = local.bootstrap.clone();
                bootstrap.secret = local_bootstrap_secret.clone();
                bootstrap
            })
            .unwrap_or(LocalBootstrap {
                secret: local_bootstrap_secret,
                developer_email: "developer@local.enscrive".to_string(),
                developer_name: "Local Developer".to_string(),
                tenant_name: "Local Developer".to_string(),
                environment_name: "development".to_string(),
                api_key_label: "local-cli".to_string(),
                tenant_id: None,
                environment_id: None,
                api_key_id: None,
            }),
        providers,
    };

    let developer_site_root = discover_developer_site_root(&home, &local.binaries);
    let leptos_output_name = developer_site_root
        .as_deref()
        .and_then(infer_leptos_output_name)
        .unwrap_or_else(|| DEFAULT_LEPTOS_OUTPUT_NAME.to_string());

    write_text(
        &config_dir.join("docker-compose.yml"),
        &render_local_compose(
            &local,
            &data_dir,
            &config_dir,
            &postgres_password,
            &qdrant_api_key,
        ),
    )?;
    write_text(&config_dir.join("initdb.sql"), &render_postgres_init())?;
    write_text(
        &config_dir.join("loki-config.yaml"),
        &render_local_loki_config(),
    )?;
    write_text(
        &config_dir.join("infra.env"),
        &render_infra_env(&local, &postgres_password, &qdrant_api_key),
    )?;
    write_text(
        &config_dir.join("developer.env"),
        &render_developer_env(
            &local,
            &postgres_password,
            &lab_secret,
            &hmac_pepper,
            &aes_key,
            developer_site_root.as_deref(),
            &leptos_output_name,
        ),
    )?;
    write_text(
        &config_dir.join("observe.env"),
        &render_observe_env(&local, &postgres_password, &lab_secret),
    )?;
    write_text(
        &config_dir.join("embed.env"),
        &render_embed_env(&local, &qdrant_api_key, &lab_secret),
    )?;

    profiles.version = PROFILE_VERSION;
    profiles.profiles.insert(
        profile_name.clone(),
        StoredProfile {
            mode: "self_managed".to_string(),
            endpoint: format!("http://127.0.0.1:{}", local.ports.developer),
            api_key: None,
            local: Some(local.clone()),
        },
    );
    if opts.set_default || profiles.default_profile.is_none() {
        profiles.default_profile = Some(profile_name.clone());
    }
    save_profiles(&profiles)?;

    Ok(json!({
        "profile": profile_name,
        "mode": "self_managed",
        "endpoint": format!("http://127.0.0.1:{}", local.ports.developer),
        "runtime_dir": local.runtime_dir,
        "config_dir": local.config_dir,
        "docker_project": local.docker_project,
        "binaries": {
            "enscrive-developer": local.binaries.developer,
            "enscrive-observe": local.binaries.observe,
            "enscrive-embed": local.binaries.embed,
        },
        "provider_configured": provider_configured_json(&local.providers),
        "login": {
            "url": format!("http://127.0.0.1:{}/auth/login", local.ports.developer),
            "username": local.keycloak.developer_username,
            "password": local.keycloak.developer_password,
        },
        "note": "local stack bootstrap is configured; run `enscrive start` to launch infra, seed the local tenant, and capture the first API key",
        "default_profile": profiles.default_profile,
    }))
}

pub async fn start(opts: StartOptions) -> Result<Value, String> {
    let home = cli_home()?;
    let mut profiles = load_profiles()?;
    let (profile_name, mut profile) = load_local_profile(opts.profile_name.as_deref(), &profiles)?;
    let local = profile
        .local
        .clone()
        .ok_or_else(|| format!("profile '{}' is not self-managed", profile_name))?;

    let developer_site_root = discover_developer_site_root(&home, &local.binaries);
    let leptos_output_name = developer_site_root
        .as_deref()
        .and_then(infer_leptos_output_name)
        .unwrap_or_else(|| DEFAULT_LEPTOS_OUTPUT_NAME.to_string());

    ensure_local_embedding_provider(&local)?;
    ensure_valid_developer_env(
        Path::new(&local.developer_env_file),
        developer_site_root.as_deref(),
        Some(&leptos_output_name),
    )?;
    prepare_local_data_dirs(&Path::new(&local.runtime_dir).join("data"))?;
    ensure_docker_available()?;
    let log_dir = Path::new(&local.log_dir);
    let mut started_services = Vec::new();
    compose_cmd(&local)?
        .arg("up")
        .arg("-d")
        .status()
        .map_err(|e| format!("start local infra: {e}"))
        .and_then(require_success("start local infra"))?;

    wait_for_tcp("127.0.0.1", local.ports.postgres, Duration::from_secs(60))?;
    wait_for_local_postgres(&local, Duration::from_secs(90))?;
    ensure_local_postgres_databases(&local)?;
    wait_for_tcp("127.0.0.1", local.ports.keycloak, Duration::from_secs(60))?;
    wait_for_tcp(
        "127.0.0.1",
        local.ports.qdrant_http,
        Duration::from_secs(60),
    )?;
    let loki_ready = wait_for_tcp("127.0.0.1", local.ports.loki, Duration::from_secs(60)).is_ok();

    let keycloak_user = bootstrap_keycloak(&local).await?;

    let started_embed = spawn_service(
        "enscrive-embed",
        &local.binaries.embed,
        Path::new(&local.embed_env_file),
        log_dir,
    )?;
    if service_was_newly_started(&started_embed) {
        started_services.push("enscrive-embed");
    }
    if let Err(error) = wait_for_http(
        &format!("http://127.0.0.1:{}/v1/health", local.ports.embed_rest),
        Duration::from_secs(60),
    )
    .await
    {
        cleanup_started_services(&started_services, log_dir);
        return Err(format_service_start_error(error, "enscrive-embed", log_dir));
    }

    let started_observe = spawn_service(
        "enscrive-observe",
        &local.binaries.observe,
        Path::new(&local.observe_env_file),
        log_dir,
    )?;
    if service_was_newly_started(&started_observe) {
        started_services.push("enscrive-observe");
    }
    if let Err(error) = wait_for_http(
        &format!("http://127.0.0.1:{}/health", local.ports.observe_rest),
        Duration::from_secs(60),
    )
    .await
    {
        cleanup_started_services(&started_services, log_dir);
        return Err(format_service_start_error(
            error,
            "enscrive-observe",
            log_dir,
        ));
    }

    let started_developer = spawn_service(
        "enscrive-developer",
        &local.binaries.developer,
        Path::new(&local.developer_env_file),
        log_dir,
    )?;
    if service_was_newly_started(&started_developer) {
        started_services.push("enscrive-developer");
    }
    if let Err(error) = wait_for_http(
        &format!("http://127.0.0.1:{}/health", local.ports.developer),
        Duration::from_secs(60),
    )
    .await
    {
        cleanup_started_services(&started_services, log_dir);
        return Err(format_service_start_error(
            error,
            "enscrive-developer",
            log_dir,
        ));
    }

    let bootstrap = match bootstrap_local_stack(
        &profile.endpoint,
        &local,
        &keycloak_user,
        profile.api_key.is_none(),
    )
    .await
    {
        Ok(bootstrap) => bootstrap,
        Err(error) => {
            cleanup_started_services(&started_services, log_dir);
            return Err(error);
        }
    };

    if let Some(local_profile) = profile.local.as_mut() {
        local_profile.bootstrap.tenant_id = Some(bootstrap.tenant_id.clone());
        local_profile.bootstrap.environment_id = Some(bootstrap.environment_id.clone());
        local_profile.bootstrap.api_key_id = bootstrap.api_key_id.clone();
    }
    if let Some(api_key) = bootstrap.api_key.clone() {
        profile.api_key = Some(api_key);
    }
    profiles
        .profiles
        .insert(profile_name.clone(), profile.clone());
    save_profiles(&profiles)?;

    Ok(json!({
        "profile": profile_name,
        "mode": profile.mode,
        "endpoint": profile.endpoint,
        "infra": {
            "docker_project": local.docker_project,
            "compose_file": local.compose_file,
            "loki_ready": loki_ready,
        },
        "services": {
            "enscrive-embed": started_embed,
            "enscrive-observe": started_observe,
            "enscrive-developer": started_developer,
        },
        "login": {
            "portal": profile.endpoint,
            "username": local.keycloak.developer_username,
            "password": local.keycloak.developer_password,
        },
        "bootstrap": bootstrap,
        "note": "local stack is running; the default tenant, environment, and API key have been bootstrapped for this local profile"
    }))
}

pub async fn stop(opts: StopOptions) -> Result<Value, String> {
    let profiles = load_profiles()?;
    let (profile_name, profile) = load_local_profile(opts.profile_name.as_deref(), &profiles)?;
    let local = profile
        .local
        .ok_or_else(|| format!("profile '{}' is not self-managed", profile_name))?;

    let stopped = vec![
        stop_service("enscrive-developer", Path::new(&local.log_dir))?,
        stop_service("enscrive-observe", Path::new(&local.log_dir))?,
        stop_service("enscrive-embed", Path::new(&local.log_dir))?,
    ];

    let compose_action = if opts.remove_infra { "down" } else { "stop" };
    compose_cmd(&local)?
        .arg(compose_action)
        .status()
        .map_err(|e| format!("{} local infra: {e}", compose_action))
        .and_then(require_success(if opts.remove_infra {
            "remove local infra"
        } else {
            "stop local infra"
        }))?;

    Ok(json!({
        "profile": profile_name,
        "services": stopped,
        "infra_action": compose_action,
    }))
}

pub async fn status(opts: StatusOptions) -> Result<Value, String> {
    let profiles = load_profiles()?;
    let selected_name = selected_profile_name(opts.profile_name.as_deref(), &profiles)
        .ok_or_else(|| "no profile configured; run `enscrive init` first".to_string())?;
    let profile = profiles
        .profiles
        .get(&selected_name)
        .cloned()
        .ok_or_else(|| format!("profile '{}' not found", selected_name))?;

    let local_status = profile.local.as_ref().map(|local| {
        json!({
            "deployment_mode": local.deployment_mode,
            "docker_project": local.docker_project,
            "runtime_dir": local.runtime_dir,
            "config_dir": local.config_dir,
            "infra": {
                "postgres": tcp_is_open("127.0.0.1", local.ports.postgres),
                "keycloak": tcp_is_open("127.0.0.1", local.ports.keycloak),
                "qdrant": tcp_is_open("127.0.0.1", local.ports.qdrant_http),
                "loki": tcp_is_open("127.0.0.1", local.ports.loki),
                "grafana": local.ports.grafana.map(|port| tcp_is_open("127.0.0.1", port)),
            },
            "services": {
                "enscrive-embed": service_status("enscrive-embed", Path::new(&local.log_dir)),
                "enscrive-observe": service_status("enscrive-observe", Path::new(&local.log_dir)),
                "enscrive-developer": service_status("enscrive-developer", Path::new(&local.log_dir)),
            },
            "login": {
                "portal": format!("http://127.0.0.1:{}", local.ports.developer),
                "username": local.keycloak.developer_username,
            },
            "bootstrap": {
                "tenant_name": local.bootstrap.tenant_name,
                "tenant_id": local.bootstrap.tenant_id,
                "environment_name": local.bootstrap.environment_name,
                "environment_id": local.bootstrap.environment_id,
                "api_key_label": local.bootstrap.api_key_label,
                "api_key_id": local.bootstrap.api_key_id,
            },
            "provider_configured": provider_configured_json(&local.providers),
            "api_key_configured": profile.api_key.is_some(),
        })
    });

    Ok(json!({
        "profile": selected_name,
        "mode": profile.mode,
        "endpoint": profile.endpoint,
        "api_key_configured": profile.api_key.is_some(),
        "local": local_status,
    }))
}

fn selected_profile_name(profile_name: Option<&str>, profiles: &ProfilesFile) -> Option<String> {
    profile_name
        .map(ToOwned::to_owned)
        .or_else(|| env::var("ENSCRIVE_PROFILE").ok())
        .or_else(|| profiles.default_profile.clone())
}

fn load_local_profile(
    profile_name: Option<&str>,
    profiles: &ProfilesFile,
) -> Result<(String, StoredProfile), String> {
    let selected_name = selected_profile_name(profile_name, profiles).ok_or_else(|| {
        "no self-managed profile configured; run `enscrive init --mode self-managed` first"
            .to_string()
    })?;
    let profile = profiles
        .profiles
        .get(&selected_name)
        .cloned()
        .ok_or_else(|| format!("profile '{}' not found", selected_name))?;
    Ok((selected_name, profile))
}

fn load_profiles() -> Result<ProfilesFile, String> {
    let home = cli_home()?;
    let path = home.config_root.join("profiles.toml");
    if !path.exists() {
        return Ok(ProfilesFile {
            version: PROFILE_VERSION,
            default_profile: None,
            profiles: BTreeMap::new(),
        });
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("read profiles.toml: {e}"))?;
    if content.trim().is_empty() {
        return Ok(ProfilesFile {
            version: PROFILE_VERSION,
            default_profile: None,
            profiles: BTreeMap::new(),
        });
    }
    let mut profiles: ProfilesFile =
        toml::from_str(&content).map_err(|e| format!("parse profiles.toml: {e}"))?;
    for profile in profiles.profiles.values_mut() {
        if let Some(local) = profile.local.as_mut() {
            if local.bootstrap.secret.trim().is_empty() {
                local.bootstrap.secret = generate_secret(48);
            }
            if local.bootstrap.developer_email.trim().is_empty() {
                local.bootstrap.developer_email =
                    format!("{}@local.enscrive", local.keycloak.developer_username);
            }
            if local.bootstrap.developer_name.trim().is_empty() {
                local.bootstrap.developer_name = "Local Developer".to_string();
            }
            if local.bootstrap.tenant_name.trim().is_empty() {
                local.bootstrap.tenant_name = "Local Developer".to_string();
            }
            if local.bootstrap.environment_name.trim().is_empty() {
                local.bootstrap.environment_name = "development".to_string();
            }
            if local.bootstrap.api_key_label.trim().is_empty() {
                local.bootstrap.api_key_label = "local-cli".to_string();
            }
        }
    }
    Ok(profiles)
}

fn save_profiles(profiles: &ProfilesFile) -> Result<(), String> {
    let home = cli_home()?;
    fs::create_dir_all(&home.config_root).map_err(|e| format!("create config root: {e}"))?;
    let path = home.config_root.join("profiles.toml");
    let content =
        toml::to_string_pretty(profiles).map_err(|e| format!("serialize profiles.toml: {e}"))?;
    write_text(&path, &content)
}

fn cli_home() -> Result<CliHome, String> {
    let home_dir = env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let config_root = env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(&home_dir).join(".config"))
        .join("enscrive");
    let data_root = env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(&home_dir).join(".local/share"))
        .join("enscrive");
    Ok(CliHome {
        config_root,
        data_root,
    })
}

fn write_text(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("create parent dir '{}': {e}", parent.display()))?;
    }
    fs::write(path, content).map_err(|e| format!("write '{}': {e}", path.display()))
}

fn render_postgres_init() -> String {
    r#"
CREATE DATABASE enscrive_developer;
CREATE DATABASE enscrive_keycloak;
CREATE DATABASE enscrive_observe;
CREATE DATABASE enscrive_embed_backup;
"#
    .trim()
    .to_string()
}

fn render_local_loki_config() -> String {
    r#"
auth_enabled: true
server:
  http_listen_port: 3100
  grpc_listen_port: 9095
common:
  path_prefix: /loki
  replication_factor: 1
  ring:
    kvstore:
      store: inmemory
schema_config:
  configs:
    - from: 2024-01-01
      store: tsdb
      object_store: filesystem
      schema: v13
      index:
        prefix: loki_index_
        period: 24h
storage_config:
  tsdb_shipper:
    active_index_directory: /loki/tsdb-index
    cache_location: /loki/tsdb-cache
  filesystem:
    directory: /loki/chunks
limits_config:
  retention_period: 168h
"#
    .trim()
    .to_string()
}

fn render_local_compose(
    local: &LocalProfile,
    data_dir: &Path,
    config_dir: &Path,
    postgres_password: &str,
    qdrant_api_key: &str,
) -> String {
    let grafana_service = if let Some(port) = local.ports.grafana {
        format!(
            r#"
  grafana:
    image: grafana/grafana:10.4.3
    restart: unless-stopped
    ports:
      - "{port}:3000"
    environment:
      GF_SECURITY_ADMIN_USER: admin
      GF_SECURITY_ADMIN_PASSWORD: admin
    depends_on:
      - loki
"#
        )
    } else {
        String::new()
    };

    format!(
        r#"
services:
  postgres:
    image: postgres:16
    restart: unless-stopped
    ports:
      - "{postgres_port}:5432"
    environment:
      POSTGRES_USER: enscrive
      POSTGRES_PASSWORD: {postgres_password}
      POSTGRES_DB: postgres
    volumes:
      - "{postgres_data}:/var/lib/postgresql/data:Z"
      - "{initdb_sql}:/docker-entrypoint-initdb.d/01-init.sql:ro,Z"

  keycloak:
    image: quay.io/keycloak/keycloak:25.0.6
    restart: unless-stopped
    depends_on:
      - postgres
    command: start-dev
    ports:
      - "{keycloak_port}:8080"
    environment:
      KEYCLOAK_ADMIN: {keycloak_admin}
      KEYCLOAK_ADMIN_PASSWORD: {keycloak_password}
      KC_DB: postgres
      KC_DB_URL: jdbc:postgresql://postgres:5432/enscrive_keycloak
      KC_DB_USERNAME: enscrive
      KC_DB_PASSWORD: {postgres_password}
      KC_HTTP_ENABLED: "true"
      KC_HOSTNAME_STRICT: "false"

  qdrant:
    image: qdrant/qdrant:v1.14.0
    restart: unless-stopped
    ports:
      - "{qdrant_http}:6333"
      - "{qdrant_grpc}:6334"
    environment:
      QDRANT__SERVICE__API_KEY: {qdrant_api_key}
    volumes:
      - "{qdrant_data}:/qdrant/storage:Z"

  loki:
    image: grafana/loki:2.9.4
    restart: unless-stopped
    command: -config.file=/etc/loki/config.yaml
    ports:
      - "{loki_port}:3100"
    volumes:
      - "{loki_config}:/etc/loki/config.yaml:ro,Z"
      - "{loki_data}:/loki:Z"
{grafana_service}
"#,
        postgres_port = local.ports.postgres,
        postgres_password = postgres_password,
        postgres_data = data_dir.join("postgres").display(),
        initdb_sql = config_dir.join("initdb.sql").display(),
        keycloak_port = local.ports.keycloak,
        keycloak_admin = local.keycloak.admin_username,
        keycloak_password = local.keycloak.admin_password,
        qdrant_http = local.ports.qdrant_http,
        qdrant_grpc = local.ports.qdrant_grpc,
        qdrant_api_key = qdrant_api_key,
        qdrant_data = data_dir.join("qdrant").display(),
        loki_port = local.ports.loki,
        loki_config = config_dir.join("loki-config.yaml").display(),
        loki_data = data_dir.join("loki").display(),
        grafana_service = grafana_service.trim_end(),
    )
    .trim()
    .to_string()
}

fn render_infra_env(local: &LocalProfile, postgres_password: &str, qdrant_api_key: &str) -> String {
    format!(
        "POSTGRES_PASSWORD={postgres_password}\nQDRANT_API_KEY={qdrant_api_key}\nKEYCLOAK_ADMIN_PASSWORD={keycloak_admin_password}\nLOCAL_DOCKER_PROJECT={docker_project}\n",
        postgres_password = postgres_password,
        qdrant_api_key = qdrant_api_key,
        keycloak_admin_password = local.keycloak.admin_password,
        docker_project = local.docker_project,
    )
}

fn render_developer_env(
    local: &LocalProfile,
    postgres_password: &str,
    lab_secret: &str,
    hmac_pepper: &str,
    aes_key: &str,
    leptos_site_root: Option<&Path>,
    leptos_output_name: &str,
) -> String {
    let leptos_site_root_line = leptos_site_root
        .map(|path| format!("LEPTOS_SITE_ROOT={}\n", path.display()))
        .unwrap_or_default();
    format!(
        "ENSCRIVE_ENVIRONMENT=development\nDEPLOYMENT_MODE=local\nDATABASE_URL=postgresql://enscrive:{postgres_password}@127.0.0.1:{postgres_port}/enscrive_developer\nDEVELOPER_PORT={developer_port}\nKEYCLOAK_ISSUER=http://127.0.0.1:{keycloak_port}/realms/{realm}\nKEYCLOAK_CLIENT_ID={client_id}\nKEYCLOAK_CLIENT_SECRET={client_secret}\nPORTAL_OIDC_REDIRECT_URI=http://127.0.0.1:{developer_port}/auth/callback\nHMAC_PEPPER={hmac_pepper}\nAES_KEY={aes_key}\nOBSERVE_GRPC_ADDR=http://127.0.0.1:{observe_grpc_port}\nLAB_SERVICE_SECRET={lab_secret}\nLOCAL_BOOTSTRAP_SECRET={local_bootstrap_secret}\nOPENAI_API_KEY={openai}\nANTHROPIC_API_KEY={anthropic}\nLEPTOS_OUTPUT_NAME={leptos_output_name}\nLEPTOS_SITE_PKG_DIR=pkg\n{leptos_site_root_line}ALLOW_MULTI_ENVIRONMENT=false\nALLOW_VOICE_PROMOTION=false\nALLOW_PROMOTION_GATES=false\nALLOW_MANAGED_BACKUPS=false\nALLOW_COMPLIANCE_EXPORTS=false\nALLOW_OPERATOR_OBSERVABILITY=false\nALLOW_BYOK_LLM_INFERENCE=true\nALLOW_LLM_CHUNKING_SETS=true\n",
        postgres_password = postgres_password,
        postgres_port = local.ports.postgres,
        developer_port = local.ports.developer,
        keycloak_port = local.ports.keycloak,
        realm = local.keycloak.realm,
        client_id = local.keycloak.client_id,
        client_secret = local.keycloak.client_secret,
        hmac_pepper = hmac_pepper,
        aes_key = aes_key,
        observe_grpc_port = local.ports.observe_grpc,
        lab_secret = lab_secret,
        local_bootstrap_secret = local.bootstrap.secret,
        openai = local
            .providers
            .llm_inference_openai_api_key()
            .unwrap_or_default(),
        anthropic = local
            .providers
            .llm_inference_anthropic_api_key()
            .clone()
            .unwrap_or_default(),
        leptos_output_name = leptos_output_name,
        leptos_site_root_line = leptos_site_root_line,
    )
}

fn installed_developer_site_root(home: &CliHome) -> PathBuf {
    home.data_root.join(INSTALLED_DEVELOPER_SITE_SUBDIR)
}

fn discover_developer_site_root(home: &CliHome, binaries: &LocalBinaries) -> Option<PathBuf> {
    let installed = installed_developer_site_root(home);
    if installed.join("pkg").is_dir() {
        return Some(installed);
    }

    let binary_path = Path::new(&binaries.developer);
    for ancestor in binary_path.ancestors() {
        if ancestor.join("Cargo.toml").is_file() {
            let site_root = ancestor.join("target").join("site");
            if site_root.join("pkg").is_dir() {
                return Some(site_root);
            }
        }
    }

    None
}

fn infer_leptos_output_name(site_root: &Path) -> Option<String> {
    let pkg_dir = site_root.join("pkg");
    if !pkg_dir.is_dir() {
        return None;
    }

    let mut css_candidates = Vec::new();
    let mut js_candidates = Vec::new();

    for entry in fs::read_dir(&pkg_dir).ok()? {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };

        if let Some(stem) = file_name.strip_suffix(".css") {
            css_candidates.push(stem.to_string());
            continue;
        }

        if let Some(stem) = file_name.strip_suffix(".js") {
            js_candidates.push(stem.to_string());
        }
    }

    css_candidates.sort();
    css_candidates.dedup();
    if let Some(candidate) = css_candidates.into_iter().next() {
        return Some(candidate);
    }

    js_candidates.sort();
    js_candidates.dedup();
    js_candidates.into_iter().next()
}

fn render_observe_env(local: &LocalProfile, postgres_password: &str, lab_secret: &str) -> String {
    format!(
        "LISTEN_ADDR=127.0.0.1:{observe_rest_port}\nLOKI_URL=http://127.0.0.1:{loki_port}\nEMBED_URL=http://127.0.0.1:{embed_grpc_port}\nDATABASE_URL=postgresql://enscrive:{postgres_password}@127.0.0.1:{postgres_port}/enscrive_observe\nLAB_SERVICE_SECRET={lab_secret}\nRUST_LOG=info\n",
        observe_rest_port = local.ports.observe_rest,
        loki_port = local.ports.loki,
        embed_grpc_port = local.ports.embed_grpc,
        postgres_password = postgres_password,
        postgres_port = local.ports.postgres,
        lab_secret = lab_secret,
    )
}

fn render_embed_env(local: &LocalProfile, qdrant_api_key: &str, lab_secret: &str) -> String {
    format!(
        "QDRANT_URL=http://127.0.0.1:{qdrant_http}\nQDRANT_GRPC_URL=http://127.0.0.1:{qdrant_grpc}\nQDRANT_GRPC=127.0.0.1:{qdrant_grpc}\nQDRANT_API_KEY={qdrant_api_key}\nCOLLECTION_NAME=embeddings\nSERVER_ADDR=127.0.0.1:{embed_grpc_port}\nREST_ADDR=127.0.0.1:{embed_rest_port}\nMETRICS_PORT={embed_metrics_port}\nOPENAI_API_KEY={openai}\nNEBIUS_API_KEY={nebius}\nVOYAGE_API_KEY={voyage}\nANTHROPIC_API_KEY={anthropic}\nBGE_ENDPOINT={bge_endpoint}\nBGE_API_KEY={bge_api_key}\nBGE_MODEL_NAME={bge_model_name}\nLAB_SERVICE_SECRET={lab_secret}\nBACKUP_SCHEDULER_ENABLED=false\n",
        qdrant_http = local.ports.qdrant_http,
        qdrant_grpc = local.ports.qdrant_grpc,
        qdrant_api_key = qdrant_api_key,
        embed_grpc_port = local.ports.embed_grpc,
        embed_rest_port = local.ports.embed_rest,
        embed_metrics_port = local.ports.embed_metrics,
        openai = local
            .providers
            .embedding_openai_api_key()
            .unwrap_or_default(),
        nebius = local
            .providers
            .embedding_nebius_api_key()
            .unwrap_or_default(),
        voyage = local
            .providers
            .embedding_voyage_api_key()
            .unwrap_or_default(),
        anthropic = String::new(),
        bge_endpoint = local.providers.embedding_bge_endpoint().unwrap_or_default(),
        bge_api_key = local.providers.embedding_bge_api_key().unwrap_or_default(),
        bge_model_name = local
            .providers
            .embedding_bge_model_name()
            .unwrap_or_default(),
        lab_secret = lab_secret,
    )
}

fn local_has_embedding_provider(local: &LocalProfile) -> bool {
    local.providers.embedding.any_enabled()
}

fn provider_configured_json(providers: &LocalProviders) -> Value {
    json!({
        "embedding": {
            "openai": providers.embedding.openai,
            "voyage": providers.embedding.voyage,
            "nebius": providers.embedding.nebius,
            "bge": providers.embedding.bge,
            "bge_endpoint": providers.embedding_bge_endpoint(),
            "bge_model_name": providers.embedding_bge_model_name(),
        },
        "llm_inference": {
            "openai": providers.llm_inference.openai,
            "anthropic": providers.llm_inference.anthropic,
        }
    })
}

fn ensure_local_embedding_provider(local: &LocalProfile) -> Result<(), String> {
    if local_has_embedding_provider(local) {
        return Ok(());
    }

    Err(
        "self-managed local mode requires at least one embedding provider. Re-run `enscrive init --mode self-managed` with `--bge-endpoint`, `--openai-api-key`, `--nebius-api-key`, or `--voyage-api-key`.".to_string(),
    )
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

fn interactive_prompts_available() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn resolve_local_provider_config(
    existing_local: Option<&LocalProfile>,
    opts: &SelfManagedInitOptions,
) -> Result<LocalProviders, String> {
    let mut providers = existing_local
        .map(|local| local.providers.clone())
        .unwrap_or_default();

    if let Some(openai_api_key) = normalize_optional(opts.openai_api_key.clone()) {
        providers.credentials.openai_api_key = Some(openai_api_key);
        providers.embedding.openai = true;
        providers.llm_inference.openai = true;
    }
    if let Some(anthropic_api_key) = normalize_optional(opts.anthropic_api_key.clone()) {
        providers.credentials.anthropic_api_key = Some(anthropic_api_key);
        providers.llm_inference.anthropic = true;
    }
    if let Some(voyage_api_key) = normalize_optional(opts.voyage_api_key.clone()) {
        providers.credentials.voyage_api_key = Some(voyage_api_key);
        providers.embedding.voyage = true;
    }
    if let Some(nebius_api_key) = normalize_optional(opts.nebius_api_key.clone()) {
        providers.credentials.nebius_api_key = Some(nebius_api_key);
        providers.embedding.nebius = true;
    }
    if let Some(bge_endpoint) = normalize_optional(opts.bge_endpoint.clone()) {
        providers.credentials.bge_endpoint = Some(bge_endpoint);
        providers.embedding.bge = true;
    }
    if let Some(bge_api_key) = normalize_optional(opts.bge_api_key.clone()) {
        providers.credentials.bge_api_key = Some(bge_api_key);
    }
    if let Some(bge_model_name) = normalize_optional(opts.bge_model_name.clone()) {
        providers.credentials.bge_model_name = Some(bge_model_name);
    }

    sync_local_provider_capabilities(&mut providers);

    let explicit_embedding_provider = opts.openai_api_key.is_some()
        || opts.voyage_api_key.is_some()
        || opts.nebius_api_key.is_some()
        || opts.bge_endpoint.is_some();
    let explicit_llm_provider = opts.openai_api_key.is_some() || opts.anthropic_api_key.is_some();

    if interactive_prompts_available() {
        if !providers.embedding.any_enabled() && !explicit_embedding_provider {
            prompt_for_embedding_providers(&mut providers)?;
        }
        if !providers.llm_inference.any_enabled() && !explicit_llm_provider {
            prompt_for_llm_inference_providers(&mut providers)?;
        }
    }

    sync_local_provider_capabilities(&mut providers);

    if !providers.embedding.any_enabled() {
        return Err(
            "self-managed local mode requires at least one embedding provider. Configure BGE, OpenAI, Voyage, or Nebius before starting the local stack."
                .to_string(),
        );
    }

    Ok(providers)
}

fn sync_local_provider_capabilities(providers: &mut LocalProviders) {
    if providers.credentials.openai_api_key.is_none() {
        providers.embedding.openai = false;
        providers.llm_inference.openai = false;
    }
    if providers.credentials.anthropic_api_key.is_none() {
        providers.llm_inference.anthropic = false;
    }
    if providers.credentials.voyage_api_key.is_none() {
        providers.embedding.voyage = false;
    }
    if providers.credentials.nebius_api_key.is_none() {
        providers.embedding.nebius = false;
    }
    if providers.credentials.bge_endpoint.is_none() {
        providers.embedding.bge = false;
    }
}

fn prompt_for_embedding_providers(providers: &mut LocalProviders) -> Result<(), String> {
    println!(
        "Configure embedding providers. At least one is required to run self-managed local mode."
    );
    println!(
        "For BGE, point Enscrive at a reachable local or LAN-hosted embeddings endpoint. Nebius Token Factory uses a Nebius API key instead of a BGE endpoint."
    );

    let bge_endpoint = prompt_line(
        "BGE endpoint for a local or LAN-hosted service (optional)",
        Some(""),
    )?;
    if !bge_endpoint.trim().is_empty() {
        providers.credentials.bge_endpoint = Some(bge_endpoint);
        providers.embedding.bge = true;

        let bge_api_key = prompt_secret_line("BGE bearer token for secured endpoints (optional)")?;
        providers.credentials.bge_api_key = normalize_optional(Some(bge_api_key));

        let bge_model_name = prompt_line(
            "BGE model name for single-model endpoints (optional)",
            Some(""),
        )?;
        providers.credentials.bge_model_name = normalize_optional(Some(bge_model_name));
    }

    let openai_api_key = prompt_secret_line("OpenAI API key for embeddings (optional)")?;
    if !openai_api_key.is_empty() {
        providers.credentials.openai_api_key = Some(openai_api_key);
        providers.embedding.openai = true;
    }

    let voyage_api_key = prompt_secret_line("Voyage API key for embeddings (optional)")?;
    if !voyage_api_key.is_empty() {
        providers.credentials.voyage_api_key = Some(voyage_api_key);
        providers.embedding.voyage = true;
    }

    let nebius_api_key =
        prompt_secret_line("Nebius API key for Token Factory embeddings (optional)")?;
    if !nebius_api_key.is_empty() {
        providers.credentials.nebius_api_key = Some(nebius_api_key);
        providers.embedding.nebius = true;
    }

    Ok(())
}

fn prompt_for_llm_inference_providers(providers: &mut LocalProviders) -> Result<(), String> {
    println!(
        "Configure optional LLM inference providers for crafted chunking sets. Leave values blank to disable LLM chunking."
    );

    if providers.credentials.openai_api_key.is_some() {
        providers.llm_inference.openai = prompt_yes_no(
            "Reuse the configured OpenAI API key for crafted chunking sets",
            false,
        )?;
    } else {
        let openai_api_key =
            prompt_secret_line("OpenAI API key for crafted chunking sets (optional)")?;
        if !openai_api_key.is_empty() {
            providers.credentials.openai_api_key = Some(openai_api_key);
            providers.llm_inference.openai = true;
        }
    }

    let anthropic_api_key =
        prompt_secret_line("Anthropic API key for crafted chunking sets (optional)")?;
    if !anthropic_api_key.is_empty() {
        providers.credentials.anthropic_api_key = Some(anthropic_api_key);
        providers.llm_inference.anthropic = true;
    }

    Ok(())
}

fn prompt_yes_no(label: &str, default: bool) -> Result<bool, String> {
    let default_value = if default { "yes" } else { "no" };
    let response = prompt_line(&format!("{label} (yes/no)"), Some(default_value))?;
    match response.trim().to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        other => Err(format!("invalid selection '{}': expected yes or no", other)),
    }
}

fn generate_secret(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn generate_hex_secret(num_bytes: usize) -> String {
    let mut bytes = vec![0u8; num_bytes];
    rand::thread_rng().fill(bytes.as_mut_slice());

    let mut secret = String::with_capacity(num_bytes * 2);
    for byte in bytes {
        secret.push(char::from_digit((byte >> 4) as u32, 16).expect("high nibble is valid hex"));
        secret.push(char::from_digit((byte & 0x0f) as u32, 16).expect("low nibble is valid hex"));
    }
    secret
}

fn is_valid_aes_key(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn ensure_valid_developer_env(
    env_file: &Path,
    leptos_site_root: Option<&Path>,
    leptos_output_name: Option<&str>,
) -> Result<(), String> {
    let mut envs = parse_env_file(env_file)?;
    let mut found_aes_key = false;
    let mut changed = false;

    fn upsert_env(envs: &mut Vec<(String, String)>, key: &str, value: String) -> bool {
        for (candidate, current) in envs.iter_mut() {
            if candidate == key {
                if *current != value {
                    *current = value;
                    return true;
                }
                return false;
            }
        }
        envs.push((key.to_string(), value));
        true
    }

    for (key, value) in envs.iter_mut() {
        if key == "AES_KEY" {
            found_aes_key = true;
            if !is_valid_aes_key(value.trim()) {
                *value = generate_hex_secret(32);
                changed = true;
            }
        }
    }

    if !found_aes_key {
        return Err(format!(
            "developer env '{}' is missing AES_KEY",
            env_file.display()
        ));
    }

    if let Some(output_name) = leptos_output_name {
        changed |= upsert_env(&mut envs, "LEPTOS_OUTPUT_NAME", output_name.to_string());
    }

    if let Some(site_root) = leptos_site_root {
        changed |= upsert_env(
            &mut envs,
            "LEPTOS_SITE_ROOT",
            site_root.display().to_string(),
        );
        changed |= upsert_env(&mut envs, "LEPTOS_SITE_PKG_DIR", "pkg".to_string());
    }

    if changed {
        let content = envs
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");
        write_text(env_file, &(content + "\n"))?;
    }

    Ok(())
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn discover_binary(binary_name: &str) -> Option<String> {
    if let Some(found) = which_in_path(binary_name) {
        return Some(found.display().to_string());
    }

    let cwd = env::current_dir().ok()?;
    let repo_candidate = cwd.join(binary_name);
    if repo_candidate.exists() {
        return Some(repo_candidate.display().to_string());
    }

    let workspace_root = cwd.parent().unwrap_or(&cwd).to_path_buf();
    let repo_dir = workspace_root.join(binary_name);
    let debug = repo_dir.join("target/debug").join(binary_name);
    if debug.exists() {
        return Some(debug.display().to_string());
    }
    let release = repo_dir.join("target/release").join(binary_name);
    if release.exists() {
        return Some(release.display().to_string());
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

fn ensure_docker_available() -> Result<(), String> {
    resolve_compose_binary().map(|_| ())
}

fn compose_cmd(local: &LocalProfile) -> Result<Command, String> {
    let mut cmd = match resolve_compose_binary()? {
        ComposeBinary::DockerPlugin => {
            let mut cmd = Command::new("docker");
            cmd.arg("compose");
            cmd
        }
        ComposeBinary::DockerCompose => Command::new("docker-compose"),
    };
    cmd.arg("-p")
        .arg(&local.docker_project)
        .arg("-f")
        .arg(&local.compose_file);
    Ok(cmd)
}

fn resolve_compose_binary() -> Result<ComposeBinary, String> {
    if command_succeeds("docker", &["compose", "version"]) {
        return Ok(ComposeBinary::DockerPlugin);
    }

    if command_succeeds("docker-compose", &["version"]) {
        return Ok(ComposeBinary::DockerCompose);
    }

    Err(format!(
        "self-managed local mode requires Docker and Docker Compose. {}",
        FEDORA_DOCKER_HINT
    ))
}

fn command_succeeds(program: &str, args: &[&str]) -> bool {
    match Command::new(program).args(args).output() {
        Ok(output) => output.status.success(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => false,
    }
}

fn require_success<'a>(
    label: &'a str,
) -> impl FnOnce(std::process::ExitStatus) -> Result<(), String> + 'a {
    move |status| {
        if status.success() {
            Ok(())
        } else {
            Err(format!("{} failed with status {}", label, status))
        }
    }
}

fn ensure_local_postgres_databases(local: &LocalProfile) -> Result<(), String> {
    for database in LOCAL_POSTGRES_DATABASES {
        let exists = retry_local_postgres_admin(
            &format!("check postgres database '{}'", database),
            Duration::from_secs(90),
            || local_postgres_database_exists(local, database),
        )?;
        if !exists {
            retry_local_postgres_admin(
                &format!("create postgres database '{}'", database),
                Duration::from_secs(90),
                || create_local_postgres_database(local, database),
            )?;
        }
    }
    Ok(())
}

fn wait_for_local_postgres(local: &LocalProfile, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    let mut last_error = "postgres readiness check did not run".to_string();

    while started.elapsed() < timeout {
        let output = compose_cmd(local)?
            .arg("exec")
            .arg("-T")
            .arg("postgres")
            .arg("pg_isready")
            .arg("-U")
            .arg("enscrive")
            .arg("-d")
            .arg("postgres")
            .output()
            .map_err(|e| format!("check postgres readiness: {e}"))?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = stderr_string(&output);
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        last_error = if stderr == "unknown error" && !stdout.is_empty() {
            stdout
        } else {
            stderr
        };
        std::thread::sleep(Duration::from_millis(500));
    }

    Err(format!(
        "postgres did not become ready after {}s: {}",
        timeout.as_secs(),
        last_error
    ))
}

fn local_postgres_database_exists(local: &LocalProfile, database: &str) -> Result<bool, String> {
    let output = compose_cmd(local)?
        .arg("exec")
        .arg("-T")
        .arg("postgres")
        .arg("psql")
        .arg("-U")
        .arg("enscrive")
        .arg("-d")
        .arg("postgres")
        .arg("-tAc")
        .arg(format!(
            "SELECT 1 FROM pg_database WHERE datname = '{}'",
            database
        ))
        .output()
        .map_err(|e| format!("check postgres database '{}': {e}", database))?;
    if !output.status.success() {
        return Err(format!(
            "check postgres database '{}' failed: {}",
            database,
            stderr_string(&output)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "1")
}

fn create_local_postgres_database(local: &LocalProfile, database: &str) -> Result<(), String> {
    let output = compose_cmd(local)?
        .arg("exec")
        .arg("-T")
        .arg("postgres")
        .arg("psql")
        .arg("-U")
        .arg("enscrive")
        .arg("-d")
        .arg("postgres")
        .arg("-v")
        .arg("ON_ERROR_STOP=1")
        .arg("-c")
        .arg(format!("CREATE DATABASE {};", database))
        .output()
        .map_err(|e| format!("create postgres database '{}': {e}", database))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = stderr_string(&output);
    if stderr.to_lowercase().contains("already exists") {
        return Ok(());
    }
    Err(format!(
        "create postgres database '{}' failed: {}",
        database, stderr
    ))
}

fn retry_local_postgres_admin<T, F>(
    operation: &str,
    timeout: Duration,
    mut action: F,
) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    let started = Instant::now();
    let mut last_error = None;

    while started.elapsed() < timeout {
        match action() {
            Ok(value) => return Ok(value),
            Err(error) if is_transient_local_postgres_error(&error) => {
                last_error = Some(error);
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(error) => return Err(error),
        }
    }

    Err(format!(
        "{} did not become stable after {}s: {}",
        operation,
        timeout.as_secs(),
        last_error.unwrap_or_else(|| "unknown error".to_string())
    ))
}

fn is_transient_local_postgres_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("the database system is shutting down")
        || lower.contains("the database system is starting up")
        || lower.contains("server closed the connection unexpectedly")
        || lower.contains("terminating connection due to administrator command")
        || lower.contains("connection refused")
        || lower.contains("could not connect to server")
        || lower.contains("no such file or directory")
}

fn stderr_string(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        "unknown error".to_string()
    } else {
        stderr
    }
}

fn spawn_service(
    service_name: &str,
    binary: &str,
    env_file: &Path,
    log_dir: &Path,
) -> Result<Value, String> {
    let pid_path = pid_file(log_dir, service_name);
    if let Some(pid) = read_pid(&pid_path)? {
        if pid_is_running(pid) {
            return Ok(json!({
                "status": "already_running",
                "pid": pid,
                "binary": binary,
            }));
        }
        let _ = fs::remove_file(&pid_path);
    }

    let envs = parse_env_file(env_file)?;
    let log_path = log_dir.join(format!("{}.log", service_name));
    let stdout = File::create(&log_path)
        .map_err(|e| format!("create log file '{}': {e}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .map_err(|e| format!("clone log file '{}': {e}", log_path.display()))?;

    let mut cmd = Command::new(binary);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    for (key, value) in envs {
        cmd.env(key, value);
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("spawn {} via '{}': {e}", service_name, binary))?;
    fs::write(&pid_path, child.id().to_string())
        .map_err(|e| format!("write pid file '{}': {e}", pid_path.display()))?;

    Ok(json!({
        "status": "started",
        "pid": child.id(),
        "binary": binary,
        "log_file": log_path.display().to_string(),
    }))
}

fn stop_service(service_name: &str, log_dir: &Path) -> Result<Value, String> {
    let pid_path = pid_file(log_dir, service_name);
    let Some(pid) = read_pid(&pid_path)? else {
        return Ok(json!({"service": service_name, "status": "not_running"}));
    };

    if !pid_is_running(pid) {
        let _ = fs::remove_file(&pid_path);
        return Ok(json!({"service": service_name, "status": "stale_pid_removed", "pid": pid}));
    }

    if !send_pid_signal(pid, "-TERM").map_err(|e| format!("stop {}: {e}", service_name))? {
        let _ = fs::remove_file(&pid_path);
        return Ok(json!({"service": service_name, "status": "stale_pid_removed", "pid": pid}));
    }

    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if !pid_is_running(pid) {
            let _ = fs::remove_file(&pid_path);
            return Ok(json!({"service": service_name, "status": "stopped", "pid": pid}));
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    if !send_pid_signal(pid, "-KILL").map_err(|e| format!("force stop {}: {e}", service_name))? {
        let _ = fs::remove_file(&pid_path);
        return Ok(json!({"service": service_name, "status": "stale_pid_removed", "pid": pid}));
    }
    let _ = fs::remove_file(&pid_path);
    Ok(json!({"service": service_name, "status": "killed", "pid": pid}))
}

fn service_status(service_name: &str, log_dir: &Path) -> Value {
    let pid_path = pid_file(log_dir, service_name);
    match read_pid(&pid_path) {
        Ok(Some(pid)) if pid_is_running(pid) => json!({"status": "running", "pid": pid}),
        Ok(Some(pid)) => json!({"status": "stale_pid", "pid": pid}),
        Ok(None) => json!({"status": "stopped"}),
        Err(error) => json!({"status": "error", "error": error}),
    }
}

fn pid_file(log_dir: &Path, service_name: &str) -> PathBuf {
    log_dir.join(format!("{}.pid", service_name))
}

fn read_pid(pid_path: &Path) -> Result<Option<u32>, String> {
    if !pid_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(pid_path)
        .map_err(|e| format!("read pid file '{}': {e}", pid_path.display()))?;
    raw.trim()
        .parse::<u32>()
        .map(Some)
        .map_err(|e| format!("parse pid file '{}': {e}", pid_path.display()))
}

fn pid_is_running(pid: u32) -> bool {
    send_pid_signal(pid, "-0").unwrap_or(false)
}

fn send_pid_signal(pid: u32, signal: &str) -> Result<bool, String> {
    Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .map_err(|e| format!("send signal {} to pid {}: {e}", signal, pid))
}

fn service_was_newly_started(status: &Value) -> bool {
    status.get("status").and_then(Value::as_str) == Some("started")
}

fn cleanup_started_services(services: &[&str], log_dir: &Path) {
    for service_name in services.iter().rev() {
        let _ = stop_service(service_name, log_dir);
    }
}

fn format_service_start_error(error: String, service_name: &str, log_dir: &Path) -> String {
    match recent_service_log_excerpt(log_dir, service_name, 20) {
        Some(log_tail) if !log_tail.is_empty() => {
            format!("{error}\nLast {service_name} log lines:\n{log_tail}")
        }
        _ => error,
    }
}

fn recent_service_log_excerpt(
    log_dir: &Path,
    service_name: &str,
    max_lines: usize,
) -> Option<String> {
    let log_path = log_dir.join(format!("{}.log", service_name));
    let raw = fs::read_to_string(log_path).ok()?;
    let lines = raw.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(max_lines);
    let excerpt = lines[start..].join("\n").trim().to_string();
    if excerpt.is_empty() {
        None
    } else {
        Some(excerpt)
    }
}

fn parse_env_file(path: &Path) -> Result<Vec<(String, String)>, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("read env file '{}': {e}", path.display()))?;
    let mut envs = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            return Err(format!(
                "invalid env line in '{}': {}",
                path.display(),
                trimmed
            ));
        };
        envs.push((key.to_string(), value.to_string()));
    }
    Ok(envs)
}

fn read_env_value(path: &Path, key: &str) -> Option<String> {
    parse_env_file(path)
        .ok()?
        .into_iter()
        .find_map(
            |(candidate, value)| {
                if candidate == key { Some(value) } else { None }
            },
        )
}

fn database_url_password(path: &Path) -> Option<String> {
    let database_url = read_env_value(path, "DATABASE_URL")?;
    let (_, rest) = database_url.split_once("://")?;
    let credentials = rest.split('@').next()?;
    let (_, password) = credentials.split_once(':')?;
    Some(password.to_string())
}

fn prepare_local_data_dirs(data_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(data_dir.join("postgres"))
        .map_err(|e| format!("create postgres data dir '{}': {e}", data_dir.display()))?;
    fs::create_dir_all(data_dir.join("qdrant"))
        .map_err(|e| format!("create qdrant data dir '{}': {e}", data_dir.display()))?;
    let loki_dir = data_dir.join("loki");
    fs::create_dir_all(&loki_dir)
        .map_err(|e| format!("create loki data dir '{}': {e}", loki_dir.display()))?;
    set_dir_mode(&loki_dir, 0o777)?;
    Ok(())
}

#[cfg(unix)]
fn set_dir_mode(path: &Path, mode: u32) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|e| format!("stat directory '{}': {e}", path.display()))?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions)
        .map_err(|e| format!("set permissions on '{}': {e}", path.display()))
}

#[cfg(not(unix))]
fn set_dir_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

fn wait_for_tcp(host: &str, port: u16, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if TcpStream::connect((host, port)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!(
        "timed out waiting for {}:{} after {}s",
        host,
        port,
        timeout.as_secs()
    ))
}

fn tcp_is_open(host: &str, port: u16) -> bool {
    TcpStream::connect((host, port)).is_ok()
}

async fn wait_for_http(url: &str, timeout: Duration) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;
    let started = Instant::now();
    while started.elapsed() < timeout {
        match client.get(url).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(_) | Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    Err(format!(
        "timed out waiting for HTTP readiness at {} after {}s",
        url,
        timeout.as_secs()
    ))
}

async fn bootstrap_local_stack(
    endpoint: &str,
    local: &LocalProfile,
    keycloak_user: &LocalKeycloakUser,
    issue_api_key: bool,
) -> Result<LocalBootstrapResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("build local bootstrap client: {e}"))?;

    let response = client
        .post(format!(
            "{}/local/bootstrap",
            endpoint.trim_end_matches('/')
        ))
        .json(&LocalBootstrapRequest {
            secret: local.bootstrap.secret.clone(),
            developer_subject: keycloak_user.subject.clone(),
            developer_email: keycloak_user.email.clone(),
            developer_name: local.bootstrap.developer_name.clone(),
            tenant_name: local.bootstrap.tenant_name.clone(),
            environment_name: local.bootstrap.environment_name.clone(),
            api_key_label: local.bootstrap.api_key_label.clone(),
            issue_api_key,
        })
        .send()
        .await
        .map_err(|e| format!("call local bootstrap endpoint: {e}"))?;

    if !response.status().is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "unable to read error body".to_string());
        return Err(format!("local bootstrap failed: {}", body));
    }

    response
        .json::<LocalBootstrapResponse>()
        .await
        .map_err(|e| format!("parse local bootstrap response: {e}"))
}

async fn bootstrap_keycloak(local: &LocalProfile) -> Result<LocalKeycloakUser, String> {
    let base = format!("http://127.0.0.1:{}", local.ports.keycloak);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("build keycloak client: {e}"))?;

    let access_token =
        wait_for_keycloak_admin_login(&client, &base, local, Duration::from_secs(90)).await?;

    let auth = format!("Bearer {}", access_token);

    let realm_url = format!("{}/admin/realms/{}", base, local.keycloak.realm);
    let realm_resp = client
        .get(&realm_url)
        .header("Authorization", &auth)
        .send()
        .await
        .map_err(|e| format!("check keycloak realm: {e}"))?;
    if realm_resp.status() == reqwest::StatusCode::NOT_FOUND {
        let create = client
            .post(format!("{}/admin/realms", base))
            .header("Authorization", &auth)
            .json(&json!({"realm": local.keycloak.realm, "enabled": true}))
            .send()
            .await
            .map_err(|e| format!("create keycloak realm: {e}"))?;
        if !create.status().is_success() {
            let body = create.text().await.unwrap_or_default();
            return Err(format!("create keycloak realm failed: {}", body));
        }
    } else if !realm_resp.status().is_success() {
        let body = realm_resp.text().await.unwrap_or_default();
        return Err(format!("check keycloak realm failed: {}", body));
    }

    let clients_resp = client
        .get(format!(
            "{}/admin/realms/{}/clients?clientId={}",
            base, local.keycloak.realm, local.keycloak.client_id
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .map_err(|e| format!("query keycloak client: {e}"))?;
    let clients_json: Value = clients_resp
        .json()
        .await
        .map_err(|e| format!("parse keycloak clients response: {e}"))?;
    if clients_json.as_array().is_none_or(|items| items.is_empty()) {
        let create = client
            .post(format!(
                "{}/admin/realms/{}/clients",
                base, local.keycloak.realm
            ))
            .header("Authorization", &auth)
            .json(&json!({
                "clientId": local.keycloak.client_id,
                "enabled": true,
                "protocol": "openid-connect",
                "publicClient": false,
                "secret": local.keycloak.client_secret,
                "standardFlowEnabled": true,
                "directAccessGrantsEnabled": false,
                "redirectUris": [format!("http://127.0.0.1:{}/auth/callback", local.ports.developer)],
                "webOrigins": [format!("http://127.0.0.1:{}", local.ports.developer)],
            }))
            .send()
            .await
            .map_err(|e| format!("create keycloak client: {e}"))?;
        if !create.status().is_success() {
            let body = create.text().await.unwrap_or_default();
            return Err(format!("create keycloak client failed: {}", body));
        }
    }

    let users_resp = client
        .get(format!(
            "{}/admin/realms/{}/users?username={}",
            base, local.keycloak.realm, local.keycloak.developer_username
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .map_err(|e| format!("query keycloak users: {e}"))?;
    let users_json: Value = users_resp
        .json()
        .await
        .map_err(|e| format!("parse keycloak users response: {e}"))?;
    let user_id = if let Some(user) = users_json.as_array().and_then(|items| items.first()) {
        user.get("id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "existing keycloak user missing id".to_string())?
            .to_string()
    } else {
        let create = client
            .post(format!(
                "{}/admin/realms/{}/users",
                base, local.keycloak.realm
            ))
            .header("Authorization", &auth)
            .json(&json!({
                "username": local.keycloak.developer_username,
                "email": local.bootstrap.developer_email,
                "enabled": true,
                "emailVerified": true,
            }))
            .send()
            .await
            .map_err(|e| format!("create keycloak user: {e}"))?;
        if !create.status().is_success() {
            let body = create.text().await.unwrap_or_default();
            return Err(format!("create keycloak user failed: {}", body));
        }
        let users_resp = client
            .get(format!(
                "{}/admin/realms/{}/users?username={}",
                base, local.keycloak.realm, local.keycloak.developer_username
            ))
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(|e| format!("reload keycloak user: {e}"))?;
        let users_json: Value = users_resp
            .json()
            .await
            .map_err(|e| format!("parse reloaded keycloak users response: {e}"))?;
        users_json
            .as_array()
            .and_then(|items| items.first())
            .and_then(|user| user.get("id"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| "created keycloak user missing id".to_string())?
            .to_string()
    };

    let reset = client
        .put(format!(
            "{}/admin/realms/{}/users/{}/reset-password",
            base, local.keycloak.realm, user_id
        ))
        .header("Authorization", &auth)
        .json(&json!({
            "type": "password",
            "value": local.keycloak.developer_password,
            "temporary": false
        }))
        .send()
        .await
        .map_err(|e| format!("reset local developer password: {e}"))?;
    if !reset.status().is_success() {
        let body = reset.text().await.unwrap_or_default();
        return Err(format!("reset local developer password failed: {}", body));
    }

    Ok(LocalKeycloakUser {
        subject: user_id,
        email: local.bootstrap.developer_email.clone(),
    })
}

async fn wait_for_keycloak_admin_login(
    client: &reqwest::Client,
    base: &str,
    local: &LocalProfile,
    timeout: Duration,
) -> Result<String, String> {
    let started = Instant::now();
    let mut last_error = "keycloak admin login did not start".to_string();

    while started.elapsed() < timeout {
        match request_keycloak_admin_token(client, base, local).await {
            Ok(token) => return Ok(token),
            Err(error) => {
                last_error = error;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }

    Err(format!(
        "keycloak admin login did not become ready after {}s: {}",
        timeout.as_secs(),
        last_error
    ))
}

async fn request_keycloak_admin_token(
    client: &reqwest::Client,
    base: &str,
    local: &LocalProfile,
) -> Result<String, String> {
    let token_resp = client
        .post(format!(
            "{}/realms/master/protocol/openid-connect/token",
            base
        ))
        .form(&[
            ("grant_type", "password"),
            ("client_id", "admin-cli"),
            ("username", local.keycloak.admin_username.as_str()),
            ("password", local.keycloak.admin_password.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("keycloak admin login failed: {e}"))?;
    if !token_resp.status().is_success() {
        let body = token_resp
            .text()
            .await
            .unwrap_or_else(|_| "unable to read body".to_string());
        return Err(format!("keycloak admin login failed: {}", body));
    }
    let token_json: Value = token_resp
        .json()
        .await
        .map_err(|e| format!("parse keycloak token response: {e}"))?;
    token_json
        .get("access_token")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "keycloak token response missing access_token".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn set_xdg(temp: &TempDir) {
        unsafe {
            env::set_var("XDG_CONFIG_HOME", temp.path().join("config"));
            env::set_var("XDG_DATA_HOME", temp.path().join("data"));
        }
    }

    #[test]
    fn save_and_load_profiles_round_trip() {
        let _guard = crate::test_support::lock_env();
        let temp = TempDir::new().unwrap();
        set_xdg(&temp);

        let mut profiles = ProfilesFile {
            version: PROFILE_VERSION,
            default_profile: Some("managed".to_string()),
            profiles: BTreeMap::new(),
        };
        profiles.profiles.insert(
            "managed".to_string(),
            StoredProfile {
                mode: "managed".to_string(),
                endpoint: "https://api.enscrive.io".to_string(),
                api_key: Some("key".to_string()),
                local: None,
            },
        );

        save_profiles(&profiles).unwrap();
        let loaded = load_profiles().unwrap();
        assert_eq!(loaded.default_profile.as_deref(), Some("managed"));
        assert_eq!(
            loaded.profiles.get("managed").unwrap().endpoint,
            "https://api.enscrive.io"
        );
    }

    #[test]
    fn resolve_api_context_prefers_profile() {
        let _guard = crate::test_support::lock_env();
        let temp = TempDir::new().unwrap();
        set_xdg(&temp);

        let mut profiles = ProfilesFile {
            version: PROFILE_VERSION,
            default_profile: Some("managed".to_string()),
            profiles: BTreeMap::new(),
        };
        profiles.profiles.insert(
            "managed".to_string(),
            StoredProfile {
                mode: "managed".to_string(),
                endpoint: "https://api.enscrive.io".to_string(),
                api_key: Some("secret".to_string()),
                local: None,
            },
        );
        save_profiles(&profiles).unwrap();

        let resolved = resolve_api_context(None, None, None).unwrap();
        assert_eq!(resolved.endpoint, "https://api.enscrive.io");
        assert_eq!(resolved.api_key.as_deref(), Some("secret"));
        assert_eq!(resolved.profile_name.as_deref(), Some("managed"));
    }

    #[tokio::test]
    async fn init_self_managed_with_openai_flag_enables_embeddings_and_llm_inference() {
        let _guard = crate::test_support::lock_env();
        let temp = TempDir::new().unwrap();
        set_xdg(&temp);

        init_self_managed(SelfManagedInitOptions {
            profile_name: Some("local".to_string()),
            with_grafana: false,
            developer_port: None,
            developer_bin: Some("/tmp/enscrive-developer".to_string()),
            observe_bin: Some("/tmp/enscrive-observe".to_string()),
            embed_bin: Some("/tmp/enscrive-embed".to_string()),
            openai_api_key: Some("sk-test".to_string()),
            anthropic_api_key: Some("anth-test".to_string()),
            voyage_api_key: None,
            nebius_api_key: None,
            bge_endpoint: None,
            bge_api_key: None,
            bge_model_name: None,
            set_default: true,
        })
        .await
        .unwrap();

        let profiles = load_profiles().unwrap();
        let local = profiles
            .profiles
            .get("local")
            .and_then(|profile| profile.local.as_ref())
            .unwrap();

        assert_eq!(
            local.providers.credentials.openai_api_key.as_deref(),
            Some("sk-test")
        );
        assert!(local.providers.embedding.openai);
        assert!(local.providers.llm_inference.openai);
        assert!(local.providers.llm_inference.anthropic);
    }

    #[tokio::test]
    async fn init_self_managed_writes_runtime_files() {
        let _guard = crate::test_support::lock_env();
        let temp = TempDir::new().unwrap();
        set_xdg(&temp);

        let result = init_self_managed(SelfManagedInitOptions {
            profile_name: Some("local".to_string()),
            with_grafana: false,
            developer_port: None,
            developer_bin: Some("/tmp/enscrive-developer".to_string()),
            observe_bin: Some("/tmp/enscrive-observe".to_string()),
            embed_bin: Some("/tmp/enscrive-embed".to_string()),
            openai_api_key: Some("sk-test".to_string()),
            anthropic_api_key: None,
            voyage_api_key: None,
            nebius_api_key: Some("neb-test".to_string()),
            bge_endpoint: Some("http://192.168.1.10:8080".to_string()),
            bge_api_key: None,
            bge_model_name: Some("bge-large-en-v1.5".to_string()),
            set_default: true,
        })
        .await
        .unwrap();

        let config_dir = result["config_dir"].as_str().unwrap();
        assert!(Path::new(config_dir).join("docker-compose.yml").exists());
        assert!(Path::new(config_dir).join("developer.env").exists());
        assert!(Path::new(config_dir).join("observe.env").exists());
        assert!(Path::new(config_dir).join("embed.env").exists());

        let developer_env =
            std::fs::read_to_string(Path::new(config_dir).join("developer.env")).unwrap();
        assert!(
            developer_env.contains("LOCAL_BOOTSTRAP_SECRET="),
            "developer.env should include LOCAL_BOOTSTRAP_SECRET"
        );
        let aes_key = developer_env
            .lines()
            .find_map(|line| line.strip_prefix("AES_KEY="))
            .expect("developer.env should include AES_KEY");
        assert!(
            is_valid_aes_key(aes_key),
            "AES_KEY should be a 64-character hex string"
        );

        let profiles = load_profiles().unwrap();
        let local = profiles
            .profiles
            .get("local")
            .and_then(|profile| profile.local.as_ref())
            .expect("local profile should exist");
        assert!(!local.bootstrap.secret.is_empty());
        assert_eq!(local.bootstrap.environment_name, "development");
        assert_eq!(local.bootstrap.api_key_label, "local-cli");
    }

    #[tokio::test]
    async fn init_self_managed_preserves_existing_local_secrets() {
        let _guard = crate::test_support::lock_env();
        let temp = TempDir::new().unwrap();
        set_xdg(&temp);

        init_self_managed(SelfManagedInitOptions {
            profile_name: Some("local".to_string()),
            with_grafana: false,
            developer_port: None,
            developer_bin: Some("/tmp/enscrive-developer".to_string()),
            observe_bin: Some("/tmp/enscrive-observe".to_string()),
            embed_bin: Some("/tmp/enscrive-embed".to_string()),
            openai_api_key: None,
            anthropic_api_key: None,
            voyage_api_key: None,
            nebius_api_key: None,
            bge_endpoint: Some("http://127.0.0.1:8088".to_string()),
            bge_api_key: None,
            bge_model_name: Some("bge-large-en-v1.5".to_string()),
            set_default: true,
        })
        .await
        .unwrap();

        let config_dir = cli_home()
            .unwrap()
            .config_root
            .join("profiles")
            .join("local");
        let infra_env = config_dir.join("infra.env");
        let developer_env = config_dir.join("developer.env");
        let observe_env = config_dir.join("observe.env");

        let original_postgres_password = read_env_value(&infra_env, "POSTGRES_PASSWORD").unwrap();
        let original_hmac_pepper = read_env_value(&developer_env, "HMAC_PEPPER").unwrap();
        let original_aes_key = read_env_value(&developer_env, "AES_KEY").unwrap();
        let original_lab_secret = read_env_value(&observe_env, "LAB_SERVICE_SECRET").unwrap();
        let original_client_secret = load_profiles()
            .unwrap()
            .profiles
            .get("local")
            .and_then(|profile| profile.local.as_ref())
            .map(|local| local.keycloak.client_secret.clone())
            .unwrap();

        init_self_managed(SelfManagedInitOptions {
            profile_name: Some("local".to_string()),
            with_grafana: false,
            developer_port: None,
            developer_bin: Some("/tmp/enscrive-developer-v2".to_string()),
            observe_bin: Some("/tmp/enscrive-observe-v2".to_string()),
            embed_bin: Some("/tmp/enscrive-embed-v2".to_string()),
            openai_api_key: Some("sk-second".to_string()),
            anthropic_api_key: None,
            voyage_api_key: None,
            nebius_api_key: None,
            bge_endpoint: None,
            bge_api_key: None,
            bge_model_name: None,
            set_default: true,
        })
        .await
        .unwrap();

        assert_eq!(
            read_env_value(&infra_env, "POSTGRES_PASSWORD").unwrap(),
            original_postgres_password
        );
        assert_eq!(
            read_env_value(&developer_env, "HMAC_PEPPER").unwrap(),
            original_hmac_pepper
        );
        assert_eq!(
            read_env_value(&developer_env, "AES_KEY").unwrap(),
            original_aes_key
        );
        assert_eq!(
            read_env_value(&observe_env, "LAB_SERVICE_SECRET").unwrap(),
            original_lab_secret
        );

        let profiles = load_profiles().unwrap();
        let local = profiles
            .profiles
            .get("local")
            .and_then(|profile| profile.local.as_ref())
            .unwrap();
        assert_eq!(local.keycloak.client_secret, original_client_secret);
        assert_eq!(local.binaries.developer, "/tmp/enscrive-developer-v2");
        assert_eq!(
            local.providers.credentials.openai_api_key.as_deref(),
            Some("sk-second")
        );
        assert!(local.providers.embedding.openai);
        assert!(local.providers.embedding.bge);
        assert!(local.providers.llm_inference.openai);
    }

    #[tokio::test]
    async fn init_self_managed_accepts_custom_developer_port() {
        let _guard = crate::test_support::lock_env();
        let temp = TempDir::new().unwrap();
        set_xdg(&temp);

        let result = init_self_managed(SelfManagedInitOptions {
            profile_name: Some("custom".to_string()),
            with_grafana: false,
            developer_port: Some(36300),
            developer_bin: Some("/tmp/enscrive-developer".to_string()),
            observe_bin: Some("/tmp/enscrive-observe".to_string()),
            embed_bin: Some("/tmp/enscrive-embed".to_string()),
            openai_api_key: Some("sk-test".to_string()),
            anthropic_api_key: None,
            voyage_api_key: None,
            nebius_api_key: None,
            bge_endpoint: None,
            bge_api_key: None,
            bge_model_name: None,
            set_default: false,
        })
        .await
        .unwrap();

        assert_eq!(result["endpoint"].as_str(), Some("http://127.0.0.1:36300"));

        let profiles = load_profiles().unwrap();
        let local = profiles
            .profiles
            .get("custom")
            .and_then(|profile| profile.local.as_ref())
            .unwrap();
        assert_eq!(local.ports.developer, 36300);

        let config_dir = cli_home()
            .unwrap()
            .config_root
            .join("profiles")
            .join("custom");
        let developer_env = fs::read_to_string(config_dir.join("developer.env")).unwrap();
        assert!(developer_env.contains("DEVELOPER_PORT=36300"));
        assert!(
            developer_env.contains("PORTAL_OIDC_REDIRECT_URI=http://127.0.0.1:36300/auth/callback")
        );
    }

    #[test]
    fn ensure_valid_developer_env_rewrites_invalid_aes_key() {
        let temp = TempDir::new().unwrap();
        let env_path = temp.path().join("developer.env");
        std::fs::write(
            &env_path,
            "HMAC_PEPPER=test-pepper\nAES_KEY=not-a-valid-key\nLOCAL_BOOTSTRAP_SECRET=test-secret\n",
        )
        .unwrap();

        ensure_valid_developer_env(&env_path, None, None).unwrap();

        let updated = std::fs::read_to_string(&env_path).unwrap();
        let aes_key = updated
            .lines()
            .find_map(|line| line.strip_prefix("AES_KEY="))
            .expect("rewritten developer.env should include AES_KEY");
        assert!(
            is_valid_aes_key(aes_key),
            "rewritten AES_KEY should be a 64-character hex string"
        );
    }

    #[test]
    fn infer_leptos_output_name_prefers_css_bundle_name() {
        let temp = TempDir::new().unwrap();
        let pkg_dir = temp.path().join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("enscrive-developer.css"), "body{}").unwrap();
        std::fs::write(pkg_dir.join("enscrive-developer.js"), "console.log('ok');").unwrap();

        assert_eq!(
            infer_leptos_output_name(temp.path()).as_deref(),
            Some("enscrive-developer")
        );
    }

    #[test]
    fn render_developer_env_writes_leptos_site_configuration() {
        let local = LocalProfile {
            deployment_mode: "local".to_string(),
            runtime_dir: "/tmp/runtime".to_string(),
            config_dir: "/tmp/config".to_string(),
            compose_file: "/tmp/docker-compose.yml".to_string(),
            infra_env_file: "/tmp/infra.env".to_string(),
            developer_env_file: "/tmp/developer.env".to_string(),
            observe_env_file: "/tmp/observe.env".to_string(),
            embed_env_file: "/tmp/embed.env".to_string(),
            log_dir: "/tmp/logs".to_string(),
            docker_project: "enscrive-local-local".to_string(),
            binaries: LocalBinaries {
                developer: "/tmp/enscrive-developer".to_string(),
                observe: "/tmp/enscrive-observe".to_string(),
                embed: "/tmp/enscrive-embed".to_string(),
            },
            ports: LocalPorts {
                developer: 3000,
                observe_rest: 8084,
                observe_grpc: 9090,
                embed_rest: 8081,
                embed_grpc: 50052,
                embed_metrics: 9000,
                postgres: 55432,
                qdrant_http: 6333,
                qdrant_grpc: 6334,
                keycloak: 8180,
                loki: 3100,
                grafana: None,
            },
            features: LocalFeatures {
                with_grafana: false,
                local_bge_management: false,
            },
            keycloak: LocalKeycloak {
                realm: "enscrive".to_string(),
                client_id: "enscrive-developer".to_string(),
                client_secret: "secret".to_string(),
                admin_username: "admin".to_string(),
                admin_password: "admin-pass".to_string(),
                developer_username: "developer".to_string(),
                developer_password: "developer-pass".to_string(),
            },
            bootstrap: LocalBootstrap {
                secret: "bootstrap-secret".to_string(),
                developer_email: "developer@local.enscrive".to_string(),
                developer_name: "Local Developer".to_string(),
                tenant_name: "Local Developer".to_string(),
                environment_name: "development".to_string(),
                api_key_label: "local-cli".to_string(),
                tenant_id: None,
                environment_id: None,
                api_key_id: None,
            },
            providers: LocalProviders::default(),
        };

        let rendered = render_developer_env(
            &local,
            "postgres-pass",
            "lab-secret",
            "pepper",
            &generate_hex_secret(32),
            Some(Path::new("/tmp/site")),
            "enscrive-developer",
        );

        assert!(rendered.contains("LEPTOS_OUTPUT_NAME=enscrive-developer"));
        assert!(rendered.contains("LEPTOS_SITE_ROOT=/tmp/site"));
        assert!(rendered.contains("LEPTOS_SITE_PKG_DIR=pkg"));
    }

    #[test]
    fn ensure_valid_developer_env_rewrites_stale_leptos_bundle_name() {
        let temp = TempDir::new().unwrap();
        let env_path = temp.path().join("developer.env");
        let site_root = temp.path().join("site");
        std::fs::create_dir_all(site_root.join("pkg")).unwrap();
        std::fs::write(
            &env_path,
            "HMAC_PEPPER=test-pepper\nAES_KEY=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\nLOCAL_BOOTSTRAP_SECRET=test-secret\nLEPTOS_OUTPUT_NAME=enscribe-developer\nLEPTOS_SITE_ROOT=/stale/site\nLEPTOS_SITE_PKG_DIR=assets\n",
        )
        .unwrap();

        ensure_valid_developer_env(
            &env_path,
            Some(site_root.as_path()),
            Some("enscrive-developer"),
        )
        .unwrap();

        let updated = std::fs::read_to_string(&env_path).unwrap();
        assert!(updated.contains("LEPTOS_OUTPUT_NAME=enscrive-developer"));
        assert!(updated.contains(&format!("LEPTOS_SITE_ROOT={}", site_root.display())));
        assert!(updated.contains("LEPTOS_SITE_PKG_DIR=pkg"));
    }

    #[test]
    fn recent_service_log_excerpt_returns_tail_lines() {
        let temp = TempDir::new().unwrap();
        let log_dir = temp.path().join("logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        std::fs::write(
            log_dir.join("enscrive-embed.log"),
            "line-1\nline-2\nline-3\nline-4\n",
        )
        .unwrap();

        let excerpt = recent_service_log_excerpt(&log_dir, "enscrive-embed", 2).unwrap();
        assert_eq!(excerpt, "line-3\nline-4");
    }

    #[test]
    fn format_service_start_error_includes_recent_log_excerpt() {
        let temp = TempDir::new().unwrap();
        let log_dir = temp.path().join("logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        std::fs::write(
            log_dir.join("enscrive-observe.log"),
            "observe booting\nobserve waiting\nobserve failed\n",
        )
        .unwrap();

        let error = format_service_start_error(
            "timed out waiting for HTTP readiness".to_string(),
            "enscrive-observe",
            &log_dir,
        );

        assert!(error.contains("timed out waiting for HTTP readiness"));
        assert!(error.contains("Last enscrive-observe log lines:"));
        assert!(error.contains("observe failed"));
    }

    #[test]
    fn local_embedding_provider_requires_real_embedding_backend() {
        let local = LocalProfile {
            deployment_mode: "local".to_string(),
            runtime_dir: "/tmp/runtime".to_string(),
            config_dir: "/tmp/config".to_string(),
            compose_file: "/tmp/docker-compose.yml".to_string(),
            infra_env_file: "/tmp/infra.env".to_string(),
            developer_env_file: "/tmp/developer.env".to_string(),
            observe_env_file: "/tmp/observe.env".to_string(),
            embed_env_file: "/tmp/embed.env".to_string(),
            log_dir: "/tmp/logs".to_string(),
            docker_project: "enscrive-local-local".to_string(),
            binaries: LocalBinaries {
                developer: "/tmp/enscrive-developer".to_string(),
                observe: "/tmp/enscrive-observe".to_string(),
                embed: "/tmp/enscrive-embed".to_string(),
            },
            ports: LocalPorts {
                developer: 3000,
                observe_rest: 8084,
                observe_grpc: 9090,
                embed_rest: 8081,
                embed_grpc: 50052,
                embed_metrics: 9000,
                postgres: 55432,
                qdrant_http: 6333,
                qdrant_grpc: 6334,
                keycloak: 8180,
                loki: 3100,
                grafana: None,
            },
            features: LocalFeatures {
                with_grafana: false,
                local_bge_management: false,
            },
            keycloak: LocalKeycloak {
                realm: "enscrive-local".to_string(),
                client_id: "client".to_string(),
                client_secret: "secret".to_string(),
                admin_username: "admin".to_string(),
                admin_password: "password".to_string(),
                developer_username: "developer".to_string(),
                developer_password: "password".to_string(),
            },
            bootstrap: LocalBootstrap::default(),
            providers: LocalProviders {
                credentials: LocalProviderCredentials {
                    anthropic_api_key: Some("anthropic-only".to_string()),
                    ..LocalProviderCredentials::default()
                },
                embedding: LocalEmbeddingProviders::default(),
                llm_inference: LocalLlmInferenceProviders {
                    anthropic: true,
                    ..LocalLlmInferenceProviders::default()
                },
            },
        };

        let error = ensure_local_embedding_provider(&local).unwrap_err();
        assert!(error.contains("requires at least one embedding provider"));

        let mut with_bge = local.clone();
        with_bge.providers.credentials.bge_endpoint = Some("http://127.0.0.1:8088".to_string());
        with_bge.providers.embedding.bge = true;
        ensure_local_embedding_provider(&with_bge).unwrap();
    }

    #[test]
    fn local_providers_deserialize_legacy_flat_shape() {
        let providers: LocalProviders = serde_json::from_str(
            r#"{
                "openai_api_key":"sk-legacy",
                "anthropic_api_key":"anth-legacy",
                "voyage_api_key":"voy-legacy",
                "nebius_api_key":"neb-legacy",
                "bge_endpoint":"http://127.0.0.1:8088",
                "bge_api_key":"bge-secret",
                "bge_model_name":"bge-large-en-v1.5"
            }"#,
        )
        .unwrap();

        assert_eq!(
            providers.credentials.openai_api_key.as_deref(),
            Some("sk-legacy")
        );
        assert!(providers.embedding.openai);
        assert!(providers.embedding.voyage);
        assert!(providers.embedding.nebius);
        assert!(providers.embedding.bge);
        assert!(providers.llm_inference.openai);
        assert!(providers.llm_inference.anthropic);
        assert_eq!(
            providers.credentials.bge_model_name.as_deref(),
            Some("bge-large-en-v1.5")
        );
    }

    #[test]
    fn detects_transient_local_postgres_errors() {
        assert!(is_transient_local_postgres_error(
            "check postgres database 'enscrive_developer' failed: psql: error: connection to server on socket \"/var/run/postgresql/.s.PGSQL.5432\" failed: FATAL:  the database system is shutting down"
        ));
        assert!(is_transient_local_postgres_error(
            "create postgres database 'enscrive_observe' failed: could not connect to server: Connection refused"
        ));
        assert!(!is_transient_local_postgres_error(
            "check postgres database 'enscrive_keycloak' failed: password authentication failed for user \"enscrive\""
        ));
    }
}
