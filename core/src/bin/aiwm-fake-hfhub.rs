//! Test fixture: a minimal Hugging Face Hub stand-in (Phase 6.1).
//!
//! Speaks just enough of the Hub's read API for the `HuggingFaceSource` tests:
//! `GET /api/models` (honours `search`, `filter`, `limit`), `GET
//! /api/models/{owner}/{repo}` (with `expand[]` fields), and `GET
//! /api/models/{owner}/{repo}/tree/{rev}?recursive=true`.
//!
//! It serves two canned repos — a GGUF quant repo and its base — so a search,
//! a `base_model:` filter, and a detail-with-files call can all be exercised
//! without touching the network. Binds `:0` and prints `listening on <addr>`
//! so a test can read the port. Not shipped.

use std::net::Ipv4Addr;

use axum::extract::{Path, Query, RawQuery, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

#[derive(Clone)]
struct Fx;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/models", get(list_models))
        .route("/api/models/{owner}/{repo}", get(model_detail))
        .route("/api/models/{owner}/{repo}/tree/{rev}", get(model_tree))
        .with_state(Fx);

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    println!("fake-hfhub: listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

/// The full catalogue this fixture knows.
fn catalogue() -> Vec<Value> {
    vec![
        json!({
            "id": "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF",
            "author": "Qwen",
            "downloads": 256_578,
            "downloadsAllTime": 1_780_676,
            "likes": 436,
            "trendingScore": 12,
            "private": false,
            "gated": false,
            "sha": "abc123",
            "createdAt": "2024-09-18T11:40:39.000Z",
            "lastModified": "2024-11-12T07:59:32.000Z",
            "pipeline_tag": "text-generation",
            "library_name": "transformers",
            "tags": [
                "transformers", "gguf", "code", "text-generation",
                "base_model:Qwen/Qwen2.5-Coder-7B-Instruct",
                "base_model:quantized:Qwen/Qwen2.5-Coder-7B-Instruct",
                "license:apache-2.0"
            ],
            "gguf": { "total": 7_615_616_512_u64, "architecture": "qwen2", "context_length": 131_072 }
        }),
        json!({
            "id": "Qwen/Qwen2.5-Coder-7B-Instruct",
            "author": "Qwen",
            "downloads": 900_000,
            "downloadsAllTime": 5_000_000,
            "likes": 700,
            "trendingScore": 40,
            "private": false,
            "gated": false,
            "sha": "def456",
            "createdAt": "2024-09-17T00:00:00.000Z",
            "lastModified": "2024-11-01T00:00:00.000Z",
            "pipeline_tag": "text-generation",
            "library_name": "transformers",
            "tags": ["transformers", "safetensors", "code", "license:apache-2.0"],
            "safetensors": { "parameters": { "BF16": 7_615_616_512_u64 }, "total": 7_615_616_512_u64 }
        }),
    ]
}

async fn list_models(
    State(_): State<Fx>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    RawQuery(raw): RawQuery,
) -> Json<Value> {
    let raw = raw.unwrap_or_default();
    let filters: Vec<&str> = raw
        .split('&')
        .filter_map(|kv| kv.strip_prefix("filter="))
        .collect();

    let mut out: Vec<Value> = catalogue()
        .into_iter()
        .filter(|m| {
            let id = m["id"].as_str().unwrap_or_default();
            let tags: Vec<&str> = m["tags"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();

            // `search=` is a case-insensitive substring on the id.
            if let Some(s) = q.get("search") {
                if !id.to_lowercase().contains(&s.to_lowercase()) {
                    return false;
                }
            }
            // Every `filter=` must be satisfied (`gguf` tag, or `base_model:<id>`).
            filters.iter().all(|f| {
                let f = urldecode(f);
                if let Some(bm) = f.strip_prefix("base_model:") {
                    tags.iter().any(|t| *t == format!("base_model:{bm}"))
                } else {
                    tags.contains(&f.as_str())
                }
            })
        })
        .collect();

    if let Some(n) = q.get("limit").and_then(|l| l.parse::<usize>().ok()) {
        out.truncate(n);
    }
    Json(Value::Array(out))
}

async fn model_detail(
    State(_): State<Fx>,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<Value>, axum::http::StatusCode> {
    let id = format!("{owner}/{repo}");
    catalogue()
        .into_iter()
        .find(|m| m["id"] == id)
        .map(Json)
        .ok_or(axum::http::StatusCode::NOT_FOUND)
}

async fn model_tree(
    State(_): State<Fx>,
    Path((owner, repo, _rev)): Path<(String, String, String)>,
) -> Json<Value> {
    let id = format!("{owner}/{repo}");
    if id.ends_with("-GGUF") {
        Json(json!([
            { "type": "directory", "path": "sub" },
            { "type": "file", "path": "README.md", "size": 1200, "oid": "gitsha" },
            {
                "type": "file",
                "path": "qwen2.5-coder-7b-instruct-q4_k_m.gguf",
                "size": 4_683_073_536_u64,
                "oid": "gitsha2",
                "lfs": {
                    "oid": "509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c",
                    "size": 4_683_073_536_u64, "pointerSize": 135
                },
                "xetHash": "DO_NOT_USE_THIS_HASH_0000000000000000000000000000000000000000"
            },
            {
                "type": "file",
                "path": "qwen2.5-coder-7b-instruct-q8_0-00001-of-00002.gguf",
                "size": 3_980_069_280_u64,
                "lfs": { "oid": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", "size": 3_980_069_280_u64 }
            }
        ]))
    } else {
        Json(json!([
            {
                "type": "file", "path": "model.safetensors", "size": 15_231_233_920_u64,
                "lfs": { "oid": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210", "size": 15_231_233_920_u64 }
            }
        ]))
    }
}

fn urldecode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        match b {
            b'%' => {
                let hi = bytes.next();
                let lo = bytes.next();
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        let hex = [h, l];
                        if let Ok(s) = std::str::from_utf8(&hex) {
                            if let Ok(n) = u8::from_str_radix(s, 16) {
                                out.push(n as char);
                                continue;
                            }
                        }
                        out.push('%');
                    }
                    _ => out.push('%'),
                }
            }
            b'+' => out.push(' '),
            _ => out.push(b as char),
        }
    }
    out
}
