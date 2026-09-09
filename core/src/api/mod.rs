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

use crate::orchestrator::JobOutcome;
use crate::{App, CoreError, Result};

/// Idle poll interval for the job loop when the queue is empty.
const JOB_LOOP_IDLE: Duration = Duration::from_millis(250);
/// Backoff after a job comes to rest `blocked` (needs a human or freed VRAM).
/// It stays in the runnable set, so without this the loop would re-check it as
/// fast as it can spin. A few seconds keeps it responsive without the churn.
const JOB_LOOP_BLOCKED_BACKOFF: Duration = Duration::from_secs(3);

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
            Ok(Some(JobOutcome::Blocked { job_id, .. })) => {
                tracing::debug!(%job_id, "job blocked — backing off");
                tokio::time::sleep(JOB_LOOP_BLOCKED_BACKOFF).await;
            }
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
    async fn install_llamacpp_refuses_in_offline_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::AppPaths::rooted(tmp.path());
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(paths.config_file(), "offline_mode = true\n").unwrap();
        let app = Arc::new(App::load(paths).await.unwrap());

        let err = handlers::install_llamacpp(&app).unwrap_err();
        assert!(err.to_string().contains("offline mode"));
    }

    #[tokio::test]
    async fn install_llamacpp_is_a_noop_when_already_installed() {
        let (app, tmp) = test_app().await;
        let dir = tmp.path().join("runtimes").join("llamacpp").join("b10855");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        std::fs::write(dir.join(exe), b"present").unwrap();

        assert_eq!(
            handlers::install_llamacpp(&app).unwrap(),
            "already_installed"
        );
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

        // `GET /jobs/{id}` returns `{ job, events }` — the shape the chat UI polls.
        let id = created["id"].as_str().unwrap();
        let detail: serde_json::Value = reqwest::get(format!("{base}/jobs/{id}"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(detail["job"]["id"], created["id"]);
        assert!(
            detail["job"].get("result").is_some(),
            "result field present"
        );
        assert!(detail["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["message"] == "job queued"));

        let missing = reqwest::get(format!("{base}/jobs/nope")).await.unwrap();
        assert_eq!(missing.status(), 404);
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
    async fn config_round_trips_over_http_and_offline_applies_live() {
        let (app, _tmp) = test_app().await;
        assert!(!app.offline());
        let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);

        let current: serde_json::Value = reqwest::get(format!("{base}/config"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(current["offline_mode"], false);
        assert_eq!(current["llama"]["gpu_layers"], 999);
        assert_eq!(current["comfyui"]["vram_mode"], "auto");

        let saved: serde_json::Value = reqwest::Client::new()
            .put(format!("{base}/config"))
            .json(&serde_json::json!({
                "store_path": "E:\\models\\here",
                "offline_mode": true,
                "vram_budget_mb": 12000,
                "llama": { "gpu_layers": 32, "ctx_size": 4096, "flash_attention": false, "load_timeout_secs": 120 },
                "comfyui": { "vram_mode": "lowvram" }
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(saved["vram_budget_mb"], 12000);
        assert_eq!(saved["llama"]["ctx_size"], 4096);
        assert_eq!(saved["comfyui"]["vram_mode"], "lowvram");

        // Offline flipped without a restart; the file kept the change.
        assert!(app.offline());
        let reread: serde_json::Value = reqwest::get(format!("{base}/config"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(reread["store_path"], "E:\\models\\here");
        assert_eq!(reread["offline_mode"], true);
        assert_eq!(reread["comfyui"]["vram_mode"], "lowvram");

        // A bad VRAM mode is rejected.
        let bad = reqwest::Client::new()
            .put(format!("{base}/config"))
            .json(&serde_json::json!({
                "store_path": "E:\\m", "offline_mode": false, "vram_budget_mb": 0,
                "llama": { "gpu_layers": 999, "ctx_size": 0, "flash_attention": true, "load_timeout_secs": 180 },
                "comfyui": { "vram_mode": "turbo" }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), 400);
    }

    #[tokio::test]
    async fn save_config_rejects_an_invalid_ctx_size_with_400() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();

        let resp = reqwest::Client::new()
            .put(format!("http://{}/config", server.addr))
            .json(&serde_json::json!({
                "store_path": "E:\\m",
                "offline_mode": false,
                "vram_budget_mb": 0,
                "llama": { "gpu_layers": 999, "ctx_size": 64, "flash_attention": true, "load_timeout_secs": 180 }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn known_models_lists_the_catalogue_over_http() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();

        let list: serde_json::Value = reqwest::get(format!("http://{}/models/known", server.addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let arr = list.as_array().unwrap();
        assert!(arr.len() >= 4);
        assert!(arr.iter().any(|m| m["id"] == "flux1-dev-q8"));
        assert!(arr
            .iter()
            .all(|m| m["sha256"].as_str().unwrap().len() == 64));
    }

    #[tokio::test]
    async fn job_output_serves_the_image_and_404s_otherwise() {
        use crate::db::{JobPatch, NewJob};
        use crate::orchestrator::JobState;

        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);

        // A job with no output → 404.
        let job = app.jobs.submit(NewJob::new("image")).await.unwrap();
        let none = reqwest::get(format!("{base}/jobs/{}/output", job.id))
            .await
            .unwrap();
        assert_eq!(none.status(), 404);
        assert_eq!(
            reqwest::get(format!("{base}/jobs/ghost/output"))
                .await
                .unwrap()
                .status(),
            404
        );

        // Walk it to Completed with a real PNG at output_path.
        let png: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        std::fs::create_dir_all(app.paths.outputs_dir()).unwrap();
        let out = app.paths.outputs_dir().join(format!("{}.png", job.id));
        std::fs::write(&out, png).unwrap();

        let r = app.db.jobs();
        for st in [
            JobState::Scheduled,
            JobState::Preparing,
            JobState::Running,
            JobState::Post,
        ] {
            r.set_state(&job.id, st, JobPatch::default()).await.unwrap();
        }
        r.set_state(
            &job.id,
            JobState::Completed,
            JobPatch {
                output_path: Some(out.to_string_lossy().into_owned()),
                set_finished_at: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let ok = reqwest::get(format!("{base}/jobs/{}/output", job.id))
            .await
            .unwrap();
        assert_eq!(ok.status(), 200);
        assert_eq!(ok.headers()["content-type"].to_str().unwrap(), "image/png");
        assert_eq!(ok.bytes().await.unwrap().as_ref(), png);

        // A video job's `.mp4` is served as `video/mp4`.
        let vid = app.jobs.submit(NewJob::new("video")).await.unwrap();
        let mp4 = app.paths.outputs_dir().join(format!("{}.mp4", vid.id));
        std::fs::write(&mp4, b"\0\0\0\x18ftypisom").unwrap();
        for st in [
            JobState::Scheduled,
            JobState::Preparing,
            JobState::Running,
            JobState::Post,
        ] {
            r.set_state(&vid.id, st, JobPatch::default()).await.unwrap();
        }
        r.set_state(
            &vid.id,
            JobState::Completed,
            JobPatch {
                output_path: Some(mp4.to_string_lossy().into_owned()),
                set_finished_at: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let v = reqwest::get(format!("{base}/jobs/{}/output", vid.id))
            .await
            .unwrap();
        assert_eq!(v.headers()["content-type"].to_str().unwrap(), "video/mp4");
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
            .submit(crate::db::NewJob::new("noop").on("llamacpp", "m", 1_000))
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
