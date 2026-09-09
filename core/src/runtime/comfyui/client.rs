//! Loopback HTTP client for a running ComfyUI server.
//!
//! Supervision (3.1): `GET /system_stats` (health + VRAM/version), `POST /free`
//! (drop resident models). Image jobs (3.4): `POST /prompt` (queue a workflow),
//! `GET /history/{id}` (poll for the result), `GET /view` (fetch the rendered
//! image), `POST /interrupt` (cancel). All requests go to `127.0.0.1` (ADR-008);
//! no TLS.

use std::net::Ipv4Addr;
use std::time::Duration;

use serde_json::Value;

use crate::runtime::Health;
use crate::{CoreError, Result};

/// A probe must answer within this or the server counts as down.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// `POST /prompt` only validates the graph and queues it — it should be quick.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Fetching one rendered image.
const VIEW_TIMEOUT: Duration = Duration::from_secs(30);

const BYTES_PER_MB: u64 = 1024 * 1024;

fn comfy_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: super::RUNTIME_ID.into(),
        message: msg.to_string(),
    }
}

/// The slice of `GET /system_stats` the adapter cares about — ComfyUI version
/// and the first CUDA device's VRAM (Diagnostics uses this in 3.7).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SystemStats {
    pub version: Option<String>,
    pub device: Option<String>,
    pub vram_total_mb: u64,
    pub vram_free_mb: u64,
}

/// One rendered image referenced by `GET /history` — the args `GET /view` wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ImageRef {
    pub filename: String,
    pub subfolder: String,
    /// `"output"` (a saved image) or `"temp"` (a preview).
    pub kind: String,
}

/// Where a queued prompt stands, from `GET /history/{prompt_id}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PromptOutcome {
    /// Not in the history yet, or still executing.
    Pending,
    /// Finished; these are the images it produced (`SaveImage` outputs first).
    Done(Vec<ImageRef>),
    /// ComfyUI reported an execution error.
    Failed(String),
}

#[derive(Debug, Clone)]
pub(super) struct ComfyClient {
    http: reqwest::Client,
    host: Ipv4Addr,
}

