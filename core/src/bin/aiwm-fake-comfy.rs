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
//! tiny MP4 blob under the `videos` key. A `LoadImage` node makes `/history`
//! check that the referenced file exists under `<base-directory>/input/` —
//! image→video staging (4.2) is only "done" if the start frame really landed.
//!
//! `GET /__test/last_lora_chain` (fixture-only, no real-ComfyUI equivalent)
//! hands back the most recently submitted graph's `LoraLoader` chain — lets an
//! integration test prove a job's `params.loras` actually reached ComfyUI as
//! real nodes (file + strength, in chain order), not just that the job
//! completed. `GET /__test/last_graph_node_types` does the same for the
//! graph's shape: every node's `class_type`, in node-id order.
//!
//! Fixture-only flags:
//! - `--fake-ready-ms <n>`  — delay the socket bind by `n` ms (slow cold start).
//! - `--fake-render-ms <n>` — `/history` reports "pending" until `n` ms after
//!   the prompt was submitted, then "done" (exercises the poll loop / cancel).
//! - `--fake-history-error` — `/history` reports an execution error instead.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
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
    input_dir: PathBuf,
    prompts: Arc<Mutex<HashMap<String, Prompt>>>,
    /// The most recently submitted graph's `LoraLoader` chain, in node-id
    /// order (`90`, `91`, …) — read back by `GET /__test/last_lora_chain` so
    /// integration tests can prove a job's `params.loras` actually reached
    /// the ComfyUI graph as real `LoraLoader` nodes, not just that the job
    /// completed. Test-only surface; the real ComfyUI has no such endpoint.
    last_lora_chain: Arc<Mutex<Vec<LoraLink>>>,
    /// Every `class_type` of the most recently submitted graph, in node-id
    /// order — read back by `GET /__test/last_graph_node_types`. Same idea as
    /// [`Fixture::last_lora_chain`] but for the graph's *shape*: it lets a
    /// test prove a second sampler pass (Hi-Res-Fix) really reached ComfyUI.
    last_graph_node_types: Arc<Mutex<Vec<String>>>,
}

/// One `LoraLoader` node's `lora_name` + `strength_model` (mirrors
/// `core::pipeline::LoraSpec`, since `strength_model`/`strength_clip` are
/// always set equal by `apply_loras`).
#[derive(Clone, serde::Serialize)]
struct LoraLink {
    file: String,
    strength: f64,
}

struct Prompt {
    submitted_at: Instant,
    filename_prefix: String,
    is_video: bool,
    /// `LoadImage`'s `image` input, if the graph has one (image→video).
    load_image: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = 8188u16;
    let mut ready_ms = 0u64;
    let mut render_ms = 0u64;
    let mut history_error = false;
    let mut base_dir = PathBuf::from(".");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                if let Some(v) = args.next() {
                    port = v.parse().unwrap_or(port);
                }
            }
            "--base-directory" => {
                if let Some(v) = args.next() {
                    base_dir = PathBuf::from(v);
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
        input_dir: base_dir.join("input"),
        prompts: Arc::new(Mutex::new(HashMap::new())),
        last_lora_chain: Arc::new(Mutex::new(Vec::new())),
        last_graph_node_types: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route("/system_stats", get(system_stats))
        .route("/free", post(ok))
        .route("/interrupt", post(ok))
        .route("/prompt", post(submit_prompt))
        .route("/history/{id}", get(history))
        .route("/view", get(view))
        .route("/__test/last_lora_chain", get(last_lora_chain))
        .route("/__test/last_graph_node_types", get(last_graph_node_types))
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
    // Find the SaveImage / SaveVideo node and its filename_prefix, plus any
    // LoadImage (image→video start frame) and every LoraLoader in the chain
    // (node ids sorted numerically -- `apply_loras` always assigns them in
    // chain order starting at "90").
    let mut prefix = "fake".to_string();
    let mut is_video = false;
    let mut load_image = None;
    let mut lora_nodes: Vec<(u32, LoraLink)> = Vec::new();
    let mut node_types: Vec<(u32, String)> = Vec::new();
    if let Some(nodes) = body.get("prompt").and_then(Value::as_object) {
        for (node_id, node) in nodes {
            if let (Ok(id), Some(class_type)) = (
                node_id.parse::<u32>(),
                node.get("class_type").and_then(Value::as_str),
            ) {
                node_types.push((id, class_type.to_string()));
            }
            match node.get("class_type").and_then(Value::as_str) {
                Some("SaveVideo") => is_video = true,
                Some("SaveImage") => {}
                Some("LoadImage") => {
                    load_image = node
                        .pointer("/inputs/image")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    continue;
                }
                Some("LoraLoader") => {
                    let file = node
                        .pointer("/inputs/lora_name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let strength = node
                        .pointer("/inputs/strength_model")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    if let Ok(id) = node_id.parse::<u32>() {
                        lora_nodes.push((id, LoraLink { file, strength }));
                    }
                    continue;
                }
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
    lora_nodes.sort_by_key(|(id, _)| *id);
    let lora_chain: Vec<LoraLink> = lora_nodes.into_iter().map(|(_, link)| link).collect();
    if let Ok(mut last) = fx.last_lora_chain.lock() {
        *last = lora_chain;
    }

    node_types.sort_by_key(|(id, _)| *id);
    let types: Vec<String> = node_types.into_iter().map(|(_, class)| class).collect();
    if let Ok(mut last) = fx.last_graph_node_types.lock() {
        *last = types;
    }

    let id = format!("p-{}", fx.prompts.lock().map(|m| m.len()).unwrap_or(0) + 1);
    if let Ok(mut prompts) = fx.prompts.lock() {
        prompts.insert(
            id.clone(),
            Prompt {
                submitted_at: Instant::now(),
                filename_prefix: prefix,
                is_video,
                load_image,
            },
        );
    }
    Json(json!({ "prompt_id": id, "number": 1, "node_errors": {} }))
}

/// Test-only introspection: the most recently submitted graph's LoRA chain,
/// so an integration test can assert a job's `params.loras` actually reached
/// ComfyUI as real `LoraLoader` nodes (file name + strength, in chain order)
/// instead of only checking the job completed.
async fn last_lora_chain(State(fx): State<Fixture>) -> Json<Value> {
    let chain = fx
        .last_lora_chain
        .lock()
        .map(|c| c.clone())
        .unwrap_or_default();
    Json(json!({ "loras": chain }))
}

/// Test-only introspection: the `class_type` of every node in the most
/// recently submitted graph, in node-id order (numerically sorted), so an
/// integration test can assert the graph's *shape* — e.g. that a Hi-Res-Fix
/// request really produced a `LatentUpscaleBy` and a second sampler pass.
async fn last_graph_node_types(State(fx): State<Fixture>) -> Json<Value> {
    let types = fx
        .last_graph_node_types
        .lock()
        .map(|t| t.clone())
        .unwrap_or_default();
    Json(json!(types))
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
    // image→video: the staged start frame must be sitting in input/.
    let missing_frame = prompt
        .load_image
        .as_deref()
        .filter(|name| !fx.input_dir.join(name).is_file());
    let entry = if fx.history_error || missing_frame.is_some() {
        let msg = missing_frame.map_or_else(
            || "fake render error".to_string(),
            |name| format!("LoadImage: {name} is not in input/"),
        );
        json!({
            "status": {
                "status_str": "error",
                "completed": false,
                "messages": [
                    ["execution_start", {}],
                    ["execution_error", { "exception_message": msg }]
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
