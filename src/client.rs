use futures_util::StreamExt;
use reqwest::{Client, Method};
use serde_json::Value;
use std::fmt;

// ---------------------------------------------------------------------------
// ENS-84 CLI-REL-013: typed ApiError at the client boundary
// ---------------------------------------------------------------------------

/// Typed error produced by every `EnscriveClient` method.
///
/// Variants map 1-to-1 with the discriminations the previous string-heuristic
/// layer made, but are now decided inside the client rather than by pattern-
/// matching on error strings at call sites.
#[derive(Debug)]
pub enum ApiError {
    /// Transport / connection failure (reqwest could not complete the request).
    Network(reqwest::Error),

    /// HTTP response received but the body could not be parsed as JSON.
    /// Carries the raw status and the unparseable body text.
    InvalidResponse { status: u16, body: String },

    /// Server included a `failure_class` field in the JSON body.
    /// The class string is the raw server value (e.g. `"FAIL_PLAN_REQUIRED"`).
    ServerClassified {
        class: String,
        status: u16,
        body: Value,
    },

    /// 4xx response whose JSON body did NOT carry a `failure_class`.
    Http4xx {
        status: u16,
        /// Convenience extraction of a `code` field if present. Not read
        /// today (Display only prints `message`) but kept for callers that
        /// match on the variant directly; reserved for richer error
        /// reporting.
        #[allow(dead_code)]
        code: Option<String>,
        /// Convenience extraction of a `message` or `error` field.
        message: String,
        /// Raw JSON body. Not read today; reserved for richer error
        /// reporting alongside `code`.
        #[allow(dead_code)]
        body: Value,
    },

    /// 5xx response whose JSON body did NOT carry a `failure_class`.
    Http5xx { status: u16, body: Value },

    /// The response body carried a pre-launch refusal marker
    /// (`not_yet_available` / `phase: pre-launch`), on ANY status code.
    /// Maps directly to `FailureClass::Unsupported`.
    ///
    /// `status` is retained for diagnostics only — it is deliberately not a
    /// condition of this variant. See `is_pre_launch` / `interpret_response`.
    NotYetAvailable { status: u16 },