impl ComfyClient {
    pub(super) fn new() -> Self {
        let http = reqwest::Client::builder()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            host: Ipv4Addr::LOCALHOST,
        }
    }

    fn base(&self, port: u16) -> String {
        format!("http://{}:{port}", self.host)
    }

    /// `GET /system_stats` succeeds → [`Health::Healthy`]; a transport error or
    /// any non-2xx → [`Health::Unhealthy`]. ComfyUI has no "still starting"
    /// response — until it is up nothing listens — so the adapter tracks the
    /// starting phase itself.
    pub(super) async fn health(&self, port: u16) -> Health {
        match self.system_stats(port).await {
            Ok(_) => Health::Healthy,
            Err(_) => Health::Unhealthy,
        }
    }

    /// `GET /system_stats` — ComfyUI version, first CUDA device, its VRAM.
    pub(super) async fn system_stats(&self, port: u16) -> Result<SystemStats> {
        let resp = self
            .http
            .get(format!("{}/system_stats", self.base(port)))
            .timeout(PROBE_TIMEOUT)
            .send()
            .await
            .map_err(|e| comfy_err(format!("/system_stats request failed: {e}")))?
            .error_for_status()
            .map_err(|e| comfy_err(format!("/system_stats returned an error: {e}")))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| comfy_err(format!("bad /system_stats response: {e}")))?;

        let device = v.pointer("/devices/0");
        let device_mb = |key: &str| {
            device
                .and_then(|d| d.get(key))
                .and_then(Value::as_u64)
                .map(|b| b / BYTES_PER_MB)
                .unwrap_or(0)
        };
        Ok(SystemStats {
            version: v
                .pointer("/system/comfyui_version")
                .and_then(Value::as_str)
                .map(str::to_string),
            device: device
                .and_then(|d| d.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string),
            vram_total_mb: device_mb("vram_total"),
            vram_free_mb: device_mb("vram_free"),
        })
    }

    /// `POST /free` — unload resident models and free the torch cache. Best
    /// effort: ComfyUI answers `200` even when nothing was loaded.
    pub(super) async fn free(&self, port: u16) -> Result<()> {
        self.http
            .post(format!("{}/free", self.base(port)))
            .json(&serde_json::json!({ "unload_models": true, "free_memory": true }))
            .timeout(PROBE_TIMEOUT)
            .send()
            .await
            .map_err(|e| comfy_err(format!("/free request failed: {e}")))?
            .error_for_status()
            .map_err(|e| comfy_err(format!("/free returned an error: {e}")))?;
        Ok(())
    }

    /// `POST /interrupt` — stop the workflow ComfyUI is currently executing.
    /// The cancel hook for `capability::image` (3.4).
    pub(super) async fn interrupt(&self, port: u16) -> Result<()> {
        self.http
            .post(format!("{}/interrupt", self.base(port)))
            .timeout(PROBE_TIMEOUT)
            .send()
            .await
            .map_err(|e| comfy_err(format!("/interrupt request failed: {e}")))?
            .error_for_status()
            .map_err(|e| comfy_err(format!("/interrupt returned an error: {e}")))?;
        Ok(())
    }

    /// `POST /prompt` — queue an API-format workflow graph. Returns the
    /// `prompt_id` to poll [`history`](Self::history) with. A graph ComfyUI
    /// rejects (missing model, bad node) comes back as `node_errors` — surfaced
    /// as a plain-language error, not a silent failure.
    pub(super) async fn submit_prompt(
        &self,
        port: u16,
        workflow: &Value,
        client_id: &str,
    ) -> Result<String> {
        let resp = self
            .http
            .post(format!("{}/prompt", self.base(port)))
            .json(&serde_json::json!({ "prompt": workflow, "client_id": client_id }))
            .timeout(SUBMIT_TIMEOUT)
            .send()
            .await
            .map_err(|e| comfy_err(format!("/prompt request failed: {e}")))?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| comfy_err(format!("bad /prompt response: {e}")))?;

        if let Some(id) = body.get("prompt_id").and_then(Value::as_str) {
            return Ok(id.to_string());
        }
        Err(comfy_err(format!(
            "ComfyUI rejected the workflow ({status}): {}",
            prompt_error(&body)
        )))
    }

    /// `GET /history/{prompt_id}` — poll a queued prompt.
    pub(super) async fn history(&self, port: u16, prompt_id: &str) -> Result<PromptOutcome> {
        let body: Value = self
            .http
            .get(format!("{}/history/{prompt_id}", self.base(port)))
            .timeout(PROBE_TIMEOUT)
            .send()
            .await
            .map_err(|e| comfy_err(format!("/history request failed: {e}")))?
            .error_for_status()
            .map_err(|e| comfy_err(format!("/history returned an error: {e}")))?
            .json()
            .await
            .map_err(|e| comfy_err(format!("bad /history response: {e}")))?;

        let Some(entry) = body.get(prompt_id) else {
            return Ok(PromptOutcome::Pending);
        };

        let status_str = entry
            .pointer("/status/status_str")
            .and_then(Value::as_str)
            .unwrap_or("");
        if status_str == "error" {
            return Ok(PromptOutcome::Failed(history_error(entry)));
        }

        let images = collect_images(entry);
        let completed = entry
            .pointer("/status/completed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if completed || !images.is_empty() {
            return Ok(PromptOutcome::Done(images));
        }
        Ok(PromptOutcome::Pending)
    }

    /// `GET /view` — download one image the history referenced.
    pub(super) async fn view(&self, port: u16, image: &ImageRef) -> Result<Vec<u8>> {
        let url = reqwest::Url::parse_with_params(
            &format!("{}/view", self.base(port)),
            &[
                ("filename", image.filename.as_str()),
                ("subfolder", image.subfolder.as_str()),
                ("type", image.kind.as_str()),
            ],
        )
        .map_err(|e| comfy_err(format!("building the /view url failed: {e}")))?;
        let bytes = self
            .http
            .get(url)
            .timeout(VIEW_TIMEOUT)
            .send()
            .await
            .map_err(|e| comfy_err(format!("/view request failed: {e}")))?
            .error_for_status()
            .map_err(|e| comfy_err(format!("/view returned an error: {e}")))?
            .bytes()
            .await
            .map_err(|e| comfy_err(format!("reading the image failed: {e}")))?;
        Ok(bytes.to_vec())
    }
}

