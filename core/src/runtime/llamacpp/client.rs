//! Loopback HTTP client for a running `llama-server` instance.
//!
//! The endpoints the adapter needs: `GET /health`, `GET /props`, non-streaming
//! `POST /completion`, and streaming `POST /v1/chat/completions` (SSE — the
//! server applies the model's chat template). All requests go to `127.0.0.1`
//! (ADR-008); the client carries no TLS for these.

use std::net::Ipv4Addr;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::runtime::Health;
use crate::{CoreError, Result};

/// One item from a streaming chat completion: a chunk of text, then a final
/// [`Done`](GenerationEvent::Done) carrying the server's own token stats.
#[derive(Debug, Clone, PartialEq)]
pub enum GenerationEvent {
    Token(String),
    Done { tokens: u32, tokens_per_second: f64 },
}

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

    /// Streaming chat completion via `POST /v1/chat/completions` (SSE) — llama-server
    /// applies the model's own chat template. Sends a [`GenerationEvent::Token`]
    /// per chunk and a final [`GenerationEvent::Done`] to `tx`. Stops early (and
    /// returns `Ok`) if the receiver is dropped — that is how a cancel unwinds.
    pub(super) async fn complete_stream(
        &self,
        port: u16,
        prompt: &str,
        max_tokens: i32,
        tx: mpsc::Sender<GenerationEvent>,
    ) -> Result<()> {
        let mut resp = self
            .http
            .post(format!("{}/v1/chat/completions", self.base(port)))
            .json(&serde_json::json!({
                "messages": [{ "role": "user", "content": prompt }],
                "max_tokens": max_tokens,
                "stream": true,
            }))
            .send()
            .await
            .map_err(|e| llama_err(format!("chat request failed: {e}")))?
            .error_for_status()
            .map_err(|e| llama_err(format!("llama-server rejected the chat request: {e}")))?;

        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| llama_err(format!("completion stream broke: {e}")))?
        {
            buf.extend_from_slice(&chunk);
            while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=nl).collect();
                match parse_sse_event(&line) {
                    Some(ev) => {
                        let done = matches!(ev, GenerationEvent::Done { .. });
                        if tx.send(ev).await.is_err() {
                            return Ok(()); // receiver gone → treat as cancelled
                        }
                        if done {
                            return Ok(());
                        }
                    }
                    None => continue,
                }
            }
        }
        Ok(())
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

/// Parse one `data: {json}` SSE line from `/v1/chat/completions` streaming.
/// Returns `None` for blank lines, keep-alive comments, the `[DONE]` sentinel,
/// unparseable payloads, and empty deltas.
fn parse_sse_event(line: &[u8]) -> Option<GenerationEvent> {
    let text = std::str::from_utf8(line).ok()?.trim();
    let data = text.strip_prefix("data:")?.trim();
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    let v: Value = serde_json::from_str(data).ok()?;
    let choice = v.pointer("/choices/0")?;

    if let Some(c) = choice.pointer("/delta/content").and_then(Value::as_str) {
        if !c.is_empty() {
            return Some(GenerationEvent::Token(c.to_string()));
        }
    }
    if choice.get("finish_reason").is_some_and(|r| !r.is_null()) {
        // llama.cpp always fills `timings`; `usage` only with `include_usage`.
        let tokens = v
            .pointer("/timings/predicted_n")
            .or_else(|| v.pointer("/usage/completion_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        return Some(GenerationEvent::Done {
            tokens: tokens.try_into().unwrap_or(u32::MAX),
            tokens_per_second: v
                .pointer("/timings/predicted_per_second")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
        });
    }
    None
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

    /// One SSE line (`data: {…}`) for a content chunk.
    fn delta_line(c: &str) -> String {
        format!(
            "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{c}\"}},\"finish_reason\":null}}]}}\n"
        )
    }
    /// The final content-less chunk carrying `finish_reason` + stats, as one line.
    const FINISH_LINE: &str =
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\
        \"timings\":{\"predicted_n\":2,\"predicted_per_second\":50.0}}\n";
    /// A full response body: two tokens, the finish chunk, the `[DONE]` sentinel.
    fn sse_body(chunks: &[&str]) -> String {
        let mut body = String::new();
        for c in chunks {
            body.push_str(&delta_line(c));
            body.push('\n');
        }
        body.push_str(FINISH_LINE);
        body.push_str("\ndata: [DONE]\n\n");
        body
    }

    #[test]
    fn parse_sse_event_cases() {
        assert_eq!(
            parse_sse_event(delta_line("Hi").as_bytes()),
            Some(GenerationEvent::Token("Hi".into()))
        );
        assert_eq!(
            parse_sse_event(FINISH_LINE.as_bytes()),
            Some(GenerationEvent::Done {
                tokens: 2,
                tokens_per_second: 50.0
            })
        );
        // `usage.completion_tokens` is the fallback when `timings.predicted_n` is absent.
        assert_eq!(
            parse_sse_event(
                b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"completion_tokens\":9}}"
            ),
            Some(GenerationEvent::Done {
                tokens: 9,
                tokens_per_second: 0.0
            })
        );
        assert_eq!(parse_sse_event(b"\n"), None);
        assert_eq!(parse_sse_event(b": keep-alive comment\n"), None);
        assert_eq!(parse_sse_event(b"data: [DONE]\n"), None);
        assert_eq!(parse_sse_event(b"data: not-json"), None);
        assert_eq!(
            parse_sse_event(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"\"},\"finish_reason\":null}]}"
            ),
            None
        );
    }

    #[tokio::test]
    async fn complete_stream_emits_tokens_then_done() {
        let sse = sse_body(&["Hello", " world"]);
        let port = serve(Router::new().route(
            "/v1/chat/completions",
            post(move || async move {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    sse,
                )
            }),
        ))
        .await;

        let (tx, mut rx) = mpsc::channel(16);
        LlamaClient::new()
            .complete_stream(port, "hi", 8, tx)
            .await
            .unwrap();

        let mut got = Vec::new();
        while let Some(ev) = rx.recv().await {
            got.push(ev);
        }
        assert_eq!(
            got,
            [
                GenerationEvent::Token("Hello".into()),
                GenerationEvent::Token(" world".into()),
                GenerationEvent::Done {
                    tokens: 2,
                    tokens_per_second: 50.0
                },
            ]
        );
    }

    #[tokio::test]
    async fn complete_stream_stops_when_receiver_is_dropped() {
        let sse = format!("{}\n{}\n", delta_line("a"), delta_line("b"));
        let port = serve(Router::new().route(
            "/v1/chat/completions",
            post(move || async move {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    sse,
                )
            }),
        ))
        .await;

        let (tx, rx) = mpsc::channel(1);
        drop(rx); // no consumer
        LlamaClient::new()
            .complete_stream(port, "hi", 8, tx)
            .await
            .unwrap();
    }
}