    /// Request timed out (reqwest timeout fires before a response arrives).
    Timeout,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // ENS-3054 W4 §4.7. A stopped stack is the single most common
            // failure a new developer hits, and reqwest's own Display is
            // a chain ending in "tcp connect error: Connection refused (os
            // error 111)" — accurate, and useless as a next step. Name the
            // endpoint that refused and the command that fixes it.
            ApiError::Network(e) if e.is_connect() => {
                let target = e
                    .url()
                    .map(|url| {
                        let mut base = url.clone();
                        base.set_path("");
                        base.set_query(None);
                        base.to_string()
                    })
                    .unwrap_or_else(|| "the Enscrive API".to_string());
                write!(
                    f,
                    "could not connect to {target} — nothing is listening there. \
                     If this is your local stack, run `enscrive start` (then `enscrive status` \
                     to confirm it came up). If you meant a different endpoint, check \
                     `--endpoint` / ENSCRIVE_BASE_URL and your profile."
                )
            }
            ApiError::Network(e) => write!(f, "request failed: {e}"),
            ApiError::InvalidResponse { status, body } => {
                write!(f, "HTTP {status}: {body}")
            }
            // ENS-3054 W4 §4.7: these two used to print the raw JSON body,
            // burying the one sentence that mattered inside a serialized
            // object. Extract the message the same way the 4xx path already
            // does; the compact body is the fallback only when the server
            // sent nothing recognizable.
            ApiError::ServerClassified {
                class,
                status,
                body,
            } => write!(f, "HTTP {status} [{class}]: {}", body_message(body)),
            ApiError::Http4xx {
                status, message, ..
            } => {
                write!(f, "HTTP {status}: {message}")?;
                // An expired or wrong key is otherwise a dead end: the
                // server can only say "invalid", not how to get a good one.
                if *status == 401 || *status == 403 {
                    write!(
                        f,
                        ". If this key is stale, re-issue one: `enscrive bootstrap --issue-key` \
                         for a local stack, or `enscrive project init` in a project directory. \
                         Check which key is in play with `enscrive status`."
                    )?;
                }
                Ok(())
            }
            ApiError::Http5xx { status, body } => {
                write!(f, "HTTP {status}: {}", body_message(body))
            }
            // "not yet available on public /v1" left a first-contact user with
            // nowhere to go. Name the situation and the path that does work.
            ApiError::NotYetAvailable { status } => write!(
                f,
                "this endpoint refused the request as not-yet-available (HTTP {status}). \
                 The managed plane at api.enscrive.io is pre-launch and is not accepting \
                 connections yet. Run your own stack instead: \
                 `enscrive init --mode self-managed`, then `enscrive start`."
            ),
            ApiError::Timeout => write!(
                f,
                "request timed out. If this is your local stack, check it is up with \
                 `enscrive status`; otherwise check `--endpoint` / ENSCRIVE_BASE_URL \
                 and your profile."
            ),
        }
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    /// ENS-3054 W4 §4.7. Built from a REAL refused connection rather than
    /// a hand-made variant, so this asserts on what a developer with a
    /// stopped stack actually sees. Port 1 is reserved and never listening.
    #[tokio::test]
    async fn connection_refused_names_the_endpoint_and_the_fix() {
        let error = Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("build client")
            .get("http://127.0.0.1:1/v1/search")
            .send()
            .await
            .expect_err("port 1 must refuse the connection");
        assert!(
            error.is_connect(),
            "expected a connect error, got: {error} (is something listening on port 1?)"
        );

        let rendered = ApiError::Network(error).to_string();
        assert!(
            rendered.contains("http://127.0.0.1:1"),
            "must name the endpoint that refused: {rendered}"
        );
        assert!(
            rendered.contains("enscrive start"),
            "must name the command that fixes it: {rendered}"
        );
        assert!(
            !rendered.contains("os error"),
            "must not leak the raw transport chain: {rendered}"
        );
    }

    /// A server-classified failure used to render its whole JSON body,
    /// burying the sentence that mattered. `wallet_unprovisioned` is the
    /// real 503 a clean-seat user hits before their wallet is provisioned.
    #[test]
    fn server_classified_shows_the_message_not_the_raw_body() {
        let rendered = ApiError::ServerClassified {
            class: "wallet_unprovisioned".to_string(),
            status: 503,
            body: serde_json::json!({
                "error": "tenant wallet is not provisioned",
                "failure_class": "wallet_unprovisioned",
            }),
        }
        .to_string();

        assert_eq!(
            rendered,
            "HTTP 503 [wallet_unprovisioned]: tenant wallet is not provisioned"
        );
        assert!(
            !rendered.contains('{'),
            "must not print the serialized body: {rendered}"
        );
    }

    #[test]
    fn http5xx_shows_the_message_not_the_raw_body() {
        let rendered = ApiError::Http5xx {
            status: 500,
            body: serde_json::json!({ "error": "no active rate card configured" }),
        }
        .to_string();
        assert_eq!(rendered, "HTTP 500: no active rate card configured");
    }

    /// Only when the body carries nothing recognizable do we fall back to
    /// the raw JSON — losing the detail entirely would be worse.
    #[test]
    fn unrecognized_body_falls_back_to_json() {
        let rendered = ApiError::Http5xx {
            status: 500,
            body: serde_json::json!({ "unexpected": [1, 2] }),
        }
        .to_string();
        assert!(rendered.contains("unexpected"), "got: {rendered}");
    }

    /// An auth failure is otherwise a dead end — the server can only say
    /// "invalid", never how to obtain a working key.
    #[test]
    fn auth_failures_say_how_to_get_a_new_key() {
        for status in [401u16, 403] {
            let rendered = ApiError::Http4xx {
                status,
                code: None,
                message: "invalid api key".to_string(),
                body: serde_json::json!({}),
            }
            .to_string();
            assert!(
                rendered.contains("invalid api key"),
                "must keep the server's message: {rendered}"
            );
            assert!(
                rendered.contains("--issue-key") && rendered.contains("project init"),
                "HTTP {status} must name how to re-issue a key: {rendered}"
            );
        }
    }

    /// Other 4xx keep their message clean — the key hint is auth-specific
    /// and would be noise on a 404.
    #[test]
    fn other_4xx_get_no_key_hint() {
        let rendered = ApiError::Http4xx {
            status: 404,
            code: None,
            message: "corpus not found: abc".to_string(),
            body: serde_json::json!({}),
        }
        .to_string();
        assert_eq!(rendered, "HTTP 404: corpus not found: abc");
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The human-readable sentence out of a server error body.
///
/// Servers put it under `message` or `error`. When neither is present the
/// compact JSON is better than nothing — but it is the fallback, not the
/// default, so a normal error reads as prose rather than a serialized
/// object.
fn body_message(body: &Value) -> String {
    body.get("message")
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(body).unwrap_or_default())
}

