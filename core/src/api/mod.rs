//! The core's outward surface, exposed over two transports that serialize the
//! **same** [`handlers`] outputs:
//!
//! - Tauri IPC — thin `#[tauri::command]` wrappers in `src-tauri`
//! - a loopback HTTP + WebSocket server on `127.0.0.1:<core_api_port>` (ADR-008)
//!
//! [`spawn`] starts that server plus the background job loop; the returned
//! [`Services`] handle stops both when dropped.

pub mod dto;
pub mod handlers;
mod http;

pub use http::router;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::{App, CoreError, Result};

/// Idle poll interval for the job loop when the queue is empty.
const JOB_LOOP_IDLE: Duration = Duration::from_millis(250);

/// A running HTTP/WS server. Bound address in [`ApiServer::addr`]; the task is
/// aborted on drop.
#[derive(Debug)]
pub struct ApiServer {
    pub addr: SocketAddr,
    task: JoinHandle<()>,
}

impl ApiServer {
    /// Bind and serve. `addr` should always be loopback in production.
    pub async fn bind(app: Arc<App>, addr: SocketAddr) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(CoreError::Io)?;
        let addr = listener.local_addr().map_err(CoreError::Io)?;
        let task = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, http::router(app)).await {
                tracing::error!(error = %e, "core API server stopped");
            }
        });
        Ok(Self { addr, task })
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The core's long-running background work: the loopback API and the job loop.
#[derive(Debug)]
pub struct Services {
    api: ApiServer,
    job_loop: JoinHandle<()>,
}

impl Services {
    /// The address the loopback API is bound to.
    pub fn api_addr(&self) -> SocketAddr {
        self.api.addr
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        self.job_loop.abort();
    }
}

/// Start the loopback API (on `127.0.0.1:core_api_port`) and the job loop.
pub async fn spawn(app: Arc<App>) -> Result<Services> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, app.config.core_api_port));
    spawn_on(app, addr).await
}

/// Like [`spawn`] but with an explicit bind address (tests use `127.0.0.1:0`).
pub async fn spawn_on(app: Arc<App>, api_addr: SocketAddr) -> Result<Services> {
    let api = ApiServer::bind(app.clone(), api_addr).await?;
    tracing::info!(addr = %api.addr, "core API listening (loopback only)");

    let job_loop = tokio::spawn(run_job_loop(app));
    Ok(Services { api, job_loop })
}

async fn run_job_loop(app: Arc<App>) {
    loop {
        match app.jobs.run_next().await {
            Ok(Some(outcome)) => tracing::info!(?outcome, "job finished"),
            Ok(None) => tokio::time::sleep(JOB_LOOP_IDLE).await,
            Err(e) => {
                tracing::error!(error = %e, "job loop error");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;
    use crate::api::dto::AboutDto;
    use crate::runtime::FakeRuntimeAdapter;

    async fn test_app() -> (Arc<App>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::AppPaths::rooted(tmp.path());
        let app = Arc::new(App::load(paths).await.unwrap());
        (app, tmp)
    }

    #[tokio::test]
    async fn server_binds_loopback_only() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        assert!(server.addr.ip().is_loopback());
    }

    #[tokio::test]
    async fn about_is_identical_over_the_handler_and_http() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();

        // Direct handler call.
        let direct = serde_json::to_value(handlers::about(&app)).unwrap();

        // Over HTTP.
        let url = format!("http://{}/about", server.addr);
        let over_http: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();

        assert_eq!(direct, over_http);
        // And it deserializes back to the DTO.
        let _: AboutDto = serde_json::from_value(over_http).unwrap();
    }

    #[tokio::test]
    async fn submit_and_list_jobs_over_http() {
        let (app, _tmp) = test_app().await;
        app.runtimes
            .register(Arc::new(FakeRuntimeAdapter::healthy("llamacpp")));
        let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);

        let created: serde_json::Value = reqwest::Client::new()
            .post(format!("{base}/jobs"))
            .json(&serde_json::json!({
                "job_type": "chat",
                "runtime_id": "llamacpp",
                "model_id": "qwen",
                "vram_needed_mb": 4000,
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(created["state"], "queued");

        let jobs: serde_json::Value = reqwest::get(format!("{base}/jobs"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(jobs.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn ws_streams_telemetry_frames() {
        use futures_util::StreamExt;

        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();

        let url = format!("ws://{}/ws", server.addr);
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

        let frame = ws.next().await.unwrap().unwrap();
        let text = frame.into_text().unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(value["captured_at_ms"].as_u64().unwrap() > 0);
        assert!(value["host"]["ram_total_mb"].as_u64().unwrap() > 0);
        assert!(value["gpu"]["state"].is_string());
    }

    #[tokio::test]
    async fn set_setting_rejects_empty_key_with_400() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();

        let resp = reqwest::Client::new()
            .put(format!("http://{}/settings/%20", server.addr))
            .json(&serde_json::json!({ "value": "x" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn job_loop_drains_the_queue() {
        let (app, _tmp) = test_app().await;
        app.runtimes
            .register(Arc::new(FakeRuntimeAdapter::healthy("llamacpp")));
        let _services = spawn_on(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();

        app.jobs
            .submit(crate::db::NewJob::new("chat").on("llamacpp", "m", 1_000))
            .await
            .unwrap();

        for _ in 0..40 {
            let jobs = app
                .db
                .jobs()
                .list(&crate::db::JobFilter {
                    states: vec![crate::orchestrator::JobState::Completed],
                    limit: None,
                })
                .await
                .unwrap();
            if !jobs.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("job loop did not complete the job");
    }
}
