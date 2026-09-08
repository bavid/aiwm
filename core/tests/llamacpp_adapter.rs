//! `LlamaCppAdapter` driven against the `aiwm-fake-llama` stand-in server:
//! a real child process, real `RuntimeSupervisor`, real loopback HTTP.
//!
//! Windows only — that is the production target and where process supervision
//! (Job Objects) actually matters; `cargo test` elsewhere just skips this file.

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use aiwm_core::db::{Database, NewModel};
use aiwm_core::runtime::{Health, LlamaCppAdapter, LlamaServerOptions, RuntimeAdapter};

fn fake_llama_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-llama"))
}

fn fixture_adapter(db: Database) -> LlamaCppAdapter {
    LlamaCppAdapter::with_binary(db, Some(fake_llama_bin())).with_options(LlamaServerOptions {
        flash_attention: false, // the fixture ignores flags either way
        ..LlamaServerOptions::default()
    })
}

async fn register_model(db: &Database, name: &str, gguf: &Path) -> String {
    db.models()
        .insert(NewModel {
            name: name.into(),
            format: "gguf".into(),
            file_path: gguf.to_string_lossy().into_owned(),
            size_bytes: 4096,
            source: "manual".into(),
            roles: vec!["chat".into()],
            ..NewModel::default()
        })
        .await
        .unwrap()
        .id
}

/// Adapter wired to the fixture binary, plus one registered model at `gguf`.
async fn adapter_for(gguf: &Path) -> (Database, LlamaCppAdapter, String) {
    let db = Database::connect_in_memory().await.unwrap();
    let id = register_model(&db, "Fixture Model", gguf).await;
    let adapter = fixture_adapter(db.clone());
    (db, adapter, id)
}

#[tokio::test]
async fn loads_serves_a_completion_then_unloads() {
    let tmp = tempfile::tempdir().unwrap();
    let gguf = tmp.path().join("fixture.gguf");
    std::fs::write(&gguf, b"GGUF\0fixture").unwrap();
    let (_db, adapter, model_id) = adapter_for(&gguf).await;

    adapter.load_model(&model_id, 5000).await.unwrap();

    assert_eq!(adapter.loaded_models().len(), 1);
    assert_eq!(adapter.loaded_models()[0].model_id, model_id);
    assert_eq!(adapter.vram_used_mb(), 5000);
    assert_eq!(adapter.health().await, Health::Healthy);
    assert!(adapter.detail().unwrap().starts_with("serving"));

    let reply = adapter.complete("Say hello", 8).await.unwrap();
    assert!(reply.contains("fake-llama"), "got: {reply}");

    adapter.unload_model(&model_id).await.unwrap();
    assert!(adapter.loaded_models().is_empty());
    assert_eq!(adapter.health().await, Health::Unknown);
}

#[tokio::test]
async fn reloading_the_same_model_is_a_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let gguf = tmp.path().join("m.gguf");
    std::fs::write(&gguf, b"GGUF").unwrap();
    let (_db, adapter, model_id) = adapter_for(&gguf).await;

    adapter.load_model(&model_id, 1000).await.unwrap();
    let first = adapter.loaded_models();
    adapter.load_model(&model_id, 1000).await.unwrap(); // must not restart
    assert_eq!(adapter.loaded_models(), first);

    adapter.unload_model(&model_id).await.unwrap();
}

#[tokio::test]
async fn loading_a_second_model_swaps_the_resident_one() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.gguf");
    let b = tmp.path().join("b.gguf");
    std::fs::write(&a, b"GGUF-a").unwrap();
    std::fs::write(&b, b"GGUF-b").unwrap();

    let db = Database::connect_in_memory().await.unwrap();
    let id_a = register_model(&db, "Model A", &a).await;
    let id_b = register_model(&db, "Model B", &b).await;
    let adapter = fixture_adapter(db);

    adapter.load_model(&id_a, 9000).await.unwrap();
    let detail_a = adapter.detail().unwrap();
    assert_eq!(adapter.loaded_models()[0].model_id, id_a);

    adapter.load_model(&id_b, 4000).await.unwrap();
    let loaded = adapter.loaded_models();
    assert_eq!(loaded.len(), 1, "only one server at a time");
    assert_eq!(loaded[0].model_id, id_b);
    assert_eq!(adapter.vram_used_mb(), 4000);
    assert_ne!(adapter.detail().unwrap(), detail_a, "server was restarted");

    adapter.unload_model(&id_b).await.unwrap();
}

#[tokio::test]
async fn rejects_a_model_whose_file_is_missing() {
    let (_db, adapter, model_id) = adapter_for(Path::new("Z:\\definitely\\missing.gguf")).await;
    let err = adapter.load_model(&model_id, 1000).await.unwrap_err();
    assert!(err.to_string().contains("missing"), "got: {err}");
    assert!(adapter.loaded_models().is_empty());
}
