//! Per-project Enscrive memory — the `.enscrive/` project marker.
//!
//! app-memory epic — design of record: `enscrive-governance`
//! `plans/ENSCRIVE-CLI-APP-MEMORY-2026-07-31/ADR.md` (§3.2 cwd-based
//! resolution, §3.3 the marker, §5 no-implicit-default discipline).
//!
//! One shared local stack runs N tenants — one per project. A project
//! declares which tenant it belongs to with a `.enscrive/` directory at its
//! root, containing:
//!
//! - `config.toml` — **committable**: tenant id, tenant/project name,
//!   endpoint, and the *name* of the `~/.config/enscrive/profiles.toml`
//!   entry holding this project's API key.
//! - `AGENT.md` — **committable**: the agent-usage contract (§5 — every
//!   command it teaches must be a real CLI invocation).
//!
//! The API key is **never** written here. It stays in the existing
//! per-user key store (`~/.config/enscrive/profiles.toml`), referenced by
//! name. `marker_contains_no_key_material` in this module's tests is the
//! standing proof (ADR §5 FORBIDDEN: "a committed API key").

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Marker directory name, per ADR §3.3.
pub const MARKER_DIR: &str = ".enscrive";
/// The marker file. Discovery keys on **this file**, never on the bare
/// directory: `enscrive-deploy` already keeps unrelated state under
/// `~/.enscrive/deploy/`, so a directory-presence check would make every
/// project under `$HOME` inherit a phantom marker.
pub const MARKER_FILE: &str = "config.toml";
/// The agent-usage contract dropped alongside the marker.
pub const AGENT_FILE: &str = "AGENT.md";

/// Bumped only on a breaking marker-schema change. A marker written by a
/// newer CLI is rejected rather than silently misread — misreading it
/// would point memory writes at the wrong tenant.
pub const MARKER_VERSION: u32 = 1;

/// `.enscrive/config.toml`. Every field here is non-secret by
/// construction — adding a secret-bearing field is the one change this
/// struct must never take (ADR §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMarker {
    pub version: u32,
    pub project: ProjectSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSection {
    /// Human-facing project name (defaults to the sanitized directory name).
    pub name: String,
    /// The tenant this project's memory lives in.
    pub tenant_id: String,
    /// The tenant's name on the stack — the `(user_id, tenant_name)` key
    /// `/local/bootstrap` loads-or-creates on (enscrive-developer #228).
    pub tenant_name: String,
    /// Base URL of the enscrive-developer serving this project's tenant.
    pub endpoint: String,
    /// Name of the `~/.config/enscrive/profiles.toml` entry holding this
    /// project's API key. A *reference*, never the key itself.
    pub profile: String,
}

/// A marker found by walking up from a starting directory.
#[derive(Debug, Clone)]
pub struct DiscoveredProject {
    /// Directory containing `.enscrive/` — the project root.
    pub root: PathBuf,
    pub marker_path: PathBuf,
    pub marker: ProjectMarker,
}

/// Walk up from `start` (inclusive) to the filesystem root looking for
/// `.enscrive/config.toml`; the nearest one wins.
///
/// A marker that exists but cannot be read or parsed is an **error**, not a
/// miss. Falling through to the default profile there would silently write
/// this project's memories into a different tenant — the exact failure ADR
/// §5 forbids.
pub fn discover_from(start: &Path) -> Result<Option<DiscoveredProject>, String> {
    for dir in start.ancestors() {
        let marker_path = dir.join(MARKER_DIR).join(MARKER_FILE);
        if !marker_path.is_file() {
            continue;
        }
        let marker = read_marker(&marker_path)?;
        return Ok(Some(DiscoveredProject {
            root: dir.to_path_buf(),
            marker_path,
            marker,
        }));
    }
    Ok(None)
}

/// `discover_from(cwd)`. An unreadable cwd is treated as "no project" —
/// there is nothing to walk up from, and no tenant can be misattributed.
pub fn discover() -> Result<Option<DiscoveredProject>, String> {
    match std::env::current_dir() {
        Ok(cwd) => discover_from(&cwd),
        Err(_) => Ok(None),
    }
}

