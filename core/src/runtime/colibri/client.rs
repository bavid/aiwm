//! Loopback HTTP client for a running Colibri `coli serve` instance.
//!
//! The endpoints the adapter needs: `GET /health` and streaming
//! `POST /v1/chat/completions` (SSE, OpenAI-compatible — same shape llama.cpp
//! speaks, plus a bearer token since Colibri gates itself with `COLI_API_KEY`).
//! All requests go to `127.0.0.1` (ADR-008).
//!
//! Deliberately no client-level timeout: a request-level `reqwest` timeout
//! bounds the whole request including reading a streamed body, not just
//! connecting, and Colibri's own docs are explicit that decode speed can be a
//! fraction of a token per second on a slow drive — a blanket timeout here
//! would kill a legitimate long-running generation exactly like the bug fixed
//! in the OpenCode/Hermes adapters. Only the short health probe gets one.

use std::net::Ipv4Addr;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;

use crate::runtime::Health;
use crate::{CoreError, Result};

/// One item from a streaming chat completion: a chunk of text, then a final
/// [`Done`](GenerationEvent::Done) once the response's `finish_reason` lands.
#[derive(Debug, Clone, PartialEq)]
pub enum GenerationEvent {
    Token(String),
    Done { tokens: u32 },
}

/// A `/health` probe must return within this or the server counts as down.
/// The chat stream is deliberately left unbounded.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

fn colibri_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: super::RUNTIME_ID.into(),
        message: msg.to_string(),
    }
}

#[derive(Debug, Clone)]
pub(super) struct ColibriClient {
    http: reqwest::Client,
    host: Ipv4Addr,
}

impl ColibriClient {
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

    /// `200` → [`Health::Healthy`]; anything else or a transport error →
    /// [`Health::Unhealthy`]. Colibri's own docs don't describe a distinct
    /// "still loading" status the way llama.cpp's 503 does, so this is
    /// deliberately two-state until real-hardware use proves otherwise.
    pub(super) async fn health(&self, port: u16) -> Health {
        match self
            .http
            .get(format!("{}/health", self.base(port)))
            .timeout(PROBE_TIMEOUT)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => Health::Healthy,
            Ok(_) | Err(_) => Health::Unhealthy,
        }
    }

    /// Streaming chat completion via `POST /v1/chat/completions` (SSE).
    /// Sends a [`GenerationEvent::Token`] per chunk and a final
    /// [`GenerationEvent::Done`] to `tx`. Stops early (and returns `Ok`) if
    /// the receiver is dropped — that is how a cancel unwinds.
    pub(super) async fn complete_stream(
        &self,
        port: u16,
        api_key: &str,
        model_id: &str,
        prompt: &str,
        max_tokens: i32,
        tx: mpsc::Sender<GenerationEvent>,
    ) -> Result<()> {
        let mut resp = self
            .http
            .post(format!("{}/v1/chat/completions", self.base(port)))
            .bearer_auth(api_key)
            .json(&serde_json::json!({
                "model": model_id,
                "messages": [{ "role": "user", "content": prompt }],
                "max_tokens": max_tokens,
                "stream": true,
            }))
            .send()
            .await
            .map_err(|e| colibri_err(format!("chat request failed: {e}")))?
            .error_for_status()
            .map_err(|e| colibri_err(format!("colibri rejected the chat request: {e}")))?;

        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| colibri_err(format!("completion stream broke: {e}")))?
        {
            buf.extend_from_slice(&chunk);
            while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=nl).collect();
                match parse_sse_event(&line) {
                    Some(ev) => {
                        let done = matches!(ev, GenerationEvent::Done { .. });
                        if tx.send(ev).await.is_err() {
                            return Ok(()); // receiver gone -> treat as cancelled
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
}

/// Parse one `data: {json}` SSE line from `/v1/chat/completions` streaming.
/// Returns `None` for blank lines, keep-alive comments, the `[DONE]`
/// sentinel, unparseable payloads, and empty deltas.
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
        let tokens = v
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        return Some(GenerationEvent::Done {
            tokens: tokens.try_into().unwrap_or(u32::MAX),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use axum::routing::{get, post};
    use axum::Json;
    use axum::Router;

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
            get(|| async { Json(serde_json::json!({ "active": 0, "queued": 0 })) }),
        ))
        .await;
        assert_eq!(ColibriClient::new().health(port).await, Health::Healthy);
    }

    #[tokio::test]
    async fn health_is_unhealthy_when_nothing_listens() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let dead = listener.local_addr().unwrap().port();
        drop(listener);
        assert_eq!(ColibriClient::new().health(dead).await, Health::Unhealthy);
    }

    #[tokio::test]
    async fn health_is_unhealthy_on_a_server_error() {
        let port = serve(Router::new().route(
            "/health",
            get(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
        ))
        .await;
        assert_eq!(ColibriClient::new().health(port).await, Health::Unhealthy);
    }

    /// One SSE line (`data: {…}`) for a content chunk.
    fn delta_line(c: &str) -> String {
        format!(
            "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{c}\"}},\"finish_reason\":null}}]}}\n"
        )
    }
    const FINISH_LINE: &str =
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\
        \"usage\":{\"completion_tokens\":2}}\n";
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
            Some(GenerationEvent::Done { tokens: 2 })
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
        let seen_auth: std::sync::Arc<std::sync::Mutex<Option<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen_auth2 = seen_auth.clone();
        let port = serve(Router::new().route(
            "/v1/chat/completions",
            post(move |headers: axum::http::HeaderMap| {
                let sse = sse.clone();
                let seen_auth = seen_auth2.clone();
                async move {
                    *seen_auth.lock().unwrap() = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    (
                        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                        sse,
                    )
                }
            }),
        ))
        .await;

        let (tx, mut rx) = mpsc::channel(16);
        ColibriClient::new()
            .complete_stream(port, "secret-key", "qwen3.6-colibri", "hi", 8, tx)
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
                GenerationEvent::Done { tokens: 2 },
            ]
        );
        assert_eq!(
            seen_auth.lock().unwrap().as_deref(),
            Some("Bearer secret-key")
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
        ColibriClient::new()
            .complete_stream(port, "key", "m", "hi", 8, tx)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn complete_stream_errors_on_server_rejection() {
        let port = serve(Router::new().route(
            "/v1/chat/completions",
            post(|| async { axum::http::StatusCode::UNAUTHORIZED }),
        ))
        .await;

        let (tx, _rx) = mpsc::channel(1);
        let err = ColibriClient::new()
            .complete_stream(port, "wrong-key", "m", "hi", 8, tx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("rejected"));
    }
}
