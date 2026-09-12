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
    AgentMessageDto, AgentPermissionDto, NewAgentDto, NewSessionDto, OpenAgentSessionDto,
    RenameSessionDto, SetArchivedDto, SetRolesDto, SetTagsDto, SetTokenDto, SubmitJobDto,
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
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/{id}", put(rename_session).delete(delete_session))
        .route("/sessions/{id}/archived", put(set_session_archived))
        .route("/jobs/{id}", get(job_detail))
        .route("/jobs/{id}/cancel", post(cancel_job))
        .route("/jobs/{id}/output", get(job_output))
        .route("/models", get(list_models).post(import_model))
        .route("/models/known", get(known_models))
        .route("/models/stacks", get(model_stacks))
        .route("/models/featured", get(featured_models))
        .route(
            "/models/colibri",
            get(colibri_models).post(register_colibri_model),
        )
        .route("/models/{id}", axum::routing::delete(delete_model))
        .route("/models/tags", get(model_tags))
        .route("/models/{id}/tags", put(set_model_tags))
        .route("/models/{id}/roles", put(set_model_roles))
        .route("/registry/status", get(registry_status))
        .route("/registry/token", put(set_hf_token))
        .route("/storage", get(storage_report))
        .route("/models/{id}/benchmark", post(benchmark_model))
        .route("/models/{id}/benchmarks", get(model_benchmarks))
        .route("/models/{id}/upgrade-check", post(upgrade_check))
        .route("/benchmarks", get(latest_benchmarks))
        .route("/runtimes", get(runtimes))
        .route("/runtimes/llamacpp/install", post(install_llamacpp))
        .route("/runtimes/comfyui/install", post(install_comfyui))
        .route("/runtimes/hermes/install", post(install_hermes))
        .route("/runtimes/colibri/install", post(install_colibri))
        .route("/agent-runtimes", get(agent_runtimes))
        .route("/registry/search", get(registry_search))
        .route("/registry/models/{*id}", get(registry_details))
        .route("/downloads", get(list_downloads).post(enqueue_download))
        .route("/downloads/{id}/pause", post(pause_download))
        .route("/downloads/{id}/resume", post(resume_download))
        .route("/downloads/{id}/cancel", post(cancel_download))
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

#[derive(Deserialize)]
struct SessionQuery {
    capability: String,
}

async fn list_sessions(
    State(app): AppState,
    Query(q): Query<SessionQuery>,
) -> Result<Json<Vec<crate::db::Session>>, ApiError> {
    Ok(Json(handlers::list_sessions(&app, &q.capability).await?))
}

async fn create_session(
    State(app): AppState,
    Json(body): Json<NewSessionDto>,
) -> Result<(StatusCode, Json<crate::db::Session>), ApiError> {
    let session = handlers::create_session(&app, body).await?;
    Ok((StatusCode::CREATED, Json(session)))
}

async fn rename_session(
    State(app): AppState,
    Path(id): Path<String>,
    Json(body): Json<RenameSessionDto>,
) -> Result<StatusCode, ApiError> {
    handlers::rename_session(&app, &id, &body.name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_session_archived(
    State(app): AppState,
    Path(id): Path<String>,
    Json(body): Json<SetArchivedDto>,
) -> Result<StatusCode, ApiError> {
    handlers::set_session_archived(&app, &id, body.archived).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_session(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    handlers::delete_session(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
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

async fn known_models(State(app): AppState) -> Json<Vec<super::dto::KnownModelDto>> {
    Json(handlers::known_models(&app))
}

async fn model_stacks(State(app): AppState) -> Json<Vec<super::dto::ModelStackDto>> {
    Json(handlers::model_stacks(&app))
}

async fn featured_models(State(app): AppState) -> Json<Vec<super::dto::FeaturedModelDto>> {
    Json(handlers::featured_models(&app))
}

async fn storage_report(
    State(app): AppState,
) -> Result<Json<crate::cleanup::StorageReport>, ApiError> {
    Ok(Json(handlers::storage_report(&app).await?))
}

async fn model_tags(
    State(app): AppState,
) -> Result<Json<std::collections::BTreeMap<String, Vec<String>>>, ApiError> {
    Ok(Json(handlers::model_tags(&app).await?))
}

async fn set_model_tags(
    State(app): AppState,
    Path(id): Path<String>,
    Json(body): Json<SetTagsDto>,
) -> Result<Json<Vec<String>>, ApiError> {
    Ok(Json(handlers::set_model_tags(&app, &id, &body.tags).await?))
}

async fn set_model_roles(
    State(app): AppState,
    Path(id): Path<String>,
    Json(body): Json<SetRolesDto>,
) -> Result<Json<Vec<String>>, ApiError> {
    Ok(Json(
        handlers::set_model_roles(&app, &id, &body.roles).await?,
    ))
}

async fn registry_status(State(app): AppState) -> Json<crate::registry::RegistryStatus> {
    Json(handlers::registry_status(&app))
}

async fn set_hf_token(
    State(app): AppState,
    Json(body): Json<SetTokenDto>,
) -> Result<StatusCode, ApiError> {
    handlers::set_hf_token(&app, &body.token)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_model(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<Json<crate::model::DeleteOutcome>, ApiError> {
    Ok(Json(handlers::delete_model(&app, &id).await?))
}

async fn latest_benchmarks(
    State(app): AppState,
) -> Result<Json<Vec<crate::db::Benchmark>>, ApiError> {
    Ok(Json(handlers::latest_benchmarks(&app).await?))
}

async fn model_benchmarks(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::db::Benchmark>>, ApiError> {
    Ok(Json(handlers::model_benchmarks(&app, &id).await?))
}

async fn benchmark_model(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<crate::db::Job>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(handlers::benchmark_model(&app, &id).await?),
    ))
}

async fn upgrade_check(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<crate::db::Job>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(handlers::upgrade_check(&app, &id).await?),
    ))
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

async fn install_colibri(
    State(app): AppState,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    install_status(handlers::install_colibri(&app)?)
}

async fn colibri_models(State(app): AppState) -> Json<Vec<super::dto::ColibriModelDto>> {
    Json(handlers::colibri_models(&app))
}

async fn register_colibri_model(
    State(app): AppState,
    Json(body): Json<super::dto::RegisterColibriModelDto>,
) -> Result<(StatusCode, Json<crate::db::Model>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(handlers::register_colibri_model(&app, body).await?),
    ))
}

async fn agent_runtimes(State(app): AppState) -> Json<Vec<super::dto::AgentRuntimeDto>> {
    Json(handlers::agent_runtimes(&app))
}

async fn registry_search(
    State(app): AppState,
    Query(params): Query<super::dto::RegistrySearchDto>,
) -> Result<Json<crate::registry::Fetched<Vec<crate::registry::RemoteModel>>>, ApiError> {
    Ok(Json(handlers::registry_search(&app, params).await?))
}

async fn registry_details(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<Json<super::dto::RegistryDetailsDto>, ApiError> {
    Ok(Json(handlers::registry_details(&app, &id).await?))
}

async fn list_downloads(State(app): AppState) -> Result<Json<Vec<crate::db::Download>>, ApiError> {
    Ok(Json(handlers::list_downloads(&app).await?))
}

async fn enqueue_download(
    State(app): AppState,
    Json(body): Json<super::dto::EnqueueDownloadDto>,
) -> Result<(StatusCode, Json<crate::db::Download>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(handlers::enqueue_download(&app, body).await?),
    ))
}

async fn pause_download(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    handlers::pause_download(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn resume_download(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    handlers::resume_download(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn cancel_download(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    handlers::cancel_download(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
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