/// Interpret any `/v1` response — success or failure — into a JSON value or
/// an `ApiError`.
///
/// Every JSON-returning client method funnels through here so the pre-launch
/// check cannot be bypassed by one call site. That check comes FIRST, before
/// the status split, because a refusal is not required to arrive with a
/// refusal's status code: the managed edge (`api.enscrive.io`) answers every
/// path — `/v1/*` included — with an ALB fixed response carrying
/// `{"error":"not_yet_available","phase":"pre-launch"}` and **HTTP 200**.
/// Keying on 503 meant that body parsed as a successful result and the CLI
/// reported `ok:true` for what is actually a refusal to serve.
fn interpret_response(status: u16, body_text: &str) -> Result<Value, ApiError> {
    if let Ok(body) = serde_json::from_str::<Value>(body_text)
        && is_pre_launch(&body)
    {
        return Err(ApiError::NotYetAvailable { status });
    }

    if !(200..300).contains(&(status as u32)) {
        return Err(classify_error_response(status, body_text));
    }

    if body_text.trim().is_empty() {
        return Ok(Value::Null);
    }

    serde_json::from_str(body_text).map_err(|_| ApiError::InvalidResponse {
        status,
        body: body_text.to_string(),
    })
}

/// Classify an HTTP error response into the appropriate `ApiError` variant.
/// Called with a non-success status + the raw body text from any client method.
fn classify_error_response(status: u16, body_text: &str) -> ApiError {
    // Try to parse the body as JSON.
    let parsed: Option<Value> = serde_json::from_str(body_text).ok();

    match parsed {
        Some(body) => {
            // Check for a server-supplied failure_class.
            if let Some(class) = body.get("failure_class").and_then(Value::as_str) {
                // Special-case: not_yet_available always wins regardless of the
                // failure_class spelling the server uses.
                if class == "not_yet_available"
                    || class == "FAIL_UNSUPPORTED"
                    || is_pre_launch(&body)
                {
                    return ApiError::NotYetAvailable { status };
                }
                return ApiError::ServerClassified {
                    class: class.to_string(),
                    status,
                    body,
                };
            }

            // No failure_class field. Check for pre-launch markers on 503s.
            if status == 503 && is_pre_launch(&body) {
                return ApiError::NotYetAvailable { status };
            }

            // Split on 4xx vs 5xx.
            if (400..500).contains(&(status as u32)) {
                let code = body
                    .get("code")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let message = body
                    .get("message")
                    .or_else(|| body.get("error"))
                    .and_then(Value::as_str)
                    .unwrap_or("client error")
                    .to_string();
                ApiError::Http4xx {
                    status,
                    code,
                    message,
                    body,
                }
            } else {
                ApiError::Http5xx { status, body }
            }
        }
        None => {
            // Body is not JSON — cannot classify further.
            ApiError::InvalidResponse {
                status,
                body: body_text.to_string(),
            }
        }
    }
}

