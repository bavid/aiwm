//! Test fixture: a minimal ComfyUI stand-in.
//!
//! Speaks just enough of the real HTTP API for the runtime + image-capability
//! tests: `GET /system_stats`, `POST /free`, `POST /interrupt`, `POST /prompt`,
//! `GET /history/{id}`, `GET /view`. It parses `--port` the way the real server
//! does and ignores every other real flag. Not part of the shipped product —
//! it exists so the spawn / health / render / cancel cycle can be exercised
//! without a multi-GB Python install.
//!
//! A `SaveImage` node → a 1×1 PNG under the `images` key; a `SaveVideo` node → a
//! tiny MP4 blob under the `videos` key.
//!
//! Fixture-only flags:
//! - `--fake-ready-ms <n>`  — delay the socket bind by `n` ms (slow cold start).
//! - `--fake-render-ms <n>` — `/history` reports "pending" until `n` ms after
//!   the prompt was submitted, then "done" (exercises the poll loop / cancel).
//! - `--fake-history-error` — `/history` reports an execution error instead.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

/// A 1×1 transparent PNG — what `/view` hands back for an image.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

/// A minimal MP4: a 24-byte `ftyp` box + a stub `mdat` box. Not playable — just
/// enough that a caller can recognize it (`bytes[4..8] == b"ftyp"`).
const TINY_MP4: &[u8] = &[
    0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, 0x69, 0x73, 0x6f, 0x6d, 0x00, 0x00, 0x02, 0x00,
    0x69, 0x73, 0x6f, 0x6d, 0x69, 0x73, 0x6f, 0x32, 0x00, 0x00, 0x00, 0x08, 0x6d, 0x64, 0x61, 0x74,
];

#[derive(Clone)]
struct Fixture {
    render_delay: Duration,
    history_error: bool,
    prompts: Arc<Mutex<HashMap<String, Prompt>>>,
}

struct Prompt {
    submitted_at: Instant,
    filename_prefix: String,
    is_video: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = 8188u16;
    let mut ready_ms = 0u64;
    let mut render_ms = 0u64;
    let mut history_error = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                if let Some(v) = args.next() {
                    port = v.parse().unwrap_or(port);
                }
            }
            "--fake-ready-ms" => {
                ready_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            "--fake-render-ms" => {
                render_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            "--fake-history-error" => history_error = true,
            _ => {}
        }
    }

    if ready_ms > 0 {
        tokio::time::sleep(Duration::from_millis(ready_ms)).await;
    }

    let state = Fixture {
        render_delay: Duration::from_millis(render_ms),
        history_error,
        prompts: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/system_stats", get(system_stats))
        .route("/free", post(ok))
        .route("/interrupt", post(ok))
        .route("/prompt", post(submit_prompt))
        .route("/history/{id}", get(history))
        .route("/view", get(view))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("fake-comfy: listening on 127.0.0.1:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn system_stats() -> Json<Value> {
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

async fn submit_prompt(State(fx): State<Fixture>, Json(body): Json<Value>) -> Json<Value> {
    // Find the SaveImage / SaveVideo node and its filename_prefix.
    let mut prefix = "fake".to_string();
    let mut is_video = false;
    if let Some(nodes) = body.get("prompt").and_then(Value::as_object) {
        for node in nodes.values() {
            match node.get("class_type").and_then(Value::as_str) {
                Some("SaveVideo") => is_video = true,
                Some("SaveImage") => {}
                _ => continue,
            }
            if let Some(p) = node
                .pointer("/inputs/filename_prefix")
                .and_then(Value::as_str)
            {
                prefix = p.to_string();
            }
        }
    }
    let id = format!("p-{}", fx.prompts.lock().map(|m| m.len()).unwrap_or(0) + 1);
    if let Ok(mut prompts) = fx.prompts.lock() {
        prompts.insert(
            id.clone(),
            Prompt {
                submitted_at: Instant::now(),
                filename_prefix: prefix,
                is_video,
            },
        );
    }
    Json(json!({ "prompt_id": id, "number": 1, "node_errors": {} }))
}

async fn history(State(fx): State<Fixture>, Path(id): Path<String>) -> Json<Value> {
    let Ok(prompts) = fx.prompts.lock() else {
        return Json(json!({}));
    };
    let Some(prompt) = prompts.get(&id) else {
        return Json(json!({}));
    };
    if prompt.submitted_at.elapsed() < fx.render_delay {
        return Json(json!({})); // still rendering
    }
    let entry = if fx.history_error {
        json!({
            "status": {
                "status_str": "error",
                "completed": false,
                "messages": [
                    ["execution_start", {}],
                    ["execution_error", { "exception_message": "fake render error" }]
                ]
            }
        })
    } else if prompt.is_video {
        json!({
            "status": { "status_str": "success", "completed": true, "messages": [] },
            "outputs": {
                "59": { "videos": [
                    { "filename": format!("{}.mp4", prompt.filename_prefix),
                      "subfolder": "", "type": "output" }
                ]}
            }
        })
    } else {
        json!({
            "status": { "status_str": "success", "completed": true, "messages": [] },
            "outputs": {
                "9": { "images": [
                    { "filename": format!("{}_00001_.png", prompt.filename_prefix),
                      "subfolder": "", "type": "output" }
                ]}
            }
        })
    };
    let mut out = serde_json::Map::new();
    out.insert(id, entry);
    Json(Value::Object(out))
}

async fn view(Query(q): Query<HashMap<String, String>>) -> impl IntoResponse {
    let is_mp4 = q
        .get("filename")
        .is_some_and(|f| f.to_ascii_lowercase().ends_with(".mp4"));
    if is_mp4 {
        ([(axum::http::header::CONTENT_TYPE, "video/mp4")], TINY_MP4)
    } else {
        ([(axum::http::header::CONTENT_TYPE, "image/png")], TINY_PNG)
    }
}