/// Best-effort human message from a `/prompt` rejection body.
fn prompt_error(body: &Value) -> String {
    if let Some(errs) = body.get("node_errors").filter(|v| !is_empty_object(v)) {
        return errs.to_string();
    }
    body.pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| body.get("error").and_then(Value::as_str))
        .unwrap_or("no prompt_id and no error detail in the response")
        .to_string()
}

/// The execution error a `/history` entry carries (`status.messages` holds
/// `["execution_error", { … }]` pairs).
fn history_error(entry: &Value) -> String {
    let from_messages = entry
        .pointer("/status/messages")
        .and_then(Value::as_array)
        .and_then(|msgs| {
            msgs.iter().find_map(|m| {
                let pair = m.as_array()?;
                if pair.first().and_then(Value::as_str) == Some("execution_error") {
                    pair.get(1)
                        .and_then(|d| d.get("exception_message"))
                        .and_then(Value::as_str)
                } else {
                    None
                }
            })
        });
    from_messages
        .unwrap_or("ComfyUI reported an execution error")
        .to_string()
}

/// Pull every image the output nodes produced. `SaveImage` outputs (`type =
/// "output"`) come before previews.
fn collect_images(entry: &Value) -> Vec<ImageRef> {
    let Some(outputs) = entry.get("outputs").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut refs: Vec<ImageRef> = outputs
        .values()
        .filter_map(|node| node.get("images").and_then(Value::as_array))
        .flatten()
        .filter_map(|img| {
            Some(ImageRef {
                filename: img.get("filename").and_then(Value::as_str)?.to_string(),
                subfolder: img
                    .get("subfolder")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                kind: img
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("output")
                    .to_string(),
            })
        })
        .collect();
    refs.sort_by_key(|r| r.kind != "output");
    refs
}

fn is_empty_object(v: &Value) -> bool {
    v.as_object().is_some_and(serde_json::Map::is_empty)
}

#[cfg(test)]
mod tests {
    use axum::routing::{get, post};
    use axum::{Json, Router};

    use super::*;

    async fn serve(router: Router) -> u16 {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        port
    }

    fn stats_body() -> Value {
        serde_json::json!({
            "system": { "comfyui_version": "0.34.0", "python_version": "3.12.4" },
            "devices": [{
                "name": "cuda:0 NVIDIA GeForce RTX 4080 SUPER",
                "type": "cuda",
                "vram_total": 17_170_956_288u64,
                "vram_free": 15_600_000_000u64
            }]
        })
    }

    #[tokio::test]
    async fn system_stats_parses_version_and_vram() {
        let port =
            serve(Router::new().route("/system_stats", get(|| async { Json(stats_body()) }))).await;

        let stats = ComfyClient::new().system_stats(port).await.unwrap();
        assert_eq!(stats.version.as_deref(), Some("0.34.0"));
        assert!(stats.device.unwrap().contains("4080"));
        assert_eq!(stats.vram_total_mb, 17_170_956_288 / (1024 * 1024));
        assert!(stats.vram_free_mb > 14_000);
    }

