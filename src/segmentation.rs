//! ENS-752: chunking/template preview + segmentation-template CRUD.
//!
//! Thin wrappers over `POST /v1/preview-chunking`, `POST
//! /v1/preview-with-template`, and `/v1/segmentation-templates*` in
//! enscrive-developer (`crates/server/src/api/v1/chunking.rs`,
//! `crates/server/src/api/v1/ingest.rs::preview_with_template`,
//! `crates/server/src/api/v1/segmentation_templates.rs`). Request/response
//! shapes are pinned to the real handler + `types_api` structs.

use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::client::EnscriveClient;
use crate::output::{CliResponse, FailureClass, OutputFormat, EXIT_CONFIG};

fn parse_optional_json(raw: &Option<String>, flag: &str) -> Result<Value, String> {
    match raw {
        None => Ok(json!({})),
        Some(s) => serde_json::from_str(s).map_err(|e| format!("parse --{flag} as JSON: {e}")),
    }
}

/// Like [`parse_optional_json`] but distinguishes "flag not supplied"
/// (`None`, so a PATCH-style update leaves the field untouched) from "flag
/// supplied" (parsed `Value`). Used by the update path, where an absent
/// flag must not overwrite the existing field with `{}`.
fn parse_optional_json_field(raw: &Option<String>, flag: &str) -> Result<Option<Value>, String> {
    match raw {
        None => Ok(None),
        Some(s) => serde_json::from_str(s)
            .map(Some)
            .map_err(|e| format!("parse --{flag} as JSON: {e}")),
    }
}

// ---------------------------------------------------------------------------
// preview-chunking — POST /v1/preview-chunking
// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct PreviewChunkingArgs {
    /// Inline content to preview-chunk.
    #[arg(long, conflicts_with = "content_file")]
    content: Option<String>,

    /// Path to a file containing the content to preview-chunk.
    #[arg(long = "content-file", conflicts_with = "content")]
    content_file: Option<String>,

    /// Voice UUID whose chunking strategy + parameters to preview with.
    /// Omit to preview the "baseline" strategy.
    #[arg(long = "voice-id")]
    voice_id: Option<String>,
}

