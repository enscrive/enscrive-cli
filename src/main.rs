mod client;
mod deploy;
mod local;
mod output;
#[cfg(test)]
mod test_support;

use std::collections::HashMap;
use std::fs;

use clap::{ArgAction, Args, Parser, Subcommand};
use deploy::{
    DeployApplyOptions, DeployBootstrapOptions, DeployFetchOptions, DeployFetchSource,
    DeployInitOptions, DeployRenderOptions, DeploySecretsSource, DeployStatusOptions, DeployTarget,
    DeployVerifyOptions,
};
use local::{
    InitMode, ManagedInitOptions, SelfManagedInitOptions, StartOptions, StatusOptions, StopOptions,
};
use output::{
    CliResponse, EXIT_CONFIG, EXIT_FAILURE, EXIT_UNSUPPORTED, FailureClass, OutputFormat,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Parser)]
#[command(
    name = "enscrive",
    version,
    about = "Enscrive CLI - Perfect short term memory and limitless long term memory for humans and AI agents - Developer portal on localhost:3000 and enscrive.io"
)]
struct Cli {
    /// API key (or set ENSCRIVE_API_KEY)
    #[arg(long = "api-key", env = "ENSCRIVE_API_KEY", global = true)]
    api_key: Option<String>,

    /// Optional BYOK embedding provider key forwarded as X-Embedding-Provider-Key
    #[arg(
        long = "embedding-provider-key",
        env = "ENSCRIVE_EMBEDDING_PROVIDER_KEY",
        global = true
    )]
    embedding_provider_key: Option<String>,

    /// Base URL of enscrive-developer (or set ENSCRIVE_BASE_URL)
    #[arg(long = "endpoint", env = "ENSCRIVE_BASE_URL", global = true)]
    endpoint: Option<String>,

    /// Named CLI profile from ~/.config/enscrive/profiles.toml
    #[arg(long = "profile", env = "ENSCRIVE_PROFILE", global = true)]
    profile: Option<String>,

    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Human, global = true)]
    output: OutputFormat,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a managed or self-managed Enscrive profile
    Init(InitArgs),

    /// Start the local self-managed stack for the selected profile
    Start(StartArgs),

    /// Stop the local self-managed stack for the selected profile
    Stop(StopArgs),

    /// Show resolved profile and local stack status
    Status(StatusArgs),

    /// Operator-facing deployment profile and target commands
    Deploy {
        #[command(subcommand)]
        sub: DeploySubcommand,
    },

    /// Check stack health through /health
    Health,

    /// Search collections through /v1/search
    Search(SearchArgs),

    /// Embedding commands
    Embeddings {
        #[command(subcommand)]
        sub: EmbeddingsSubcommand,
    },

    /// Ingestion commands
    Ingest {
        #[command(subcommand)]
        sub: IngestSubcommand,
    },

    /// Segmentation commands
    Segment {
        #[command(subcommand)]
        sub: SegmentSubcommand,
    },

    /// Content analysis commands
    Analyze {
        #[command(subcommand)]
        sub: AnalyzeSubcommand,
    },

    /// Model discovery commands
    Models {
        #[command(subcommand)]
        sub: ModelsSubcommand,
    },

    /// Collection commands
    Collections {
        #[command(subcommand)]
        sub: CollectionsSubcommand,
    },

    /// Voice commands
    Voices {
        #[command(subcommand)]
        sub: VoicesSubcommand,
    },

    /// Evaluation commands
    Evals {
        #[command(subcommand)]
        sub: EvalsSubcommand,
    },

    /// Logs and observability commands
    Logs {
        #[command(subcommand)]
        sub: LogsSubcommand,
    },

    /// Backup and restore commands
    Backup {
        #[command(subcommand)]
        sub: BackupSubcommand,
    },

    /// Data export commands
    Export {
        #[command(subcommand)]
        sub: ExportSubcommand,
    },

    /// Usage and metering commands
    Usage(UsageArgs),

    /// Background job management commands
    Jobs {
        #[command(subcommand)]
        sub: JobsSubcommand,
    },
}

#[derive(Args)]
struct InitArgs {
    /// Initialization mode: managed or self-managed
    #[arg(long, value_enum)]
    mode: Option<InitMode>,

    /// Profile name to create or update
    #[arg(long = "profile-name")]
    profile_name: Option<String>,

    /// Enable Grafana in the local stack
    #[arg(long, default_value_t = false)]
    with_grafana: bool,

    /// Local port for enscrive-developer in self-managed mode
    #[arg(long = "developer-port")]
    developer_port: Option<u16>,

    /// Path to enscrive-developer binary for self-managed mode
    #[arg(long = "developer-bin")]
    developer_bin: Option<String>,

    /// Path to enscrive-observe binary for self-managed mode
    #[arg(long = "observe-bin")]
    observe_bin: Option<String>,

    /// Path to enscrive-embed binary for self-managed mode
    #[arg(long = "embed-bin")]
    embed_bin: Option<String>,

    /// Bring-your-own OpenAI key for local embeddings and optional LLM chunking
    #[arg(long = "openai-api-key")]
    openai_api_key: Option<String>,

    /// Bring-your-own Anthropic key for optional local LLM chunking
    #[arg(long = "anthropic-api-key")]
    anthropic_api_key: Option<String>,

    /// Bring-your-own Voyage key for local embeddings
    #[arg(long = "voyage-api-key")]
    voyage_api_key: Option<String>,

    /// Bring-your-own Nebius key for local Token Factory-backed embeddings
    #[arg(long = "nebius-api-key")]
    nebius_api_key: Option<String>,

    /// Optional local or LAN-hosted BGE embeddings endpoint
    #[arg(long = "bge-endpoint")]
    bge_endpoint: Option<String>,

    /// Optional bearer token for a secured BGE endpoint
    #[arg(long = "bge-api-key")]
    bge_api_key: Option<String>,

    /// Pinned BGE model name for the endpoint
    #[arg(long = "bge-model-name")]
    bge_model_name: Option<String>,

    /// Set this profile as the default CLI profile
    #[arg(long, default_value_t = false)]
    set_default: bool,
}

#[derive(Args)]
struct StartArgs {}

#[derive(Args)]
struct StopArgs {
    /// Remove local infrastructure containers instead of only stopping them
    #[arg(long, default_value_t = false)]
    remove_infra: bool,
}

#[derive(Args)]
struct StatusArgs {}

#[derive(Subcommand)]
enum DeploySubcommand {
    /// Initialize an operator-facing deploy profile for DEV/STAGE/US/EU/AP
    Init(DeployInitArgs),

    /// Show the configured deploy profile and current ESM detection status
    Status(DeployStatusArgs),

    /// Render deterministic managed-host artifacts for the selected deploy profile
    Render(DeployRenderArgs),

    /// Stage managed artifacts for the selected deploy profile from hosted manifest or local source builds
    Fetch(DeployFetchArgs),

    /// Install a rendered managed-host bundle onto the local host
    Apply(DeployApplyArgs),

    /// Verify the managed endpoint for the selected deploy profile through /health
    Verify(DeployVerifyArgs),

    /// Consume a signed bootstrap bundle and persist returned operator authority
    Bootstrap(DeployBootstrapArgs),
}

#[derive(Args)]
struct DeployInitArgs {
    /// Deploy target profile: dev, stage, us, eu, ap
    #[arg(long, value_enum)]
    target: Option<DeployTarget>,

    /// Managed API endpoint for this operator profile
    #[arg(long)]
    endpoint: Option<String>,

    /// Deploy profile name to create or update
    #[arg(long = "profile-name")]
    profile_name: Option<String>,

    /// Secrets source for operator rollout
    #[arg(long = "secrets-source", value_enum)]
    secrets_source: Option<DeploySecretsSource>,

    /// Set this deploy profile as the default operator target
    #[arg(long, default_value_t = false)]
    set_default: bool,
}

#[derive(Args)]
struct DeployStatusArgs {
    /// Deploy profile name to inspect
    #[arg(long = "profile-name")]
    profile_name: Option<String>,
}

#[derive(Args)]
struct DeployRenderArgs {
    /// Deploy profile name to render
    #[arg(long = "profile-name")]
    profile_name: Option<String>,

    /// Output directory for rendered artifacts
    #[arg(long = "out-dir")]
    out_dir: Option<String>,

    /// Host root expected on the managed instance
    #[arg(long = "host-root")]
    host_root: Option<String>,

    /// Trusted bootstrap public key to write into developer.env
    #[arg(long = "eba-trusted-public-key")]
    bootstrap_public_key: Option<String>,
}

#[derive(Args)]
struct DeployFetchArgs {
    /// Deploy profile name to fetch artifacts for
    #[arg(long = "profile-name")]
    profile_name: Option<String>,

    /// Artifact source: hosted manifest or locally built workspace artifacts
    #[arg(long, value_enum)]
    source: Option<DeployFetchSource>,

    /// Output directory for staged artifacts
    #[arg(long = "out-dir")]
    out_dir: Option<String>,

    /// Explicit release manifest URL
    #[arg(long = "manifest-url")]
    manifest_url: Option<String>,

    /// Enscrive workspace root for local-build artifact staging
    #[arg(long = "workspace-root")]
    workspace_root: Option<String>,

    /// Build local artifacts from source before staging them
    #[arg(long = "build", default_value_t = false)]
    build_local: bool,
}

#[derive(Args)]
struct DeployVerifyArgs {
    /// Deploy profile name to verify
    #[arg(long = "profile-name")]
    profile_name: Option<String>,

    /// Temporary endpoint override for verification
    #[arg(long = "endpoint")]
    endpoint: Option<String>,
}

#[derive(Args)]
struct DeployApplyArgs {
    /// Deploy profile name to apply
    #[arg(long = "profile-name")]
    profile_name: Option<String>,

    /// Directory containing the previously rendered bundle
    #[arg(long = "render-dir")]
    render_dir: Option<String>,

    /// Directory containing enscrive-developer, enscrive-observe, and enscrive-embed
    #[arg(long = "binary-dir")]
    binary_dir: Option<String>,

    /// Site root for the developer portal bundle (must contain pkg/)
    #[arg(long = "site-root")]
    site_root: Option<String>,

    /// Destination for installed systemd units
    #[arg(long = "systemd-dir")]
    systemd_dir: Option<String>,

    /// Destination for installed nginx config
    #[arg(long = "nginx-dir")]
    nginx_dir: Option<String>,

    /// Run systemctl daemon-reload after installing units
    #[arg(long, default_value_t = false)]
    reload_systemd: bool,

    /// Enable and start enscrive services after installation
    #[arg(long, default_value_t = false)]
    start_services: bool,

    /// Validate nginx config and reload nginx after installation
    #[arg(long, default_value_t = false)]
    reload_nginx: bool,
}

#[derive(Args)]
struct DeployBootstrapArgs {
    /// Deploy profile name to use
    #[arg(long = "profile-name")]
    profile_name: Option<String>,

    /// Bootstrap endpoint hosting /bootstrap/consume; use this for first bring-up or private access
    #[arg(long)]
    endpoint: Option<String>,

    /// Path to a signed bootstrap bundle TOML file
    #[arg(long = "bundle-path")]
    bundle_path: Option<String>,

    /// ESM secret key containing the signed bootstrap bundle
    #[arg(long = "bundle-secret-key")]
    bundle_secret_key: Option<String>,
}

#[derive(Args)]
struct SearchArgs {
    /// Search query text
    #[arg(long)]
    query: String,

    /// Optional collection ID
    #[arg(long)]
    collection: Option<String>,

    /// Number of results to return
    #[arg(long, default_value_t = 10)]
    limit: u32,

    /// Include vectors in the response
    #[arg(long, default_value_t = false)]
    include_vectors: bool,

    /// Optional score threshold
    #[arg(long)]
    score_threshold: Option<f32>,

    /// Optional search granularity
    #[arg(long)]
    granularity: Option<String>,

    /// Optional oversample factor
    #[arg(long)]
    oversample_factor: Option<u32>,

    /// Include below-threshold results when supported
    #[arg(long, default_value_t = false)]
    extended_results: bool,

    /// Optional minimum score for extended results
    #[arg(long)]
    score_floor: Option<f32>,

    #[arg(long)]
    filter_document_id: Option<String>,

    #[arg(long)]
    filter_user_id: Option<String>,

    #[arg(long)]
    filter_layer: Option<String>,

    #[arg(long)]
    filter_strategy: Option<String>,

    /// Metadata filter in key=value form. Pass multiple times as needed.
    #[arg(long = "metadata")]
    filter_metadata: Vec<String>,

    /// Hybrid search alpha: 0.0 = pure dense, 1.0 = pure BM25 sparse
    #[arg(long)]
    hybrid_alpha: Option<f32>,

    /// Target named vector resolution (e.g. "dense_256", "dense_512")
    #[arg(long)]
    resolution: Option<String>,
}

#[derive(Subcommand)]
enum EmbeddingsSubcommand {
    /// Generate query embeddings
    Query(EmbeddingsQueryArgs),
}

#[derive(Subcommand)]
enum IngestSubcommand {
    /// Ingest pre-segmented documents
    Prepared(IngestPreparedArgs),

    /// Ingest documents with automatic segmentation and embedding
    Documents(IngestDocumentsArgs),
}

#[derive(Subcommand)]
enum SegmentSubcommand {
    /// Run single-pass segmentation through /v1/segment-document
    Document(SegmentDocumentArgs),
}

#[derive(Subcommand)]
enum AnalyzeSubcommand {
    /// Analyze document content and recommend chunking strategy
    Content(ContentAnalysisArgs),
}

#[derive(Subcommand)]
enum ModelsSubcommand {
    /// List public embedding and chunking model names
    List,
}

#[derive(Args)]
struct EmbeddingsQueryArgs {
    /// Text to embed. Pass multiple times for batch requests.
    #[arg(long = "text", required = true)]
    texts: Vec<String>,

    /// Optional voice ID for voice-backed embeddings
    #[arg(long)]
    voice_id: Option<String>,

    /// Optional collection ID to resolve the collection embedding model
    #[arg(long)]
    collection: Option<String>,
}

#[derive(Args)]
struct IngestPreparedArgs {
    #[arg(long = "collection-id")]
    collection_id: String,

    #[arg(long = "document-id")]
    document_id: String,

    #[arg(long)]
    voice_id: Option<String>,

    /// JSON string containing an array of PreparedSegment objects
    #[arg(long, conflicts_with = "segments_file")]
    segments_json: Option<String>,

    /// Path to a JSON file containing an array of PreparedSegment objects
    #[arg(long, conflicts_with = "segments_json")]
    segments_file: Option<String>,
}

#[derive(Args)]
struct IngestDocumentsArgs {
    #[arg(long = "collection-id")]
    collection_id: String,

    /// Single document ID (for single-document ingest)
    #[arg(long = "document-id")]
    document_id: Option<String>,

    /// Content as inline text (single doc)
    #[arg(long, conflicts_with = "content_file", conflicts_with = "documents_json", conflicts_with = "documents_file")]
    content: Option<String>,

    /// Content from file (single doc)
    #[arg(long = "content-file", conflicts_with = "content", conflicts_with = "documents_json", conflicts_with = "documents_file")]
    content_file: Option<String>,

    /// Multiple documents as inline JSON array
    #[arg(long = "documents-json", conflicts_with = "content", conflicts_with = "content_file", conflicts_with = "documents_file")]
    documents_json: Option<String>,

    /// Multiple documents from JSON file
    #[arg(long = "documents-file", conflicts_with = "content", conflicts_with = "content_file", conflicts_with = "documents_json")]
    documents_file: Option<String>,

    #[arg(long = "voice-id")]
    voice_id: Option<String>,

    /// Run synchronously instead of as background job
    #[arg(long)]
    sync: bool,

    /// Disable batch embedding (force synchronous embedding)
    #[arg(long = "no-batch")]
    no_batch: bool,

    /// Preview without actually ingesting
    #[arg(long = "dry-run")]
    dry_run: bool,
}

#[derive(Args)]
struct SegmentDocumentArgs {
    #[arg(long)]
    voice_id: String,

    /// Inline content to segment
    #[arg(long, conflicts_with = "content_file")]
    content: Option<String>,

    /// Path to a file containing the content to segment
    #[arg(long, conflicts_with = "content")]
    content_file: Option<String>,
}

#[derive(Args)]
struct ContentAnalysisArgs {
    /// Inline content to analyze
    #[arg(long, conflicts_with = "content_file")]
    content: Option<String>,

    /// Path to a file containing the content to analyze
    #[arg(long, conflicts_with = "content")]
    content_file: Option<String>,
}

#[derive(Subcommand)]
enum CollectionsSubcommand {
    /// List collections
    List,

    /// Create a collection
    Create(CreateCollectionArgs),

    /// Update a collection
    Update(UpdateCollectionArgs),

    /// Delete a collection
    Delete {
        #[arg(long)]
        id: String,
    },

    /// Get collection stats
    Stats {
        #[arg(long)]
        id: String,
    },

    /// List documents in a collection
    Documents {
        #[arg(long)]
        id: String,
    },

