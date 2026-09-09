//! The loopback HTTP + WebSocket transport. Routes are thin wrappers over
//! [`super::handlers`]; the server binds `127.0.0.1` only (ADR-008).

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::dto::{
    AgentMessageDto, AgentPermissionDto, NewAgentDto, OpenAgentSessionDto, SubmitJobDto,
};
use super::handlers;
use crate::db::JobFilter;
use crate::orchestrator::JobState;
use crate::{App, CoreError};

type AppState = State<Arc<App>>;

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/about", get(about))
        .route("/telemetry", get(telemetry))
        .route("/config", get(config).put(save_config))
        .route("/settings", get(settings))
        .route("/settings/{key}", put(set_setting))
        .route("/jobs", get(list_jobs).post(submit_job))
        .route("/jobs/{id}", get(job_detail))
        .route("/jobs/{id}/cancel", post(cancel_job))
        .route("/jobs/{id}/output", get(job_output))
        .route("/models", get(list_models).post(import_model))
        .route("/models/known", get(known_models))
        .route("/runtimes", get(runtimes))
        .route("/runtimes/llamacpp/install", post(install_llamacpp))
        .route("/runtimes/comfyui/install", post(install_comfyui))
        .route("/runtimes/hermes/install", post(install_hermes))
        .route("/agent-runtimes", get(agent_runtimes))
        .route("/agents", get(list_agents).post(create_agent))
        .route("/agents/{id}", axum::routing::delete(delete_agent))
        .route("/agent-sessions", post(open_agent_session))
        .route("/agent-sessions/{id}", get(agent_session_detail))
        .route("/agent-sessions/{id}/message", post(agent_session_message))
        .route(
            "/agent-sessions/{id}/permission",
            post(agent_session_permission),
        )
        .route("/agent-sessions/{id}/stop", post(stop_agent_session))
        .route("/export", get(export_backup))
        .route("/import", post(import_backup))
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
            CoreError::Config(_)
            | CoreError::InvalidJobTransition { .. }
            | CoreError::SchedulerBlocked(_) => StatusCode::BAD_REQUEST,
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

async fn config(State(app): AppState) -> Result<Json<crate::config::Config>, ApiError> {
    Ok(Json(handlers::config(&app)?))
}

async fn save_config(
    State(app): AppState,
    Json(update): Json<super::dto::ConfigUpdate>,
) -> Result<Json<crate::config::Config>, ApiError> {
    Ok(Json(handlers::save_config(&app, update)?))
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
    match handlers::job_detail(&app, &id).await? {
        Some(detail) => Ok(Json(detail).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no such job" })),
        )
            .into_response()),
    }
}

async fn cancel_job(State(app): AppState, Path(id): Path<String>) -> Result<Response, ApiError> {
    match handlers::cancel_job(&app, &id).await? {
        Some(applied) => Ok(Json(serde_json::json!({ "cancelled": applied })).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no such job" })),
        )
            .into_response()),
    }
}

/// Serve the image a finished `job_type=image` job produced. Loopback only
/// (ADR-008); the Image tab points an `<img>` here.
async fn job_output(State(app): AppState, Path(id): Path<String>) -> Result<Response, ApiError> {
    let Some(path) = handlers::job_output_path(&app, &id).await? else {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no output for this job" })),
        )
            .into_response());
    };
    let bytes = tokio::fs::read(&path).await.map_err(CoreError::Io)?;
    let content_type = match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    };
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, content_type),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    )
        .into_response())
}

async fn list_models(State(app): AppState) -> Result<Json<Vec<crate::db::Model>>, ApiError> {
    Ok(Json(handlers::list_models(&app).await?))
}

async fn known_models(State(app): AppState) -> Json<&'static [crate::model::KnownModel]> {
    Json(handlers::known_models(&app))
}

async fn import_model(
    State(app): AppState,
    Json(req): Json<crate::model::ImportRequest>,
) -> Result<(StatusCode, Json<crate::model::ImportOutcome>), ApiError> {
    let outcome = handlers::import_model(&app, req).await?;
    let code = if outcome.already_present {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((code, Json(outcome)))
}

async fn runtimes(State(app): AppState) -> Json<Vec<super::dto::RuntimeStatusDto>> {
    Json(handlers::runtimes(&app).await)
}

async fn install_hermes(
    State(app): AppState,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    install_status(handlers::install_hermes(&app)?)
}

async fn agent_runtimes(State(app): AppState) -> Json<Vec<super::dto::AgentRuntimeDto>> {
    Json(handlers::agent_runtimes(&app))
}

async fn export_backup(State(app): AppState) -> Result<Response, ApiError> {
    let bytes = handlers::export_backup(&app).await?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/zip"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"aiwm-export.zip\"",
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn import_backup(
    State(app): AppState,
    body: axum::body::Bytes,
) -> Result<Json<crate::backup::ImportSummary>, ApiError> {
    Ok(Json(handlers::import_backup(&app, &body).await?))
}

// --- agents (Phase 5.1c) ---------------------------------------------------

async fn list_agents(State(app): AppState) -> Result<Json<Vec<crate::db::Agent>>, ApiError> {
    Ok(Json(handlers::list_agents(&app).await?))
}

async fn create_agent(
    State(app): AppState,
    Json(body): Json<NewAgentDto>,
) -> Result<(StatusCode, Json<crate::db::Agent>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(handlers::create_agent(&app, body).await?),
    ))
}

async fn delete_agent(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    handlers::delete_agent(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn open_agent_session(
    State(app): AppState,
    Json(body): Json<OpenAgentSessionDto>,
) -> Result<(StatusCode, Json<crate::db::AgentSession>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(handlers::open_agent_session(&app, body).await?),
    ))
}

async fn agent_session_detail(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    match handlers::agent_session_detail(&app, &id).await? {
        Some(detail) => Ok(Json(detail).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no such agent session" })),
        )
            .into_response()),
    }
}

async fn agent_session_message(
    State(app): AppState,
    Path(id): Path<String>,
    Json(body): Json<AgentMessageDto>,
) -> Result<StatusCode, ApiError> {
    handlers::agent_session_message(&app, &id, &body.text).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn agent_session_permission(
    State(app): AppState,
    Path(id): Path<String>,
    Json(body): Json<AgentPermissionDto>,
) -> Result<StatusCode, ApiError> {
    handlers::agent_session_permission(&app, &id, body).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop_agent_session(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    handlers::stop_agent_session(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn install_llamacpp(
    State(app): AppState,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    install_status(handlers::install_llamacpp(&app)?)
}

async fn install_comfyui(
    State(app): AppState,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    install_status(handlers::install_comfyui(&app)?)
}

fn install_status(status: &str) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let code = if status == "started" {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    Ok((code, Json(serde_json::json!({ "status": status }))))
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