/// Return true when the JSON body carries a pre-launch refusal marker.
///
/// Status-independent by design: the marker is the evidence, not the code it
/// arrives with. Checked across every field the edge and the service are
/// known to put it in, so a body is recognized whether it says
/// `{"phase":"pre-launch"}`, `{"error":"not_yet_available"}`, or carries the
/// class in `failure_class`.
fn is_pre_launch(body: &Value) -> bool {
    if body
        .get("phase")
        .and_then(Value::as_str)
        .map(|p| p == "pre-launch")
        .unwrap_or(false)
    {
        return true;
    }
    for field in ["error", "message", "failure_class"] {
        if body
            .get(field)
            .and_then(Value::as_str)
            .map(|v| v.contains("not_yet_available"))
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct BinaryResponse {
    pub content: Vec<u8>,
    pub content_type: Option<String>,
    pub content_disposition: Option<String>,
}

pub struct EnscriveClient {
    http: Client,
    base_url: String,
    api_key: String,
    embedding_provider_key: Option<String>,
}

impl EnscriveClient {
    pub fn new(base_url: String, api_key: String, embedding_provider_key: Option<String>) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("build http client");
        Self {
            http,
            base_url,
            api_key,
            embedding_provider_key: embedding_provider_key
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        }
    }

    fn with_auth_headers(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let request = request.header("X-API-Key", &self.api_key);
        if let Some(provider_key) = &self.embedding_provider_key {
            return request.header("X-Embedding-Provider-Key", provider_key);
        }
        request
    }

    pub async fn get_json(&self, path: &str) -> Result<Value, ApiError> {
        self.send_json(Method::GET, path, None).await
    }

    pub async fn get_json_with_query(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Value, ApiError> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );

        let response = self
            .with_auth_headers(self.http.request(Method::GET, &url))
            .query(query)
            .send()
            .await
            .map_err(map_reqwest_err)?;

        let status = response.status().as_u16();
        let body_text = response
            .text()
            .await
            .map_err(map_reqwest_err)?;

        interpret_response(status, &body_text)
    }

    pub async fn get_bytes_with_query(
        &self,
        path: &str,
        query: &[(&str, String)],
        accept: &str,
    ) -> Result<BinaryResponse, String> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );

        let response = self
            .with_auth_headers(self.http.request(Method::GET, &url))
            .header("Accept", accept)
            .query(query)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let content_disposition = response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        if !status.is_success() {
            let body_text = response
                .text()
                .await
                .map_err(|e| format!("read body: {e}"))?;
            return Err(format!("HTTP {status}: {body_text}"));
        }

        let content = response
            .bytes()
            .await
            .map_err(|e| format!("read body: {e}"))?
            .to_vec();

        Ok(BinaryResponse {
            content,
            content_type,
            content_disposition,
        })
    }

    pub async fn get_text_with_query(
        &self,
        path: &str,
        query: &[(&str, String)],
        accept: &str,
        timeout_secs: Option<u64>,
    ) -> Result<String, String> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );

        let response = self
            .with_auth_headers(self.http.request(Method::GET, &url))
            .header("Accept", accept)
            .query(query)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        let status = response.status();
        if let Some(timeout_secs) = timeout_secs {
            if !status.is_success() {
                let body_text = response
                    .text()
                    .await
                    .map_err(|e| format!("read body: {e}"))?;
                return Err(format!("HTTP {status}: {body_text}"));
            }

            let mut body_text = String::new();
            let mut stream = response.bytes_stream();
            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

            loop {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    break;
                }

                let remaining = deadline.saturating_duration_since(now);
                match tokio::time::timeout(remaining, stream.next()).await {
                    Ok(Some(Ok(chunk))) => body_text.push_str(&String::from_utf8_lossy(&chunk)),
                    Ok(Some(Err(e))) => return Err(format!("read stream: {e}")),
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            if body_text.trim().is_empty() {
                return Err(format!(
                    "stream timed out after {}s without receiving any data",
                    timeout_secs
                ));
            }

            return Ok(body_text);
        }

        let body_text = response
            .text()
            .await
            .map_err(|e| format!("read body: {e}"))?;

        if !status.is_success() {
            return Err(format!("HTTP {status}: {body_text}"));
        }

        Ok(body_text)
    }

    pub async fn post_json(&self, path: &str, body: Value) -> Result<Value, ApiError> {
        self.send_json(Method::POST, path, Some(body)).await
    }

    /// POST with URL query parameters (not a JSON body). Used by endpoints
    /// whose handler extracts `Query<..>` on a POST route (e.g.
    /// `POST /v1/admin/metering/backfill`) rather than `Json<..>`.
    pub async fn post_json_with_query(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Value, ApiError> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );

        let response = self
            .with_auth_headers(self.http.request(Method::POST, &url))
            .query(query)
            .send()
            .await
            .map_err(map_reqwest_err)?;

        let status = response.status().as_u16();
        let body_text = response.text().await.map_err(map_reqwest_err)?;

        interpret_response(status, &body_text)
    }

    pub async fn post_text(&self, path: &str, body: Value, accept: &str) -> Result<String, String> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );

        let response = self
            .with_auth_headers(self.http.request(Method::POST, &url))
            .header("Accept", accept)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        let status = response.status();
        let body_text = response
            .text()
            .await
            .map_err(|e| format!("read body: {e}"))?;

        if !status.is_success() {
            return Err(format!("HTTP {status}: {body_text}"));
        }

        Ok(body_text)
    }

    pub async fn patch_json(&self, path: &str, body: Value) -> Result<Value, ApiError> {
        self.send_json(Method::PATCH, path, Some(body)).await
    }

    pub async fn put_json(&self, path: &str, body: Value) -> Result<Value, ApiError> {
        self.send_json(Method::PUT, path, Some(body)).await
    }

    pub async fn delete_json(&self, path: &str) -> Result<Value, ApiError> {
        self.send_json(Method::DELETE, path, None).await
    }

    /// Post a multipart/form-data body with a JSON `metadata` part plus three
    /// file parts (corpus, queries, qrels). Used by
    /// `POST /v1/datasets/upload` (EV-006) — the only multipart endpoint today.
    pub async fn post_dataset_upload(
        &self,
        path: &str,
        metadata: Value,
        corpus_bytes: Vec<u8>,
        queries_bytes: Vec<u8>,
        qrels_bytes: Vec<u8>,
    ) -> Result<Value, ApiError> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        let form = reqwest::multipart::Form::new()
            .text(
                "metadata",
                serde_json::to_string(&metadata)
                    .map_err(|e| ApiError::InvalidResponse {
                        status: 0,
                        body: format!("serialize metadata: {e}"),
                    })?,
            )
            .part(
                "corpus",
                reqwest::multipart::Part::bytes(corpus_bytes)
                    .file_name("corpus.jsonl")
                    .mime_str("application/x-ndjson")
                    .map_err(|e| ApiError::InvalidResponse {
                        status: 0,
                        body: format!("corpus mime: {e}"),
                    })?,
            )
            .part(
                "queries",
                reqwest::multipart::Part::bytes(queries_bytes)
                    .file_name("queries.jsonl")
                    .mime_str("application/x-ndjson")
                    .map_err(|e| ApiError::InvalidResponse {
                        status: 0,
                        body: format!("queries mime: {e}"),
                    })?,
            )
            .part(
                "qrels",
                reqwest::multipart::Part::bytes(qrels_bytes)
                    .file_name("qrels.tsv")
                    .mime_str("text/tab-separated-values")
                    .map_err(|e| ApiError::InvalidResponse {
                        status: 0,
                        body: format!("qrels mime: {e}"),
                    })?,
            );

        let response = self
            .with_auth_headers(self.http.request(Method::POST, &url))
            .multipart(form)
            .send()
            .await
            .map_err(map_reqwest_err)?;

        let status = response.status().as_u16();
        let body_text = response.text().await.map_err(map_reqwest_err)?;

        interpret_response(status, &body_text)
    }

    async fn send_json(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, ApiError> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );

        let mut request = self.with_auth_headers(self.http.request(method, &url));

        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().await.map_err(map_reqwest_err)?;

        let status = response.status().as_u16();
        let body_text = response.text().await.map_err(map_reqwest_err)?;

        interpret_response(status, &body_text)
    }
}

