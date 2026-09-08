//! Loopback HTTP client for a running ComfyUI server.
//!
//! Phase 3.1 uses only what the adapter needs to supervise the process:
//! `GET /system_stats` (health + VRAM/version), `POST /free` (drop resident
//! models), `POST /interrupt` (stop the running workflow — the cancel hook for
//! 3.4). Queueing prompts and fetching images (`/prompt`, `/history`, `/view`)
//! arrive with `capability::image` in 3.4. All requests go to `127.0.0.1`
//! (ADR-008); no TLS.

use std::net::Ipv4Addr;
use std::time::Duration;

use serde_json::Value;

use crate::runtime::Health;
use crate::{CoreError, Result};

/// A probe must answer within this or the server counts as down.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

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
    #[allow(dead_code)]
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
}
