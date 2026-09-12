//! Test fixture: a minimal `hermes gateway` stand-in.
//!
//! Speaks just enough of Hermes' API-server surface for the `HermesAgentAdapter`
//! tests: `GET /health`, `POST /api/sessions`, `POST /api/sessions/{id}/chat/stream`
//! (SSE), `POST /v1/runs/{id}/approval`, `POST /v1/runs/{id}/stop`,
//! `DELETE /api/sessions/{id}`. Every route checks the `Authorization: Bearer`
//! header against `API_SERVER_KEY`.
//!
//! On a turn it scripts: `run.started` → an `assistant.delta` → a `tool.start`
//! (terminal) → `approval.request`, then waits. On approval it emits the tool
//! result, a closing delta and `run.completed`; on rejection it goes straight to
//! `run.completed`.
//!
//! Not shipped — it exists so the open → send → approve → idle cycle can run
//! without the real `hermes` install or a coding model.

use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::broadcast;

const SESSION_ID: &str = "ses_fake_1";
const RUN_ID: &str = "run_fake_1";

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
struct Fx {
    key: String,
    events: broadcast::Sender<String>,
    replies: Arc<Mutex<Vec<(String, bool)>>>,
}

impl Fx {
    fn emit(&self, name: &str, data: Value) {
        let _ = self.events.send(format!("event: {name}\ndata: {data}\n\n"));
    }
}

fn authed(fx: &Fx, headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        == Some(fx.key.as_str())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("API_SERVER_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let key = std::env::var("API_SERVER_KEY").unwrap_or_default();

    let (events, _) = broadcast::channel(64);
    let state = Fx {
        key,
        events,
        replies: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/sessions", post(create_session))
        .route("/api/sessions/{id}/chat/stream", post(chat_stream))
        .route("/api/sessions/{id}", delete(delete_session))
        .route("/v1/runs/{id}/approval", post(approval))
        .route("/v1/runs/{id}/stop", post(|| async { StatusCode::OK }))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("fake-hermes: listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(fx): State<Fx>, headers: HeaderMap) -> Response {
    if !authed(&fx, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({ "status": "ok" })).into_response()
}

async fn create_session(State(fx): State<Fx>, headers: HeaderMap) -> Response {
    if !authed(&fx, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // Matches the real `hermes-agent` 0.19.0 API server: the id is nested
    // under `session`, not top-level (found against a real install — the
    // adapter originally expected a flat `{"id": ...}`).
    Json(json!({
        "object": "hermes.session",
        "session": { "id": SESSION_ID, "title": Value::Null, "started_at": 0 }
    }))
    .into_response()
}

async fn delete_session(
    State(fx): State<Fx>,
    headers: HeaderMap,
    Path(_id): Path<String>,
) -> StatusCode {
    if authed(&fx, &headers) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::UNAUTHORIZED
    }
}

async fn chat_stream(
    State(fx): State<Fx>,
    headers: HeaderMap,
    Path(_id): Path<String>,
    _body: String,
) -> Response {
    if !authed(&fx, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let rx = fx.events.subscribe();

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        fx.emit("run.started", json!({ "run_id": RUN_ID }));
        fx.emit("assistant.delta", json!({ "delta": "I'll read the file." }));
        fx.emit(
            "tool.start",
            json!({ "call_id": "c1", "tool_name": "terminal", "command": "cat readme.txt" }),
        );
        fx.emit(
            "approval.request",
            json!({ "run_id": RUN_ID, "tool": "terminal", "command": "cat readme.txt" }),
        );
    });

    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(line) => return Some((Ok::<String, Infallible>(line), rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

async fn approval(
    State(fx): State<Fx>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> StatusCode {
    if !authed(&fx, &headers) {
        return StatusCode::UNAUTHORIZED;
    }
    let approved = body
        .get("approved")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    guard(&fx.replies).push((id, approved));

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        if approved {
            fx.emit(
                "tool.complete",
                json!({ "call_id": "c1", "tool_name": "terminal", "output": "hello from the readme\n" }),
            );
            fx.emit(
                "assistant.delta",
                json!({ "delta": "The readme says hello." }),
            );
        }
        fx.emit("run.completed", json!({ "run_id": RUN_ID }));
    });
    StatusCode::OK
}
