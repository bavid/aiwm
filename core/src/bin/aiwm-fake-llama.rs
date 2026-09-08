//! Test fixture: a minimal `llama-server` stand-in.
//!
//! Speaks just enough of the real HTTP API for the runtime + chat-job tests —
//! `GET /health`, `GET /props`, `POST /completion`, and streaming
//! `POST /v1/chat/completions` (SSE) — and parses `--port` / `-m` from the
//! command line the way the real server does, ignoring every other flag. Not
//! part of the shipped product; it exists so the spawn / health / serve / stream
//! / stop cycle can be exercised without a 600 MB download.
//!
//! `AIWM_FAKE_LLAMA_READY_MS=<n>` makes `/health` return `503` for the first `n`
//! milliseconds, simulating a slow model load.

use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{self, Stream, StreamExt};
use serde_json::{json, Value};

#[derive(Clone)]
struct Fixture {
    model_path: String,
    ready_at: Instant,
    /// Chat stream shape — bumped in the cancel test so there is time to cancel.
    token_ms: u64,
    tokens: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = 8080u16;
    let mut model_path = String::new();
    let mut token_ms = 10u64;
    let mut tokens = 12usize;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                if let Some(v) = args.next() {
                    port = v.parse().unwrap_or(port);
                }
            }
            "-m" | "--model" => model_path = args.next().unwrap_or_default(),
            "--fake-token-ms" => {
                token_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(token_ms);
            }
            "--fake-tokens" => {
                tokens = args.next().and_then(|v| v.parse().ok()).unwrap_or(tokens);
            }
            _ => {}
        }
    }

    let ready_delay = std::env::var("AIWM_FAKE_LLAMA_READY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(Duration::ZERO, Duration::from_millis);

    let state = Fixture {
        model_path,
        ready_at: Instant::now() + ready_delay,
        token_ms,
        tokens,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/props", get(props))
        .route("/completion", post(completion))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("fake-llama: listening on 127.0.0.1:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(fx): State<Fixture>) -> impl IntoResponse {
    if Instant::now() >= fx.ready_at {
        (StatusCode::OK, Json(json!({ "status": "ok" })))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": { "message": "Loading model", "code": 503 } })),
        )
    }
}

async fn props(State(fx): State<Fixture>) -> Json<Value> {
    Json(json!({ "model_path": fx.model_path }))
}

async fn completion(body: Json<Value>) -> Json<Value> {
    let prompt = body.0.get("prompt").and_then(Value::as_str).unwrap_or("");
    let head: String = prompt.chars().take(40).collect();
    Json(json!({
        "content": format!("fake-llama reply to: {head}"),
        "stop_type": "eos",
        "tokens_predicted": 5,
    }))
}

/// Streaming OpenAI-compatible chat completion. Echoes a canned sentence one
/// word per SSE event (with a small delay so the client sees real chunks), then
/// a `finish_reason` chunk with stats and the `[DONE]` sentinel.
async fn chat_completions(
    State(fx): State<Fixture>,
    body: Json<Value>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let prompt = body
        .0
        .pointer("/messages/0/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(30)
        .collect::<String>();
    let base = format!("fake-llama here — you said: {prompt} .");
    let mut words: Vec<String> = base.split_inclusive(' ').map(str::to_string).collect();
    while words.len() < fx.tokens {
        words.push(format!("word{} ", words.len()));
    }
    let total = words.len() as u64;
    let token_ms = fx.token_ms;

    let tokens = stream::iter(words).then(move |w| async move {
        tokio::time::sleep(Duration::from_millis(token_ms)).await;
        Ok(Event::default().data(
            json!({ "choices": [{ "index": 0, "delta": { "content": w }, "finish_reason": null }] })
                .to_string(),
        ))
    });
    let finish = stream::once(async move {
        Ok(Event::default().data(
            json!({
                "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }],
                "usage": { "completion_tokens": total },
                "timings": { "predicted_n": total, "predicted_per_second": 42.0 }
            })
            .to_string(),
        ))
    });
    let done = stream::once(async { Ok(Event::default().data("[DONE]")) });

    Sse::new(tokens.chain(finish).chain(done))
}