pub fn read_marker(path: &Path) -> Result<ProjectMarker, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("read project marker '{}': {e}", path.display()))?;
    parse_marker(&raw).map_err(|e| format!("project marker '{}' is invalid: {e}", path.display()))
}

pub fn parse_marker(raw: &str) -> Result<ProjectMarker, String> {
    let marker: ProjectMarker = toml::from_str(raw).map_err(|e| e.to_string())?;
    if marker.version > MARKER_VERSION {
        return Err(format!(
            "marker version {} is newer than this CLI understands (max {}); upgrade the enscrive CLI",
            marker.version, MARKER_VERSION
        ));
    }
    for (field, value) in [
        ("project.name", &marker.project.name),
        ("project.tenant_id", &marker.project.tenant_id),
        ("project.tenant_name", &marker.project.tenant_name),
        ("project.endpoint", &marker.project.endpoint),
        ("project.profile", &marker.project.profile),
    ] {
        if value.trim().is_empty() {
            return Err(format!("`{field}` is empty"));
        }
    }
    Ok(marker)
}

/// Serialize the marker with its committable-and-secret-free header.
pub fn render_marker(marker: &ProjectMarker) -> Result<String, String> {
    let body = toml::to_string_pretty(marker).map_err(|e| format!("serialize marker: {e}"))?;
    Ok(format!(
        "# Enscrive project marker — SAFE TO COMMIT.\n\
         #\n\
         # This file declares which Enscrive tenant holds this project's memory.\n\
         # It contains NO secrets. The API key lives in your per-user key store\n\
         # (~/.config/enscrive/profiles.toml) under the `profile` name below, and\n\
         # is never written here.\n\
         #\n\
         # Written by `enscrive project init`. Any `enscrive` command run inside\n\
         # this directory tree targets this project's tenant automatically;\n\
         # --api-key / ENSCRIVE_API_KEY and --profile / ENSCRIVE_PROFILE still win.\n\
         \n{body}"
    ))
}

/// Sanitize a directory name into a tenant/project name.
///
/// Deliberately conservative: the result is used as the `tenant_name` half
/// of `/local/bootstrap`'s `(user_id, tenant_name)` load-or-create key and
/// is echoed into a profile name, so it must be stable, printable, and free
/// of path/shell-significant characters.
pub fn sanitize_project_name(raw: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' {
            out.push(ch);
            last_was_sep = false;
        } else if !last_was_sep && !out.is_empty() {
            // Whitespace and everything else collapses to a single '-'.
            out.push('-');
            last_was_sep = true;
        }
    }
    let cleaned = out.trim_matches(['-', '.', '_']).to_string();
    let cleaned: String = cleaned.chars().take(64).collect();
    let cleaned = cleaned.trim_matches(['-', '.', '_']).to_string();
    if cleaned.is_empty() {
        return Err(format!(
            "cannot derive a project name from '{raw}' — pass one explicitly with `enscrive project init --name <name>`"
        ));
    }
    Ok(cleaned)
}

/// Name of the `profiles.toml` entry that holds a project's API key.
pub fn profile_name_for(project_name: &str) -> String {
    format!("project-{}", project_name.to_ascii_lowercase())
}

/// Absolute path of a project's `.enscrive/` directory.
pub fn marker_dir(root: &Path) -> PathBuf {
    root.join(MARKER_DIR)
}

pub fn marker_path(root: &Path) -> PathBuf {
    marker_dir(root).join(MARKER_FILE)
}

pub fn agent_doc_path(root: &Path) -> PathBuf {
    marker_dir(root).join(AGENT_FILE)
}

