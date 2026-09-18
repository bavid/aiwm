//! Test fixture: a minimal Colibri `coli serve` stand-in.
//!
//! Speaks just enough of the real OpenAI-compatible API for the adapter
//! integration test — `GET /health` and streaming `POST /v1/chat/completions`
//! (SSE, bearer-token checked) — and parses `--port` from `coli serve`'s own
//! flags, reading `COLI_MODEL` / `COLI_API_KEY` from the environment the way
//! the real launcher does. Not part of the shipped product; it exists so the
//! spawn (via `cmd.exe /C coli.cmd ...`) / health / stream / stop cycle can be
//! exercised without the real ~4.3 MB download plus a multi-GB model.
//!
//! One test-only extra the real server does not have, mirroring fake-llama's:
//! `GET /__test/last_request` returns the last `/v1/chat/completions` body, so a
//! test can assert *what* was asked for (e.g. a persona's system message).

use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{self, StreamExt};
use serde_json::{json, Value};

#[derive(Clone)]
struct Fixture {
    api_key: String,
    /// Last `/v1/chat/completions` body, for `GET /__test/last_request`.
    last_request: Arc<Mutex<Option<Value>>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = 8080u16;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--port" {
            if let Some(v) = args.next() {
                port = v.parse().unwrap_or(port);
            }
        }
    }

    let state = Fixture {
        api_key: std::env::var("COLI_API_KEY").unwrap_or_default(),
        last_request: Arc::new(Mutex::new(None)),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/__test/last_request", get(last_request))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("fake-colibri: listening on 127.0.0.1:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "active": 0, "queued": 0 })))
}

/// Streaming OpenAI-compatible chat completion. Rejects a missing/wrong
/// bearer token with 401 (proving the client sends `COLI_API_KEY`), else
/// echoes a canned sentence one word per SSE event, then a `finish_reason`
/// chunk with `usage.completion_tokens` and the `[DONE]` sentinel.
async fn chat_completions(
    State(fx): State<Fixture>,
    headers: HeaderMap,
    body: Json<Value>,
) -> axum::response::Response {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth != format!("Bearer {}", fx.api_key) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    if let Ok(mut slot) = fx.last_request.lock() {
        *slot = Some(body.0.clone());
    }

    let prompt = body
        .0
        .pointer("/messages/0/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(30)
        .collect::<String>();
    let base = format!("fake-colibri here — you said: {prompt} .");
    let words: Vec<String> = base.split_inclusive(' ').map(str::to_string).collect();
    let total = words.len() as u64;

    let tokens = stream::iter(words).then(|w| async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        Ok::<_, Infallible>(Event::default().data(
            json!({ "choices": [{ "index": 0, "delta": { "content": w }, "finish_reason": null }] })
                .to_string(),
        ))
    });
    let finish = stream::once(async move {
        Ok(Event::default().data(
            json!({
                "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }],
                "usage": { "completion_tokens": total }
            })
            .to_string(),
        ))
    });
    let done = stream::once(async { Ok(Event::default().data("[DONE]")) });

    Sse::new(tokens.chain(finish).chain(done)).into_response()
}

/// The last chat-completion body the fixture served, or `null` before the first
/// one — test-only, no equivalent on the real server.
async fn last_request(State(fx): State<Fixture>) -> Json<Value> {
    let body = fx
        .last_request
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or(Value::Null);
    Json(body)
}
