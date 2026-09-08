//! The loopback HTTP + WebSocket transport. Routes are thin wrappers over
//! [`super::handlers`]; the server binds `127.0.0.1` only (ADR-008).

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::dto::SubmitJobDto;
use super::handlers;
use crate::db::JobFilter;
use crate::orchestrator::JobState;
use crate::{App, CoreError};

type AppState = State<Arc<App>>;

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/about", get(about))
        .route("/telemetry", get(telemetry))
        .route("/settings", get(settings))
        .route("/settings/{key}", put(set_setting))
        .route("/jobs", get(list_jobs).post(submit_job))
        .route("/jobs/{id}", get(job_detail))
        .route("/runtimes", get(runtimes))
        .route("/logs", get(logs))
        .route("/ws", get(ws_upgrade))
        .with_state(app)
}

// --- error mapping ------------------------------------------------------------

struct ApiError(CoreError);

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            CoreError::Config(_) | CoreError::InvalidJobTransition { .. } => {
                StatusCode::BAD_REQUEST
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        tracing::warn!(error = %self.0, %status, "api error");
        (
            status,
            Json(serde_json::json!({ "error": self.0.to_string() })),
        )
            .into_response()
    }
}

// --- routes -----------------------------------------------------------------

async fn about(State(app): AppState) -> Json<super::dto::AboutDto> {
    Json(handlers::about(&app))
}

async fn telemetry(State(app): AppState) -> Json<crate::telemetry::SystemTelemetry> {
    Json(handlers::telemetry(&app))
}

async fn settings(State(app): AppState) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!(handlers::settings(&app).await?)))
}

#[derive(Deserialize)]
struct SetSetting {
    value: String,
}

async fn set_setting(
    State(app): AppState,
    Path(key): Path<String>,
    Json(body): Json<SetSetting>,
) -> Result<StatusCode, ApiError> {
    handlers::set_setting(&app, &key, &body.value).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct JobQuery {
    states: Option<String>,
    limit: Option<u32>,
}

async fn list_jobs(
    State(app): AppState,
    Query(q): Query<JobQuery>,
) -> Result<Json<Vec<crate::db::Job>>, ApiError> {
    let states = match q.states {
        Some(s) => s
            .split(',')
            .filter(|p| !p.is_empty())
            .map(str::parse::<JobState>)
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    let filter = JobFilter {
        states,
        limit: q.limit,
    };
    Ok(Json(handlers::list_jobs(&app, filter).await?))
}

async fn submit_job(
    State(app): AppState,
    Json(body): Json<SubmitJobDto>,
) -> Result<(StatusCode, Json<crate::db::Job>), ApiError> {
    let job = handlers::submit_job(&app, body).await?;
    Ok((StatusCode::CREATED, Json(job)))
}

async fn job_detail(State(app): AppState, Path(id): Path<String>) -> Result<Response, ApiError> {
    match handlers::job_events(&app, &id).await? {
        Some((job, events)) => {
            Ok(Json(serde_json::json!({ "job": job, "events": events })).into_response())
        }
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no such job" })),
        )
            .into_response()),
    }
}

async fn runtimes(State(app): AppState) -> Json<Vec<super::dto::RuntimeStatusDto>> {
    Json(handlers::runtimes(&app).await)
}

#[derive(Deserialize)]
struct LogsQuery {
    lines: Option<usize>,
}

async fn logs(
    State(app): AppState,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    Ok(Json(handlers::recent_logs(&app, q.lines.unwrap_or(200))?))
}

// --- websocket: telemetry stream ------------------------------------------------

async fn ws_upgrade(State(app): AppState, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| telemetry_stream(socket, app))
}

async fn telemetry_stream(mut socket: WebSocket, app: Arc<App>) {
    let mut rx = app.telemetry.subscribe();
    // Send the current reading immediately, then one per update.
    loop {
        let payload = match serde_json::to_string(&*rx.borrow_and_update()) {
            Ok(s) => s,
            Err(_) => return,
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            return; // client gone
        }
        if rx.changed().await.is_err() {
            return; // sampler stopped
        }
    }
}