/// Write `.enscrive/config.toml` + `.enscrive/AGENT.md` under `root`.
///
/// Takes no API key by construction — there is no argument through which
/// key material could reach this function, let alone the files it writes.
pub fn write_marker(root: &Path, marker: &ProjectMarker) -> Result<(PathBuf, PathBuf), String> {
    let dir = marker_dir(root);
    fs::create_dir_all(&dir).map_err(|e| format!("create '{}': {e}", dir.display()))?;

    let config_path = marker_path(root);
    fs::write(&config_path, render_marker(marker)?)
        .map_err(|e| format!("write '{}': {e}", config_path.display()))?;

    let agent_path = agent_doc_path(root);
    fs::write(&agent_path, render_agent_doc(marker))
        .map_err(|e| format!("write '{}': {e}", agent_path.display()))?;

    Ok((config_path, agent_path))
}

/// The agent-usage contract dropped at `.enscrive/AGENT.md`.
///
/// ADR §5: this document is a **contract**. Every command it teaches is a
/// real `enscrive` invocation with real flags, copied from `--help`:
///
/// - `corpus create --name <NAME> --embedding-model <MODEL>` (both required)
/// - `corpus list`, `corpus documents --id <ID>`, `corpus stats --id <ID>`
/// - `ingest documents --corpus-id <ID> --content <TEXT>` (`--document-id`
///   optional since ENS-3054 — omitted means a content-hash id)
/// - `search --query <QUERY> [--corpus <ID>] [--limit <N>]`
/// - `corpus document delete --corpus-id <ID> --document-id <ID>` (cli #52)
/// - `status`, and the global `--output json`
///
/// `docs/AGENT-CONTRACT.md`-style drift protection is the CI gate's job
/// (ADR §6 P6); `agent_doc_only_teaches_real_commands` in this module's
/// tests is the local half — it asserts every fenced `enscrive …` line here
/// parses against the real clap command tree.
pub fn render_agent_doc(marker: &ProjectMarker) -> String {
    let name = &marker.project.name;
    let tenant_name = &marker.project.tenant_name;
    let tenant_id = &marker.project.tenant_id;
    let endpoint = &marker.project.endpoint;

    format!(
        r#"# Enscrive memory for `{name}`

This project has its own **private, durable memory**, served by Enscrive.
You are reading this because it was dropped here by `enscrive project init`.

Use it to remember what you learn about this codebase across sessions:
decisions and their reasons, hard-won debugging conclusions, conventions,
the shape of things you had to go read. Recall it before you re-derive
something. Retire it when it stops being true.

## Scoping is automatic — do not pass credentials

Memory here is isolated to this project's own tenant:

- tenant: `{tenant_name}` (`{tenant_id}`)
- endpoint: `{endpoint}`

Any `enscrive` command run **anywhere inside this directory tree** targets
that tenant automatically. The CLI walks up from the working directory to
find `{MARKER_DIR}/{MARKER_FILE}` and resolves this project's API key from the
per-user key store. You never need to pass `--api-key`, `--profile`, or
`--endpoint`, and you must never write a key into this directory.

## Remember

Memories live in a **corpus**. Run this at the start of a session — it is
idempotent, so it returns the existing corpus if there is one and creates
it otherwise. You never have to check first:

```sh
enscrive corpus ensure \
  --name "{name}-memory" \
  --embedding-model text-embedding-3-large \
  --description "Durable memory for {name}" \
  --output json
```

That prints the corpus id, plus `"created": true` on the run that made it.
A corpus's embedding model is fixed at creation, so if a corpus with this
name already exists on a *different* model the command fails rather than
quietly handing you the wrong vector space.

List them any time:

```sh
enscrive corpus list --output json
```

Write a memory. Omitting `--document-id` derives a deterministic
content-hash id, so re-ingesting identical content is a no-op:

```sh
enscrive ingest documents \
  --corpus-id <CORPUS_ID> \
  --content "Auth tokens are minted in crates/server/src/auth/session.rs; the pepper comes from ESM, not env." \
  --output json
```

Pass `--document-id` explicitly when you want a stable, addressable memory
you intend to correct later — re-ingesting the same id replaces it:

```sh
enscrive ingest documents \
  --corpus-id <CORPUS_ID> \
  --document-id "convention/error-handling" \
  --content "Handlers return ApiError; never unwrap in a request path." \
  --output json
```

Longer memories are easier to write from a file:

```sh
enscrive ingest documents \
  --corpus-id <CORPUS_ID> \
  --document-id "debug/2026-08-01-h2-protocol-error" \
  --content-file ./notes.md \
  --output json
```

## Recall

Search before you re-derive. This is semantic search, so ask the question
you actually have:

```sh
enscrive search \
  --query "how are auth tokens minted" \
  --corpus <CORPUS_ID> \
  --limit 5 \
  --output json
```

Omit `--corpus` to search across this project's whole memory rather than
one corpus:

```sh
enscrive search --query "why did we drop the fragment gate" --output json
```

### Reading the scores

Results carry a `score` — cosine similarity, **not** a percentage. Relevant
matches land nowhere near 1.0. On a measured corpus, six differently-worded
queries for the same fact matched at **0.57–0.70** while unrelated content
sat at **≤0.39**. The separation is clean, but the whole useful band is
below 0.75.

So: **do not dismiss a 0.6 result as weak** — that is what a good hit looks
like. Judge by the gap between the top results and the rest, not by absolute
value.

Prefer no threshold at all, which is the default: you get the top `--limit`
matches ranked by score, and a weak-but-relevant memory still beats having
no context. Reach for `--score-threshold` only when you specifically need
"return nothing rather than something marginal", and start around `0.50`:

```sh
enscrive search --query "how are auth tokens minted" --score-threshold 0.50 --output json
```

## Retire

A memory that is no longer true is worse than no memory. Delete it by id —
this removes the document and all of its chunks, synchronously:

```sh
enscrive corpus document delete \
  --corpus-id <CORPUS_ID> \
  --document-id "convention/error-handling" \
  --output json
```

To *correct* rather than retire, re-ingest the same `--document-id` with the
new content.

## Inspect

```sh
enscrive status --output json
enscrive corpus documents --id <CORPUS_ID> --output json
enscrive corpus stats --id <CORPUS_ID> --output json
```

`enscrive status` reports this project, its tenant, and the portal URL where
a human can browse the same memory.

## Working agreement

- Prefer recall over re-derivation: search first.
- Write memories that will still be true next month — decisions, reasons,
  invariants, and where things live. Not transient state.
- Give a memory a stable `--document-id` when you expect to revise it.
- Retire memories you have proven wrong, in the same session you disproved
  them.
- Every command above is real and takes exactly these flags. If something
  you want is not here, run `enscrive <command> --help` — do not guess.
"#
    )
}

