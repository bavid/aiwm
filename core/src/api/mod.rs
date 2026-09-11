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

/// The core's long-running background work: the loopback API, the job loop and
/// the download worker.
#[derive(Debug)]
pub struct Services {
    api: ApiServer,
    job_loop: JoinHandle<()>,
    download_worker: JoinHandle<()>,
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
        self.download_worker.abort();
    }
}

/// Start the loopback API (on `127.0.0.1:core_api_port`), the job loop and the
/// download worker.
pub async fn spawn(app: Arc<App>) -> Result<Services> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, app.config.core_api_port));
    spawn_on(app, addr).await
}

/// Like [`spawn`] but with an explicit bind address (tests use `127.0.0.1:0`).
pub async fn spawn_on(app: Arc<App>, api_addr: SocketAddr) -> Result<Services> {
    let api = ApiServer::bind(app.clone(), api_addr).await?;
    tracing::info!(addr = %api.addr, "core API listening (loopback only)");

    let download_worker = tokio::spawn(app.downloads.clone().run());
    let job_loop = tokio::spawn(run_job_loop(app));
    Ok(Services {
        api,
        job_loop,
        download_worker,
    })
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
        assert_eq!(current["models"]["auto_preference"], "balanced");

        let saved: serde_json::Value = reqwest::Client::new()
            .put(format!("{base}/config"))
            .json(&serde_json::json!({
                "store_path": "E:\\models\\here",
                "offline_mode": true,
                "vram_budget_mb": 12000,
                "llama": { "gpu_layers": 32, "ctx_size": 4096, "flash_attention": false, "load_timeout_secs": 120 },
                "comfyui": { "vram_mode": "lowvram" },
                "models": { "auto_preference": "quality" }
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
        assert_eq!(saved["models"]["auto_preference"], "quality");

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
        assert_eq!(reread["models"]["auto_preference"], "quality");

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
    async fn paths_override_round_trips_over_http_and_blank_clears_it() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);
        let http = reqwest::Client::new();

        let base_update = serde_json::json!({
            "store_path": "E:\\m", "offline_mode": false, "vram_budget_mb": 0,
            "llama": { "gpu_layers": 999, "ctx_size": 0, "flash_attention": true, "load_timeout_secs": 180 },
        });

        // No `paths` key at all -- defaults to no overrides.
        let saved: serde_json::Value = http
            .put(format!("{base}/config"))
            .json(&base_update)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(saved["paths"]["outputs_path"], serde_json::Value::Null);

        // Set an override, with padding whitespace the handler should trim.
        let mut with_paths = base_update.clone();
        with_paths["paths"] = serde_json::json!({
            "outputs_path": "  E:\\media\\outputs  ",
            "runtimes_path": "",
            "cache_path": "E:\\fast\\cache",
        });
        let saved: serde_json::Value = http
            .put(format!("{base}/config"))
            .json(&with_paths)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(saved["paths"]["outputs_path"], "E:\\media\\outputs");
        assert_eq!(saved["paths"]["runtimes_path"], serde_json::Value::Null);
        assert_eq!(saved["paths"]["cache_path"], "E:\\fast\\cache");

        // A re-read from disk agrees (not just the in-memory response).
        let reread: serde_json::Value = reqwest::get(format!("{base}/config"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(reread["paths"]["outputs_path"], "E:\\media\\outputs");

        // Blanking it out again clears the override.
        let mut cleared = base_update;
        cleared["paths"] = serde_json::json!({
            "outputs_path": "", "runtimes_path": "", "cache_path": "",
        });
        let saved: serde_json::Value = http
            .put(format!("{base}/config"))
            .json(&cleared)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(saved["paths"]["outputs_path"], serde_json::Value::Null);
        assert_eq!(saved["paths"]["cache_path"], serde_json::Value::Null);
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
        // Every entry now carries a computed fit verdict + the curated pick flag.
        // (Not asserting a specific level -- it depends on the test machine's
        // real detected VRAM budget, unlike `test_app()`'s other fixed fields.)
        assert!(arr.iter().all(|m| m["fit"]["level"].is_string()));
        let sdxl = arr.iter().find(|m| m["id"] == "sdxl-base-1.0").unwrap();
        assert_eq!(sdxl["is_default"], true);
    }

    #[tokio::test]
    async fn model_stacks_bundles_every_companion_file_with_a_base_model() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();

        let list: serde_json::Value = reqwest::get(format!("http://{}/models/stacks", server.addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let arr = list.as_array().unwrap();
        assert!(arr.len() >= 4);

        let flux = arr.iter().find(|s| s["id"] == "flux").unwrap();
        let members = flux["members"].as_array().unwrap();
        assert_eq!(members.len(), 4, "flux stack: model + T5 + CLIP-L + VAE");
        assert!(members.iter().any(|m| m["id"] == "flux1-dev-q8"));
        assert!(members.iter().any(|m| m["id"] == "flux-vae"));
        // Every member carries the same enrichment as GET /models/known.
        assert!(members.iter().all(|m| m["fit"]["level"].is_string()));

        let sdxl = arr.iter().find(|s| s["id"] == "sdxl").unwrap();
        assert_eq!(sdxl["is_default"], true);
        assert_eq!(sdxl["members"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn featured_models_lists_curated_chat_and_coding_picks_with_fit() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();

        let list: serde_json::Value =
            reqwest::get(format!("http://{}/models/featured", server.addr))
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
        let arr = list.as_array().unwrap();
        assert!(arr
            .iter()
            .any(|m| m["role"] == "chat" && m["is_default"] == true));
        assert!(arr
            .iter()
            .any(|m| m["role"] == "coding" && m["is_default"] == true));
        assert!(arr
            .iter()
            .all(|m| m["repo"].as_str().unwrap().contains('/')));
        assert!(arr.iter().all(|m| m["fit"]["level"].is_string()));
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

    // --- agents (Phase 5.1c) ---

    #[tokio::test]
    async fn agent_profiles_crud_over_http() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);
        let http = reqwest::Client::new();

        let created: serde_json::Value = http
            .post(format!("{base}/agents"))
            .json(&serde_json::json!({
                "name": "  Repo coder  ",
                "adapter": "opencode",
                "workspace_path": "E:\\proj",
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(created["name"], "Repo coder"); // trimmed
        assert_eq!(created["adapter"], "opencode");
        assert!(created["model_id"].is_null());
        let id = created["id"].as_str().unwrap().to_string();

        let list: serde_json::Value = reqwest::get(format!("{base}/agents"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);

        let del = http
            .delete(format!("{base}/agents/{id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(del.status(), 204);
        let list: serde_json::Value = reqwest::get(format!("{base}/agents"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(list.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_agent_rejects_a_blank_name_and_an_unknown_adapter() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);
        let http = reqwest::Client::new();

        let blank = http
            .post(format!("{base}/agents"))
            .json(&serde_json::json!({ "name": "  ", "adapter": "opencode", "workspace_path": "E:\\p" }))
            .send()
            .await
            .unwrap();
        assert_eq!(blank.status(), 400);

        let bad_adapter = http
            .post(format!("{base}/agents"))
            .json(
                &serde_json::json!({ "name": "x", "adapter": "cursor", "workspace_path": "E:\\p" }),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(bad_adapter.status(), 400);
    }

    #[tokio::test]
    async fn opening_a_session_without_a_coding_model_is_a_400() {
        let (app, _tmp) = test_app().await;
        let agent = app
            .db
            .agents()
            .create(crate::db::NewAgent {
                name: "Coder".into(),
                adapter: "opencode".into(),
                model_id: None,
                workspace_path: "E:\\proj".into(),
                allowed_paths: vec![],
                toolset: None,
            })
            .await
            .unwrap();
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);

        let resp = reqwest::Client::new()
            .post(format!("{base}/agent-sessions"))
            .json(&serde_json::json!({ "agent_id": agent.id }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["error"].as_str().unwrap().contains("coding"), "{body}");
    }

    #[tokio::test]
    async fn agent_session_detail_404s_and_stop_is_idempotent() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);

        let missing = reqwest::get(format!("{base}/agent-sessions/nope"))
            .await
            .unwrap();
        assert_eq!(missing.status(), 404);

        let stop = reqwest::Client::new()
            .post(format!("{base}/agent-sessions/nope/stop"))
            .send()
            .await
            .unwrap();
        assert_eq!(stop.status(), 204);
    }

    #[tokio::test]
    async fn agent_runtimes_lists_opencode_and_hermes() {
        let (app, _tmp) = test_app().await;
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let list: serde_json::Value =
            reqwest::get(format!("http://{}/agent-runtimes", server.addr))
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
        let arr = list.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let hermes = arr.iter().find(|r| r["id"] == "hermes").unwrap();
        assert_eq!(hermes["installed"], false); // nothing installed in a test app
        assert_eq!(hermes["install"]["state"], "idle");
        // opencode has no AIWM installer → no `install` field.
        let opencode = arr.iter().find(|r| r["id"] == "opencode").unwrap();
        assert!(opencode.get("install").is_none());
    }

    #[tokio::test]
    async fn export_then_import_round_trips_over_http() {
        let (app, tmp) = test_app().await;
        let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);

        let resp = reqwest::get(format!("{base}/export")).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["content-type"], "application/zip");
        let zip = resp.bytes().await.unwrap();
        assert_eq!(&zip[..2], b"PK", "a zip archive");

        let summary: serde_json::Value = reqwest::Client::new()
            .post(format!("{base}/import"))
            .body(zip.clone())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(summary["restart_required"], true);
        assert_eq!(summary["model_count"], 0);
        assert!(summary["missing_models"].as_array().unwrap().is_empty());
        // The archive is staged, the live db untouched until the next startup.
        assert!(tmp.path().join(".pending-import").join("aiwm.db").is_file());

        let junk = reqwest::Client::new()
            .post(format!("{base}/import"))
            .body(b"not a zip".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(junk.status(), 400);
    }

    #[tokio::test]
    async fn benchmark_endpoints_over_http() {
        use crate::db::NewModel;

        let (app, _tmp) = test_app().await;
        let gguf = app
            .db
            .models()
            .insert(NewModel {
                name: "Qwen2.5 7B".into(),
                format: "gguf".into(),
                file_path: "E:\\AI\\models\\llm\\qwen\\qwen.gguf".into(),
                size_bytes: 4_500 * 1024 * 1024,
                ctx_max: Some(32_768),
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap();
        let sdxl = app
            .db
            .models()
            .insert(NewModel {
                name: "SDXL".into(),
                format: "safetensors".into(),
                file_path: "E:\\AI\\models\\image\\sdxl.safetensors".into(),
                size_bytes: 6_500 * 1024 * 1024,
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap();

        let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);
        let http = reqwest::Client::new();

        // Nothing measured yet.
        let empty: serde_json::Value = reqwest::get(format!("{base}/benchmarks"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(empty.as_array().unwrap().is_empty());

        // Queue a "Test model" job for the GGUF.
        let created = http
            .post(format!("{base}/models/{}/benchmark", gguf.id))
            .send()
            .await
            .unwrap();
        assert_eq!(created.status(), 201);
        let job: serde_json::Value = created.json().await.unwrap();
        assert_eq!(job["job_type"], "bench");
        assert_eq!(job["state"], "queued");
        assert_eq!(job["model_id"], gguf.id);

        // A non-GGUF model is refused with 400.
        let bad = http
            .post(format!("{base}/models/{}/benchmark", sdxl.id))
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), 400);

        // An unknown model is refused too.
        let ghost = http
            .post(format!("{base}/models/ghost/benchmark"))
            .send()
            .await
            .unwrap();
        assert_eq!(ghost.status(), 400);

        // Per-model history endpoint is empty until a run finishes.
        let hist: serde_json::Value = reqwest::get(format!("{base}/models/{}/benchmarks", gguf.id))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(hist.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn storage_report_and_delete_model_over_http() {
        use crate::db::NewModel;

        let (app, tmp) = test_app().await;
        let store = tmp.path().join("models");
        let mk = |name: &str, hash: Option<&str>, size: i64, sub: &str| {
            let dir = store.join(sub).join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join(format!("{name}.gguf"));
            std::fs::write(&file, vec![0u8; size.max(0) as usize]).unwrap();
            NewModel {
                name: name.into(),
                format: "gguf".into(),
                file_path: file.to_string_lossy().into_owned(),
                sha256: hash.map(str::to_string),
                size_bytes: size,
                source: "manual".into(),
                roles: vec!["chat".into()],
                ..NewModel::default()
            }
        };
        // Point the config's store at our temp store so kind_of resolves.
        {
            let mut cfg = crate::config::Config::read_from(&app.paths).unwrap();
            cfg.store_path = store.clone();
            cfg.save(&app.paths).unwrap();
        }
        let a = app
            .db
            .models()
            .insert(mk("dup-a", Some("dead"), 2_000, "llm"))
            .await
            .unwrap();
        let b = app
            .db
            .models()
            .insert(mk("dup-b", Some("dead"), 2_000, "llm"))
            .await
            .unwrap();

        // Re-open so the handler sees the updated store path.
        let app = Arc::new(App::load(app.paths.clone()).await.unwrap());
        let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);
        let http = reqwest::Client::new();

        let report: serde_json::Value = reqwest::get(format!("{base}/storage"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(report["store_bytes"], 4_000);
        assert_eq!(report["duplicates"].as_array().unwrap().len(), 1);
        assert_eq!(report["duplicates"][0]["wasted_bytes"], 2_000);
        assert_eq!(report["unused"].as_array().unwrap().len(), 2);

        // Delete the redundant copy.
        let del = http
            .delete(format!("{base}/models/{}", b.id))
            .send()
            .await
            .unwrap();
        assert_eq!(del.status(), 200);
        let out: serde_json::Value = del.json().await.unwrap();
        assert_eq!(out["file_removed"], true);
        assert_eq!(out["freed_bytes"], 2_000);
        assert!(app.db.models().get(&b.id).await.unwrap().is_none());
        assert!(app.db.models().get(&a.id).await.unwrap().is_some());

        // Deleting an unknown model → 400.
        let ghost = http
            .delete(format!("{base}/models/ghost"))
            .send()
            .await
            .unwrap();
        assert_eq!(ghost.status(), 400);
    }

    #[tokio::test]
    async fn tags_registry_status_and_hf_token_over_http() {
        use crate::db::NewModel;

        let (app, tmp) = test_app().await;
        let model = app
            .db
            .models()
            .insert(NewModel {
                name: "Qwen".into(),
                format: "gguf".into(),
                file_path: "E:\\AI\\models\\llm\\qwen\\q.gguf".into(),
                size_bytes: 1,
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap();
        let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);
        let http = reqwest::Client::new();

        // Set + read tags.
        let set: serde_json::Value = http
            .put(format!("{base}/models/{}/tags", model.id))
            .json(&serde_json::json!({ "tags": [" Favourite ", "coding", "coding"] }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(set, serde_json::json!(["coding", "favourite"]));

        let all: serde_json::Value = reqwest::get(format!("{base}/models/tags"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(all[&model.id], serde_json::json!(["coding", "favourite"]));

        let ghost = http
            .put(format!("{base}/models/ghost/tags"))
            .json(&serde_json::json!({ "tags": ["x"] }))
            .send()
            .await
            .unwrap();
        assert_eq!(ghost.status(), 400);

        // Registry status — no token, no fetch yet.
        let status: serde_json::Value = reqwest::get(format!("{base}/registry/status"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(status["source_id"], "huggingface");
        assert_eq!(status["token_set"], false);

        // Set a token → it lands in the machine-local file (not the backup).
        let resp = http
            .put(format!("{base}/registry/token"))
            .json(&serde_json::json!({ "token": "hf_secret123" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("hf_token.txt")).unwrap(),
            "hf_secret123"
        );

        // Clearing removes it.
        http.put(format!("{base}/registry/token"))
            .json(&serde_json::json!({ "token": "  " }))
            .send()
            .await
            .unwrap();
        assert!(!tmp.path().join("hf_token.txt").exists());
    }

    #[tokio::test]
    async fn set_model_roles_over_http_replaces_the_set_and_rejects_a_ghost() {
        use crate::db::NewModel;

        let (app, _tmp) = test_app().await;
        let model = app
            .db
            .models()
            .insert(NewModel {
                name: "Qwen".into(),
                format: "gguf".into(),
                file_path: "E:\\AI\\models\\llm\\qwen\\q.gguf".into(),
                size_bytes: 1,
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap();
        let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);
        let http = reqwest::Client::new();

        let set: serde_json::Value = http
            .put(format!("{base}/models/{}/roles", model.id))
            .json(&serde_json::json!({ "roles": ["coding", " chat ", "chat"] }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(set, serde_json::json!(["chat", "coding"]));

        // GET /models shows it embedded directly (unlike tags -- no separate map).
        let models: serde_json::Value = reqwest::get(format!("{base}/models"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let m = models
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == model.id)
            .unwrap();
        assert_eq!(m["roles"], serde_json::json!(["chat", "coding"]));

        // Re-setting replaces, doesn't accumulate.
        let replaced: serde_json::Value = http
            .put(format!("{base}/models/{}/roles", model.id))
            .json(&serde_json::json!({ "roles": ["reasoning"] }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(replaced, serde_json::json!(["reasoning"]));

        let ghost = http
            .put(format!("{base}/models/ghost/roles"))
            .json(&serde_json::json!({ "roles": ["chat"] }))
            .send()
            .await
            .unwrap();
        assert_eq!(ghost.status(), 400);
    }

    #[tokio::test]
    async fn upgrade_check_queues_a_job_and_refuses_offline() {
        use crate::db::NewModel;

        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::AppPaths::rooted(tmp.path());
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(paths.config_file(), "offline_mode = true\n").unwrap();
        let app = Arc::new(App::load(paths).await.unwrap());
        let model = app
            .db
            .models()
            .insert(NewModel {
                name: "Qwen2.5 7B".into(),
                format: "gguf".into(),
                family: Some("qwen2".into()),
                file_path: "E:\\AI\\models\\llm\\qwen\\qwen.gguf".into(),
                size_bytes: 4_500 * 1024 * 1024,
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap();
        let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let base = format!("http://{}", server.addr);
        let http = reqwest::Client::new();

        // Offline → 400.
        let offline = http
            .post(format!("{base}/models/{}/upgrade-check", model.id))
            .send()
            .await
            .unwrap();
        assert_eq!(offline.status(), 400);

        // Back online → 201 + a queued upgrade_check job carrying the target id.
        app.set_offline(false);
        let created = http
            .post(format!("{base}/models/{}/upgrade-check", model.id))
            .send()
            .await
            .unwrap();
        assert_eq!(created.status(), 201);
        let job: serde_json::Value = created.json().await.unwrap();
        assert_eq!(job["job_type"], "upgrade_check");
        assert_eq!(job["state"], "queued");
        assert_eq!(job["params"]["target_model_id"], model.id);

        // Unknown model → 400.
        let ghost = http
            .post(format!("{base}/models/ghost/upgrade-check"))
            .send()
            .await
            .unwrap();
        assert_eq!(ghost.status(), 400);
    }

    #[tokio::test]
    async fn install_hermes_refuses_in_offline_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::AppPaths::rooted(tmp.path());
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(paths.config_file(), "offline_mode = true\n").unwrap();
        let app = Arc::new(App::load(paths).await.unwrap());

        let err = handlers::install_hermes(&app).unwrap_err();
        assert!(err.to_string().contains("offline mode"));
    }
}