pub async fn run_preview_chunking(client: &EnscriveClient, fmt: OutputFormat, args: &PreviewChunkingArgs) {
    let command = "preview-chunking";

    let content = match crate::parse_text_source(&args.content, &args.content_file) {
        Ok(c) => c,
        Err(e) => CliResponse::fail(command, e, FailureClass::Bug, EXIT_CONFIG).emit(fmt),
    };

    let body = json!({
        "content": content,
        "voice_id": args.voice_id,
    });

    match client.post_json("/v1/preview-chunking", body).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// preview-with-template — POST /v1/preview-with-template
// ---------------------------------------------------------------------------

#[derive(Args)]
pub struct PreviewWithTemplateArgs {
    /// Inline text to segment with the template.
    #[arg(long, conflicts_with = "text_file")]
    text: Option<String>,

    /// Path to a file containing the text to segment.
    #[arg(long = "text-file", conflicts_with = "text")]
    text_file: Option<String>,

    /// Segmentation template UUID.
    #[arg(long = "template-id", required = true)]
    template_id: String,

    /// Override the template's default minimum segment length.
    #[arg(long = "min-segment-length")]
    min_segment_length: Option<u32>,

    /// Override the template's default maximum segment length.
    #[arg(long = "max-segment-length")]
    max_segment_length: Option<u32>,

    /// Override the template's default LLM model.
    #[arg(long)]
    model: Option<String>,
}

pub async fn run_preview_with_template(client: &EnscriveClient, fmt: OutputFormat, args: &PreviewWithTemplateArgs) {
    let command = "preview-with-template";

    let text = match crate::parse_text_source(&args.text, &args.text_file) {
        Ok(t) => t,
        Err(e) => CliResponse::fail(command, e, FailureClass::Bug, EXIT_CONFIG).emit(fmt),
    };

    let body = json!({
        "text": text,
        "template_id": args.template_id,
        "min_segment_length": args.min_segment_length,
        "max_segment_length": args.max_segment_length,
        "model": args.model,
    });

    match client.post_json("/v1/preview-with-template", body).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

// ---------------------------------------------------------------------------
// segmentation-templates — /v1/segmentation-templates*
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum SegmentationTemplatesSubcommand {
    /// List templates visible to the caller's tenant (system + own).
    /// `GET /v1/segmentation-templates`.
    List,

    /// Create a new template. `POST /v1/segmentation-templates`.
    Create(SegTemplateCreateArgs),

    /// Get a single template. `GET /v1/segmentation-templates/{id}`.
    Get(SegTemplateIdArgs),

    /// Update a template (own only, not system).
    /// `PUT /v1/segmentation-templates/{id}`.
    Update(SegTemplateUpdateArgs),

    /// Delete a template (own only, not system).
    /// `DELETE /v1/segmentation-templates/{id}`.
    Delete(SegTemplateIdArgs),

    /// Clone a template (system or own) into a new tenant-owned copy.
    /// `POST /v1/segmentation-templates/{id}/clone`.
    Clone(SegTemplateIdArgs),
}

#[derive(Args)]
pub struct SegTemplateIdArgs {
    /// Template UUID.
    id: String,
}

#[derive(Args)]
pub struct SegTemplateCreateArgs {
    #[arg(long, required = true)]
    name: String,

    /// URL-friendly identifier: lowercase alphanumeric + hyphens only.
    #[arg(long, required = true)]
    slug: String,

    #[arg(long)]
    description: Option<String>,

    #[arg(long = "system-prompt", required = true)]
    system_prompt: String,

    /// Raw JSON object for the output schema. Default `{}`.
    #[arg(long = "output-schema")]
    output_schema: Option<String>,

    /// Raw JSON object for the default segmentation parameters
    /// (`min_segment_length`, `max_segment_length`, `model`, ...). Default `{}`.
    #[arg(long)]
    defaults: Option<String>,

    /// Comma-separated tags.
    #[arg(long, value_delimiter = ',')]
    tags: Vec<String>,
}

#[derive(Args)]
pub struct SegTemplateUpdateArgs {
    /// Template UUID.
    id: String,

    /// New name, if changing.
    #[arg(long)]
    name: Option<String>,

    /// New slug, if changing (lowercase alphanumeric + hyphens only).
    #[arg(long)]
    slug: Option<String>,

    /// New description, if changing.
    #[arg(long)]
    description: Option<String>,

    /// New system prompt, if changing.
    #[arg(long = "system-prompt")]
    system_prompt: Option<String>,

    /// Raw JSON object to replace the output schema.
    #[arg(long = "output-schema")]
    output_schema: Option<String>,

    /// Raw JSON object to replace the default segmentation parameters.
    #[arg(long)]
    defaults: Option<String>,

    /// Comma-separated tags (replaces the existing tag set).
    #[arg(long, value_delimiter = ',')]
    tags: Option<Vec<String>>,
}

pub async fn run_templates_list(client: &EnscriveClient, fmt: OutputFormat) {
    let command = "segmentation-templates list";
    match client.get_json("/v1/segmentation-templates").await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

pub async fn run_templates_get(client: &EnscriveClient, fmt: OutputFormat, args: &SegTemplateIdArgs) {
    let command = "segmentation-templates get";
    let path = format!("/v1/segmentation-templates/{}", args.id);
    match client.get_json(&path).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

pub async fn run_templates_create(client: &EnscriveClient, fmt: OutputFormat, args: &SegTemplateCreateArgs) {
    let command = "segmentation-templates create";

    let output_schema = match parse_optional_json(&args.output_schema, "output-schema") {
        Ok(v) => v,
        Err(e) => CliResponse::fail(command, e, FailureClass::Bug, EXIT_CONFIG).emit(fmt),
    };
    let defaults = match parse_optional_json(&args.defaults, "defaults") {
        Ok(v) => v,
        Err(e) => CliResponse::fail(command, e, FailureClass::Bug, EXIT_CONFIG).emit(fmt),
    };

    let body = json!({
        "name": args.name,
        "slug": args.slug,
        "description": args.description,
        "system_prompt": args.system_prompt,
        "output_schema": output_schema,
        "defaults": defaults,
        "tags": args.tags,
    });

    match client.post_json("/v1/segmentation-templates", body).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

pub async fn run_templates_update(client: &EnscriveClient, fmt: OutputFormat, args: &SegTemplateUpdateArgs) {
    let command = "segmentation-templates update";

    let output_schema = match parse_optional_json_field(&args.output_schema, "output-schema") {
        Ok(v) => v,
        Err(e) => CliResponse::fail(command, e, FailureClass::Bug, EXIT_CONFIG).emit(fmt),
    };
    let defaults = match parse_optional_json_field(&args.defaults, "defaults") {
        Ok(v) => v,
        Err(e) => CliResponse::fail(command, e, FailureClass::Bug, EXIT_CONFIG).emit(fmt),
    };

    let path = format!("/v1/segmentation-templates/{}", args.id);
    let body = json!({
        "name": args.name,
        "slug": args.slug,
        "description": args.description,
        "system_prompt": args.system_prompt,
        "output_schema": output_schema,
        "defaults": defaults,
        "tags": args.tags,
    });

    match client.put_json(&path, body).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

pub async fn run_templates_delete(client: &EnscriveClient, fmt: OutputFormat, args: &SegTemplateIdArgs) {
    let command = "segmentation-templates delete";
    let path = format!("/v1/segmentation-templates/{}", args.id);
    match client.delete_json(&path).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

pub async fn run_templates_clone(client: &EnscriveClient, fmt: OutputFormat, args: &SegTemplateIdArgs) {
    let command = "segmentation-templates clone";
    let path = format!("/v1/segmentation-templates/{}/clone", args.id);
    match client.post_json(&path, json!({})).await {
        Ok(data) => CliResponse::success(command, data).emit(fmt),
        Err(e) => crate::request_failure(command, e).emit(fmt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_preview_chunking_inline_content() {
        let args = crate::Cli::parse_from([
            "enscrive",
            "preview-chunking",
            "--content",
            "hello world",
            "--voice-id",
            "v-1",
        ]);
        match args.command {
            crate::Commands::PreviewChunking(PreviewChunkingArgs {
                content,
                content_file,
                voice_id,
            }) => {
                assert_eq!(content.as_deref(), Some("hello world"));
                assert_eq!(content_file, None);
                assert_eq!(voice_id.as_deref(), Some("v-1"));
            }
            _ => panic!("expected preview-chunking"),
        }
    }

    #[test]
    fn parse_segmentation_templates_create_with_tags() {
        let args = crate::Cli::parse_from([
            "enscrive",
            "segmentation-templates",
            "create",
            "--name",
            "Story Beats",
            "--slug",
            "story-beats",
            "--system-prompt",
            "Segment by story beat.",
            "--tags",
            "fiction,beats",
        ]);
        match args.command {
            crate::Commands::SegmentationTemplates {
                sub:
                    SegmentationTemplatesSubcommand::Create(SegTemplateCreateArgs {
                        name,
                        slug,
                        system_prompt,
                        tags,
                        ..
                    }),
            } => {
                assert_eq!(name, "Story Beats");
                assert_eq!(slug, "story-beats");
                assert_eq!(system_prompt, "Segment by story beat.");
                assert_eq!(tags, vec!["fiction".to_string(), "beats".to_string()]);
            }
            _ => panic!("expected segmentation-templates create"),
        }
    }

    #[test]
    fn parse_segmentation_templates_get_positional_id() {
        let args = crate::Cli::parse_from([
            "enscrive",
            "segmentation-templates",
            "get",
            "11111111-1111-1111-1111-111111111111",
        ]);
        match args.command {
            crate::Commands::SegmentationTemplates {
                sub: SegmentationTemplatesSubcommand::Get(SegTemplateIdArgs { id }),
            } => {
                assert_eq!(id, "11111111-1111-1111-1111-111111111111");
            }
            _ => panic!("expected segmentation-templates get"),
        }
    }

    #[test]
    fn parse_optional_json_defaults_to_empty_object() {
        assert_eq!(parse_optional_json(&None, "defaults").unwrap(), json!({}));
    }

    #[test]
    fn parse_optional_json_parses_supplied_value() {
        let raw = Some(r#"{"model": "gpt-5-mini"}"#.to_string());
        assert_eq!(
            parse_optional_json(&raw, "defaults").unwrap(),
            json!({"model": "gpt-5-mini"})
        );
    }

    #[test]
    fn parse_optional_json_rejects_invalid_json() {
        let raw = Some("not json".to_string());
        let err = parse_optional_json(&raw, "defaults").expect_err("must reject invalid JSON");
        assert!(err.contains("defaults"));
    }

    #[test]
    fn parse_optional_json_field_absent_stays_none() {
        // Unlike `parse_optional_json`, an absent flag must NOT default to
        // `{}` here — the update path relies on `None` meaning "leave this
        // field untouched server-side" (COALESCE semantics).
        assert_eq!(parse_optional_json_field(&None, "defaults").unwrap(), None);
    }

    #[test]
    fn parse_optional_json_field_present_parses() {
        let raw = Some(r#"{"model": "gpt-5-mini"}"#.to_string());
        assert_eq!(
            parse_optional_json_field(&raw, "defaults").unwrap(),
            Some(json!({"model": "gpt-5-mini"}))
        );
    }
}
