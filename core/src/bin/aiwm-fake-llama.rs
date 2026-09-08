//! Test fixture: a minimal `llama-server` stand-in.
//!
//! Speaks just enough of the real HTTP API for [`LlamaCppAdapter`] integration
//! tests — `GET /health`, `GET /props`, `POST /completion` — and parses `--port`
//! / `-m` from the command line the way the real server does, ignoring every
//! other flag. Not part of the shipped product; it only exists so the adapter's
//! spawn/health/serve/stop cycle can be exercised without a 600 MB download.
//!
//! `AIWM_FAKE_LLAMA_READY_MS=<n>` makes `/health` return `503` for the first `n`
//! milliseconds, simulating a slow model load.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

#[derive(Clone)]
struct Fixture {
    model_path: String,
    ready_at: Instant,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = 8080u16;
    let mut model_path = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                if let Some(v) = args.next() {
                    port = v.parse().unwrap_or(port);
                }
            }
            "-m" | "--model" => model_path = args.next().unwrap_or_default(),
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
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/props", get(props))
        .route("/completion", post(completion))
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
