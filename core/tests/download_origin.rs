//! Plan 14: a download remembers what it is. A Civitai / Hugging Face
//! download carries its source and base label through the queue, and the
//! import records them on the model (`source`, `base_family`,
//! `family_source`) — without touching the legacy `family` column.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use axum::Router;

use aiwm_core::db::{Database, DownloadState};
use aiwm_core::download::{DownloadManager, DownloadOrigin, EnqueueRequest};

/// A tiny valid safetensors file with one kohya SDXL-style LoRA tensor.
fn lora_body() -> Vec<u8> {
    let header = br#"{"lora_unet_input_blocks_4_1_proj_in.lora_down.weight":{"dtype":"F16","shape":[1,1],"data_offsets":[0,2]}}"#;
    let mut out = Vec::new();
    out.extend_from_slice(&(header.len() as u64).to_le_bytes());
    out.extend_from_slice(header);
    out.extend_from_slice(&[0, 0]);
    out
}

async fn serve(body: Vec<u8>) -> String {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/f", get(move || async move { body.clone() }));
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    format!("http://{addr}/f")
}

async fn download(
    origin: DownloadOrigin,
    filename: &str,
) -> (tempfile::TempDir, aiwm_core::db::Model) {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect(&tmp.path().join("aiwm.db"))
        .await
        .unwrap();
    let m = Arc::new(DownloadManager::new(
        db.clone(),
        tmp.path().join("store"),
        tmp.path().join(".downloads"),
        Arc::new(AtomicBool::new(false)),
    ));
    tokio::spawn(m.clone().run());
    let url = serve(lora_body()).await;
    let d = m
        .enqueue(EnqueueRequest {
            url,
            filename: filename.into(),
            model_type: Some("lora".into()),
            sha256: None,
            size_bytes: None,
            roles: vec![],
            origin,
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let done = loop {
        let d = m.get(&d.id).await.unwrap().unwrap();
        if d.state.is_terminal() {
            break d;
        }
        assert!(tokio::time::Instant::now() < deadline, "stuck");
        tokio::time::sleep(Duration::from_millis(40)).await;
    };
    assert_eq!(done.state, DownloadState::Done, "{:?}", done.error_text);
    let model = db
        .models()
        .get(&done.model_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    // The temp dir lives as long as the caller holds it.
    (tmp, model)
}

#[tokio::test]
async fn a_civitai_download_records_its_source_and_base_family() {
    let origin = DownloadOrigin::civitai("257749", Some("290640"), Some("Pony")).unwrap();
    let (_tmp, model) = download(origin, "pony_style.safetensors").await;
    assert_eq!(model.source, "civitai:257749/290640");
    assert_eq!(model.base_family.as_deref(), Some("pony"));
    assert_eq!(model.family_source.as_deref(), Some("civitai"));
    // The legacy column keeps whatever the import derived from the name.
    assert_ne!(model.family.as_deref(), Some("pony"));
}

#[tokio::test]
async fn a_hugging_face_download_records_repo_revision_and_base_family() {
    let origin = DownloadOrigin::hf(
        "someone/klein-style",
        Some("abc123"),
        Some("black-forest-labs/FLUX.2-klein-4B"),
    )
    .unwrap();
    let (_tmp, model) = download(origin, "klein_style.safetensors").await;
    assert_eq!(model.source, "hf:someone/klein-style@abc123");
    assert_eq!(model.base_family.as_deref(), Some("flux2-klein-4b"));
    assert_eq!(model.family_source.as_deref(), Some("hf"));
}

#[tokio::test]
async fn a_download_without_origin_records_nothing() {
    let (_tmp, model) = download(DownloadOrigin::default(), "plain.safetensors").await;
    assert_eq!(model.source, "manual");
    assert_eq!(model.base_family, None);
    assert_eq!(model.family_source, None);
}
