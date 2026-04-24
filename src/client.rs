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
        /// Convenience extraction of a `code` field if present.
        code: Option<String>,
        /// Convenience extraction of a `message` or `error` field.
        message: String,
        body: Value,
    },

    /// 5xx response whose JSON body did NOT carry a `failure_class`.
    Http5xx { status: u16, body: Value },

    /// 503 with `failure_class` == `"not_yet_available"` **or** without a
    /// `failure_class` but with pre-launch markers in the body.
    /// Maps directly to `FailureClass::Unsupported`.
    NotYetAvailable { status: u16 },

    /// Request timed out (reqwest timeout fires before a response arrives).
    Timeout,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Network(e) => write!(f, "request failed: {e}"),
            ApiError::InvalidResponse { status, body } => {
                write!(f, "HTTP {status}: {body}")
            }
            ApiError::ServerClassified {
                class,
                status,
                body,
            } => {
                let body_str = serde_json::to_string(body).unwrap_or_default();
                write!(f, "HTTP {status} [{class}]: {body_str}")
            }
            ApiError::Http4xx {
                status, message, ..
            } => write!(f, "HTTP {status}: {message}"),
            ApiError::Http5xx { status, body } => {
                let body_str = serde_json::to_string(body).unwrap_or_default();
                write!(f, "HTTP {status}: {body_str}")
            }
            ApiError::NotYetAvailable { status } => {
                write!(f, "HTTP {status}: not yet available on public /v1")
            }
            ApiError::Timeout => write!(f, "request timed out"),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

/// Return true when the JSON body contains pre-launch markers that the
/// original string heuristics keyed on.
fn is_pre_launch(body: &Value) -> bool {
    // `"phase": "pre-launch"` or error/message containing "not_yet_available"
    if body
        .get("phase")
        .and_then(Value::as_str)
        .map(|p| p == "pre-launch")
        .unwrap_or(false)
    {
        return true;
    }
    // error field containing the literal string
    if body
        .get("error")
        .and_then(Value::as_str)
        .map(|e| e.contains("not_yet_available"))
        .unwrap_or(false)
    {
        return true;
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

        if !(200..300).contains(&(status as u32)) {
            return Err(classify_error_response(status, &body_text));
        }

        if body_text.trim().is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&body_text).map_err(|_| ApiError::InvalidResponse {
            status,
            body: body_text,
        })
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

        if !(200..300).contains(&(status as u32)) {
            return Err(classify_error_response(status, &body_text));
        }
        if body_text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&body_text).map_err(|_| ApiError::InvalidResponse {
            status,
            body: body_text,
        })
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

        if !(200..300).contains(&(status as u32)) {
            return Err(classify_error_response(status, &body_text));
        }

        if body_text.trim().is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&body_text).map_err(|_| ApiError::InvalidResponse {
            status,
            body: body_text,
        })
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