    /// Get stored chunks for a document in a collection
    Chunks {
        #[arg(long = "collection-id")]
        collection_id: String,

        #[arg(long = "document-id")]
        document_id: String,

        #[arg(long, default_value_t = true, action = ArgAction::Set)]
        include_vectors: bool,

        #[arg(long, default_value_t = true, action = ArgAction::Set)]
        include_content: bool,
    },

    /// Get a single collection (not yet exposed on public /v1)
    Get {
        #[arg(long)]
        id: String,
    },

    /// Stage document changes for later commit
    Stage(CollectionsStageArgs),

    /// Commit staged changes to the collection
    Commit(CollectionsCommitArgs),

    /// List pending staged changes
    Pending(CollectionsPendingArgs),

    /// Delete a specific pending staged change
    PendingDelete(CollectionsPendingDeleteArgs),
}

#[derive(Args)]
struct CollectionsStageArgs {
    #[arg(long)]
    id: String,

    /// Documents to stage as inline JSON
    #[arg(long = "documents-json", conflicts_with = "documents_file")]
    documents_json: Option<String>,

    /// Documents to stage from JSON file
    #[arg(long = "documents-file", conflicts_with = "documents_json")]
    documents_file: Option<String>,

    /// Document IDs to delete (repeatable)
    #[arg(long = "delete")]
    deletes: Vec<String>,

    #[arg(long = "voice-id")]
    voice_id: Option<String>,
}

#[derive(Args)]
struct CollectionsCommitArgs {
    #[arg(long)]
    id: String,

    /// Force synchronous execution
    #[arg(long = "force-sync")]
    force_sync: bool,
}

#[derive(Args)]
struct CollectionsPendingArgs {
    #[arg(long)]
    id: String,
}

#[derive(Args)]
struct CollectionsPendingDeleteArgs {
    #[arg(long)]
    id: String,

    #[arg(long = "document-id")]
    document_id: String,
}

#[derive(Args)]
struct CreateCollectionArgs {
    #[arg(long)]
    name: String,

    #[arg(long)]
    embedding_model: String,

    #[arg(long)]
    description: Option<String>,

    #[arg(long)]
    dimensions: Option<u32>,
}

#[derive(Args)]
struct UpdateCollectionArgs {
    #[arg(long)]
    id: String,

    #[arg(long)]
    name: Option<String>,

    #[arg(long)]
    description: Option<String>,
}

#[derive(Subcommand)]
enum VoicesSubcommand {
    /// List voices
    List,

    /// Get a voice
    Get {
        #[arg(long)]
        id: String,
    },

    /// Create a voice
    Create(CreateVoiceArgs),

    /// Delete a voice
    Delete {
        #[arg(long)]
        id: String,
    },

    /// Compare two voices against the same query and collection
    Compare(VoiceCompareArgs),

    /// Promote a voice to another environment
    Promote(VoicePromoteArgs),

    /// Manage promotion gates for a voice
    Gates {
        #[command(subcommand)]
        sub: VoiceGatesSubcommand,
    },

    /// Search with a voice profile
    Search(VoiceSearchArgs),
}

const VOICE_CONFIG_SCHEMA_SUMMARY: &str = "expected VoiceConfigApi keys: chunking_strategy, parameters, optional template_id, score_threshold, default_limit, description, tags";
const EVAL_QUERY_ITEM_SCHEMA_SUMMARY: &str = "expected each EvalQueryItem to include query_id, query_text, relevant_doc_ids, relevance_scores, and optional collection_id, match_mode";
const CREATE_VOICE_AFTER_HELP: &str = r#"Voice config JSON schema:
  {
    "chunking_strategy": "story_beats",
    "parameters": {
      "model": "gpt-4o",
      "prompt_template": "conversational",
      "min_beat_length": "120",
      "max_beat_length": "600"
    },
    "template_id": "narrative-poetry-v1",
    "score_threshold": 0.72,
    "default_limit": 10,
    "description": "Narrative document voice",
    "tags": ["docs", "story"]
  }"#;
const EVAL_DATASET_AFTER_HELP: &str = r#"Expected eval dataset query JSON:
  [
    {
      "query_id": "q-1",
      "query_text": "Who blesses the water snakes?",
      "relevant_doc_ids": ["doc-1"],
      "relevance_scores": {"doc-1": 2},
      "collection_id": "collection-uuid",
      "match_mode": "document_prefix"
    }
  ]"#;
const RUN_EVAL_CAMPAIGN_AFTER_HELP: &str = r#"Query JSON uses the same EvalQueryItem schema as eval datasets.

Use --collection-id to set a campaign-level default collection for all queries that do not include collection_id.

Example query item:
  {
    "query_id": "q-1",
    "query_text": "Who blesses the water snakes?",
    "relevant_doc_ids": ["doc-1"],
    "relevance_scores": {"doc-1": 2},
    "collection_id": "collection-uuid",
    "match_mode": "document_prefix"
  }"#;

#[derive(Args)]
#[command(after_long_help = CREATE_VOICE_AFTER_HELP)]
struct CreateVoiceArgs {
    #[arg(long)]
    name: String,

    /// JSON string containing the VoiceConfigApi object
    #[arg(long, conflicts_with = "config_file")]
    config_json: Option<String>,

    /// Path to a JSON file containing the VoiceConfigApi object
    #[arg(long, conflicts_with = "config_json")]
    config_file: Option<String>,
}

#[derive(Args)]
struct VoiceCompareArgs {
    #[arg(long = "voice-a-id")]
    voice_a_id: String,

    #[arg(long = "voice-b-id")]
    voice_b_id: String,

    #[arg(long)]
    query: String,

    #[arg(long = "collection-id")]
    collection_id: String,

    #[arg(long, default_value_t = false)]
    include_vectors: bool,
}

#[derive(Args)]
struct VoicePromoteArgs {
    #[arg(long = "voice-id")]
    voice_id: String,

    #[arg(long = "target-environment-id")]
    target_environment_id: String,
}

#[derive(Subcommand)]
enum VoiceGatesSubcommand {
    /// List promotion gates for a voice
    List {
        #[arg(long = "voice-id")]
        voice_id: String,
    },

    /// Add or update a promotion gate
    Set(VoiceGateSetArgs),

    /// Delete a promotion gate
    Delete(VoiceGateDeleteArgs),
}

#[derive(Args)]
struct VoiceGateSetArgs {
    #[arg(long = "voice-id")]
    voice_id: String,

    #[arg(long)]
    metric: String,

    #[arg(long)]
    threshold: f64,

    #[arg(long)]
    operator: String,
}

#[derive(Args)]
struct VoiceGateDeleteArgs {
    #[arg(long = "voice-id")]
    voice_id: String,

    #[arg(long)]
    metric: String,
}

#[derive(Args)]
struct VoiceSearchArgs {
    #[arg(long)]
    query: String,

    #[arg(long)]
    voice_id: String,

    #[arg(long)]
    collection: Option<String>,

    #[arg(long, default_value_t = 10)]
    limit: u32,

    #[arg(long, default_value_t = false)]
    include_vectors: bool,

    #[arg(long)]
    score_threshold: Option<f32>,

    #[arg(long)]
    granularity: Option<String>,

    #[arg(long)]
    oversample_factor: Option<u32>,

    #[arg(long, default_value_t = false)]
    extended_results: bool,

    #[arg(long)]
    score_floor: Option<f32>,

    #[arg(long)]
    filter_document_id: Option<String>,

    #[arg(long)]
    filter_user_id: Option<String>,

    #[arg(long)]
    filter_layer: Option<String>,

    #[arg(long)]
    filter_strategy: Option<String>,

    /// Metadata filter in key=value form. Pass multiple times as needed.
    #[arg(long = "metadata")]
    filter_metadata: Vec<String>,

    /// Hybrid search alpha: 0.0 = pure dense, 1.0 = pure BM25 sparse
    #[arg(long)]
    hybrid_alpha: Option<f32>,

    /// Target named vector resolution (e.g. "dense_256", "dense_512")
    #[arg(long)]
    resolution: Option<String>,
}

#[derive(Subcommand)]
enum EvalsSubcommand {
    /// Eval campaign commands
    Campaigns {
        #[command(subcommand)]
        sub: EvalCampaignsSubcommand,
    },

    /// Run an eval campaign
    RunCampaign(RunEvalCampaignArgs),

    /// Run an eval campaign with SSE streaming
    RunCampaignStream(RunEvalCampaignArgs),

    /// Import benchmark data from standard formats (BEIR, MTEB, etc.)
    Import(ImportEvalsArgs),

    /// Eval dataset commands
    Datasets {
        #[command(subcommand)]
        sub: EvalDatasetsSubcommand,
    },

    /// Get latest promotion-gate status for a voice
    VoiceStatus {
        #[arg(long = "voice-id")]
        voice_id: String,
    },
}

#[derive(Subcommand)]
enum EvalCampaignsSubcommand {
    /// List eval campaigns
    List,