/// Map a reqwest error to `ApiError::Timeout` or `ApiError::Network`.
fn map_reqwest_err(e: reqwest::Error) -> ApiError {
    if e.is_timeout() {
        ApiError::Timeout
    } else {
        ApiError::Network(e)
    }
}

// ---------------------------------------------------------------------------
// Unit tests — ENS-84 acceptance criteria
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::FailureClass;

    /// Map an `ApiError` to a `FailureClass` the same way `request_failure` does
    /// in main.rs. Duplicated here so these unit tests are self-contained.
    fn api_error_to_class(err: &ApiError) -> FailureClass {
        match err {
            ApiError::NotYetAvailable { .. } => FailureClass::Unsupported,
            ApiError::Timeout => FailureClass::Bug,
            ApiError::Network(_) => FailureClass::Bug,
            ApiError::InvalidResponse { .. } => FailureClass::Bug,
            ApiError::ServerClassified { class, .. } => map_class_str(class),
            ApiError::Http4xx { .. } => FailureClass::Bug,
            ApiError::Http5xx { .. } => FailureClass::Bug,
        }
    }

    fn map_class_str(raw: &str) -> FailureClass {
        match raw {
            "FAIL_BUG" => FailureClass::Bug,
            "FAIL_UNSUPPORTED" => FailureClass::Unsupported,
            "FAIL_UNSUPPORTED_IN_LOCAL_MODE" => FailureClass::UnsupportedInLocalMode,
            "FAIL_PLAN_REQUIRED" => FailureClass::PlanRequired,
            "FAIL_CONFIRMATION_REQUIRED" => FailureClass::ConfirmationRequired,
            "FAIL_QUOTA_EXCEEDED" => FailureClass::QuotaExceeded,
            "FAIL_LICENSE_INVALID" => FailureClass::LicenseInvalid,
            "FAIL_UNIMPLEMENTED" => FailureClass::Unimplemented,
            "FAIL_FALSE_CLAIM" => FailureClass::FalseClaim,
            _ => FailureClass::Bug,
        }
    }

    // --- ENS-84 acceptance tests ---

    /// 503 with `failure_class: not_yet_available` → NotYetAvailable → Unsupported
    #[test]
    fn a503_not_yet_available_maps_to_unsupported() {
        let body = r#"{"failure_class":"not_yet_available","region":"us","phase":"pre-launch","retry_after":null}"#;
        let err = classify_error_response(503, body);
        assert!(
            matches!(err, ApiError::NotYetAvailable { status: 503 }),
            "expected NotYetAvailable, got {err:?}"
        );
        assert_eq!(api_error_to_class(&err), FailureClass::Unsupported);
    }

    /// 503 with pre-launch markers but no failure_class → NotYetAvailable → Unsupported
    #[test]
    fn a503_pre_launch_phase_no_class_maps_to_unsupported() {
        let body = r#"{"error":"not_yet_available","region":"us","phase":"pre-launch"}"#;
        let err = classify_error_response(503, body);
        assert!(
            matches!(err, ApiError::NotYetAvailable { .. }),
            "expected NotYetAvailable, got {err:?}"
        );
        assert_eq!(api_error_to_class(&err), FailureClass::Unsupported);
    }

    // --- ENS-3243: pre-launch detection must not be status-code-brittle ---

    /// The regression this closes. `api.enscrive.io` fronts `/v1/*` with an
    /// ALB fixed response that carries the pre-launch refusal under **HTTP
    /// 200** — verified live: every path, including `/v1/corpora`, answers
    /// `200 {"error":"not_yet_available","region":"us","phase":"pre-launch",
    /// "retry_after":null}`. The old code only looked for the marker on
    /// 503s, so this body parsed as a successful result and the CLI printed
    /// `ok:true` for a refusal to serve.
    #[test]
    fn pre_launch_marker_on_200_is_a_failure_not_a_success() {
        let body = r#"{"error":"not_yet_available","region":"us","phase":"pre-launch","retry_after":null}"#;
        let result = interpret_response(200, body);

        let err = result.expect_err("a pre-launch refusal must never be Ok");
        assert!(
            matches!(err, ApiError::NotYetAvailable { status: 200 }),
            "expected NotYetAvailable, got {err:?}"
        );
        assert_eq!(api_error_to_class(&err), FailureClass::Unsupported);

        // The message must leave a first-contact user somewhere to go.
        let rendered = err.to_string();
        assert!(
            rendered.contains("api.enscrive.io") && rendered.contains("pre-launch"),
            "must name the situation: {rendered}"
        );
        assert!(
            rendered.contains("self-managed"),
            "must name the path that works: {rendered}"
        );
    }

    /// The marker is the evidence, not the status code it rides on.
    #[test]
    fn pre_launch_marker_is_recognized_on_any_status() {
        for status in [200, 201, 403, 500, 502, 503] {
            let body = r#"{"error":"not_yet_available","phase":"pre-launch"}"#;
            let err = interpret_response(status, body)
                .expect_err("pre-launch must fail on status {status}");
            assert!(
                matches!(err, ApiError::NotYetAvailable { .. }),
                "status {status} must classify as NotYetAvailable, got {err:?}"
            );
        }
    }

    /// End-to-end through the real `EnscriveClient` against a mock that
    /// answers exactly like the live managed edge: HTTP 200 carrying the
    /// pre-launch body. Asserts on what a caller receives, not on the
    /// classifier in isolation — the unit tests above can't catch a call
    /// site that bypasses `interpret_response`.
    #[tokio::test]
    async fn client_treats_200_pre_launch_edge_response_as_a_failure() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        const EDGE_BODY: &str =
            r#"{"error":"not_yet_available","region":"us","phase":"pre-launch","retry_after":null}"#;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock edge");
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            for stream in listener.incoming().take(1) {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    EDGE_BODY.len(),
                    EDGE_BODY
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let client = EnscriveClient::new(
            format!("http://127.0.0.1:{port}"),
            "test-key".to_string(),
            None,
        );
        let result = client.get_json("/v1/corpora").await;

        let err = result.expect_err(
            "a 200 carrying the pre-launch marker must not be reported as success",
        );
        assert!(
            matches!(err, ApiError::NotYetAvailable { status: 200 }),
            "expected NotYetAvailable, got {err:?}"
        );
    }

    /// Guard the other direction: an ordinary 200 must still succeed. A
    /// marker check that swallowed healthy responses would be worse than
    /// the bug it replaced.
    #[test]
    fn ordinary_200_still_succeeds() {
        let body = r#"{"corpora":[],"total":0}"#;
        let value = interpret_response(200, body).expect("a normal 200 must stay Ok");
        assert_eq!(value["total"], 0);

        // Including bodies that merely mention a phase that is not pre-launch.
        let live = r#"{"phase":"live","results":[]}"#;
        assert!(interpret_response(200, live).is_ok(), "phase:live must pass");
    }

    /// 403 with `failure_class: plan_required` → ServerClassified → PlanRequired
    #[test]
    fn a403_plan_required_maps_to_server_classified() {
        let body = r#"{"error":"plan_required","failure_class":"FAIL_PLAN_REQUIRED","required_plan":"professional"}"#;
        let err = classify_error_response(403, body);
        match &err {
            ApiError::ServerClassified { class, status, .. } => {
                assert_eq!(class, "FAIL_PLAN_REQUIRED");
                assert_eq!(*status, 403);
            }
            _ => panic!("expected ServerClassified, got {err:?}"),
        }
        assert_eq!(api_error_to_class(&err), FailureClass::PlanRequired);
    }

    /// 500 with JSON body (no failure_class) → Http5xx → Bug
    #[test]
    fn a500_json_no_class_maps_to_http5xx_bug() {
        let body = r#"{"error":"database connection lost"}"#;
        let err = classify_error_response(500, body);
        assert!(
            matches!(err, ApiError::Http5xx { status: 500, .. }),
            "expected Http5xx, got {err:?}"
        );
        assert_eq!(api_error_to_class(&err), FailureClass::Bug);
    }

    /// Timeout → ApiError::Timeout → Bug
    #[test]
    fn timeout_variant_maps_to_bug() {
        let err = ApiError::Timeout;
        assert_eq!(api_error_to_class(&err), FailureClass::Bug);
        // Ensure Display works without panic
        let s = err.to_string();
        assert!(s.contains("timed out"), "unexpected display: {s}");
    }

    /// Plain 404 with non-JSON body → InvalidResponse → Bug
    #[test]
    fn a404_non_json_maps_to_invalid_response() {
        let err = classify_error_response(404, "Not Found");
        assert!(
            matches!(err, ApiError::InvalidResponse { status: 404, .. }),
            "expected InvalidResponse, got {err:?}"
        );
        assert_eq!(api_error_to_class(&err), FailureClass::Bug);
    }

    /// 403 plain JSON without failure_class → Http4xx → Bug
    #[test]
    fn a403_no_failure_class_maps_to_http4xx() {
        let body = r#"{"error":"forbidden","message":"access denied"}"#;
        let err = classify_error_response(403, body);
        assert!(
            matches!(err, ApiError::Http4xx { status: 403, .. }),
            "expected Http4xx, got {err:?}"
        );
        assert_eq!(api_error_to_class(&err), FailureClass::Bug);
    }

    /// ServerClassified with unknown class string → Bug (map_class_str fallback)
    #[test]
    fn unknown_server_class_maps_to_bug() {
        let body = r#"{"failure_class":"FAIL_BOGUS","message":"something"}"#;
        let err = classify_error_response(500, body);
        match &err {
            ApiError::ServerClassified { class, .. } => {
                assert_eq!(class, "FAIL_BOGUS");
            }
            _ => panic!("expected ServerClassified, got {err:?}"),
        }
        assert_eq!(api_error_to_class(&err), FailureClass::Bug);
    }

    /// Display impls don't panic for every variant
    #[test]
    fn display_impls_are_non_empty() {
        let cases: Vec<ApiError> = vec![
            ApiError::InvalidResponse {
                status: 404,
                body: "not found".into(),
            },
            ApiError::ServerClassified {
                class: "FAIL_BUG".into(),
                status: 500,
                body: serde_json::json!({}),
            },
            ApiError::Http4xx {
                status: 400,
                code: None,
                message: "bad request".into(),
                body: serde_json::json!({}),
            },
            ApiError::Http5xx {
                status: 502,
                body: serde_json::json!({}),
            },
            ApiError::NotYetAvailable { status: 503 },
            ApiError::Timeout,
        ];
        for e in &cases {
            assert!(!e.to_string().is_empty(), "empty Display for {e:?}");
        }
    }
}
