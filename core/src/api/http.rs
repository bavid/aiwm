//! The loopback HTTP + WebSocket transport. Routes are thin wrappers over
//! [`super::handlers`]; the server binds `127.0.0.1` only (ADR-008).

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::dto::{
    AgentMessageDto, AgentPermissionDto, AttachDocumentDto, AttachExternalDto, DetachEngineDto,
    LaunchExternalDto, NewAgentDto, NewSessionDto, OpenAgentSessionDto, RenameModelDto,
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
        .route(
            "/sessions/{id}/documents",
            get(list_documents).post(attach_document),
        )
        .route("/documents/{id}", axum::routing::delete(delete_document))
        .route("/jobs/{id}", get(job_detail).delete(delete_job))
        .route("/jobs/{id}/cancel", post(cancel_job))
        .route("/jobs/{id}/output", get(job_output))
        .route("/jobs/{id}/clean-audio", post(clean_audio))
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
        .route("/models/{id}/name", put(rename_model))
        .route("/models/{id}/unload", post(unload_model))
        .route("/registry/status", get(registry_status))
        .route("/registry/token", put(set_hf_token))
        .route("/civitai/status", get(civitai_status))
        .route("/civitai/token", put(set_civitai_token))
        .route("/civitai/search", get(civitai_search))
        .route("/civitai/models/{*id}", get(civitai_details))
        .route("/local-api/status", get(local_api_status))
        .route("/local-api/token", put(set_local_api_token))
        .route("/v1/{*path}", any(local_api_proxy))
        .route("/external-engines", get(external_engines))
        .route("/external-engines/attach", post(attach_external_engine))
        .route("/external-engines/detach", post(detach_engine))
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
        .route(
            "/downloads/finished",
            axum::routing::delete(clear_finished_downloads),
        )
        .route("/downloads/{id}/pause", post(pause_download))
        .route("/downloads/{id}/resume", post(resume_download))
        .route("/downloads/{id}/cancel", post(cancel_download))
        .route("/downloads/{id}", axum::routing::delete(delete_download))
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
        .route(
            "/launcher",
            get(launcher_status)
                .post(launch_external)
                .delete(stop_external_launch),
        )
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

async fn list_documents(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::db::Document>>, ApiError> {
    Ok(Json(handlers::list_documents(&app, &id).await?))
}

async fn attach_document(
    State(app): AppState,
    Path(id): Path<String>,
    Json(body): Json<AttachDocumentDto>,
) -> Result<(StatusCode, Json<crate::db::Document>), ApiError> {
    let doc = handlers::attach_document(&app, &id, &body.path).await?;
    Ok((StatusCode::CREATED, Json(doc)))
}

async fn delete_document(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    handlers::delete_document(&app, &id).await?;
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

async fn delete_job(State(app): AppState, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    handlers::delete_job(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
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
        Some("wav") => "audio/wav",
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

/// Run the sidecar's noise-reduction pass on an already-rendered narration
/// clip, overwriting it in place.
async fn clean_audio(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let duration_secs = handlers::clean_audio(&app, &id).await?;
    Ok(Json(serde_json::json!({ "duration_secs": duration_secs })))
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

async fn rename_model(
    State(app): AppState,
    Path(id): Path<String>,
    Json(body): Json<RenameModelDto>,
) -> Result<Json<crate::db::Model>, ApiError> {
    Ok(Json(handlers::rename_model(&app, &id, &body.name).await?))
}

async fn unload_model(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    handlers::unload_model(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
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

async fn civitai_status(State(app): AppState) -> Json<crate::registry::RegistryStatus> {
    Json(handlers::civitai_status(&app))
}

async fn set_civitai_token(
    State(app): AppState,
    Json(body): Json<SetTokenDto>,
) -> Result<StatusCode, ApiError> {
    handlers::set_civitai_token(&app, &body.token)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn local_api_status(State(app): AppState) -> Json<super::dto::LocalApiStatusDto> {
    Json(handlers::local_api_status(&app))
}

async fn set_local_api_token(
    State(app): AppState,
    Json(body): Json<SetTokenDto>,
) -> Result<StatusCode, ApiError> {
    handlers::set_local_api_token(&app, &body.token)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn external_engines(State(app): AppState) -> Json<Vec<crate::runtime::DetectedEngine>> {
    Json(handlers::external_engines(&app).await)
}

async fn attach_external_engine(
    State(app): AppState,
    Json(body): Json<AttachExternalDto>,
) -> Result<StatusCode, ApiError> {
    handlers::attach_external_engine(&app, body).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn detach_engine(
    State(app): AppState,
    Json(body): Json<DetachEngineDto>,
) -> Result<StatusCode, ApiError> {
    handlers::detach_engine(&app, body).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A minimal JSON error body, shaped like [`ApiError`]'s, for the hand-rolled
/// responses below that need a status `ApiError`'s `CoreError` mapping
/// doesn't have (401 unauthenticated, 503 no model loaded).
fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

/// Constant-time equality — the bearer token is a secret compared on every
/// proxied request, so this avoids leaking its value through response-time
/// timing differences.
fn token_matches(provided: &str, expected: &str) -> bool {
    let (p, e) = (provided.as_bytes(), expected.as_bytes());
    p.len() == e.len() && p.iter().zip(e).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Only `content-type` is worth carrying from the caller to llama-server: the
/// proxy has already consumed `Authorization` for its own auth, and nothing
/// else an OpenAI-compatible client sends (`Host`, `content-length`, …) should
/// reach the upstream request unchanged.
fn forward_request_headers(incoming: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    if let Some(ct) = incoming.get(axum::http::header::CONTENT_TYPE) {
        out.insert(axum::http::header::CONTENT_TYPE, ct.clone());
    }
    out
}

/// Stream `resp`'s body straight through as the axum response, carrying over
/// its status and content-type (llama.cpp's chat streaming is SSE — buffering
/// the whole body first would defeat the point).
fn stream_upstream_response(resp: reqwest::Response) -> Response {
    let status = resp.status();
    let content_type = resp.headers().get(reqwest::header::CONTENT_TYPE).cloned();
    let mut builder = Response::builder().status(status);
    if let Some(ct) = content_type {
        builder = builder.header(axum::http::header::CONTENT_TYPE, ct);
    }
    let body = axum::body::Body::from_stream(resp.bytes_stream());
    builder
        .body(body)
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

/// `ANY /v1/*` — the unified local API endpoint (7.x). Forwards to whichever
/// model is currently resident on llama.cpp, so external OpenAI-compatible
/// tools (continue.dev, aider, …) can point at one stable address instead of
/// tracking per-runtime ports. Gated by a bearer token set via
/// `PUT /local-api/token`; the proxy refuses everything until one is
/// configured, since this is the one route on the loopback server meant to be
/// reachable by processes other than the AIWM UI itself.
async fn local_api_proxy(
    State(app): AppState,
    method: Method,
    Path(path): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(expected_token) = handlers::local_api_token(&app) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "the local API token has not been set — configure one in Settings first",
        );
    };
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if !provided.is_some_and(|p| token_matches(p, &expected_token)) {
        return error_response(StatusCode::UNAUTHORIZED, "missing or invalid bearer token");
    }
    if app.llama.base_url().is_none() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "no model is currently loaded — load one from the Chat or Models tab first",
        );
    }
    match app
        .llama
        .proxy_v1(
            method,
            &path,
            forward_request_headers(&headers),
            body.to_vec(),
        )
        .await
    {
        Ok(resp) => stream_upstream_response(resp),
        Err(e) => ApiError::from(e).into_response(),
    }
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

async fn civitai_search(
    State(app): AppState,
    Query(params): Query<super::dto::CivitaiSearchDto>,
) -> Result<Json<crate::registry::Fetched<Vec<crate::registry::RemoteModel>>>, ApiError> {
    Ok(Json(handlers::civitai_search(&app, params).await?))
}

async fn civitai_details(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<Json<super::dto::RegistryDetailsDto>, ApiError> {
    Ok(Json(handlers::civitai_details(&app, &id).await?))
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

async fn delete_download(
    State(app): AppState,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    handlers::delete_download(&app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn clear_finished_downloads(State(app): AppState) -> Result<Json<u64>, ApiError> {
    Ok(Json(handlers::clear_finished_downloads(&app).await?))
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

// --- external launcher -------------------------------------------------

async fn launcher_status(State(app): AppState) -> Json<Option<crate::LaunchInfo>> {
    Json(handlers::launcher_status(&app))
}

async fn launch_external(
    State(app): AppState,
    Json(body): Json<LaunchExternalDto>,
) -> Result<(StatusCode, Json<crate::LaunchInfo>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(handlers::launch_external(&app, body).await?),
    ))
}

async fn stop_external_launch(State(app): AppState) -> Result<StatusCode, ApiError> {
    handlers::stop_external_launch(&app).await?;
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
