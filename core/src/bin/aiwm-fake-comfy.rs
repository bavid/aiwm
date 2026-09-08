//! Test fixture: a minimal ComfyUI stand-in.
//!
//! Speaks just enough of the real HTTP API for the runtime adapter tests —
//! `GET /system_stats`, `POST /free`, `POST /interrupt` — and parses `--port`
//! from the command line the way the real server does, ignoring every other
//! flag. Queueing prompts (`/prompt`, `/history`, `/view`) is added when
//! `capability::image` lands (3.4). Not part of the shipped product; it exists
//! so the spawn / health / serve / stop cycle can be exercised without a
//! multi-GB Python install.
//!
//! `--fake-ready-ms <n>` delays the socket bind by `n` milliseconds, simulating
//! a slow Python + torch cold start (nothing listens until then).

use std::net::Ipv4Addr;
use std::time::Duration;

use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = 8188u16;
    let mut ready_ms = 0u64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        // The real server takes many flags; the fixture needs only these two.
        match arg.as_str() {
            "--port" => {
                if let Some(v) = args.next() {
                    port = v.parse().unwrap_or(port);
                }
            }
            "--fake-ready-ms" => {
                ready_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            _ => {}
        }
    }

    if ready_ms > 0 {
        tokio::time::sleep(Duration::from_millis(ready_ms)).await;
    }

    let app = Router::new()
        .route("/system_stats", get(system_stats))
        .route("/free", post(ok))
        .route("/interrupt", post(ok));

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("fake-comfy: listening on 127.0.0.1:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn system_stats() -> Json<serde_json::Value> {
    Json(json!({
        "system": {
            "os": "nt",
            "comfyui_version": "0.34.0-fake",
            "python_version": "3.12.4"
        },
        "devices": [{
            "name": "cuda:0 fake-comfy device",
            "type": "cuda",
            "index": 0,
            "vram_total": 17_170_956_288u64,
            "vram_free": 15_600_000_000u64
        }]
    }))
}

async fn ok() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}
