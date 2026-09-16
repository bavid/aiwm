//! Test fixture: a minimal Civitai API stand-in (Phase 6.1 extension).
//!
//! Speaks just enough of the real API for the `CivitaiSource` tests: `GET
//! /api/v1/models` (honours `query`, `types`, `nsfw`, `limit`) and `GET
//! /api/v1/models/{id}`.
//!
//! It serves three canned models — a clean SafeTensor checkpoint, a LoRA, and
//! a checkpoint with a non-`"Success"` pickle-scan result on one file (so a
//! test can assert the Discover UI's pipeline surfaces that instead of
//! hiding it) plus an NSFW-flagged model (so a test can assert the default
//! `nsfw=false` search excludes it). Binds `:0` and prints `listening on
//! <addr>` so a test can read the port. Not shipped.

use std::net::Ipv4Addr;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

#[derive(Clone)]
struct Fx;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/v1/models", get(list_models))
        .route("/api/v1/models/{id}", get(model_detail))
        .with_state(Fx);

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    println!("fake-civitai: listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

/// The full catalogue this fixture knows. Field names/nesting mirror a real
/// (anonymous) `GET /api/v1/models` response, verified live.
fn catalogue() -> Vec<Value> {
    vec![
        json!({
            "id": 257_749,
            "name": "Pony Diffusion V6 XL",
            "type": "Checkpoint",
            "nsfw": false,
            "allowCommercialUse": ["Image", "RentCivit"],
            "baseModels": ["Pony", "SD 1.5"],
            "tags": ["western art", "base model"],
            "creator": { "username": "AstraliteHeart" },
            "stats": { "downloadCount": 1_200_000, "thumbsUpCount": 34_000 },
            "modelVersions": [{
                "id": 290_640,
                "baseModel": "Pony",
                "publishedAt": "2023-07-18T00:00:00.000Z",
                "images": [{ "url": "https://image.civitai.com/xyz/preview.jpeg" }],
                "downloadUrl": "https://civitai.com/api/download/models/290640",
                "files": [{
                    "name": "ponyDiffusionV6XL_v6StartWithThisOne.safetensors",
                    "sizeKB": 6_617_170.5,
                    "type": "Model",
                    "pickleScanResult": "Success",
                    "virusScanResult": "Success",
                    "metadata": { "format": "SafeTensor", "size": "pruned", "fp": "fp16" },
                    "hashes": { "SHA256": "67AB2FD8EC439A89B3FEDB15CC65F54336AF163C7EB5E4F2ACC98F090A29B0B3" },
                    "downloadUrl": "https://civitai.com/api/download/models/290640"
                }]
            }]
        }),
        json!({
            "id": 99_263,
            "name": "Add More Detail (detail enhancer LoRA)",
            "type": "LORA",
            "nsfw": false,
            "allowCommercialUse": ["Image"],
            "baseModels": ["SD 1.5"],
            "tags": ["detailed", "enhancer"],
            "creator": { "username": "Lykon" },
            "stats": { "downloadCount": 500_000, "thumbsUpCount": 9_000 },
            "modelVersions": [{
                "id": 135_867,
                "baseModel": "SD 1.5",
                "publishedAt": "2023-08-07T00:00:00.000Z",
                "images": [{ "url": "https://image.civitai.com/abc/preview.jpeg" }],
                "downloadUrl": "https://civitai.com/api/download/models/135867",
                "files": [{
                    "name": "add-detail-xl.safetensors",
                    "sizeKB": 223_097.99,
                    "type": "Model",
                    "pickleScanResult": "Success",
                    "virusScanResult": "Success",
                    "metadata": { "format": "SafeTensor", "size": null, "fp": null },
                    "hashes": { "SHA256": "0D9BD1B873A7863E128B4672E3E245838858F71469A3CEC58123C16C06F83BD7" },
                    "downloadUrl": "https://civitai.com/api/download/models/135867"
                }]
            }]
        }),
        // A version whose only weight file failed its own pickle scan — the
        // fixture for "the UI must surface this, never hide it".
        json!({
            "id": 424_242,
            "name": "Suspicious Upload",
            "type": "Checkpoint",
            "nsfw": false,
            "allowCommercialUse": [],
            "baseModels": ["SD 1.5"],
            "tags": [],
            "creator": { "username": "rando" },
            "stats": { "downloadCount": 12, "thumbsUpCount": 0 },
            "modelVersions": [{
                "id": 424_243,
                "baseModel": "SD 1.5",
                "publishedAt": "2024-01-01T00:00:00.000Z",
                "images": [],
                "downloadUrl": "https://civitai.com/api/download/models/424242",
                "files": [{
                    "name": "suspicious.safetensors",
                    "sizeKB": 100_000.0,
                    "type": "Model",
                    "pickleScanResult": "Danger",
                    "pickleScanMessage": "Suspicious pickle import found",
                    "virusScanResult": "Success",
                    "metadata": { "format": "SafeTensor", "size": null, "fp": null },
                    "hashes": { "SHA256": "aa".repeat(32) },
                    "downloadUrl": "https://civitai.com/api/download/models/424242"
                }]
            }]
        }),
        // NSFW-flagged — must never appear in the default `nsfw=false` search.
        json!({
            "id": 777_777,
            "name": "NSFW Only Model",
            "type": "Checkpoint",
            "nsfw": true,
            "allowCommercialUse": [],
            "baseModels": ["SD 1.5"],
            "tags": ["nsfw"],
            "creator": { "username": "rando2" },
            "stats": { "downloadCount": 5, "thumbsUpCount": 0 },
            "modelVersions": [{
                "id": 777_778,
                "baseModel": "SD 1.5",
                "publishedAt": "2024-01-01T00:00:00.000Z",
                "images": [],
                "downloadUrl": "https://civitai.com/api/download/models/777777",
                "files": [{
                    "name": "nsfw.safetensors",
                    "sizeKB": 100_000.0,
                    "type": "Model",
                    "pickleScanResult": "Success",
                    "virusScanResult": "Success",
                    "metadata": { "format": "SafeTensor", "size": null, "fp": null },
                    "hashes": { "SHA256": "bb".repeat(32) },
                    "downloadUrl": "https://civitai.com/api/download/models/777777"
                }]
            }]
        }),
    ]
}

async fn list_models(
    State(_): State<Fx>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let query_text = q.get("query").map(|s| s.to_lowercase());
    let nsfw_requested = q.get("nsfw").map(|v| v == "true").unwrap_or(false);
    let types: Vec<&str> = q
        .get("types")
        .map(|v| v.split(',').collect())
        .unwrap_or_default();

    let mut items: Vec<Value> = catalogue()
        .into_iter()
        .filter(|m| {
            if let Some(text) = &query_text {
                let name = m["name"].as_str().unwrap_or_default().to_lowercase();
                if !name.contains(text.as_str()) {
                    return false;
                }
            }
            // The fixture mirrors the real API's own `nsfw=false` filtering:
            // an anonymous request that says `nsfw=false` must never see an
            // NSFW-flagged model, matching the safety contract under test.
            if !nsfw_requested && m["nsfw"].as_bool().unwrap_or(false) {
                return false;
            }
            if !types.is_empty() {
                let t = m["type"].as_str().unwrap_or_default();
                if !types.contains(&t) {
                    return false;
                }
            }
            true
        })
        .collect();

    if let Some(n) = q.get("limit").and_then(|l| l.parse::<usize>().ok()) {
        items.truncate(n);
    }
    Json(json!({ "items": items, "metadata": {} }))
}

async fn model_detail(
    State(_): State<Fx>,
    Path(id): Path<String>,
) -> Result<Json<Value>, axum::http::StatusCode> {
    let wanted = id.parse::<u64>().ok();
    catalogue()
        .into_iter()
        .find(|m| m["id"].as_u64() == wanted)
        .map(Json)
        .ok_or(axum::http::StatusCode::NOT_FOUND)
}