    #[tokio::test]
    async fn health_reflects_system_stats() {
        let ok =
            serve(Router::new().route("/system_stats", get(|| async { Json(stats_body()) }))).await;
        assert_eq!(ComfyClient::new().health(ok).await, Health::Healthy);

        let bad = serve(Router::new().route(
            "/system_stats",
            get(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
        ))
        .await;
        assert_eq!(ComfyClient::new().health(bad).await, Health::Unhealthy);
    }

    #[tokio::test]
    async fn health_is_unhealthy_when_nothing_listens() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let dead = listener.local_addr().unwrap().port();
        drop(listener);
        assert_eq!(ComfyClient::new().health(dead).await, Health::Unhealthy);
    }

    #[tokio::test]
    async fn free_and_interrupt_post_and_accept_200() {
        let port = serve(
            Router::new()
                .route("/free", post(|| async { axum::http::StatusCode::OK }))
                .route("/interrupt", post(|| async { axum::http::StatusCode::OK })),
        )
        .await;
        let c = ComfyClient::new();
        c.free(port).await.unwrap();
        c.interrupt(port).await.unwrap();
    }

    #[tokio::test]
    async fn submit_prompt_returns_the_prompt_id() {
        let port = serve(Router::new().route(
            "/prompt",
            post(|| async {
                Json(serde_json::json!({ "prompt_id": "p-123", "number": 1, "node_errors": {} }))
            }),
        ))
        .await;

        let id = ComfyClient::new()
            .submit_prompt(port, &serde_json::json!({ "4": {} }), "c-1")
            .await
            .unwrap();
        assert_eq!(id, "p-123");
    }

    #[tokio::test]
    async fn submit_prompt_surfaces_node_errors() {
        let port = serve(Router::new().route(
            "/prompt",
            post(|| async {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": { "message": "invalid prompt" },
                        "node_errors": { "4": { "errors": [{ "message": "checkpoint not found" }] } }
                    })),
                )
            }),
        ))
        .await;

        let err = ComfyClient::new()
            .submit_prompt(port, &serde_json::json!({}), "c-1")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("checkpoint not found"), "{err}");
    }

    #[tokio::test]
    async fn history_reports_pending_then_done() {
        let pending = serve(Router::new().route(
            "/history/{id}",
            get(|| async { Json(serde_json::json!({})) }),
        ))
        .await;
        assert_eq!(
            ComfyClient::new().history(pending, "p-1").await.unwrap(),
            PromptOutcome::Pending
        );

        let done = serve(Router::new().route(
            "/history/{id}",
            get(|| async {
                Json(serde_json::json!({
                    "p-1": {
                        "status": { "status_str": "success", "completed": true },
                        "outputs": { "9": { "images": [
                            { "filename": "job-abc_00001_.png", "subfolder": "", "type": "output" }
                        ]}}
                    }
                }))
            }),
        ))
        .await;
        match ComfyClient::new().history(done, "p-1").await.unwrap() {
            PromptOutcome::Done(images) => {
                assert_eq!(images.len(), 1);
                assert_eq!(images[0].filename, "job-abc_00001_.png");
                assert_eq!(images[0].kind, "output");
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn history_reports_an_execution_error() {
        let port = serve(Router::new().route(
            "/history/{id}",
            get(|| async {
                Json(serde_json::json!({
                    "p-1": { "status": { "status_str": "error", "completed": false, "messages": [
                        ["execution_start", {}],
                        ["execution_error", { "exception_message": "CUDA out of memory" }]
                    ]}}
                }))
            }),
        ))
        .await;
        match ComfyClient::new().history(port, "p-1").await.unwrap() {
            PromptOutcome::Failed(msg) => assert!(msg.contains("out of memory"), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn view_downloads_the_image_bytes() {
        let port = serve(Router::new().route(
            "/view",
            get(|| async { [0x89u8, 0x50, 0x4e, 0x47].to_vec() }),
        ))
        .await;
        let bytes = ComfyClient::new()
            .view(
                port,
                &ImageRef {
                    filename: "x.png".into(),
                    subfolder: String::new(),
                    kind: "output".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(bytes, [0x89, 0x50, 0x4e, 0x47]);
    }
}
