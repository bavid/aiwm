//! Test fixture: a minimal `opencode serve` stand-in.
//!
//! Speaks just enough of OpenCode's HTTP API for the `OpenCodeAdapter` tests:
//! `GET /config` (health), `POST /session`, `POST /session/{id}/prompt_async`,
//! `GET /event` (SSE), `GET /permission`, `POST /permission/{id}/reply`,
//! `POST /session/{id}/abort`, `DELETE /session/{id}`.
//!
//! On a prompt it scripts one turn: a text part, a `bash` tool call, a
//! `permission.asked`, then it waits. On the permission reply it emits the tool
//! result, a closing text part, and `session.idle` — unless the reply is
//! `reject`, in which case it just goes idle.
//!
//! Not shipped — it exists so the open → send → approve → idle cycle can run
//! without the real `opencode` binary or a coding model.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard, PoisonError};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::broadcast;

const SESSION_ID: &str = "ses_fake_1";

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
struct Fx {
    events: broadcast::Sender<String>,
    /// The id of the permission request currently awaiting a reply.
    pending: Arc<Mutex<Option<String>>>,
    replies: Arc<Mutex<Vec<(String, String)>>>,
}

impl Fx {
    fn emit(&self, ty: &str, properties: Value) {
        let _ = self.events.send(format!(
            "data: {}\n\n",
            json!({ "type": ty, "properties": properties })
        ));
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = 0u16;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--port" {
            if let Some(v) = args.next() {
                port = v.parse().unwrap_or(0);
            }
        }
    }

    let (events, _) = broadcast::channel(64);
    let state = Fx {
        events,
        pending: Arc::new(Mutex::new(None)),
        replies: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route(
            "/config",
            get(|| async { Json(json!({ "model": "local/coder" })) }),
        )
        .route("/session", post(create_session))
        .route("/session/{id}/prompt_async", post(prompt))
        .route("/session/{id}/abort", post(|| async { StatusCode::OK }))
        .route("/session/{id}", delete(|| async { StatusCode::OK }))
        .route("/event", get(event_stream))
        .route("/permission", get(list_permissions))
        .route("/permission/{id}/reply", post(reply_permission))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("fake-opencode: listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn create_session() -> Json<Value> {
    Json(json!({ "id": SESSION_ID, "title": "aiwm agent session" }))
}

async fn event_stream(State(fx): State<Fx>) -> Response {
    let rx = fx.events.subscribe();
    let _ = fx
        .events
        .send("data: {\"type\":\"server.connected\",\"properties\":{}}\n\n".to_string());
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(line) => return Some((Ok::<String, std::convert::Infallible>(line), rx)),
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

async fn prompt(State(fx): State<Fx>, Path(_id): Path<String>, _body: String) -> StatusCode {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        fx.emit(
            "message.updated",
            json!({ "info": { "id": "msg_a", "role": "assistant", "sessionID": SESSION_ID } }),
        );
        fx.emit(
            "message.part.updated",
            json!({ "part": { "id": "prt_1", "messageID": "msg_a", "sessionID": SESSION_ID,
                              "type": "text", "text": "I'll read the file." } }),
        );
        fx.emit(
            "message.part.updated",
            json!({ "part": { "id": "prt_2", "messageID": "msg_a", "sessionID": SESSION_ID, "type": "tool",
                              "tool": "bash", "callID": "call_1",
                              "state": { "status": "running", "input": { "command": "cat readme.txt" } } } }),
        );
        *guard(&fx.pending) = Some("per_1".to_string());
        fx.emit(
            "permission.asked",
            json!({ "id": "per_1", "sessionID": SESSION_ID, "permission": "bash",
                    "patterns": ["cat readme.txt"], "metadata": { "command": "cat readme.txt" },
                    "always": ["cat *"], "tool": { "messageID": "msg_a", "callID": "call_1" } }),
        );
    });
    StatusCode::OK
}

async fn list_permissions(State(fx): State<Fx>) -> Json<Value> {
    match guard(&fx.pending).clone() {
        Some(id) => Json(json!([{
            "id": id, "sessionID": SESSION_ID, "permission": "bash",
            "patterns": ["cat readme.txt"], "metadata": { "command": "cat readme.txt" },
            "always": ["cat *"]
        }])),
        None => Json(json!([])),
    }
}

async fn reply_permission(
    State(fx): State<Fx>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> StatusCode {
    let response = body
        .get("response")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    guard(&fx.replies).push((id, response.clone()));
    *guard(&fx.pending) = None;

    if response == "reject" {
        fx.emit("session.idle", json!({ "sessionID": SESSION_ID }));
        return StatusCode::OK;
    }

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        fx.emit(
            "message.part.updated",
            json!({ "part": { "id": "prt_2", "messageID": "msg_a", "sessionID": SESSION_ID, "type": "tool",
                              "tool": "bash", "callID": "call_1",
                              "state": { "status": "completed", "input": { "command": "cat readme.txt" },
                                         "output": "hello from the readme\n" } } }),
        );
        fx.emit(
            "message.part.updated",
            json!({ "part": { "id": "prt_3", "messageID": "msg_a", "sessionID": SESSION_ID,
                              "type": "text", "text": "The readme says hello." } }),
        );
        fx.emit("session.idle", json!({ "sessionID": SESSION_ID }));
    });
    StatusCode::OK
}