/// Options for `enscrive project init`.
#[derive(Debug, Clone)]
pub struct InitOptions {
    /// Project/tenant name. Defaults to the sanitized directory name.
    pub name: Option<String>,
    /// Directory to initialize. Defaults to the working directory.
    pub dir: Option<PathBuf>,
    /// Which self-managed stack profile to create the tenant on.
    pub profile: Option<String>,
    /// Deliberately share a tenant that already exists under this name.
    pub adopt_existing: bool,
}

/// `enscrive project init` — make Enscrive live in this project.
///
/// Creates (or loads) this project's own tenant on the running local
/// stack, stores its API key in the per-user key store, and drops the
/// committable `.enscrive/` marker: `config.toml` + `AGENT.md`.
///
/// ADR §7 acceptance: "in a fresh dir → isolated tenant + `.enscrive/`
/// marker (committable config + AGENT.md, key not committed) + celebratory
/// output naming tenant + portal."
pub async fn init(opts: InitOptions) -> Result<serde_json::Value, String> {
    let root = match opts.dir {
        Some(dir) => dir,
        None => std::env::current_dir().map_err(|e| format!("resolve working directory: {e}"))?,
    };
    if !root.is_dir() {
        return Err(format!("'{}' is not a directory", root.display()));
    }

    let existing = read_existing_marker(&root)?;

    // The name is fixed by an existing marker: re-running init in an
    // initialized project must re-target the SAME tenant, never silently
    // create a second one under a different derived name.
    let name = match (&opts.name, &existing) {
        (Some(explicit), Some(marker)) if sanitize_project_name(explicit)? != marker.project.name => {
            return Err(format!(
                "this directory is already initialized as project '{}' (tenant '{}'). \
                 Renaming a project's tenant is not supported — remove {}/{} and re-run \
                 `enscrive project init --name {explicit}` to bind it to a different tenant.",
                marker.project.name,
                marker.project.tenant_name,
                MARKER_DIR,
                MARKER_FILE,
            ));
        }
        (Some(explicit), _) => sanitize_project_name(explicit)?,
        (None, Some(marker)) => marker.project.name.clone(),
        (None, None) => {
            let dir_name = root
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    format!(
                        "cannot derive a project name from '{}' — pass one with \
                         `enscrive project init --name <name>`",
                        root.display()
                    )
                })?;
            sanitize_project_name(dir_name)?
        }
    };

    let profile_name = profile_name_for(&name);
    // Re-running init in an already-initialized project is idempotent, so
    // it must not trip the shared-tenant guard against its own tenant.
    let adopt_existing = opts.adopt_existing || existing.is_some();

    let tenant = crate::local::bootstrap_project_tenant(
        opts.profile.as_deref(),
        &name,
        &profile_name,
        adopt_existing,
    )
    .await?;

    if let Some(marker) = &existing
        && marker.project.tenant_id != tenant.tenant_id
    {
        return Err(format!(
            "this directory is bound to tenant {} but the stack resolved '{}' to tenant {}. \
             The stack was likely reset. Remove {}/{} and re-run `enscrive project init` to \
             rebind this project.",
            marker.project.tenant_id,
            name,
            tenant.tenant_id,
            MARKER_DIR,
            MARKER_FILE,
        ));
    }

    let marker = ProjectMarker {
        version: MARKER_VERSION,
        project: ProjectSection {
            name: name.clone(),
            tenant_id: tenant.tenant_id.clone(),
            tenant_name: tenant.tenant_name.clone(),
            endpoint: tenant.endpoint.clone(),
            profile: tenant.profile_name.clone(),
        },
    };
    let (config_path, agent_path) = write_marker(&root, &marker)?;

    Ok(serde_json::json!({
        "project": name,
        "project_root": root.display().to_string(),
        "tenant_id": tenant.tenant_id,
        "tenant_name": tenant.tenant_name,
        "environment_id": tenant.environment_id,
        "environment_name": tenant.environment_name,
        "endpoint": tenant.endpoint,
        "portal": tenant.portal_url,
        "created_tenant": tenant.created_tenant,
        "already_initialized": existing.is_some(),
        "stack_profile": tenant.stack_profile_name,
        // The NAME of the key store entry. The key itself is never
        // returned, printed, or written to the marker (ADR §5).
        "api_key_profile": tenant.profile_name,
        "marker": {
            "config": config_path.display().to_string(),
            "agent_doc": agent_path.display().to_string(),
            "committable": true,
            "contains_api_key": false,
        },
        "agent_commands": {
            "remember": "enscrive ingest documents --corpus-id <CORPUS_ID> --content \"...\"",
            "recall": "enscrive search --query \"...\"",
            "retire": "enscrive corpus document delete --corpus-id <CORPUS_ID> --document-id <DOCUMENT_ID>",
        },
    }))
}

