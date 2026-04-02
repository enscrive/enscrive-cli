use futures_util::StreamExt;
use reqwest::{Client, Method};
use serde_json::Value;

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

    pub async fn get_json(&self, path: &str) -> Result<Value, String> {
        self.send_json(Method::GET, path, None).await
    }

    pub async fn get_json_with_query(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Value, String> {
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
            .map_err(|e| format!("request failed: {e}"))?;

        let status = response.status();
        let body_text = response
            .text()
            .await
            .map_err(|e| format!("read body: {e}"))?;

        if !status.is_success() {
            return Err(format!("HTTP {status}: {body_text}"));
        }

        if body_text.trim().is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&body_text).map_err(|e| format!("parse response: {e}"))
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

    pub async fn post_json(&self, path: &str, body: Value) -> Result<Value, String> {
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

    pub async fn patch_json(&self, path: &str, body: Value) -> Result<Value, String> {
        self.send_json(Method::PATCH, path, Some(body)).await
    }

    pub async fn put_json(&self, path: &str, body: Value) -> Result<Value, String> {
        self.send_json(Method::PUT, path, Some(body)).await
    }

    pub async fn delete_json(&self, path: &str) -> Result<Value, String> {
        self.send_json(Method::DELETE, path, None).await
    }

    async fn send_json(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );

        let mut request = self.with_auth_headers(self.http.request(method, &url));

        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request
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

        if body_text.trim().is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&body_text).map_err(|e| format!("parse response: {e}"))
    }
}