    /// Get an eval campaign
    Get {
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
enum EvalDatasetsSubcommand {
    /// List eval datasets
    List,

    /// Create an eval dataset
    Create(CreateEvalDatasetArgs),

    /// Get an eval dataset
    Get {
        #[arg(long)]
        id: String,
    },

    /// Fetch the flat queries + qrels view of an eval dataset
    /// (`GET /v1/evals/datasets/{id}/queries`)
    Queries {
        #[arg(long)]
        id: String,
    },

    /// Update an eval dataset
    Update(UpdateEvalDatasetArgs),

    /// Delete an eval dataset
    Delete {
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
enum LogsSubcommand {
    /// Stream logs through /v1/logs/stream
    Stream(LogStreamArgs),

    /// Search historical logs through /v1/logs/search
    Search(LogSearchArgs),

    /// Fetch performance metrics through /v1/logs/metrics
    Metrics(LogMetricsArgs),
}

#[derive(Subcommand)]
enum BackupSubcommand {
    /// Trigger a backup for the current tenant scope
    Create,

    /// List backups for the current tenant scope
    List(BackupListArgs),

    /// Get a single backup by ID
    Get {
        #[arg(long = "backup-id")]
        backup_id: String,
    },

    /// Restore tenant data to a target point in time
    Restore(BackupRestoreArgs),

    /// Validate a restore without executing it
    DryRun(BackupRestoreDryRunArgs),
}

#[derive(Args)]
struct BackupListArgs {
    #[arg(long)]
    limit: Option<u32>,
}

#[derive(Args)]
struct BackupRestoreArgs {
    #[arg(long = "target-time")]
    target_time: String,

    /// Required explicit acknowledgement for destructive restore execution
    #[arg(long, default_value_t = false)]
    confirm: bool,
}

#[derive(Args)]
struct BackupRestoreDryRunArgs {
    #[arg(long = "target-time")]
    target_time: String,
}

#[derive(Subcommand)]
enum ExportSubcommand {
    /// Export tenant data from the public portability endpoint
    Tenant(ExportTenantArgs),

    /// Export raw embedding records from the public admin surface
    Embeddings(ExportEmbeddingsArgs),

    /// Export granular token-usage records from the public admin surface
    TokenUsage(ExportTokenUsageArgs),
}

#[derive(Args)]
struct ExportTenantArgs {
    #[arg(long = "out-file")]
    out_file: String,

    #[arg(long, default_value_t = false)]
    include_vectors: bool,

    #[arg(long = "document-id")]
    document_id: Option<String>,

    #[arg(long)]
    layer: Option<String>,
}

#[derive(Args)]
struct ExportEmbeddingsArgs {
    #[arg(long)]
    user_id: Option<String>,

    #[arg(long = "document-id")]
    document_id: Option<String>,

    #[arg(long)]
    layer: Option<String>,

    #[arg(long = "conversation-id")]
    conversation_id: Option<String>,

    /// Restrict export to specific paragraph IDs. Pass multiple times as needed.
    #[arg(long = "paragraph-id")]
    paragraph_ids: Vec<String>,

    #[arg(long)]
    limit: Option<u32>,

    #[arg(long = "page-token")]
    page_token: Option<String>,

    #[arg(long, default_value_t = false)]
    include_vectors: bool,
}

#[derive(Args)]
struct ExportTokenUsageArgs {
    #[arg(long)]
    user_id: Option<String>,

    #[arg(long = "document-id")]
    document_id: Option<String>,

    #[arg(long)]
    layer: Option<String>,

    #[arg(long = "conversation-id")]
    conversation_id: Option<String>,

    /// Restrict export to specific paragraph IDs. Pass multiple times as needed.
    #[arg(long = "paragraph-id")]
    paragraph_ids: Vec<String>,

    #[arg(long)]
    limit: Option<u32>,

    #[arg(long = "page-token")]
    page_token: Option<String>,
}

#[derive(Args)]
struct LogStreamArgs {
    #[arg(long)]
    severity: Option<String>,

    #[arg(long)]
    operation: Option<String>,

    #[arg(long)]
    request_id: Option<String>,

    /// Max seconds to wait for stream output before returning
    #[arg(long, default_value_t = 10)]
    timeout_secs: u64,
}

#[derive(Args)]
struct LogSearchArgs {
    #[arg(long)]
    start_time: String,

    #[arg(long)]
    end_time: String,

    #[arg(long)]
    query: Option<String>,

    #[arg(long)]
    severity: Option<String>,

    #[arg(long)]
    operation: Option<String>,

    #[arg(long)]
    request_id: Option<String>,

    #[arg(long, default_value_t = 100)]
    limit: i32,

    #[arg(long)]
    page_token: Option<String>,
}

#[derive(Args)]
struct LogMetricsArgs {
    #[arg(long)]
    start_time: String,

    #[arg(long)]
    end_time: String,

    #[arg(long)]
    operation: Option<String>,

    #[arg(long, default_value = "minute")]
    granularity: String,
}

#[derive(Args)]
#[command(after_long_help = RUN_EVAL_CAMPAIGN_AFTER_HELP)]
struct RunEvalCampaignArgs {
    #[arg(long)]
    name: String,

    #[arg(long = "voice-id")]
    voice_id: String,

    #[arg(long = "dataset-id")]
    dataset_id: String,

    /// Metric name to compute. Pass multiple times as needed.
    #[arg(long = "metric", required = true)]
    metrics: Vec<String>,

    /// Campaign-level default collection ID for queries that do not include collection_id.
    #[arg(long = "collection-id")]
    collection_id: Option<String>,

    /// JSON string containing an array of EvalQueryItem objects
    #[arg(long, conflicts_with = "queries_file")]
    queries_json: Option<String>,

    /// Path to a JSON file containing an array of EvalQueryItem objects
    #[arg(long, conflicts_with = "queries_json")]
    queries_file: Option<String>,

    /// Campaign-level default match mode: exact or document_prefix
    #[arg(long = "match-mode")]
    match_mode: Option<String>,
}


#[derive(Args)]
struct ImportEvalsArgs {
    /// Import format (currently: "beir")
    #[arg(long, default_value = "beir")]
    format: String,

    /// Name for the created eval dataset
    #[arg(long = "dataset-name")]
    dataset_name: String,

    /// Path to queries file (BEIR: queries.jsonl)
    #[arg(long = "queries-file")]
    queries_file: String,

    /// Path to relevance judgments file (BEIR: qrels/test.tsv)
    #[arg(long = "qrels-file")]
    qrels_file: String,

    /// Optional: also ingest corpus into a collection
    #[arg(long = "corpus-file")]
    corpus_file: Option<String>,

    /// Collection ID for corpus ingestion (required if --corpus-file is provided)
    #[arg(long = "collection-id")]
    collection_id: Option<String>,

    /// Voice ID for corpus ingestion (optional, determines chunking strategy)
    #[arg(long = "voice-id")]
    voice_id: Option<String>,
}

#[derive(Args)]
#[command(after_long_help = EVAL_DATASET_AFTER_HELP)]
struct CreateEvalDatasetArgs {
    #[arg(long)]
    name: String,

    /// JSON string containing an array of EvalQueryItem objects
    #[arg(long, conflicts_with = "queries_file")]
    queries_json: Option<String>,

    /// Path to a JSON file containing an array of EvalQueryItem objects
    #[arg(long, conflicts_with = "queries_json")]
    queries_file: Option<String>,
}

#[derive(Args)]
#[command(after_long_help = EVAL_DATASET_AFTER_HELP)]
struct UpdateEvalDatasetArgs {
    #[arg(long)]
    id: String,

    #[arg(long)]
    name: String,

    #[arg(long, conflicts_with = "queries_file")]
    queries_json: Option<String>,

    #[arg(long, conflicts_with = "queries_json")]
    queries_file: Option<String>,
}

#[derive(Args)]
struct UsageArgs {
    /// RFC3339/ISO8601 start timestamp
    #[arg(long)]
    start_time: String,

    /// RFC3339/ISO8601 end timestamp
    #[arg(long)]
    end_time: String,

    #[arg(long)]
    document_id: Option<String>,

    #[arg(long)]
    collection_id: Option<String>,

    #[arg(long)]
    operation: Option<String>,

    #[arg(long)]
    embedding_model: Option<String>,

    #[arg(long)]
    limit: Option<i32>,

    #[arg(long)]
    page_token: Option<String>,
}

#[derive(Subcommand)]
enum JobsSubcommand {
    /// List background jobs
    List(JobsListArgs),

    /// Get details of a specific job
    Get(JobsGetArgs),

    /// Cancel a running job
    Cancel(JobsCancelArgs),
}

#[derive(Args)]
struct JobsListArgs {
    /// Filter by job status
    #[arg(long)]
    status: Option<String>,
}

#[derive(Args)]
struct JobsGetArgs {
    /// Job ID
    #[arg(long)]
    id: String,
}

#[derive(Args)]
struct JobsCancelArgs {
    /// Job ID
    #[arg(long)]
    id: String,
}

fn require_api_key(api_key: Option<String>, fmt: OutputFormat) -> String {
    match api_key {
        Some(key) if !key.is_empty() => key,
        _ => {
            CliResponse::fail(
                "",
                "API key required: set ENSCRIVE_API_KEY or pass --api-key".to_string(),
                FailureClass::Bug,
                EXIT_CONFIG,
            )
            .emit(fmt);
        }
    }
}

fn request_failure(command: &str, error: String) -> CliResponse {
    let error = augment_request_error(command, error);
    let lower = error.to_lowercase();
    if lower.contains("failedprecondition")
        || lower.contains("not yet supported")
        || lower.contains("not yet available on public /v1")
        || lower.contains("unsupported")
    {
        CliResponse::fail(command, error, FailureClass::Unsupported, EXIT_UNSUPPORTED)
    } else {
        CliResponse::fail(command, error, FailureClass::Bug, EXIT_FAILURE)
    }
}

fn augment_request_error(_command: &str, error: String) -> String {
    error
}

fn local_runtime_failure(command: &str, error: String) -> CliResponse {
    let lower = error.to_lowercase();
    if lower.contains("self-managed local mode requires docker")
        || lower.contains("docker compose unavailable")
        || lower.contains("cannot connect to the docker daemon")
        || lower.contains("permission denied while trying to connect to the docker daemon")
        || lower.contains("self-managed local mode requires at least one embedding provider")
    {
        CliResponse::fail(command, error, FailureClass::Unsupported, EXIT_CONFIG)
    } else {
        CliResponse::fail(command, error, FailureClass::Bug, EXIT_FAILURE)
    }
}

fn parse_config_source(
    config_json: &Option<String>,
    config_file: &Option<String>,
) -> Result<Value, String> {
    let raw = match (config_json, config_file) {
        (Some(json), None) => json.clone(),
        (None, Some(path)) => {
            fs::read_to_string(path).map_err(|e| format!("read config file '{}': {e}", path))?
        }
        (None, None) => {
            return Err("provide exactly one of --config-json or --config-file".to_string());
        }
        (Some(_), Some(_)) => {
            return Err("provide exactly one of --config-json or --config-file".to_string());
        }
    };

    let value: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("parse config JSON: {e}; {VOICE_CONFIG_SCHEMA_SUMMARY}"))?;
    let config: CliVoiceConfig = serde_json::from_value(value)
        .map_err(|e| format!("parse config JSON: {e}; {VOICE_CONFIG_SCHEMA_SUMMARY}"))?;
    serde_json::to_value(config).map_err(|e| format!("serialize config JSON: {e}"))
}

fn parse_json_source(
    inline_json: &Option<String>,
    file_path: &Option<String>,
    label: &str,
) -> Result<Value, String> {
    let raw = match (inline_json, file_path) {
        (Some(json), None) => json.clone(),
        (None, Some(path)) => {
            fs::read_to_string(path).map_err(|e| format!("read {label} file '{}': {e}", path))?
        }
        (None, None) => {
            return Err(format!(
                "provide exactly one of --{label}-json or --{label}-file"
            ));
        }
        (Some(_), Some(_)) => {
            return Err(format!(
                "provide exactly one of --{label}-json or --{label}-file"
            ));
        }
    };

    serde_json::from_str(&raw).map_err(|e| format!("parse {label} JSON: {e}"))
}

fn parse_segments_source(args: &IngestPreparedArgs) -> Result<Value, String> {
    let value = parse_json_source(&args.segments_json, &args.segments_file, "segments")?;
    let segments = coerce_prepared_segments(value)?;
    if segments.is_empty() {
        Err("segments array must not be empty".to_string())
    } else {
        serde_json::to_value(segments).map_err(|e| format!("serialize prepared segments: {e}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct CliPreparedSegment {
    content: String,
    label: String,
    confidence: f64,
    reasoning: String,
    start_paragraph: u32,
    end_paragraph: u32,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct CliVoiceConfig {
    chunking_strategy: String,
    parameters: HashMap<String, String>,
    #[serde(default)]
    template_id: Option<String>,
    #[serde(default)]
    score_threshold: Option<f32>,
    #[serde(default)]
    default_limit: Option<u32>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct CliEvalQueryItem {
    query_id: String,
    query_text: String,
    relevant_doc_ids: Vec<String>,
    relevance_scores: HashMap<String, i32>,
    #[serde(default)]
    collection_id: Option<String>,
    #[serde(default)]
    match_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CliSegmentInfo {
    index: u32,
    content: String,
    label: String,
    confidence: f64,
    reasoning: String,
    start_paragraph: u32,
    end_paragraph: u32,
    estimated_tokens: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CliSegmentCompletion {
    #[serde(default)]
    processing_time_ms: Option<u64>,
    #[serde(default)]
    template_name: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    total_paragraphs: Option<u32>,
}

#[derive(Debug, Clone)]
struct CliSseEvent {
    event: String,
    data: String,
}

impl CliSegmentInfo {
    fn into_prepared(self, completion: &CliSegmentCompletion) -> CliPreparedSegment {
        let mut metadata = HashMap::new();
        metadata.insert("segment_index".to_string(), self.index.to_string());
        metadata.insert(
            "estimated_tokens".to_string(),
            self.estimated_tokens.to_string(),
        );

        if let Some(template_name) = completion.template_name.as_ref() {
            metadata.insert("template_name".to_string(), template_name.clone());
        }
        if let Some(model) = completion.model.as_ref() {
            metadata.insert("segmentation_model".to_string(), model.clone());
        }
        if let Some(total_paragraphs) = completion.total_paragraphs {
            metadata.insert("total_paragraphs".to_string(), total_paragraphs.to_string());
        }
        if let Some(processing_time_ms) = completion.processing_time_ms {
            metadata.insert(
                "processing_time_ms".to_string(),
                processing_time_ms.to_string(),
            );
        }

        CliPreparedSegment {
            content: self.content,
            label: self.label,
            confidence: self.confidence,
            reasoning: self.reasoning,
            start_paragraph: self.start_paragraph,
            end_paragraph: self.end_paragraph,
            metadata,
        }
    }
}

fn coerce_prepared_segments(value: Value) -> Result<Vec<CliPreparedSegment>, String> {
    let normalized = unwrap_segments_value(value)?;

    if let Ok(segments) = serde_json::from_value::<Vec<CliSegmentInfo>>(normalized.clone()) {
        let completion = CliSegmentCompletion::default();
        return Ok(segments
            .into_iter()
            .map(|segment| segment.into_prepared(&completion))
            .collect());
    }

    if let Ok(segments) = serde_json::from_value::<Vec<CliPreparedSegment>>(normalized) {
        return Ok(segments);
    }

    Err(
        "segments JSON must be an array of prepared segments or segment-document output"
            .to_string(),
    )
}

fn unwrap_segments_value(value: Value) -> Result<Value, String> {
    match value {
        Value::Array(_) => Ok(value),
        Value::Object(mut object) => {
            if let Some(data) = object.remove("data") {
                return unwrap_segments_value(data);
            }
            if let Some(segments) = object
                .remove("prepared_segments")
                .or_else(|| object.remove("segments"))
            {
                return unwrap_segments_value(segments);
            }
            Err("segments JSON must be an array or object containing a segments array".to_string())
        }
        _ => Err("segments JSON must be an array".to_string()),
    }
}

fn parse_segment_sse(raw: &str) -> Result<Value, String> {
    let events = parse_sse_events(raw);
    let mut segment_infos = Vec::new();
    let mut completion = CliSegmentCompletion::default();

    for event in events {
        match event.event.as_str() {
            "segment" => {
                let segment: CliSegmentInfo = serde_json::from_str(&event.data)
                    .map_err(|e| format!("parse segment-document segment event: {e}"))?;
                segment_infos.push(segment);
            }
            "complete" => {
                completion = serde_json::from_str(&event.data)
                    .map_err(|e| format!("parse segment-document completion event: {e}"))?;
            }
            "error" => return Err(parse_segment_error_message(&event.data)),
            _ => {}
        }
    }

    let segments: Vec<CliPreparedSegment> = segment_infos
        .into_iter()
        .map(|segment| segment.into_prepared(&completion))
        .collect();

    serde_json::to_value(segments).map_err(|e| format!("serialize segment-document output: {e}"))
}

fn parse_sse_events(raw: &str) -> Vec<CliSseEvent> {
    raw.replace("\r\n", "\n")
        .split("\n\n")
        .filter_map(|block| {
            let mut event = "message".to_string();
            let mut data_lines = Vec::new();

            for line in block.lines().map(str::trim_end) {
                if let Some(rest) = line.strip_prefix("event:") {
                    event = rest.trim().to_string();
                } else if let Some(rest) = line.strip_prefix("data:") {
                    data_lines.push(rest.trim_start().to_string());
                }
            }

            if data_lines.is_empty() {
                None
            } else {
                Some(CliSseEvent {
                    event,
                    data: data_lines.join("\n"),
                })
            }
        })
        .collect()
}

fn parse_segment_error_message(raw: &str) -> String {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(mut object)) => object
            .remove("message")
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| raw.to_string()),
        _ => raw.to_string(),
    }
}

fn parse_text_source(
    content: &Option<String>,
    content_file: &Option<String>,
) -> Result<String, String> {
    match (content, content_file) {
        (Some(content), None) => Ok(content.clone()),
        (None, Some(path)) => {
            fs::read_to_string(path).map_err(|e| format!("read content file '{}': {e}", path))
        }
        (None, None) => Err("provide exactly one of --content or --content-file".to_string()),
        (Some(_), Some(_)) => Err("provide exactly one of --content or --content-file".to_string()),
    }
}

fn parse_content_source(args: &SegmentDocumentArgs) -> Result<String, String> {
    parse_text_source(&args.content, &args.content_file)
}

fn parse_analysis_source(args: &ContentAnalysisArgs) -> Result<String, String> {
    parse_text_source(&args.content, &args.content_file)
}

fn parse_eval_queries_source(
    queries_json: &Option<String>,
    queries_file: &Option<String>,
) -> Result<Value, String> {
    if queries_json.is_none() && queries_file.is_none() {
        return Ok(Value::Array(Vec::new()));
    }
    let value = parse_json_source(queries_json, queries_file, "queries")?;
    match value {
        Value::Array(_) => {
            let queries: Vec<CliEvalQueryItem> = serde_json::from_value(value).map_err(|e| {
                format!("parse queries JSON: {e}; {EVAL_QUERY_ITEM_SCHEMA_SUMMARY}")
            })?;
            for query in &queries {
                parse_eval_match_mode(&query.match_mode)
                    .map_err(|e| format!("{e}; {EVAL_QUERY_ITEM_SCHEMA_SUMMARY}"))?;
            }
            serde_json::to_value(queries).map_err(|e| format!("serialize queries JSON: {e}"))
        }
        _ => Err(format!(
            "queries JSON must be an array; {EVAL_QUERY_ITEM_SCHEMA_SUMMARY}"
        )),
    }
}

fn parse_eval_match_mode(raw: &Option<String>) -> Result<Option<String>, String> {
    match raw.as_deref() {
        None => Ok(None),
        Some("exact") => Ok(Some("exact".to_string())),
        Some("document_prefix") => Ok(Some("document_prefix".to_string())),
        Some(other) => Err(format!(
            "invalid --match-mode '{}': expected exact or document_prefix",
            other
        )),
    }
}

fn build_eval_campaign_body(args: &RunEvalCampaignArgs) -> Result<Value, String> {
    let queries = parse_eval_queries_source(&args.queries_json, &args.queries_file)?;
    let match_mode = parse_eval_match_mode(&args.match_mode)?;
    Ok(json!({
        "name": args.name,
        "voice_id": args.voice_id,
        "dataset_id": args.dataset_id,
        "metrics": args.metrics,
        "collection_id": args.collection_id,
        "queries": queries,
        "match_mode": match_mode,
    }))
}

fn parse_metadata_filters(entries: &[String]) -> Result<Map<String, Value>, String> {
    let mut metadata = Map::new();
    for entry in entries {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("invalid metadata filter '{}': expected key=value", entry))?;
        if key.is_empty() {
            return Err(format!("invalid metadata filter '{}': key is empty", entry));
        }
        metadata.insert(key.to_string(), Value::String(value.to_string()));
    }
    Ok(metadata)
}

fn build_search_body(args: &SearchArgs) -> Result<Value, String> {
    let metadata = parse_metadata_filters(&args.filter_metadata)?;
    let filters = if args.filter_document_id.is_some()
        || args.filter_user_id.is_some()
        || !metadata.is_empty()
        || args.filter_layer.is_some()
        || args.filter_strategy.is_some()
    {
        Some(json!({
            "document_id": args.filter_document_id,
            "user_id": args.filter_user_id,
            "metadata": metadata,
            "layer": args.filter_layer,
            "strategy": args.filter_strategy,
        }))
    } else {
        None
    };

    Ok(json!({
        "query": args.query,
        "collection_id": args.collection,
        "filters": filters,
        "limit": args.limit,
        "include_vectors": args.include_vectors,
        "score_threshold": args.score_threshold,
        "granularity": args.granularity,
        "oversample_factor": args.oversample_factor,
        "extended_results": args.extended_results,
        "score_floor": args.score_floor,
        "hybrid_alpha": args.hybrid_alpha,
        "resolution": args.resolution,
    }))
}

fn build_voice_search_body(args: &VoiceSearchArgs) -> Result<Value, String> {
    let metadata = parse_metadata_filters(&args.filter_metadata)?;
    let filters = if args.filter_document_id.is_some()
        || args.filter_user_id.is_some()
        || !metadata.is_empty()
        || args.filter_layer.is_some()
        || args.filter_strategy.is_some()
    {
        Some(json!({
            "document_id": args.filter_document_id,
            "user_id": args.filter_user_id,
            "metadata": metadata,
            "layer": args.filter_layer,
            "strategy": args.filter_strategy,
        }))
    } else {
        None
    };

    Ok(json!({
        "query": args.query,
        "voice_id": args.voice_id,
        "collection_id": args.collection,
        "limit": args.limit,
        "include_vectors": args.include_vectors,
        "filters": filters,
        "granularity": args.granularity,
        "oversample_factor": args.oversample_factor,
        "score_threshold": args.score_threshold,
        "extended_results": args.extended_results,
        "score_floor": args.score_floor,
        "hybrid_alpha": args.hybrid_alpha,
        "resolution": args.resolution,
    }))
}

fn build_usage_query(args: &UsageArgs) -> Vec<(&'static str, String)> {
    let mut query = vec![
        ("start_time", args.start_time.clone()),
        ("end_time", args.end_time.clone()),
    ];

    if let Some(value) = &args.document_id {
        query.push(("document_id", value.clone()));
    }
    if let Some(value) = &args.collection_id {
        query.push(("collection_id", value.clone()));
    }
    if let Some(value) = &args.operation {
        query.push(("operation", value.clone()));
    }
    if let Some(value) = &args.embedding_model {
        query.push(("embedding_model", value.clone()));
    }
    if let Some(value) = args.limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = &args.page_token {
        query.push(("page_token", value.clone()));
    }

    query
}

fn build_backup_list_query(args: &BackupListArgs) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(limit) = args.limit {
        query.push(("limit", limit.to_string()));
    }
    query
}

fn build_export_tenant_query(args: &ExportTenantArgs) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if args.include_vectors {
        query.push(("include_vectors", "true".to_string()));
    }
    if let Some(document_id) = &args.document_id {
        query.push(("document_id", document_id.clone()));
    }
    if let Some(layer) = &args.layer {
        query.push(("layer", layer.clone()));
    }
    query
}

fn build_export_embeddings_query(args: &ExportEmbeddingsArgs) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(value) = &args.user_id {
        query.push(("user_id", value.clone()));
    }
    if let Some(value) = &args.document_id {
        query.push(("document_id", value.clone()));
    }
    if let Some(value) = &args.layer {
        query.push(("layer", value.clone()));
    }
    if let Some(value) = &args.conversation_id {
        query.push(("conversation_id", value.clone()));
    }
    for value in &args.paragraph_ids {
        query.push(("paragraph_ids", value.clone()));
    }
    if let Some(value) = args.limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = &args.page_token {
        query.push(("page_token", value.clone()));
    }
    if args.include_vectors {
        query.push(("include_vectors", "true".to_string()));
    }
    query
}

fn build_export_token_usage_query(args: &ExportTokenUsageArgs) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    if let Some(value) = &args.user_id {
        query.push(("user_id", value.clone()));
    }
    if let Some(value) = &args.document_id {
        query.push(("document_id", value.clone()));
    }
    if let Some(value) = &args.layer {
        query.push(("layer", value.clone()));
    }
    if let Some(value) = &args.conversation_id {
        query.push(("conversation_id", value.clone()));
    }
    for value in &args.paragraph_ids {
        query.push(("paragraph_ids", value.clone()));
    }
    if let Some(value) = args.limit {
        query.push(("limit", value.to_string()));
    }
    if let Some(value) = &args.page_token {
        query.push(("page_token", value.clone()));
    }
    query
}

fn build_log_stream_query(args: &LogStreamArgs) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();

    if let Some(value) = &args.severity {
        query.push(("severity", value.clone()));
    }
    if let Some(value) = &args.operation {
        query.push(("operation", value.clone()));
    }
    if let Some(value) = &args.request_id {
        query.push(("request_id", value.clone()));
    }

    query
}

fn build_log_search_body(args: &LogSearchArgs) -> Value {
    json!({
        "query": args.query,
        "start_time": args.start_time,
        "end_time": args.end_time,
        "severity": args.severity,
        "operation": args.operation,
        "request_id": args.request_id,
        "limit": args.limit,
        "page_token": args.page_token,
    })
}

fn build_log_metrics_body(args: &LogMetricsArgs) -> Value {
    json!({
        "start_time": args.start_time,
        "end_time": args.end_time,
        "operation": args.operation,
        "granularity": args.granularity,
    })
}

fn local_prompt_mode() -> Result<InitMode, String> {
    use std::io::{self, Write};

    print!(
        "Initialize in managed with an enscrive.io API key or self-managed mode? [managed/self-managed]: "
    );
    io::stdout()
        .flush()
        .map_err(|e| format!("flush stdout: {e}"))?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| format!("read input: {e}"))?;
    match input.trim() {
        "managed" => Ok(InitMode::Managed),
        "self-managed" | "self_managed" | "local" => Ok(InitMode::SelfManaged),
        other => Err(format!(
            "invalid mode '{}': expected managed or self-managed",
            other
        )),
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let fmt = cli.output;
    let make_client = |endpoint: String, api_key: String| {
        client::EnscriveClient::new(endpoint, api_key, cli.embedding_provider_key.clone())
    };
    let api_context = match &cli.command {
        Commands::Init(_)
        | Commands::Start(_)
        | Commands::Stop(_)
        | Commands::Status(_)
        | Commands::Deploy { .. } => None,
        Commands::Health => None,
        Commands::Collections {
            sub: CollectionsSubcommand::Get { .. },
        } => None,
        _ => match local::resolve_api_context(
            cli.profile.as_deref(),
            cli.endpoint.clone(),
            cli.api_key.clone(),
        ) {
            Ok(ctx) => Some(ctx),
            Err(e) => CliResponse::fail("", e, FailureClass::Bug, EXIT_CONFIG).emit(fmt),
        },
    };

    match &cli.command {
        Commands::Init(args) => {
            let mode = match args.mode {
                Some(mode) => mode,
                None => {
                    let raw = match local_prompt_mode() {
                        Ok(mode) => mode,
                        Err(e) => {
                            CliResponse::fail("init", e, FailureClass::Bug, EXIT_CONFIG).emit(fmt)
                        }
                    };
                    raw
                }
            };

            let result = match mode {
                InitMode::Managed => {
                    local::init_managed(ManagedInitOptions {
                        profile_name: args.profile_name.clone(),
                        endpoint: cli.endpoint.clone(),
                        api_key: cli.api_key.clone(),
                        set_default: args.set_default,
                    })
                    .await
                }
                InitMode::SelfManaged => {
                    local::init_self_managed(SelfManagedInitOptions {
                        profile_name: args.profile_name.clone(),
                        with_grafana: args.with_grafana,
                        developer_port: args.developer_port,
                        developer_bin: args.developer_bin.clone(),
                        observe_bin: args.observe_bin.clone(),
                        embed_bin: args.embed_bin.clone(),
                        openai_api_key: args.openai_api_key.clone(),
                        anthropic_api_key: args.anthropic_api_key.clone(),
                        voyage_api_key: args.voyage_api_key.clone(),
                        nebius_api_key: args.nebius_api_key.clone(),
                        bge_endpoint: args.bge_endpoint.clone(),
                        bge_api_key: args.bge_api_key.clone(),
                        bge_model_name: args.bge_model_name.clone(),
                        set_default: args.set_default,
                    })
                    .await
                }
            };

            match result {
                Ok(data) => CliResponse::success("init", data).emit(fmt),
                Err(e) => CliResponse::fail("init", e, FailureClass::Bug, EXIT_CONFIG).emit(fmt),
            }
        }
        Commands::Start(_) => match local::start(StartOptions {
            profile_name: cli.profile.clone(),
        })
        .await
        {
            Ok(data) => CliResponse::success("start", data).emit(fmt),
            Err(e) => local_runtime_failure("start", e).emit(fmt),
        },
        Commands::Stop(args) => match local::stop(StopOptions {
            profile_name: cli.profile.clone(),
            remove_infra: args.remove_infra,
        })
        .await
        {
            Ok(data) => CliResponse::success("stop", data).emit(fmt),
            Err(e) => local_runtime_failure("stop", e).emit(fmt),
        },
        Commands::Status(_) => match local::status(StatusOptions {
            profile_name: cli.profile.clone(),
        })
        .await
        {
            Ok(data) => CliResponse::success("status", data).emit(fmt),
            Err(e) => CliResponse::fail("status", e, FailureClass::Bug, EXIT_FAILURE).emit(fmt),
        },
        Commands::Deploy { sub } => match sub {
            DeploySubcommand::Init(args) => {
                match deploy::init(DeployInitOptions {
                    target: args.target,
                    endpoint_override: args.endpoint.clone(),
                    profile_name: args.profile_name.clone(),
                    secrets_source: args.secrets_source,
                    set_default: args.set_default,
                })
                .await
                {
                    Ok(data) => CliResponse::success("deploy.init", data).emit(fmt),
                    Err(e) => CliResponse::fail("deploy.init", e, FailureClass::Bug, EXIT_CONFIG)
                        .emit(fmt),
                }
            }
            DeploySubcommand::Status(args) => {
                match deploy::status(DeployStatusOptions {
                    profile_name: args.profile_name.clone(),
                })
                .await
                {
                    Ok(data) => CliResponse::success("deploy.status", data).emit(fmt),
                    Err(e) => {
                        CliResponse::fail("deploy.status", e, FailureClass::Bug, EXIT_FAILURE)
                            .emit(fmt)
                    }
                }
            }
            DeploySubcommand::Render(args) => {
                match deploy::render(DeployRenderOptions {
                    profile_name: args.profile_name.clone(),
                    output_dir: args.out_dir.clone(),
                    host_root: args.host_root.clone(),
                    bootstrap_public_key: args.bootstrap_public_key.clone(),
                })
                .await
                {
                    Ok(data) => CliResponse::success("deploy.render", data).emit(fmt),
                    Err(e) => {
                        CliResponse::fail("deploy.render", e, FailureClass::Bug, EXIT_FAILURE)
                            .emit(fmt)
                    }
                }
            }
            DeploySubcommand::Fetch(args) => {
                match deploy::fetch(DeployFetchOptions {
                    profile_name: args.profile_name.clone(),
                    output_dir: args.out_dir.clone(),
                    manifest_url: args.manifest_url.clone(),
                    source: args.source,
                    workspace_root: args.workspace_root.clone(),
                    build_local: args.build_local,
                })
                .await
                {
                    Ok(data) => CliResponse::success("deploy.fetch", data).emit(fmt),
                    Err(e) => CliResponse::fail("deploy.fetch", e, FailureClass::Bug, EXIT_FAILURE)
                        .emit(fmt),
                }
            }
            DeploySubcommand::Apply(args) => {
                match deploy::apply(DeployApplyOptions {
                    profile_name: args.profile_name.clone(),
                    render_dir: args.render_dir.clone(),
                    binary_dir: args.binary_dir.clone(),
                    site_root: args.site_root.clone(),
                    systemd_dir: args.systemd_dir.clone(),
                    nginx_dir: args.nginx_dir.clone(),
                    reload_systemd: args.reload_systemd,
                    start_services: args.start_services,
                    reload_nginx: args.reload_nginx,
                })
                .await
                {
                    Ok(data) => CliResponse::success("deploy.apply", data).emit(fmt),
                    Err(e) => CliResponse::fail("deploy.apply", e, FailureClass::Bug, EXIT_FAILURE)
                        .emit(fmt),
                }
            }
            DeploySubcommand::Verify(args) => {
                match deploy::verify(DeployVerifyOptions {
                    profile_name: args.profile_name.clone(),
                    endpoint_override: args.endpoint.clone().or_else(|| cli.endpoint.clone()),
                })
                .await
                {
                    Ok(data) => CliResponse::success("deploy.verify", data).emit(fmt),
                    Err(e) => {
                        CliResponse::fail("deploy.verify", e, FailureClass::Bug, EXIT_FAILURE)
                            .emit(fmt)
                    }
                }
            }
            DeploySubcommand::Bootstrap(args) => {
                match deploy::bootstrap(DeployBootstrapOptions {
                    profile_name: args.profile_name.clone(),
                    endpoint_override: args.endpoint.clone().or_else(|| cli.endpoint.clone()),
                    bundle_path: args.bundle_path.clone(),
                    bundle_secret_key: args.bundle_secret_key.clone(),
                })
                .await
                {
                    Ok(data) => CliResponse::success("deploy.bootstrap", data).emit(fmt),
                    Err(e) => {
                        CliResponse::fail("deploy.bootstrap", e, FailureClass::Bug, EXIT_FAILURE)
                            .emit(fmt)
                    }
                }
            }
        },
        Commands::Health => {
            let endpoint = local::resolve_api_context(
                cli.profile.as_deref(),
                cli.endpoint.clone(),
                cli.api_key.clone(),
            )
            .map(|ctx| ctx.endpoint)
            .unwrap_or_else(|_| "http://localhost:3000".to_string());
            let client = make_client(endpoint, cli.api_key.clone().unwrap_or_default());
            match client.get_json("/health").await {
                Ok(data) => CliResponse::success("health", data).emit(fmt),
                Err(e) => request_failure("health", e).emit(fmt),
            }
        }
        Commands::Search(args) => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match build_search_body(args) {
                Ok(body) => match client.post_json("/v1/search", body).await {
                    Ok(data) => CliResponse::success("search", data).emit(fmt),
                    Err(e) => request_failure("search", e).emit(fmt),
                },
                Err(e) => CliResponse::fail("search", e, FailureClass::Bug, EXIT_CONFIG).emit(fmt),
            }
        }
        Commands::Embeddings { sub } => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match sub {
                EmbeddingsSubcommand::Query(args) => {
                    let body = json!({
                        "texts": args.texts,
                        "voice_id": args.voice_id,
                        "collection_id": args.collection,
                    });
                    match client.post_json("/v1/query-embeddings", body).await {
                        Ok(data) => CliResponse::success("embeddings query", data).emit(fmt),
                        Err(e) => request_failure("embeddings query", e).emit(fmt),
                    }
                }
            }
        }
        Commands::Ingest { sub } => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match sub {
                IngestSubcommand::Prepared(args) => match parse_segments_source(args) {
                    Ok(segments) => {
                        let body = json!({
                            "collection_id": args.collection_id,
                            "document_id": args.document_id,
                            "segments": segments,
                            "voice_id": args.voice_id,
                        });
                        match client.post_json("/v1/ingest-prepared", body).await {
                            Ok(data) => CliResponse::success("ingest prepared", data).emit(fmt),
                            Err(e) => request_failure("ingest prepared", e).emit(fmt),
                        }
                    }
                    Err(e) => {
                        CliResponse::fail("ingest prepared", e, FailureClass::Bug, EXIT_CONFIG)
                            .emit(fmt)
                    }
                },
                IngestSubcommand::Documents(args) => {
                    let documents = if let Some(ref content) = args.content {
                        let doc_id = args.document_id.clone().unwrap_or_default();
                        json!([{ "id": doc_id, "content": content, "metadata": {}, "fingerprint": "" }])
                    } else if let Some(ref content_file) = args.content_file {
                        match fs::read_to_string(content_file) {
                            Ok(content) => {
                                let doc_id = args.document_id.clone().unwrap_or_default();
                                json!([{ "id": doc_id, "content": content, "metadata": {}, "fingerprint": "" }])
                            }
                            Err(e) => {
                                CliResponse::fail(
                                    "ingest documents",
                                    format!("read content file '{}': {e}", content_file),
                                    FailureClass::Bug,
                                    EXIT_CONFIG,
                                )
                                .emit(fmt)
                            }
                        }
                    } else if args.documents_json.is_some() || args.documents_file.is_some() {
                        match parse_json_source(&args.documents_json, &args.documents_file, "documents") {
                            Ok(docs) => docs,
                            Err(e) => {
                                CliResponse::fail("ingest documents", e, FailureClass::Bug, EXIT_CONFIG)
                                    .emit(fmt)
                            }
                        }
                    } else {
                        CliResponse::fail(
                            "ingest documents",
                            "provide one of --content, --content-file, --documents-json, or --documents-file".to_string(),
                            FailureClass::Bug,
                            EXIT_CONFIG,
                        )
                        .emit(fmt)
                    };
                    let body = json!({
                        "collection_id": args.collection_id,
                        "documents": documents,
                        "voice_id": args.voice_id,
                        "dry_run": args.dry_run,
                        "sync": if args.sync { Some(true) } else { None::<bool> },
                        "no_batch": if args.no_batch { Some(true) } else { None::<bool> },
                    });
                    match client.post_json("/v1/ingest", body).await {
                        Ok(data) => CliResponse::success("ingest documents", data).emit(fmt),
                        Err(e) => request_failure("ingest documents", e).emit(fmt),
                    }
                }
            }
        }
        Commands::Segment { sub } => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match sub {
                SegmentSubcommand::Document(args) => match parse_content_source(args) {
                    Ok(content) => {
                        let body = json!({
                            "content": content,
                            "voice_id": args.voice_id,
                        });
                        match client
                            .post_text("/v1/segment-document", body, "text/event-stream")
                            .await
                        {
                            Ok(data) => match parse_segment_sse(&data) {
                                Ok(segments) => {
                                    CliResponse::success("segment document", segments).emit(fmt)
                                }
                                Err(e) => request_failure("segment document", e).emit(fmt),
                            },
                            Err(e) => request_failure("segment document", e).emit(fmt),
                        }
                    }
                    Err(e) => {
                        CliResponse::fail("segment document", e, FailureClass::Bug, EXIT_CONFIG)
                            .emit(fmt)
                    }
                },
            }
        }
        Commands::Analyze { sub } => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match sub {
                AnalyzeSubcommand::Content(args) => match parse_analysis_source(args) {
                    Ok(text) => {
                        let body = json!({ "text": text });
                        match client.post_json("/v1/analyze-content", body).await {
                            Ok(data) => CliResponse::success("analyze content", data).emit(fmt),
                            Err(e) => request_failure("analyze content", e).emit(fmt),
                        }
                    }
                    Err(e) => {
                        CliResponse::fail("analyze content", e, FailureClass::Bug, EXIT_CONFIG)
                            .emit(fmt)
                    }
                },
            }
        }
        Commands::Models { sub } => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match sub {
                ModelsSubcommand::List => match client.get_json("/v1/models").await {
                    Ok(data) => CliResponse::success("models list", data).emit(fmt),
                    Err(e) => request_failure("models list", e).emit(fmt),
                },
            }
        }
        Commands::Collections { sub } => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match sub {
                CollectionsSubcommand::List => match client.get_json("/v1/collections").await {
                    Ok(data) => CliResponse::success("collections list", data).emit(fmt),
                    Err(e) => request_failure("collections list", e).emit(fmt),
                },
                CollectionsSubcommand::Create(args) => {
                    let body = json!({
                        "name": args.name,
                        "embedding_model": args.embedding_model,
                        "description": args.description,
                        "dimensions": args.dimensions,
                    });
                    match client.post_json("/v1/collections", body).await {
                        Ok(data) => CliResponse::success("collections create", data).emit(fmt),
                        Err(e) => request_failure("collections create", e).emit(fmt),
                    }
                }
                CollectionsSubcommand::Update(args) => {
                    let body = json!({
                        "name": args.name,
                        "description": args.description,
                    });
                    let path = format!("/v1/collections/{}", args.id);
                    match client.patch_json(&path, body).await {
                        Ok(data) => CliResponse::success("collections update", data).emit(fmt),
                        Err(e) => request_failure("collections update", e).emit(fmt),
                    }
                }
                CollectionsSubcommand::Delete { id } => {
                    let path = format!("/v1/collections/{}", id);
                    match client.delete_json(&path).await {
                        Ok(data) => CliResponse::success("collections delete", data).emit(fmt),
                        Err(e) => request_failure("collections delete", e).emit(fmt),
                    }
                }
                CollectionsSubcommand::Stats { id } => {
                    let path = format!("/v1/collections/{}/stats", id);
                    match client.get_json(&path).await {
                        Ok(data) => CliResponse::success("collections stats", data).emit(fmt),
                        Err(e) => request_failure("collections stats", e).emit(fmt),
                    }
                }
                CollectionsSubcommand::Documents { id } => {
                    let path = format!("/v1/collections/{}/documents", id);
                    match client.get_json(&path).await {
                        Ok(data) => CliResponse::success("collections documents", data).emit(fmt),
                        Err(e) => request_failure("collections documents", e).emit(fmt),
                    }
                }
                CollectionsSubcommand::Chunks {
                    collection_id,
                    document_id,
                    include_vectors,
                    include_content,
                } => {
                    let path = format!(
                        "/v1/collections/{}/documents/{}/chunks?include_vectors={}&include_content={}",
                        collection_id, document_id, include_vectors, include_content
                    );
                    match client.get_json(&path).await {
                        Ok(data) => CliResponse::success("collections chunks", data).emit(fmt),
                        Err(e) => request_failure("collections chunks", e).emit(fmt),
                    }
                }
                CollectionsSubcommand::Get { .. } => {
                    CliResponse::unsupported(
                        "collections get",
                        "GET /v1/collections/{id} is not yet available on public /v1; use collections list or collections stats",
                    )
                    .emit(fmt);
                }
                CollectionsSubcommand::Stage(args) => {
                    let documents = if args.documents_json.is_some() || args.documents_file.is_some() {
                        match parse_json_source(&args.documents_json, &args.documents_file, "documents") {
                            Ok(docs) => docs,
                            Err(e) => {
                                CliResponse::fail("collections stage", e, FailureClass::Bug, EXIT_CONFIG)
                                    .emit(fmt)
                            }
                        }
                    } else {
                        json!([])
                    };
                    let body = json!({
                        "documents": documents,
                        "deletes": args.deletes,
                        "voice_id": args.voice_id,
                    });
                    let path = format!("/v1/collections/{}/stage", args.id);
                    match client.post_json(&path, body).await {
                        Ok(data) => CliResponse::success("collections stage", data).emit(fmt),
                        Err(e) => request_failure("collections stage", e).emit(fmt),
                    }
                }
                CollectionsSubcommand::Commit(args) => {
                    let body = if args.force_sync {
                        json!({ "force_sync": true })
                    } else {
                        json!({})
                    };
                    let path = format!("/v1/collections/{}/commit", args.id);
                    match client.post_json(&path, body).await {
                        Ok(data) => CliResponse::success("collections commit", data).emit(fmt),
                        Err(e) => request_failure("collections commit", e).emit(fmt),
                    }
                }
                CollectionsSubcommand::Pending(args) => {
                    let path = format!("/v1/collections/{}/pending", args.id);
                    match client.get_json(&path).await {
                        Ok(data) => CliResponse::success("collections pending", data).emit(fmt),
                        Err(e) => request_failure("collections pending", e).emit(fmt),
                    }
                }
                CollectionsSubcommand::PendingDelete(args) => {
                    let path = format!("/v1/collections/{}/pending/{}", args.id, args.document_id);
                    match client.delete_json(&path).await {
                        Ok(data) => CliResponse::success("collections pending delete", data).emit(fmt),
                        Err(e) => request_failure("collections pending delete", e).emit(fmt),
                    }
                }
            }
        }
        Commands::Voices { sub } => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match sub {
                VoicesSubcommand::List => match client.get_json("/v1/voices").await {
                    Ok(data) => CliResponse::success("voices list", data).emit(fmt),
                    Err(e) => request_failure("voices list", e).emit(fmt),
                },
                VoicesSubcommand::Get { id } => {
                    let path = format!("/v1/voices/{}", id);
                    match client.get_json(&path).await {
                        Ok(data) => CliResponse::success("voices get", data).emit(fmt),
                        Err(e) => request_failure("voices get", e).emit(fmt),
                    }
                }
                VoicesSubcommand::Create(args) => {
                    match parse_config_source(&args.config_json, &args.config_file) {
                        Ok(config) => {
                            let body = json!({
                                "name": args.name,
                                "config": config,
                            });
                            match client.post_json("/v1/voices", body).await {
                                Ok(data) => CliResponse::success("voices create", data).emit(fmt),
                                Err(e) => request_failure("voices create", e).emit(fmt),
                            }
                        }
                        Err(e) => {
                            CliResponse::fail("voices create", e, FailureClass::Bug, EXIT_CONFIG)
                                .emit(fmt)
                        }
                    }
                }
                VoicesSubcommand::Delete { id } => {
                    let path = format!("/v1/voices/{}", id);
                    match client.delete_json(&path).await {
                        Ok(data) => CliResponse::success("voices delete", data).emit(fmt),
                        Err(e) => request_failure("voices delete", e).emit(fmt),
                    }
                }
                VoicesSubcommand::Compare(args) => {
                    let body = json!({
                        "voice_a_id": args.voice_a_id,
                        "voice_b_id": args.voice_b_id,
                        "query": args.query,
                        "collection_id": args.collection_id,
                        "include_vectors": args.include_vectors,
                    });
                    match client.post_json("/v1/voices/compare", body).await {
                        Ok(data) => CliResponse::success("voices compare", data).emit(fmt),
                        Err(e) => request_failure("voices compare", e).emit(fmt),
                    }
                }
                VoicesSubcommand::Promote(args) => {
                    let path = format!("/v1/voices/{}/promote", args.voice_id);
                    let body = json!({
                        "target_environment_id": args.target_environment_id,
                    });
                    match client.post_json(&path, body).await {
                        Ok(data) => CliResponse::success("voices promote", data).emit(fmt),
                        Err(e) => request_failure("voices promote", e).emit(fmt),
                    }
                }
                VoicesSubcommand::Gates { sub } => match sub {
                    VoiceGatesSubcommand::List { voice_id } => {
                        let path = format!("/v1/voices/{}/gates", voice_id);
                        match client.get_json(&path).await {
                            Ok(data) => CliResponse::success("voices gates list", data).emit(fmt),
                            Err(e) => request_failure("voices gates list", e).emit(fmt),
                        }
                    }
                    VoiceGatesSubcommand::Set(args) => {
                        let path = format!("/v1/voices/{}/gates", args.voice_id);
                        let body = json!({
                            "metric": args.metric,
                            "threshold": args.threshold,
                            "operator": args.operator,
                        });
                        match client.post_json(&path, body).await {
                            Ok(data) => CliResponse::success("voices gates set", data).emit(fmt),
                            Err(e) => request_failure("voices gates set", e).emit(fmt),
                        }
                    }
                    VoiceGatesSubcommand::Delete(args) => {
                        let path = format!("/v1/voices/{}/gates/{}", args.voice_id, args.metric);
                        match client.delete_json(&path).await {
                            Ok(data) => CliResponse::success("voices gates delete", data).emit(fmt),
                            Err(e) => request_failure("voices gates delete", e).emit(fmt),
                        }
                    }
                },
                VoicesSubcommand::Search(args) => match build_voice_search_body(args) {
                    Ok(body) => match client.post_json("/v1/voices/search", body).await {
                        Ok(data) => CliResponse::success("voices search", data).emit(fmt),
                        Err(e) => request_failure("voices search", e).emit(fmt),
                    },
                    Err(e) => CliResponse::fail("voices search", e, FailureClass::Bug, EXIT_CONFIG)
                        .emit(fmt),
                },
            }
        }
        Commands::Evals { sub } => match sub {
            EvalsSubcommand::Campaigns { sub } => {
                let ctx = api_context.clone().unwrap();
                let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
                match sub {
                    EvalCampaignsSubcommand::List => {
                        match client.get_json("/v1/evals/campaigns").await {
                            Ok(data) => {
                                CliResponse::success("evals campaigns list", data).emit(fmt)
                            }
                            Err(e) => request_failure("evals campaigns list", e).emit(fmt),
                        }
                    }
                    EvalCampaignsSubcommand::Get { id } => {
                        let path = format!("/v1/evals/{}", id);
                        match client.get_json(&path).await {
                            Ok(data) => CliResponse::success("evals campaigns get", data).emit(fmt),
                            Err(e) => request_failure("evals campaigns get", e).emit(fmt),
                        }
                    }
                }
            }
            EvalsSubcommand::RunCampaign(args) => {
                let ctx = api_context.clone().unwrap();
                let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
                match build_eval_campaign_body(args) {
                    Ok(body) => match client.post_json("/v1/evals/run-campaign", body).await {
                        Ok(data) => CliResponse::success("evals run-campaign", data).emit(fmt),
                        Err(e) => request_failure("evals run-campaign", e).emit(fmt),
                    },
                    Err(e) => {
                        CliResponse::fail("evals run-campaign", e, FailureClass::Bug, EXIT_CONFIG)
                            .emit(fmt)
                    }
                }
            }
            EvalsSubcommand::RunCampaignStream(args) => {
                let ctx = api_context.clone().unwrap();
                let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
                match build_eval_campaign_body(args) {
                    Ok(body) => match client
                        .post_text("/v1/evals/run-campaign-stream", body, "text/event-stream")
                        .await
                    {
                        Ok(data) => {
                            CliResponse::success("evals run-campaign-stream", Value::String(data))
                                .emit(fmt)
                        }
                        Err(e) => request_failure("evals run-campaign-stream", e).emit(fmt),
                    },
                    Err(e) => CliResponse::fail(
                        "evals run-campaign-stream",
                        e,
                        FailureClass::Bug,
                        EXIT_CONFIG,
                    )
                    .emit(fmt),
                }
            }
            EvalsSubcommand::Import(args) => {
                let ctx = api_context.clone().unwrap();
                let client = make_client(ctx.endpoint.clone(), require_api_key(ctx.api_key, fmt));

                if args.format != "beir" {
                    CliResponse::fail(
                        "evals import",
                        format!("unsupported import format '{}': currently only 'beir' is supported", args.format),
                        FailureClass::Bug,
                        EXIT_CONFIG,
                    )
                    .emit(fmt);
                }

                // Parse BEIR queries file (queries.jsonl)
                let queries_raw = match fs::read_to_string(&args.queries_file) {
                    Ok(data) => data,
                    Err(e) => {
                        CliResponse::fail(
                            "evals import",
                            format!("read queries file '{}': {e}", args.queries_file),
                            FailureClass::Bug,
                            EXIT_CONFIG,
                        )
                        .emit(fmt)
                    }
                };

                let mut queries_map: HashMap<String, String> = HashMap::new();
                for line in queries_raw.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let parsed: Value = match serde_json::from_str(line) {
                        Ok(v) => v,
                        Err(e) => {
                            CliResponse::fail(
                                "evals import",
                                format!("parse queries line: {e}"),
                                FailureClass::Bug,
                                EXIT_CONFIG,
                            )
                            .emit(fmt)
                        }
                    };
                    let id = parsed["_id"].as_str().unwrap_or("").to_string();
                    let text = parsed["text"].as_str().unwrap_or("").to_string();
                    if !id.is_empty() && !text.is_empty() {
                        queries_map.insert(id, text);
                    }
                }

                // Parse BEIR qrels file (TSV)
                let qrels_raw = match fs::read_to_string(&args.qrels_file) {
                    Ok(data) => data,
                    Err(e) => {
                        CliResponse::fail(
                            "evals import",
                            format!("read qrels file '{}': {e}", args.qrels_file),
                            FailureClass::Bug,
                            EXIT_CONFIG,
                        )
                        .emit(fmt)
                    }
                };

                // qrels: query_id -> { doc_id -> score }
                let mut qrels: HashMap<String, HashMap<String, i64>> = HashMap::new();
                for (i, line) in qrels_raw.lines().enumerate() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    // Skip header line
                    if i == 0 && (line.starts_with("query-id") || line.starts_with("query_id")) {
                        continue;
                    }
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() < 3 {
                        continue;
                    }
                    let query_id = parts[0].to_string();
                    let doc_id = parts[1].to_string();
                    let score: i64 = parts[parts.len() - 1].trim().parse().unwrap_or(0);
                    qrels.entry(query_id).or_default().insert(doc_id, score);
                }

                // Join queries + qrels into eval dataset format
                let mut eval_queries: Vec<Value> = Vec::new();
                for (query_id, judgments) in &qrels {
                    let query_text = match queries_map.get(query_id) {
                        Some(text) => text.clone(),
                        None => continue, // skip qrels without matching query
                    };
                    let relevant_doc_ids: Vec<String> = judgments
                        .iter()
                        .filter(|(_, score)| **score > 0)
                        .map(|(doc_id, _)| doc_id.clone())
                        .collect();
                    if relevant_doc_ids.is_empty() {
                        continue;
                    }
                    let relevance_scores: serde_json::Map<String, Value> = judgments
                        .iter()
                        .map(|(doc_id, &score)| (doc_id.clone(), json!(score)))
                        .collect();
                    eval_queries.push(json!({
                        "query_id": query_id,
                        "query_text": query_text,
                        "relevant_doc_ids": relevant_doc_ids,
                        "relevance_scores": relevance_scores,
                    }));
                }

                if eval_queries.is_empty() {
                    CliResponse::fail(
                        "evals import",
                        "no valid query-relevance pairs found after joining queries and qrels".to_string(),
                        FailureClass::Bug,
                        EXIT_CONFIG,
                    )
                    .emit(fmt);
                }

                // Create the eval dataset via API
                let body = json!({
                    "name": args.dataset_name,
                    "queries": eval_queries,
                });
                let dataset_result = match client.post_json("/v1/evals/datasets", body).await {
                    Ok(data) => data,
                    Err(e) => {
                        request_failure("evals import", e).emit(fmt)
                    }
                };

                let query_count = eval_queries.len();

                // Optionally ingest corpus
                if let Some(corpus_path) = &args.corpus_file {
                    let collection_id = match &args.collection_id {
                        Some(id) => id.clone(),
                        None => {
                            CliResponse::fail(
                                "evals import",
                                "--collection-id is required when --corpus-file is provided".to_string(),
                                FailureClass::Bug,
                                EXIT_CONFIG,
                            )
                            .emit(fmt)
                        }
                    };

                    let corpus_raw = match fs::read_to_string(corpus_path) {
                        Ok(data) => data,
                        Err(e) => {
                            CliResponse::fail(
                                "evals import",
                                format!("read corpus file '{}': {e}", corpus_path),
                                FailureClass::Bug,
                                EXIT_CONFIG,
                            )
                            .emit(fmt)
                        }
                    };

                    let mut documents: Vec<Value> = Vec::new();
                    for line in corpus_raw.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        let parsed: Value = match serde_json::from_str(line) {
                            Ok(v) => v,
                            Err(e) => {
                                CliResponse::fail(
                                    "evals import",
                                    format!("parse corpus line: {e}"),
                                    FailureClass::Bug,
                                    EXIT_CONFIG,
                                )
                                .emit(fmt)
                            }
                        };
                        let id = parsed["_id"].as_str().unwrap_or("").to_string();
                        let title = parsed["title"].as_str().unwrap_or("").to_string();
                        let text = parsed["text"].as_str().unwrap_or("").to_string();
                        let content = if title.is_empty() {
                            text
                        } else {
                            format!("{title} {text}")
                        };
                        if !id.is_empty() && !content.is_empty() {
                            documents.push(json!({
                                "id": id,
                                "content": content,
                                "metadata": {},
                            }));
                        }
                    }

                    let batch_size = 50;
                    let total_batches = documents.len().div_ceil(batch_size);
                    let mut ingested = 0;

                    for (batch_idx, batch) in documents.chunks(batch_size).enumerate() {
                        eprintln!("Ingesting batch {}/{}...", batch_idx + 1, total_batches);
                        let body = json!({
                            "collection_id": collection_id,
                            "voice_id": args.voice_id,
                            "documents": batch,
                            "dry_run": false,
                        });
                        match client.post_json("/v1/ingest", body).await {
                            Ok(_) => {
                                ingested += batch.len();
                            }
                            Err(e) => {
                                CliResponse::fail(
                                    "evals import",
                                    format!("ingest batch {}: {e}", batch_idx + 1),
                                    FailureClass::Bug,
                                    EXIT_FAILURE,
                                )
                                .emit(fmt);
                            }
                        }
                    }

                    CliResponse::success(
                        "evals import",
                        json!({
                            "dataset": dataset_result,
                            "query_count": query_count,
                            "corpus_ingested": true,
                            "documents_ingested": ingested,
                        }),
                    )
                    .emit(fmt);
                } else {
                    CliResponse::success(
                        "evals import",
                        json!({
                            "dataset": dataset_result,
                            "query_count": query_count,
                        }),
                    )
                    .emit(fmt);
                }
            }
            EvalsSubcommand::Datasets { sub } => {
                let ctx = api_context.clone().unwrap();
                let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
                match sub {
                    EvalDatasetsSubcommand::List => {
                        match client.get_json("/v1/evals/datasets").await {
                            Ok(data) => CliResponse::success("evals datasets list", data).emit(fmt),
                            Err(e) => request_failure("evals datasets list", e).emit(fmt),
                        }
                    }
                    EvalDatasetsSubcommand::Create(args) => {
                        match parse_eval_queries_source(&args.queries_json, &args.queries_file) {
                            Ok(queries) => {
                                let body = json!({
                                    "name": args.name,
                                    "queries": queries,
                                });
                                match client.post_json("/v1/evals/datasets", body).await {
                                    Ok(data) => CliResponse::success("evals datasets create", data)
                                        .emit(fmt),
                                    Err(e) => request_failure("evals datasets create", e).emit(fmt),
                                }
                            }
                            Err(e) => CliResponse::fail(
                                "evals datasets create",
                                e,
                                FailureClass::Bug,
                                EXIT_CONFIG,
                            )
                            .emit(fmt),
                        }
                    }
                    EvalDatasetsSubcommand::Get { id } => {
                        let path = format!("/v1/evals/datasets/{}", id);
                        match client.get_json(&path).await {
                            Ok(data) => CliResponse::success("evals datasets get", data).emit(fmt),
                            Err(e) => request_failure("evals datasets get", e).emit(fmt),
                        }
                    }
                    EvalDatasetsSubcommand::Queries { id } => {
                        let path = format!("/v1/evals/datasets/{}/queries", id);
                        match client.get_json(&path).await {
                            Ok(data) => {
                                CliResponse::success("evals datasets queries", data).emit(fmt)
                            }
                            Err(e) => request_failure("evals datasets queries", e).emit(fmt),
                        }
                    }
                    EvalDatasetsSubcommand::Update(args) => {
                        match parse_eval_queries_source(&args.queries_json, &args.queries_file) {
                            Ok(queries) => {
                                let body = json!({
                                    "name": args.name,
                                    "queries": queries,
                                });
                                let path = format!("/v1/evals/datasets/{}", args.id);
                                match client.put_json(&path, body).await {
                                    Ok(data) => CliResponse::success("evals datasets update", data)
                                        .emit(fmt),
                                    Err(e) => request_failure("evals datasets update", e).emit(fmt),
                                }
                            }
                            Err(e) => CliResponse::fail(
                                "evals datasets update",
                                e,
                                FailureClass::Bug,
                                EXIT_CONFIG,
                            )
                            .emit(fmt),
                        }
                    }
                    EvalDatasetsSubcommand::Delete { id } => {
                        let path = format!("/v1/evals/datasets/{}", id);
                        match client.delete_json(&path).await {
                            Ok(data) => {
                                CliResponse::success("evals datasets delete", data).emit(fmt)
                            }
                            Err(e) => request_failure("evals datasets delete", e).emit(fmt),
                        }
                    }
                }
            }
            EvalsSubcommand::VoiceStatus { voice_id } => {
                let ctx = api_context.clone().unwrap();
                let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
                let path = format!("/v1/evals/voice-status/{}", voice_id);
                match client.get_json(&path).await {
                    Ok(data) => CliResponse::success("evals voice-status", data).emit(fmt),
                    Err(e) => request_failure("evals voice-status", e).emit(fmt),
                }
            }
        },
        Commands::Backup { sub } => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match sub {
                BackupSubcommand::Create => {
                    match client.post_json("/v1/admin/backups", json!({})).await {
                        Ok(data) => CliResponse::success("backup create", data).emit(fmt),
                        Err(e) => request_failure("backup create", e).emit(fmt),
                    }
                }
                BackupSubcommand::List(args) => {
                    let query = build_backup_list_query(args);
                    match client
                        .get_json_with_query("/v1/admin/backups", &query)
                        .await
                    {
                        Ok(data) => CliResponse::success("backup list", data).emit(fmt),
                        Err(e) => request_failure("backup list", e).emit(fmt),
                    }
                }
                BackupSubcommand::Get { backup_id } => {
                    let path = format!("/v1/admin/backups/{}", backup_id);
                    match client.get_json(&path).await {
                        Ok(data) => CliResponse::success("backup get", data).emit(fmt),
                        Err(e) => request_failure("backup get", e).emit(fmt),
                    }
                }
                BackupSubcommand::Restore(args) => {
                    if !args.confirm {
                        CliResponse::fail(
                            "backup restore",
                            "restore requires --confirm".to_string(),
                            FailureClass::Bug,
                            EXIT_CONFIG,
                        )
                        .emit(fmt);
                    }

                    let body = json!({
                        "target_time": args.target_time,
                        "confirm": true,
                    });
                    match client.post_json("/v1/admin/restore", body).await {
                        Ok(data) => CliResponse::success("backup restore", data).emit(fmt),
                        Err(e) => request_failure("backup restore", e).emit(fmt),
                    }
                }
                BackupSubcommand::DryRun(args) => {
                    let body = json!({
                        "target_time": args.target_time,
                    });
                    match client.post_json("/v1/admin/restore/dry-run", body).await {
                        Ok(data) => CliResponse::success("backup dry-run", data).emit(fmt),
                        Err(e) => request_failure("backup dry-run", e).emit(fmt),
                    }
                }
            }
        }
        Commands::Export { sub } => match sub {
            ExportSubcommand::Tenant(args) => {
                let ctx = api_context.clone().unwrap();
                let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
                let query = build_export_tenant_query(args);
                match client
                    .get_bytes_with_query("/v1/admin/export", &query, "application/octet-stream")
                    .await
                {
                    Ok(response) => match fs::write(&args.out_file, &response.content) {
                        Ok(()) => CliResponse::success(
                            "export tenant",
                            json!({
                                "out_file": args.out_file,
                                "bytes_written": response.content.len(),
                                "content_type": response.content_type,
                                "content_disposition": response.content_disposition,
                                "include_vectors": args.include_vectors,
                                "document_id": args.document_id,
                                "layer": args.layer,
                            }),
                        )
                        .emit(fmt),
                        Err(e) => CliResponse::fail(
                            "export tenant",
                            format!("write export file '{}': {e}", args.out_file),
                            FailureClass::Bug,
                            EXIT_CONFIG,
                        )
                        .emit(fmt),
                    },
                    Err(e) => request_failure("export tenant", e).emit(fmt),
                }
            }
            ExportSubcommand::Embeddings(args) => {
                let ctx = api_context.clone().unwrap();
                let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
                let query = build_export_embeddings_query(args);
                match client
                    .get_json_with_query("/v1/admin/export/embeddings", &query)
                    .await
                {
                    Ok(data) => CliResponse::success("export embeddings", data).emit(fmt),
                    Err(e) => request_failure("export embeddings", e).emit(fmt),
                }
            }
            ExportSubcommand::TokenUsage(args) => {
                let ctx = api_context.clone().unwrap();
                let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
                let query = build_export_token_usage_query(args);
                match client
                    .get_json_with_query("/v1/admin/export/token-usage", &query)
                    .await
                {
                    Ok(data) => CliResponse::success("export token-usage", data).emit(fmt),
                    Err(e) => request_failure("export token-usage", e).emit(fmt),
                }
            }
        },
        Commands::Logs { sub } => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match sub {
                LogsSubcommand::Stream(args) => {
                    let query = build_log_stream_query(args);
                    match client
                        .get_text_with_query(
                            "/v1/logs/stream",
                            &query,
                            "text/event-stream",
                            Some(args.timeout_secs),
                        )
                        .await
                    {
                        Ok(data) => {
                            CliResponse::success("logs stream", Value::String(data)).emit(fmt)
                        }
                        Err(e) => request_failure("logs stream", e).emit(fmt),
                    }
                }
                LogsSubcommand::Search(args) => {
                    let body = build_log_search_body(args);
                    match client.post_json("/v1/logs/search", body).await {
                        Ok(data) => CliResponse::success("logs search", data).emit(fmt),
                        Err(e) => request_failure("logs search", e).emit(fmt),
                    }
                }
                LogsSubcommand::Metrics(args) => {
                    let body = build_log_metrics_body(args);
                    match client.post_json("/v1/logs/metrics", body).await {
                        Ok(data) => CliResponse::success("logs metrics", data).emit(fmt),
                        Err(e) => request_failure("logs metrics", e).emit(fmt),
                    }
                }
            }
        }
        Commands::Usage(args) => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            let query = build_usage_query(args);
            match client.get_json_with_query("/v1/usage", &query).await {
                Ok(data) => CliResponse::success("usage", data).emit(fmt),
                Err(e) => request_failure("usage", e).emit(fmt),
            }
        }
        Commands::Jobs { sub } => {
            let ctx = api_context.clone().unwrap();
            let client = make_client(ctx.endpoint, require_api_key(ctx.api_key, fmt));
            match sub {
                JobsSubcommand::List(args) => {
                    let mut query = vec![];
                    if let Some(ref s) = args.status {
                        query.push(("status", s.clone()));
                    }
                    match client.get_json_with_query("/v1/jobs", &query).await {
                        Ok(data) => CliResponse::success("jobs list", data).emit(fmt),
                        Err(e) => request_failure("jobs list", e).emit(fmt),
                    }
                }
                JobsSubcommand::Get(args) => {
                    let path = format!("/v1/jobs/{}", args.id);
                    match client.get_json(&path).await {
                        Ok(data) => CliResponse::success("jobs get", data).emit(fmt),
                        Err(e) => request_failure("jobs get", e).emit(fmt),
                    }
                }
                JobsSubcommand::Cancel(args) => {
                    let path = format!("/v1/jobs/{}/cancel", args.id);
                    match client.post_json(&path, json!({})).await {
                        Ok(data) => CliResponse::success("jobs cancel", data).emit(fmt),
                        Err(e) => request_failure("jobs cancel", e).emit(fmt),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_health_command() {
        let args = Cli::parse_from(["enscrive", "health"]);
        match args.command {
            Commands::Health => {}
            _ => panic!("expected health"),
        }
    }

    #[test]
    fn parse_deploy_init_command() {
        let args = Cli::parse_from([
            "enscrive",
            "deploy",
            "init",
            "--target",
            "stage",
            "--endpoint",
            "https://stage.api.enscrive.io",
            "--secrets-source",
            "esm",
            "--set-default",
        ]);
        match args.command {
            Commands::Deploy {
                sub:
                    DeploySubcommand::Init(DeployInitArgs {
                        target,
                        endpoint,
                        secrets_source,
                        set_default,
                        ..
                    }),
            } => {
                assert_eq!(target, Some(DeployTarget::Stage));
                assert_eq!(endpoint.as_deref(), Some("https://stage.api.enscrive.io"));
                assert_eq!(secrets_source, Some(DeploySecretsSource::Esm));
                assert!(set_default);
            }
            _ => panic!("expected deploy init"),
        }
    }

    #[test]
    fn parse_self_managed_init_with_custom_developer_port() {
        let args = Cli::parse_from([
            "enscrive",
            "init",
            "--mode",
            "self-managed",
            "--developer-port",
            "36300",
        ]);
        match args.command {
            Commands::Init(InitArgs {
                mode,
                developer_port,
                ..
            }) => {
                assert!(matches!(mode, Some(InitMode::SelfManaged)));
                assert_eq!(developer_port, Some(36300));
            }
            _ => panic!("expected init"),
        }
    }

    #[test]
    fn parse_deploy_status_command() {
        let args = Cli::parse_from(["enscrive", "deploy", "status", "--profile-name", "stage"]);
        match args.command {
            Commands::Deploy {
                sub: DeploySubcommand::Status(DeployStatusArgs { profile_name }),
            } => {
                assert_eq!(profile_name.as_deref(), Some("stage"));
            }
            _ => panic!("expected deploy status"),
        }
    }

    #[test]
    fn parse_deploy_render_command() {
        let args = Cli::parse_from([
            "enscrive",
            "deploy",
            "render",
            "--profile-name",
            "stage",
            "--out-dir",
            "./enscrive-deploy/stage",
            "--host-root",
            "/opt/enscrive/stage",
            "--eba-trusted-public-key",
            "test-bootstrap-public-key",
        ]);
        match args.command {
            Commands::Deploy {
                sub:
                    DeploySubcommand::Render(DeployRenderArgs {
                        profile_name,
                        out_dir,
                        host_root,
                        bootstrap_public_key,
                    }),
            } => {
                assert_eq!(profile_name.as_deref(), Some("stage"));
                assert_eq!(out_dir.as_deref(), Some("./enscrive-deploy/stage"));
                assert_eq!(host_root.as_deref(), Some("/opt/enscrive/stage"));
                assert_eq!(
                    bootstrap_public_key.as_deref(),
                    Some("test-bootstrap-public-key")
                );
            }
            _ => panic!("expected deploy render"),
        }
    }

    #[test]
    fn parse_deploy_verify_command() {
        let args = Cli::parse_from([
            "enscrive",
            "deploy",
            "verify",
            "--profile-name",
            "stage",
            "--endpoint",
            "https://stage.api.enscrive.io",
        ]);
        match args.command {
            Commands::Deploy {
                sub:
                    DeploySubcommand::Verify(DeployVerifyArgs {
                        profile_name,
                        endpoint,
                    }),
            } => {
                assert_eq!(profile_name.as_deref(), Some("stage"));
                assert_eq!(endpoint.as_deref(), Some("https://stage.api.enscrive.io"));
            }
            _ => panic!("expected deploy verify"),
        }
    }

    #[test]
    fn parse_deploy_fetch_command() {
        let args = Cli::parse_from([
            "enscrive",
            "deploy",
            "fetch",
            "--profile-name",
            "stage",
            "--source",
            "local-build",
            "--build",
            "--workspace-root",
            "/home/christopher/enscrive-io",
            "--out-dir",
            "./enscrive-artifacts/stage",
            "--manifest-url",
            "https://stage.enscrive.io/releases/stage/latest.json",
        ]);
        match args.command {
            Commands::Deploy {
                sub:
                    DeploySubcommand::Fetch(DeployFetchArgs {
                        profile_name,
                        source,
                        out_dir,
                        manifest_url,
                        workspace_root,
                        build_local,
                    }),
            } => {
                assert_eq!(profile_name.as_deref(), Some("stage"));
                assert_eq!(source, Some(DeployFetchSource::LocalBuild));
                assert_eq!(out_dir.as_deref(), Some("./enscrive-artifacts/stage"));
                assert_eq!(
                    manifest_url.as_deref(),
                    Some("https://stage.enscrive.io/releases/stage/latest.json")
                );
                assert_eq!(
                    workspace_root.as_deref(),
                    Some("/home/christopher/enscrive-io")
                );
                assert!(build_local);
            }
            _ => panic!("expected deploy fetch"),
        }
    }

    #[test]
    fn parse_deploy_apply_command() {
        let args = Cli::parse_from([
            "enscrive",
            "deploy",
            "apply",
            "--profile-name",
            "stage",
            "--render-dir",
            "./enscrive-deploy/stage",
            "--binary-dir",
            "/tmp/bin",
            "--site-root",
            "/tmp/site",
            "--systemd-dir",
            "/tmp/systemd",
            "--nginx-dir",
            "/tmp/nginx",
            "--reload-systemd",
            "--start-services",
            "--reload-nginx",
        ]);
        match args.command {
            Commands::Deploy {
                sub:
                    DeploySubcommand::Apply(DeployApplyArgs {
                        profile_name,
                        render_dir,
                        binary_dir,
                        site_root,
                        systemd_dir,
                        nginx_dir,
                        reload_systemd,
                        start_services,
                        reload_nginx,
                    }),
            } => {
                assert_eq!(profile_name.as_deref(), Some("stage"));
                assert_eq!(render_dir.as_deref(), Some("./enscrive-deploy/stage"));
                assert_eq!(binary_dir.as_deref(), Some("/tmp/bin"));
                assert_eq!(site_root.as_deref(), Some("/tmp/site"));
                assert_eq!(systemd_dir.as_deref(), Some("/tmp/systemd"));
                assert_eq!(nginx_dir.as_deref(), Some("/tmp/nginx"));
                assert!(reload_systemd);
                assert!(start_services);
                assert!(reload_nginx);
            }
            _ => panic!("expected deploy apply"),
        }
    }

    #[test]
    fn parse_deploy_bootstrap_command() {
        let args = Cli::parse_from([
            "enscrive",
            "deploy",
            "bootstrap",
            "--profile-name",
            "stage",
            "--bundle-secret-key",
            "ENSCRIVE_BOOTSTRAP_BUNDLE",
            "--endpoint",
            "http://127.0.0.1:3000",
        ]);
        match args.command {
            Commands::Deploy {
                sub:
                    DeploySubcommand::Bootstrap(DeployBootstrapArgs {
                        profile_name,
                        bundle_secret_key,
                        endpoint,
                        ..
                    }),
            } => {
                assert_eq!(profile_name.as_deref(), Some("stage"));
                assert_eq!(
                    bundle_secret_key.as_deref(),
                    Some("ENSCRIVE_BOOTSTRAP_BUNDLE")
                );
                assert_eq!(endpoint.as_deref(), Some("http://127.0.0.1:3000"));
            }
            _ => panic!("expected deploy bootstrap"),
        }
    }

    #[test]
    fn parse_search_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "search",
            "--query",
            "hello",
            "--collection",
            "col-1",
            "--limit",
            "5",
        ]);
        match args.command {
            Commands::Search(SearchArgs {
                query,
                collection,
                limit,
                ..
            }) => {
                assert_eq!(query, "hello");
                assert_eq!(collection.as_deref(), Some("col-1"));
                assert_eq!(limit, 5);
            }
            _ => panic!("expected Search"),
        }
    }

    #[test]
    fn parse_logs_stream_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "logs",
            "stream",
            "--severity",
            "info",
            "--operation",
            "search",
            "--timeout-secs",
            "5",
        ]);

        match args.command {
            Commands::Logs {
                sub:
                    LogsSubcommand::Stream(LogStreamArgs {
                        severity,
                        operation,
                        timeout_secs,
                        ..
                    }),
            } => {
                assert_eq!(severity.as_deref(), Some("info"));
                assert_eq!(operation.as_deref(), Some("search"));
                assert_eq!(timeout_secs, 5);
            }
            _ => panic!("expected logs stream"),
        }
    }

    #[test]
    fn parse_logs_search_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "logs",
            "search",
            "--start-time",
            "2026-03-15T00:00:00Z",
            "--end-time",
            "2026-03-15T01:00:00Z",
            "--query",
            "request_id:abc",
            "--limit",
            "25",
        ]);

        match args.command {
            Commands::Logs {
                sub:
                    LogsSubcommand::Search(LogSearchArgs {
                        start_time,
                        end_time,
                        query,
                        limit,
                        ..
                    }),
            } => {
                assert_eq!(start_time, "2026-03-15T00:00:00Z");
                assert_eq!(end_time, "2026-03-15T01:00:00Z");
                assert_eq!(query.as_deref(), Some("request_id:abc"));
                assert_eq!(limit, 25);
            }
            _ => panic!("expected logs search"),
        }
    }

    #[test]
    fn parse_logs_metrics_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "logs",
            "metrics",
            "--start-time",
            "2026-03-15T00:00:00Z",
            "--end-time",
            "2026-03-15T01:00:00Z",
            "--granularity",
            "hour",
        ]);

        match args.command {
            Commands::Logs {
                sub:
                    LogsSubcommand::Metrics(LogMetricsArgs {
                        start_time,
                        end_time,
                        granularity,
                        ..
                    }),
            } => {
                assert_eq!(start_time, "2026-03-15T00:00:00Z");
                assert_eq!(end_time, "2026-03-15T01:00:00Z");
                assert_eq!(granularity, "hour");
            }
            _ => panic!("expected logs metrics"),
        }
    }

    #[test]
    fn parse_analyze_content_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "analyze",
            "content",
            "--content-file",
            "analysis.txt",
        ]);

        match args.command {
            Commands::Analyze {
                sub:
                    AnalyzeSubcommand::Content(ContentAnalysisArgs {
                        content,
                        content_file,
                    }),
            } => {
                assert!(content.is_none());
                assert_eq!(content_file.as_deref(), Some("analysis.txt"));
            }
            _ => panic!("expected analyze content"),
        }
    }

    #[test]
    fn parse_models_list_command() {
        let args = Cli::parse_from(["enscrive", "--api-key", "test-key", "models", "list"]);

        match args.command {
            Commands::Models {
                sub: ModelsSubcommand::List,
            } => {}
            _ => panic!("expected models list"),
        }
    }

    #[test]
    fn parse_ingest_prepared_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "ingest",
            "prepared",
            "--collection-id",
            "col-1",
            "--document-id",
            "doc-1",
            "--segments-file",
            "segments.json",
        ]);

        match args.command {
            Commands::Ingest {
                sub:
                    IngestSubcommand::Prepared(IngestPreparedArgs {
                        collection_id,
                        document_id,
                        segments_file,
                        ..
                    }),
            } => {
                assert_eq!(collection_id, "col-1");
                assert_eq!(document_id, "doc-1");
                assert_eq!(segments_file.as_deref(), Some("segments.json"));
            }
            _ => panic!("expected ingest prepared"),
        }
    }

    #[test]
    fn parse_segment_document_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "segment",
            "document",
            "--voice-id",
            "voice-1",
            "--content-file",
            "doc.txt",
        ]);

        match args.command {
            Commands::Segment {
                sub:
                    SegmentSubcommand::Document(SegmentDocumentArgs {
                        voice_id,
                        content_file,
                        ..
                    }),
            } => {
                assert_eq!(voice_id, "voice-1");
                assert_eq!(content_file.as_deref(), Some("doc.txt"));
            }
            _ => panic!("expected segment document"),
        }
    }

    #[test]
    fn parse_eval_dataset_create_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "evals",
            "datasets",
            "create",
            "--name",
            "dataset-1",
            "--queries-file",
            "queries.json",
        ]);

        match args.command {
            Commands::Evals {
                sub:
                    EvalsSubcommand::Datasets {
                        sub:
                            EvalDatasetsSubcommand::Create(CreateEvalDatasetArgs {
                                name,
                                queries_file,
                                ..
                            }),
                    },
            } => {
                assert_eq!(name, "dataset-1");
                assert_eq!(queries_file.as_deref(), Some("queries.json"));
            }
            _ => panic!("expected evals datasets create"),
        }
    }

    #[test]
    fn parse_eval_dataset_queries_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "evals",
            "datasets",
            "queries",
            "--id",
            "2ca50d50-5085-425b-8185-f3692d51de1b",
        ]);

        match args.command {
            Commands::Evals {
                sub:
                    EvalsSubcommand::Datasets {
                        sub: EvalDatasetsSubcommand::Queries { id },
                    },
            } => {
                assert_eq!(id, "2ca50d50-5085-425b-8185-f3692d51de1b");
            }
            _ => panic!("expected evals datasets queries"),
        }
    }

    #[test]
    fn parse_usage_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "usage",
            "--start-time",
            "2026-03-14T00:00:00Z",
            "--end-time",
            "2026-03-15T00:00:00Z",
            "--collection-id",
            "col-1",
            "--limit",
            "25",
        ]);
        match args.command {
            Commands::Usage(UsageArgs {
                start_time,
                end_time,
                collection_id,
                limit,
                ..
            }) => {
                assert_eq!(start_time, "2026-03-14T00:00:00Z");
                assert_eq!(end_time, "2026-03-15T00:00:00Z");
                assert_eq!(collection_id.as_deref(), Some("col-1"));
                assert_eq!(limit, Some(25));
            }
            _ => panic!("expected Usage"),
        }
    }

    #[test]
    fn build_usage_query_includes_optional_filters() {
        let query = build_usage_query(&UsageArgs {
            start_time: "2026-03-14T00:00:00Z".to_string(),
            end_time: "2026-03-15T00:00:00Z".to_string(),
            document_id: Some("doc-1".to_string()),
            collection_id: Some("col-1".to_string()),
            operation: Some("search".to_string()),
            embedding_model: Some("bge-small-en-v1.5".to_string()),
            limit: Some(50),
            page_token: Some("page-2".to_string()),
        });

        assert_eq!(
            query,
            vec![
                ("start_time", "2026-03-14T00:00:00Z".to_string()),
                ("end_time", "2026-03-15T00:00:00Z".to_string()),
                ("document_id", "doc-1".to_string()),
                ("collection_id", "col-1".to_string()),
                ("operation", "search".to_string()),
                ("embedding_model", "bge-small-en-v1.5".to_string()),
                ("limit", "50".to_string()),
                ("page_token", "page-2".to_string()),
            ]
        );
    }

    #[test]
    fn reject_non_array_segments_json() {
        let args = IngestPreparedArgs {
            collection_id: "col-1".to_string(),
            document_id: "doc-1".to_string(),
            voice_id: None,
            segments_json: Some("{\"content\":\"nope\"}".to_string()),
            segments_file: None,
        };

        let error = parse_segments_source(&args).unwrap_err();
        assert_eq!(
            error,
            "segments JSON must be an array or object containing a segments array"
        );
    }

    #[test]
    fn reject_missing_segment_content_source() {
        let args = SegmentDocumentArgs {
            voice_id: "voice-1".to_string(),
            content: None,
            content_file: None,
        };

        let error = parse_content_source(&args).unwrap_err();
        assert_eq!(error, "provide exactly one of --content or --content-file");
    }

    #[test]
    fn reject_missing_analysis_content_source() {
        let args = ContentAnalysisArgs {
            content: None,
            content_file: None,
        };

        let error = parse_analysis_source(&args).unwrap_err();
        assert_eq!(error, "provide exactly one of --content or --content-file");
    }

    #[test]
    fn reject_non_array_eval_queries_json() {
        let error = parse_eval_queries_source(&Some("{\"query\":\"nope\"}".to_string()), &None)
            .unwrap_err();
        assert!(error.contains("queries JSON must be an array"));
        assert!(error.contains("optional collection_id"));
    }

    #[test]
    fn parse_eval_queries_source_accepts_collection_id() {
        let value = parse_eval_queries_source(
            &Some(
                r#"[{"query_id":"q1","query_text":"hello","relevant_doc_ids":["doc-1"],"relevance_scores":{"doc-1":1},"collection_id":"col-1","match_mode":"document_prefix"}]"#
                    .to_string(),
            ),
            &None,
        )
        .unwrap();

        assert_eq!(value[0]["collection_id"], "col-1");
        assert_eq!(value[0]["match_mode"], "document_prefix");
    }

    #[test]
    fn parse_voice_config_source_accepts_template_id() {
        let value = parse_config_source(
            &Some(
                r#"{"chunking_strategy":"story_beats","parameters":{"model":"gpt-4o"},"template_id":"tmpl-1","score_threshold":0.8,"default_limit":5,"description":"voice","tags":["docs"]}"#
                    .to_string(),
            ),
            &None,
        )
        .unwrap();

        assert_eq!(value["template_id"], "tmpl-1");
    }

    #[test]
    fn build_search_body_includes_filters() {
        let body = build_search_body(&SearchArgs {
            query: "hello".to_string(),
            collection: Some("col-1".to_string()),
            limit: 5,
            include_vectors: false,
            score_threshold: None,
            granularity: None,
            oversample_factor: None,
            extended_results: false,
            score_floor: None,
            filter_document_id: Some("doc-1".to_string()),
            filter_user_id: Some("user-1".to_string()),
            filter_layer: Some("baseline".to_string()),
            filter_strategy: Some("baseline".to_string()),
            filter_metadata: vec!["tag=alpha".to_string(), "color=red".to_string()],
            hybrid_alpha: None,
            resolution: None,
        })
        .unwrap();

        assert_eq!(body["filters"]["document_id"], "doc-1");
        assert_eq!(body["filters"]["user_id"], "user-1");
        assert_eq!(body["filters"]["layer"], "baseline");
        assert_eq!(body["filters"]["strategy"], "baseline");
        assert_eq!(body["filters"]["metadata"]["tag"], "alpha");
        assert_eq!(body["filters"]["metadata"]["color"], "red");
    }

    #[test]
    fn parse_embeddings_query_with_voice_id() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "k",
            "embeddings",
            "query",
            "--text",
            "hello",
            "--text",
            "world",
            "--voice-id",
            "voice-1",
            "--collection",
            "col-1",
        ]);
        match args.command {
            Commands::Embeddings {
                sub:
                    EmbeddingsSubcommand::Query(EmbeddingsQueryArgs {
                        texts,
                        voice_id,
                        collection,
                    }),
            } => {
                assert_eq!(texts, vec!["hello", "world"]);
                assert_eq!(voice_id.as_deref(), Some("voice-1"));
                assert_eq!(collection.as_deref(), Some("col-1"));
            }
            _ => panic!("expected embeddings query"),
        }
    }

    #[test]
    fn parse_collections_create() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "k",
            "collections",
            "create",
            "--name",
            "test",
            "--embedding-model",
            "bge-small-en-v1.5",
            "--dimensions",
            "384",
        ]);
        match args.command {
            Commands::Collections {
                sub:
                    CollectionsSubcommand::Create(CreateCollectionArgs {
                        name,
                        embedding_model,
                        dimensions,
                        ..
                    }),
            } => {
                assert_eq!(name, "test");
                assert_eq!(embedding_model, "bge-small-en-v1.5");
                assert_eq!(dimensions, Some(384));
            }
            _ => panic!("expected collections create"),
        }
    }

    #[test]
    fn parse_voices_create_with_config_file() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "k",
            "voices",
            "create",
            "--name",
            "voice-a",
            "--config-file",
            "voice.json",
        ]);
        match args.command {
            Commands::Voices {
                sub:
                    VoicesSubcommand::Create(CreateVoiceArgs {
                        name,
                        config_file,
                        config_json,
                    }),
            } => {
                assert_eq!(name, "voice-a");
                assert_eq!(config_file.as_deref(), Some("voice.json"));
                assert!(config_json.is_none());
            }
            _ => panic!("expected voices create"),
        }
    }

    #[test]
    fn parse_segment_document_sse_into_prepared_segments() {
        let sse = "\
event: status\n\
data: {\"phase\":\"resolving\"}\n\n\
event: segment\n\
data: {\"index\":0,\"content\":\"First beat\",\"label\":\"opening\",\"confidence\":0.94,\"reasoning\":\"Clear setup\",\"start_paragraph\":1,\"end_paragraph\":2,\"estimated_tokens\":123}\n\n\
event: complete\n\
data: {\"total_segments\":1,\"processing_time_ms\":42,\"template_name\":\"Narrative\",\"model\":\"gpt-5-mini\",\"total_paragraphs\":7}\n\n";

        let parsed = parse_segment_sse(sse).unwrap();
        let segments: Vec<CliPreparedSegment> = serde_json::from_value(parsed).unwrap();

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].content, "First beat");
        assert_eq!(segments[0].metadata.get("segment_index").unwrap(), "0");
        assert_eq!(segments[0].metadata.get("estimated_tokens").unwrap(), "123");
        assert_eq!(
            segments[0].metadata.get("template_name").unwrap(),
            "Narrative"
        );
        assert_eq!(
            segments[0].metadata.get("segmentation_model").unwrap(),
            "gpt-5-mini"
        );
    }

    #[test]
    fn parse_segments_source_accepts_segment_command_envelope() {
        let args = IngestPreparedArgs {
            collection_id: "col-1".to_string(),
            document_id: "doc-1".to_string(),
            voice_id: None,
            segments_json: Some(
                r#"{
                    "ok": true,
                    "command": "segment document",
                    "data": [{
                        "content": "First beat",
                        "label": "opening",
                        "confidence": 0.9,
                        "reasoning": "setup",
                        "start_paragraph": 1,
                        "end_paragraph": 2,
                        "metadata": {"segment_index": "0"}
                    }],
                    "exit_code": 0
                }"#
                .to_string(),
            ),
            segments_file: None,
        };

        let parsed = parse_segments_source(&args).unwrap();
        let segments: Vec<CliPreparedSegment> = serde_json::from_value(parsed).unwrap();

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].metadata.get("segment_index").unwrap(), "0");
    }

    #[test]
    fn parse_segments_source_converts_segment_info_arrays() {
        let args = IngestPreparedArgs {
            collection_id: "col-1".to_string(),
            document_id: "doc-1".to_string(),
            voice_id: None,
            segments_json: Some(
                r#"[{
                    "index": 0,
                    "content": "First beat",
                    "label": "opening",
                    "confidence": 0.9,
                    "reasoning": "setup",
                    "start_paragraph": 1,
                    "end_paragraph": 2,
                    "estimated_tokens": 88
                }]"#
                .to_string(),
            ),
            segments_file: None,
        };

        let parsed = parse_segments_source(&args).unwrap();
        let segments: Vec<CliPreparedSegment> = serde_json::from_value(parsed).unwrap();

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].metadata.get("segment_index").unwrap(), "0");
        assert_eq!(segments[0].metadata.get("estimated_tokens").unwrap(), "88");
    }

    #[test]
    fn parse_voice_compare_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "k",
            "voices",
            "compare",
            "--voice-a-id",
            "voice-a",
            "--voice-b-id",
            "voice-b",
            "--query",
            "compare me",
            "--collection-id",
            "col-1",
            "--include-vectors",
        ]);
        match args.command {
            Commands::Voices {
                sub:
                    VoicesSubcommand::Compare(VoiceCompareArgs {
                        voice_a_id,
                        voice_b_id,
                        query,
                        collection_id,
                        include_vectors,
                    }),
            } => {
                assert_eq!(voice_a_id, "voice-a");
                assert_eq!(voice_b_id, "voice-b");
                assert_eq!(query, "compare me");
                assert_eq!(collection_id, "col-1");
                assert!(include_vectors);
            }
            _ => panic!("expected voices compare"),
        }
    }

    #[test]
    fn parse_voice_promote_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "k",
            "voices",
            "promote",
            "--voice-id",
            "voice-a",
            "--target-environment-id",
            "env-2",
        ]);
        match args.command {
            Commands::Voices {
                sub:
                    VoicesSubcommand::Promote(VoicePromoteArgs {
                        voice_id,
                        target_environment_id,
                    }),
            } => {
                assert_eq!(voice_id, "voice-a");
                assert_eq!(target_environment_id, "env-2");
            }
            _ => panic!("expected voices promote"),
        }
    }

    #[test]
    fn parse_voice_gates_list_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "k",
            "voices",
            "gates",
            "list",
            "--voice-id",
            "voice-a",
        ]);
        match args.command {
            Commands::Voices {
                sub:
                    VoicesSubcommand::Gates {
                        sub: VoiceGatesSubcommand::List { voice_id },
                    },
            } => assert_eq!(voice_id, "voice-a"),
            _ => panic!("expected voices gates list"),
        }
    }

    #[test]
    fn parse_voice_gates_set_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "k",
            "voices",
            "gates",
            "set",
            "--voice-id",
            "voice-a",
            "--metric",
            "ndcg_at_10",
            "--threshold",
            "0.85",
            "--operator",
            "gte",
        ]);
        match args.command {
            Commands::Voices {
                sub:
                    VoicesSubcommand::Gates {
                        sub:
                            VoiceGatesSubcommand::Set(VoiceGateSetArgs {
                                voice_id,
                                metric,
                                threshold,
                                operator,
                            }),
                    },
            } => {
                assert_eq!(voice_id, "voice-a");
                assert_eq!(metric, "ndcg_at_10");
                assert_eq!(threshold, 0.85);
                assert_eq!(operator, "gte");
            }
            _ => panic!("expected voices gates set"),
        }
    }

    #[test]
    fn parse_voice_gates_delete_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "k",
            "voices",
            "gates",
            "delete",
            "--voice-id",
            "voice-a",
            "--metric",
            "ndcg_at_10",
        ]);
        match args.command {
            Commands::Voices {
                sub:
                    VoicesSubcommand::Gates {
                        sub: VoiceGatesSubcommand::Delete(VoiceGateDeleteArgs { voice_id, metric }),
                    },
            } => {
                assert_eq!(voice_id, "voice-a");
                assert_eq!(metric, "ndcg_at_10");
            }
            _ => panic!("expected voices gates delete"),
        }
    }

    #[test]
    fn parse_voice_search_metadata_filters() {
        let metadata =
            parse_metadata_filters(&["source=codex".to_string(), "lane=bge".to_string()]).unwrap();
        assert_eq!(
            metadata.get("source"),
            Some(&Value::String("codex".to_string()))
        );
        assert_eq!(
            metadata.get("lane"),
            Some(&Value::String("bge".to_string()))
        );
    }

    #[test]
    fn reject_invalid_metadata_filter() {
        let err = parse_metadata_filters(&["broken".to_string()]).unwrap_err();
        assert!(err.contains("expected key=value"));
    }


    #[test]
    fn collections_get_unsupported_response() {
        let response = CliResponse::unsupported(
            "collections get",
            "GET /v1/collections/{id} is not yet available on public /v1",
        );
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["failure_class"], "FAIL_UNSUPPORTED");
        assert_eq!(json["exit_code"], 2);
    }

    #[test]
    fn parse_evals_import() {
        let args = Cli::parse_from([
            "enscrive",
            "evals",
            "import",
            "--dataset-name",
            "beir-scifact",
            "--queries-file",
            "queries.jsonl",
            "--qrels-file",
            "qrels/test.tsv",
        ]);
        match args.command {
            Commands::Evals {
                sub: EvalsSubcommand::Import(import),
            } => {
                assert_eq!(import.dataset_name, "beir-scifact");
                assert_eq!(import.format, "beir");
                assert_eq!(import.queries_file, "queries.jsonl");
                assert_eq!(import.qrels_file, "qrels/test.tsv");
                assert!(import.corpus_file.is_none());
                assert!(import.collection_id.is_none());
                assert!(import.voice_id.is_none());
            }
            _ => panic!("expected evals import"),
        }
    }

    #[test]
    fn parse_evals_import_with_corpus() {
        let args = Cli::parse_from([
            "enscrive",
            "evals",
            "import",
            "--dataset-name",
            "beir-scifact",
            "--queries-file",
            "queries.jsonl",
            "--qrels-file",
            "qrels/test.tsv",
            "--corpus-file",
            "corpus.jsonl",
            "--collection-id",
            "col-1",
            "--voice-id",
            "voice-1",
        ]);
        match args.command {
            Commands::Evals {
                sub: EvalsSubcommand::Import(import),
            } => {
                assert_eq!(import.dataset_name, "beir-scifact");
                assert_eq!(import.corpus_file.as_deref(), Some("corpus.jsonl"));
                assert_eq!(import.collection_id.as_deref(), Some("col-1"));
                assert_eq!(import.voice_id.as_deref(), Some("voice-1"));
            }
            _ => panic!("expected evals import"),
        }
    }

    #[test]
    fn parse_evals_campaigns_get() {
        let args = Cli::parse_from(["enscrive", "evals", "campaigns", "get", "--id", "camp-1"]);
        match args.command {
            Commands::Evals {
                sub:
                    EvalsSubcommand::Campaigns {
                        sub: EvalCampaignsSubcommand::Get { id },
                    },
            } => assert_eq!(id, "camp-1"),
            _ => panic!("expected evals campaigns get"),
        }
    }

    #[test]
    fn parse_evals_run_campaign() {
        let args = Cli::parse_from([
            "enscrive",
            "evals",
            "run-campaign",
            "--name",
            "campaign-1",
            "--voice-id",
            "voice-1",
            "--dataset-id",
            "dataset-1",
            "--metric",
            "ndcg@10",
            "--metric",
            "recall@10",
            "--queries-file",
            "queries.json",
            "--collection-id",
            "collection-1",
        ]);
        match args.command {
            Commands::Evals {
                sub: EvalsSubcommand::RunCampaign(run),
            } => {
                assert_eq!(run.name, "campaign-1");
                assert_eq!(run.voice_id, "voice-1");
                assert_eq!(run.dataset_id, "dataset-1");
                assert_eq!(run.metrics, vec!["ndcg@10", "recall@10"]);
                assert_eq!(run.queries_file.as_deref(), Some("queries.json"));
                assert!(run.queries_json.is_none());
                assert_eq!(run.collection_id.as_deref(), Some("collection-1"));
                assert!(run.match_mode.is_none());
            }
            _ => panic!("expected evals run-campaign"),
        }
    }

    #[test]
    fn parse_evals_run_campaign_with_match_mode() {
        let args = Cli::parse_from([
            "enscrive",
            "evals",
            "run-campaign",
            "--name",
            "campaign-1",
            "--voice-id",
            "voice-1",
            "--dataset-id",
            "dataset-1",
            "--metric",
            "ndcg@10",
            "--queries-file",
            "queries.json",
            "--match-mode",
            "document_prefix",
        ]);
        match args.command {
            Commands::Evals {
                sub: EvalsSubcommand::RunCampaign(run),
            } => {
                assert_eq!(run.match_mode.as_deref(), Some("document_prefix"));
            }
            _ => panic!("expected evals run-campaign"),
        }
    }

    #[test]
    fn build_eval_campaign_body_includes_match_mode() {
        let args = RunEvalCampaignArgs {
            name: "campaign-1".to_string(),
            voice_id: "voice-1".to_string(),
            dataset_id: "dataset-1".to_string(),
            metrics: vec!["ndcg@10".to_string()],
            collection_id: Some("collection-1".to_string()),
            queries_json: Some(
                r#"[{"query_id":"q1","query_text":"hello","relevant_doc_ids":["doc-1"],"relevance_scores":{"doc-1":1}}]"#
                    .to_string(),
            ),
            queries_file: None,
            match_mode: Some("document_prefix".to_string()),
        };

        let body = build_eval_campaign_body(&args).unwrap();
        assert_eq!(body["match_mode"], "document_prefix");
        assert_eq!(body["collection_id"], "collection-1");
    }

    #[test]
    fn build_eval_campaign_body_rejects_invalid_match_mode() {
        let args = RunEvalCampaignArgs {
            name: "campaign-1".to_string(),
            voice_id: "voice-1".to_string(),
            dataset_id: "dataset-1".to_string(),
            metrics: vec!["ndcg@10".to_string()],
            collection_id: None,
            queries_json: Some(
                r#"[{"query_id":"q1","query_text":"hello","relevant_doc_ids":["doc-1"],"relevance_scores":{"doc-1":1}}]"#
                    .to_string(),
            ),
            queries_file: None,
            match_mode: Some("prefix".to_string()),
        };

        let err = build_eval_campaign_body(&args).unwrap_err();
        assert!(err.contains("invalid --match-mode"));
    }

    #[test]
    fn build_eval_campaign_body_allows_dataset_backed_runs_without_queries_source() {
        let args = RunEvalCampaignArgs {
            name: "campaign-1".to_string(),
            voice_id: "voice-1".to_string(),
            dataset_id: "dataset-1".to_string(),
            metrics: vec!["ndcg@10".to_string()],
            collection_id: Some("collection-1".to_string()),
            queries_json: None,
            queries_file: None,
            match_mode: Some("document_prefix".to_string()),
        };

        let body = build_eval_campaign_body(&args).unwrap();
        assert_eq!(body["queries"], Value::Array(Vec::new()));
        assert_eq!(body["match_mode"], "document_prefix");
        assert_eq!(body["collection_id"], "collection-1");
    }


    #[test]
    fn parse_backup_create_command() {
        let args = Cli::parse_from(["enscrive", "--api-key", "test-key", "backup", "create"]);
        match args.command {
            Commands::Backup {
                sub: BackupSubcommand::Create,
            } => {}
            _ => panic!("expected backup create"),
        }
    }

    #[test]
    fn parse_backup_restore_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "backup",
            "restore",
            "--target-time",
            "2026-03-15T00:00:00Z",
            "--confirm",
        ]);
        match args.command {
            Commands::Backup {
                sub:
                    BackupSubcommand::Restore(BackupRestoreArgs {
                        target_time,
                        confirm,
                    }),
            } => {
                assert_eq!(target_time, "2026-03-15T00:00:00Z");
                assert!(confirm);
            }
            _ => panic!("expected backup restore"),
        }
    }

    #[test]
    fn parse_export_tenant_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "export",
            "tenant",
            "--out-file",
            "tenant-export.jsonl",
            "--include-vectors",
            "--document-id",
            "doc-1",
            "--layer",
            "baseline",
        ]);
        match args.command {
            Commands::Export {
                sub:
                    ExportSubcommand::Tenant(ExportTenantArgs {
                        out_file,
                        include_vectors,
                        document_id,
                        layer,
                    }),
            } => {
                assert_eq!(out_file, "tenant-export.jsonl");
                assert!(include_vectors);
                assert_eq!(document_id.as_deref(), Some("doc-1"));
                assert_eq!(layer.as_deref(), Some("baseline"));
            }
            _ => panic!("expected export tenant"),
        }
    }

    #[test]
    fn build_export_tenant_query_includes_requested_filters() {
        let query = build_export_tenant_query(&ExportTenantArgs {
            out_file: "tenant-export.jsonl".to_string(),
            include_vectors: true,
            document_id: Some("doc-1".to_string()),
            layer: Some("baseline".to_string()),
        });

        assert_eq!(
            query,
            vec![
                ("include_vectors", "true".to_string()),
                ("document_id", "doc-1".to_string()),
                ("layer", "baseline".to_string()),
            ]
        );
    }

    #[test]
    fn parse_export_embeddings_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "export",
            "embeddings",
            "--user-id",
            "user-1",
            "--document-id",
            "doc-1",
            "--layer",
            "baseline",
            "--conversation-id",
            "conv-1",
            "--paragraph-id",
            "p-1",
            "--paragraph-id",
            "p-2",
            "--limit",
            "25",
            "--page-token",
            "cursor-1",
            "--include-vectors",
        ]);
        match args.command {
            Commands::Export {
                sub:
                    ExportSubcommand::Embeddings(ExportEmbeddingsArgs {
                        user_id,
                        document_id,
                        layer,
                        conversation_id,
                        paragraph_ids,
                        limit,
                        page_token,
                        include_vectors,
                    }),
            } => {
                assert_eq!(user_id.as_deref(), Some("user-1"));
                assert_eq!(document_id.as_deref(), Some("doc-1"));
                assert_eq!(layer.as_deref(), Some("baseline"));
                assert_eq!(conversation_id.as_deref(), Some("conv-1"));
                assert_eq!(paragraph_ids, vec!["p-1".to_string(), "p-2".to_string()]);
                assert_eq!(limit, Some(25));
                assert_eq!(page_token.as_deref(), Some("cursor-1"));
                assert!(include_vectors);
            }
            _ => panic!("expected export embeddings"),
        }
    }

    #[test]
    fn build_export_embeddings_query_includes_requested_filters() {
        let query = build_export_embeddings_query(&ExportEmbeddingsArgs {
            user_id: Some("user-1".to_string()),
            document_id: Some("doc-1".to_string()),
            layer: Some("baseline".to_string()),
            conversation_id: Some("conv-1".to_string()),
            paragraph_ids: vec!["p-1".to_string(), "p-2".to_string()],
            limit: Some(25),
            page_token: Some("cursor-1".to_string()),
            include_vectors: true,
        });

        assert_eq!(
            query,
            vec![
                ("user_id", "user-1".to_string()),
                ("document_id", "doc-1".to_string()),
                ("layer", "baseline".to_string()),
                ("conversation_id", "conv-1".to_string()),
                ("paragraph_ids", "p-1".to_string()),
                ("paragraph_ids", "p-2".to_string()),
                ("limit", "25".to_string()),
                ("page_token", "cursor-1".to_string()),
                ("include_vectors", "true".to_string()),
            ]
        );
    }

    #[test]
    fn parse_export_token_usage_command() {
        let args = Cli::parse_from([
            "enscrive",
            "--api-key",
            "test-key",
            "export",
            "token-usage",
            "--user-id",
            "user-1",
            "--document-id",
            "doc-1",
            "--layer",
            "baseline",
            "--conversation-id",
            "conv-1",
            "--paragraph-id",
            "p-1",
            "--limit",
            "10",
            "--page-token",
            "cursor-2",
        ]);
        match args.command {
            Commands::Export {
                sub:
                    ExportSubcommand::TokenUsage(ExportTokenUsageArgs {
                        user_id,
                        document_id,
                        layer,
                        conversation_id,
                        paragraph_ids,
                        limit,
                        page_token,
                    }),
            } => {
                assert_eq!(user_id.as_deref(), Some("user-1"));
                assert_eq!(document_id.as_deref(), Some("doc-1"));
                assert_eq!(layer.as_deref(), Some("baseline"));
                assert_eq!(conversation_id.as_deref(), Some("conv-1"));
                assert_eq!(paragraph_ids, vec!["p-1".to_string()]);
                assert_eq!(limit, Some(10));
                assert_eq!(page_token.as_deref(), Some("cursor-2"));
            }
            _ => panic!("expected export token-usage"),
        }
    }

    #[test]
    fn build_export_token_usage_query_includes_requested_filters() {
        let query = build_export_token_usage_query(&ExportTokenUsageArgs {
            user_id: Some("user-1".to_string()),
            document_id: Some("doc-1".to_string()),
            layer: Some("baseline".to_string()),
            conversation_id: Some("conv-1".to_string()),
            paragraph_ids: vec!["p-1".to_string()],
            limit: Some(10),
            page_token: Some("cursor-2".to_string()),
        });

        assert_eq!(
            query,
            vec![
                ("user_id", "user-1".to_string()),
                ("document_id", "doc-1".to_string()),
                ("layer", "baseline".to_string()),
                ("conversation_id", "conv-1".to_string()),
                ("paragraph_ids", "p-1".to_string()),
                ("limit", "10".to_string()),
                ("page_token", "cursor-2".to_string()),
            ]
        );
    }

    #[test]
    fn parse_collections_documents() {
        let args = Cli::parse_from(["enscrive", "collections", "documents", "--id", "col-1"]);
        match args.command {
            Commands::Collections {
                sub: CollectionsSubcommand::Documents { id },
            } => assert_eq!(id, "col-1"),
            _ => panic!("expected collections documents"),
        }
    }

    #[test]
    fn parse_collections_chunks() {
        let args = Cli::parse_from([
            "enscrive",
            "collections",
            "chunks",
            "--collection-id",
            "col-1",
            "--document-id",
            "doc-1",
            "--include-vectors",
            "false",
            "--include-content",
            "false",
        ]);
        match args.command {
            Commands::Collections {
                sub:
                    CollectionsSubcommand::Chunks {
                        collection_id,
                        document_id,
                        include_vectors,
                        include_content,
                    },
            } => {
                assert_eq!(collection_id, "col-1");
                assert_eq!(document_id, "doc-1");
                assert!(!include_vectors);
                assert!(!include_content);
            }
            _ => panic!("expected collections chunks"),
        }
    }

    #[test]
    fn request_failure_classifies_failed_precondition_as_unsupported() {
        let response = request_failure(
            "embeddings query",
            "HTTP 500 Internal Server Error: query_embeddings RPC failed: status: FailedPrecondition, message: \"unsupported\"".to_string(),
        );
        assert_eq!(response.failure_class, Some(FailureClass::Unsupported));
        assert_eq!(response.exit_code, EXIT_UNSUPPORTED);
    }
}
