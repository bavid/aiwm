//! Loopback HTTP client for a running `llama-server` instance.
//!
//! Only the handful of endpoints the adapter needs: `GET /health`,
//! `POST /completion` (non-streaming), `GET /props`. All requests go to
//! `127.0.0.1` (ADR-008); the client carries no TLS.

use std::net::Ipv4Addr;
use std::time::Duration;

use serde::Deserialize;

use crate::runtime::Health;
use crate::{CoreError, Result};

/// A `/health` (or `/props`) probe must return within this or the server counts
/// as down. `/completion` is deliberately left unbounded — generation is slow.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

fn llama_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: super::RUNTIME_ID.into(),
        message: msg.to_string(),
    }
}

#[derive(Debug, Clone)]
pub(super) struct LlamaClient {
    http: reqwest::Client,
    host: Ipv4Addr,
}

#[derive(Deserialize)]
struct CompletionResponse {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct PropsResponse {
    #[serde(default)]
    model_path: Option<String>,
}

impl LlamaClient {
    pub(super) fn new() -> Self {
        // With no TLS features enabled the builder cannot fail; fall back to the
        // default client rather than panic if that ever changes.
        let http = reqwest::Client::builder()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            host: Ipv4Addr::LOCALHOST,
        }
    }

    fn base(&self, port: u16) -> String {
        format!("http://{}:{port}", self.host)
    }

    /// `200` → [`Health::Healthy`], `503` (model still loading) →
    /// [`Health::Starting`], anything else or a transport error →
    /// [`Health::Unhealthy`].
    pub(super) async fn health(&self, port: u16) -> Health {
        match self
            .http
            .get(format!("{}/health", self.base(port)))
            .timeout(PROBE_TIMEOUT)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => Health::Healthy,
            Ok(resp) if resp.status().as_u16() == 503 => Health::Starting,
            Ok(_) | Err(_) => Health::Unhealthy,
        }
    }

    /// Non-streaming `POST /completion`; returns the generated text.
    pub(super) async fn complete(&self, port: u16, prompt: &str, n_predict: i32) -> Result<String> {
        let resp = self
            .http
            .post(format!("{}/completion", self.base(port)))
            .json(&serde_json::json!({
                "prompt": prompt,
                "n_predict": n_predict,
                "stream": false,
            }))
            .send()
            .await
            .map_err(|e| llama_err(format!("/completion request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(llama_err(format!(
                "llama-server returned {} for /completion",
                resp.status()
            )));
        }
        let body: CompletionResponse = resp
            .json()
            .await
            .map_err(|e| llama_err(format!("bad /completion response: {e}")))?;
        Ok(body.content)
    }

    /// `GET /props` → the resident model's file path, when the server reports it.
    /// Best-effort: any failure yields `None`.
    pub(super) async fn model_path(&self, port: u16) -> Option<String> {
        let resp = self
            .http
            .get(format!("{}/props", self.base(port)))
            .timeout(PROBE_TIMEOUT)
            .send()
            .await
            .ok()?;
        let props: PropsResponse = resp.json().await.ok()?;
        props.model_path.filter(|p| !p.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use axum::routing::{get, post};
    use axum::{Json, Router};

    use super::*;

    /// Serve `router` on a fresh loopback port and return that port.
    async fn serve(router: Router) -> u16 {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        port
    }

    #[tokio::test]
    async fn health_ok_maps_to_healthy() {
        let port = serve(Router::new().route(
            "/health",
            get(|| async { Json(serde_json::json!({ "status": "ok" })) }),
        ))
        .await;
        assert_eq!(LlamaClient::new().health(port).await, Health::Healthy);
    }

    #[tokio::test]
    async fn health_503_maps_to_starting() {
        let port = serve(Router::new().route(
            "/health",
            get(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({ "error": { "message": "Loading model" } })),
                )
            }),
        ))
        .await;
        assert_eq!(LlamaClient::new().health(port).await, Health::Starting);
    }

    #[tokio::test]
    async fn health_is_unhealthy_when_nothing_listens() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let dead = listener.local_addr().unwrap().port();
        drop(listener);
        assert_eq!(LlamaClient::new().health(dead).await, Health::Unhealthy);
    }

    #[tokio::test]
    async fn completion_returns_content() {
        let port = serve(Router::new().route(
            "/completion",
            post(|| async { Json(serde_json::json!({ "content": "hello there" })) }),
        ))
        .await;
        let got = LlamaClient::new().complete(port, "hi", 16).await.unwrap();
        assert_eq!(got, "hello there");
    }

    #[tokio::test]
    async fn completion_errors_on_server_error() {
        let port = serve(Router::new().route(
            "/completion",
            post(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
        ))
        .await;
        assert!(LlamaClient::new().complete(port, "hi", 16).await.is_err());
    }

    #[tokio::test]
    async fn model_path_reads_props() {
        let port = serve(Router::new().route(
            "/props",
            get(|| async { Json(serde_json::json!({ "model_path": "E:\\m\\x.gguf" })) }),
        ))
        .await;
        assert_eq!(
            LlamaClient::new().model_path(port).await.as_deref(),
            Some("E:\\m\\x.gguf")
        );
    }
}