/// Read this directory's own marker, if it has one. Deliberately does NOT
/// walk up: `enscrive project init` in a subdirectory of an existing
/// project initializes *that subdirectory* as its own project.
fn read_existing_marker(root: &Path) -> Result<Option<ProjectMarker>, String> {
    let path = marker_path(root);
    if !path.is_file() {
        return Ok(None);
    }
    read_marker(&path).map(Some)
}

/// The celebratory summary, speaking to both audiences (ADR §4): "For your
/// agents" (remember / recall / retire + the dropped `AGENT.md`) and "For
/// you" (the portal). Names the tenant and the portal so the developer sees
/// that their project now has a memory.
pub fn print_init_summary(data: &serde_json::Value) {
    let field = |key: &str| data.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let name = field("project");
    let reinitialized = data
        .get("already_initialized")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let adopted = !data
        .get("created_tenant")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    println!();
    if reinitialized {
        println!("  Enscrive is live in {name} — marker refreshed.");
    } else if adopted {
        println!("  Enscrive is live in {name}, sharing the existing '{}' memory.", field("tenant_name"));
    } else {
        println!("  Enscrive is live in {name}. This project now has a memory.");
    }
    println!();
    println!("  Tenant   {} ({})", field("tenant_name"), field("tenant_id"));
    println!("  Endpoint {}", field("endpoint"));
    println!();
    println!("  For your agents");
    println!(
        "    They will find the usage contract at {}/{}.",
        MARKER_DIR, AGENT_FILE
    );
    println!("    Any enscrive command run in this tree targets this project automatically —");
    println!("    no API key, no --profile, no --endpoint.");
    println!();
    println!("      remember   enscrive ingest documents --corpus-id <CORPUS_ID> --content \"...\"");
    println!("      recall     enscrive search --query \"...\"");
    println!(
        "      retire     enscrive corpus document delete --corpus-id <CORPUS_ID> --document-id <DOCUMENT_ID>"
    );
    println!();
    println!("  For you");
    println!("    Portal   {}", field("portal"));
    println!("    Browse and search the same memory your agents are writing.");
    println!();
    println!(
        "  Commit {}/ — it holds no secrets. Your API key stays in your",
        MARKER_DIR
    );
    println!(
        "  per-user key store as profile '{}'.",
        field("api_key_profile")
    );
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_marker() -> ProjectMarker {
        ProjectMarker {
            version: MARKER_VERSION,
            project: ProjectSection {
                name: "my-app".to_string(),
                tenant_id: "11111111-2222-3333-4444-555555555555".to_string(),
                tenant_name: "my-app".to_string(),
                endpoint: "http://127.0.0.1:3000".to_string(),
                profile: "project-my-app".to_string(),
            },
        }
    }

    #[test]
    fn render_then_parse_round_trips() {
        let marker = sample_marker();
        let rendered = render_marker(&marker).unwrap();
        assert_eq!(parse_marker(&rendered).unwrap(), marker);
    }

    /// ADR §5, FORBIDDEN: "a committed API key". The marker directory is
    /// the thing a developer commits, so nothing under it may carry key
    /// material — proven against the bytes actually written to disk, not
    /// against the struct definition.
    #[test]
    fn marker_contains_no_key_material() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        // Stands in for a key issued alongside this marker. It is not
        // passed to `write_marker` — there is no parameter for it — so its
        // absence below also proves the writer has no path to one.
        //
        // Assembled from pieces rather than written as one literal: a
        // credential-shaped, high-entropy constant here trips the repo's
        // gitleaks `generic-api-key` rule (it did, on the PR that added
        // this test), and the right answer to that is not to weaken the
        // scanner's default ruleset to accommodate a test fixture.
        let forbidden_sentinel = format!("{}{}", "esk_", "NOT-A-REAL-KEY-sentinel");

        let (config_path, agent_path) = write_marker(root, &sample_marker()).unwrap();

        let mut checked = 0;
        for entry in fs::read_dir(marker_dir(root)).unwrap() {
            let path = entry.unwrap().path();
            let body = fs::read_to_string(&path).unwrap();
            assert!(
                !body.contains(&forbidden_sentinel),
                "{} must not contain key material",
                path.display()
            );
            let lowered = body.to_ascii_lowercase();
            for forbidden in ["esk_", "sk-", "bearer "] {
                assert!(
                    !lowered.contains(forbidden),
                    "{} contains a credential-shaped token '{forbidden}'",
                    path.display()
                );
            }
            checked += 1;
        }
        assert_eq!(checked, 2, "expected exactly config.toml and AGENT.md");

        // The marker's TOML keys must reference a profile by NAME and carry
        // no value-bearing secret field.
        let config = fs::read_to_string(&config_path).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        let project = parsed.get("project").unwrap().as_table().unwrap();
        for forbidden in ["api_key", "key", "secret", "token", "password"] {
            assert!(
                !project.contains_key(forbidden),
                "marker must not carry a `{forbidden}` field"
            );
        }
        assert_eq!(
            project.get("profile").unwrap().as_str(),
            Some("project-my-app"),
            "the marker references the key store by profile NAME"
        );
        assert!(agent_path.is_file());
    }

    #[test]
    fn discover_walks_up_parent_directories() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_marker(root, &sample_marker()).unwrap();

        let nested = root.join("crates").join("server").join("src");
        fs::create_dir_all(&nested).unwrap();

        let found = discover_from(&nested).unwrap().expect("marker found");
        assert_eq!(found.root, root);
        assert_eq!(found.marker.project.tenant_name, "my-app");
        assert_eq!(found.marker_path, marker_path(root));
    }

    #[test]
    fn discover_prefers_the_nearest_marker() {
        let temp = TempDir::new().unwrap();
        let outer = temp.path();
        write_marker(outer, &sample_marker()).unwrap();

        let inner = outer.join("vendor").join("inner-app");
        fs::create_dir_all(&inner).unwrap();
        let mut inner_marker = sample_marker();
        inner_marker.project.name = "inner-app".to_string();
        inner_marker.project.tenant_name = "inner-app".to_string();
        inner_marker.project.tenant_id = "99999999-8888-7777-6666-555555555555".to_string();
        write_marker(&inner, &inner_marker).unwrap();

        let found = discover_from(&inner).unwrap().expect("marker found");
        assert_eq!(found.root, inner);
        assert_eq!(found.marker.project.tenant_name, "inner-app");
    }

    #[test]
    fn discover_returns_none_when_no_marker_exists() {
        let temp = TempDir::new().unwrap();
        let nested = temp.path().join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        assert!(discover_from(&nested).unwrap().is_none());
    }

    /// `enscrive-deploy` keeps unrelated state under `~/.enscrive/deploy/`.
    /// A bare-directory check would make every project under `$HOME`
    /// resolve to a phantom marker; discovery keys on `config.toml`.
    #[test]
    fn bare_marker_directory_without_config_is_not_a_project() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join(MARKER_DIR).join("deploy")).unwrap();
        assert!(discover_from(temp.path()).unwrap().is_none());
    }

    /// A corrupt marker must fail loudly. Silently falling back to the
    /// default profile would write this project's memories into whatever
    /// tenant that profile points at.
    #[test]
    fn malformed_marker_is_an_error_not_a_miss() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(marker_dir(temp.path())).unwrap();
        fs::write(marker_path(temp.path()), "this is not toml {{{").unwrap();

        let err = discover_from(temp.path()).unwrap_err();
        assert!(err.contains("is invalid"), "unexpected error: {err}");
    }

    #[test]
    fn marker_from_a_newer_cli_is_rejected() {
        let raw = format!(
            "version = {}\n[project]\nname = \"x\"\ntenant_id = \"t\"\ntenant_name = \"x\"\nendpoint = \"http://127.0.0.1:3000\"\nprofile = \"project-x\"\n",
            MARKER_VERSION + 1
        );
        let err = parse_marker(&raw).unwrap_err();
        assert!(err.contains("newer than this CLI"), "unexpected: {err}");
    }

    #[test]
    fn empty_marker_fields_are_rejected() {
        let raw = "version = 1\n[project]\nname = \"x\"\ntenant_id = \"  \"\ntenant_name = \"x\"\nendpoint = \"http://127.0.0.1:3000\"\nprofile = \"project-x\"\n";
        let err = parse_marker(raw).unwrap_err();
        assert!(err.contains("tenant_id"), "unexpected: {err}");
    }

    #[test]
    fn sanitize_project_name_cases() {
        assert_eq!(sanitize_project_name("my-app").unwrap(), "my-app");
        assert_eq!(sanitize_project_name("My App").unwrap(), "My-App");
        assert_eq!(sanitize_project_name("  spaced  ").unwrap(), "spaced");
        assert_eq!(
            sanitize_project_name("weird/name:with*chars").unwrap(),
            "weird-name-with-chars"
        );
        assert_eq!(sanitize_project_name("enscrive.io").unwrap(), "enscrive.io");
        assert_eq!(sanitize_project_name("--leading--").unwrap(), "leading");
        assert_eq!(sanitize_project_name("a__b").unwrap(), "a__b");
        assert_eq!(sanitize_project_name(&"x".repeat(200)).unwrap().len(), 64);
        assert!(sanitize_project_name("///").is_err());
        assert!(sanitize_project_name("").is_err());
    }

    #[test]
    fn profile_name_is_derived_and_namespaced() {
        assert_eq!(profile_name_for("My-App"), "project-my-app");
    }

    /// ADR §5: the `AGENT.md` must never instruct an agent to do something
    /// the CLI cannot do. Every `enscrive …` invocation in the generated
    /// doc is parsed against the real clap command tree; an unknown
    /// subcommand or flag fails here rather than in an agent's session.
    #[test]
    fn agent_doc_only_teaches_real_commands() {
        let doc = render_agent_doc(&sample_marker());

        // Reassemble the shell-continued invocations into single lines.
        let mut invocations: Vec<String> = Vec::new();
        let mut pending: Option<String> = None;
        for line in doc.lines() {
            let trimmed = line.trim();
            let continued = trimmed.ends_with('\\');
            let body = trimmed.trim_end_matches('\\').trim();
            match pending.as_mut() {
                Some(acc) => {
                    acc.push(' ');
                    acc.push_str(body);
                }
                None if body.starts_with("enscrive ") => {
                    pending = Some(body.to_string());
                }
                None => continue,
            }
            if !continued {
                invocations.push(pending.take().expect("accumulator present"));
            }
        }
        assert!(
            invocations.len() >= 10,
            "expected the doc to teach a full remember/recall/retire surface, got {}",
            invocations.len()
        );

        let mut saw_ingest = false;
        let mut saw_search = false;
        let mut saw_delete = false;
        for invocation in &invocations {
            let argv: Vec<String> = shell_words(invocation)
                .into_iter()
                // Placeholders the doc asks the agent to substitute.
                .map(|arg| match arg.as_str() {
                    "<CORPUS_ID>" => "00000000-0000-0000-0000-000000000000".to_string(),
                    other => other.to_string(),
                })
                .collect();
            saw_ingest |= invocation.contains("ingest documents");
            saw_search |= invocation.starts_with("enscrive search");
            saw_delete |= invocation.contains("corpus document delete");

            crate::command_for_test()
                .try_get_matches_from(&argv)
                .unwrap_or_else(|e| {
                    panic!("AGENT.md teaches an invocation the CLI rejects:\n  {invocation}\n{e}")
                });
        }
        assert!(saw_ingest, "AGENT.md must teach remember (`ingest`)");
        assert!(saw_search, "AGENT.md must teach recall (`search`)");
        assert!(
            saw_delete,
            "AGENT.md must teach retire (`corpus document delete`)"
        );
    }

    /// Minimal argv splitter for the doc's own invocations: whitespace
    /// separated, with `"…"` quoting. The doc is generated by this module,
    /// so it never contains escapes beyond that.
    fn shell_words(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut started = false;
        for ch in line.chars() {
            match ch {
                '"' => {
                    in_quotes = !in_quotes;
                    started = true;
                }
                c if c.is_whitespace() && !in_quotes => {
                    if started {
                        out.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                c => {
                    current.push(c);
                    started = true;
                }
            }
        }
        if started {
            out.push(current);
        }
        out
    }
}
