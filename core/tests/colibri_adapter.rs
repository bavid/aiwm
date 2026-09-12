//! `ColibriAdapter` driven against the `aiwm-fake-colibri` stand-in server:
//! a real child process (spawned via `cmd.exe /C`, matching how the real
//! `coli.cmd` launcher must be run on Windows), real `RuntimeSupervisor`,
//! real loopback HTTP.
//!
//! Windows only — that is the production target and where process supervision
//! (Job Objects) actually matters; `cargo test` elsewhere just skips this file.

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use aiwm_core::db::{Database, NewModel};
use aiwm_core::runtime::{ColibriAdapter, Health, RuntimeAdapter};

fn fake_colibri_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-colibri"))
}

fn fixture_adapter(db: Database) -> ColibriAdapter {
    ColibriAdapter::with_binary(db, Some(fake_colibri_bin()))
}

async fn register_model(db: &Database, name: &str, dir: &Path) -> String {
    db.models()
        .insert(NewModel {
            name: name.into(),
            format: "colibri".into(),
            file_path: dir.to_string_lossy().into_owned(),
            size_bytes: 20_000_000_000,
            source: "manual".into(),
            roles: vec!["chat".into()],
            ..NewModel::default()
        })
        .await
        .unwrap()
        .id
}

/// Adapter wired to the fixture binary, plus one registered model at `dir`.
async fn adapter_for(dir: &Path) -> (Database, ColibriAdapter, String) {
    let db = Database::connect_in_memory().await.unwrap();
    let id = register_model(&db, "Fixture Model", dir).await;
    let adapter = fixture_adapter(db.clone());
    (db, adapter, id)
}

#[tokio::test]
async fn loads_streams_a_completion_then_unloads() {
    let tmp = tempfile::tempdir().unwrap();
    let model_dir = tmp.path().join("qwen36-fixture");
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::write(model_dir.join("config.json"), b"{}").unwrap();
    let (_db, adapter, model_id) = adapter_for(&model_dir).await;

    adapter.load_model(&model_id, 0).await.unwrap();

    assert_eq!(adapter.loaded_models().len(), 1);
    assert_eq!(adapter.loaded_models()[0].model_id, model_id);
    assert_eq!(
        adapter.vram_used_mb(),
        0,
        "colibri never charges against the VRAM budget"
    );
    assert_eq!(adapter.health().await, Health::Healthy);
    assert!(adapter.detail().unwrap().starts_with("serving"));

    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    adapter.stream_completion("Say hello", 8, tx).await.unwrap();
    let mut text = String::new();
    let mut done_tokens = None;
    while let Some(ev) = rx.recv().await {
        match ev {
            aiwm_core::runtime::ColibriGenerationEvent::Token(t) => text.push_str(&t),
            aiwm_core::runtime::ColibriGenerationEvent::Done { tokens } => {
                done_tokens = Some(tokens)
            }
        }
    }
    assert!(text.contains("fake-colibri"), "got: {text}");
    assert!(done_tokens.is_some());

    adapter.unload_model(&model_id).await.unwrap();
    assert!(adapter.loaded_models().is_empty());
    assert_eq!(adapter.health().await, Health::Unknown);
}

#[tokio::test]
async fn reloading_the_same_model_is_a_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let model_dir = tmp.path().join("m");
    std::fs::create_dir_all(&model_dir).unwrap();
    let (_db, adapter, model_id) = adapter_for(&model_dir).await;

    adapter.load_model(&model_id, 0).await.unwrap();
    let first = adapter.loaded_models();
    adapter.load_model(&model_id, 0).await.unwrap(); // must not restart
    assert_eq!(adapter.loaded_models(), first);

    adapter.unload_model(&model_id).await.unwrap();
}

#[tokio::test]
async fn loading_a_second_model_swaps_the_resident_one() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();

    let db = Database::connect_in_memory().await.unwrap();
    let id_a = register_model(&db, "Model A", &a).await;
    let id_b = register_model(&db, "Model B", &b).await;
    let adapter = fixture_adapter(db);

    adapter.load_model(&id_a, 0).await.unwrap();
    let detail_a = adapter.detail().unwrap();
    assert_eq!(adapter.loaded_models()[0].model_id, id_a);

    adapter.load_model(&id_b, 0).await.unwrap();
    let loaded = adapter.loaded_models();
    assert_eq!(loaded.len(), 1, "only one server at a time");
    assert_eq!(loaded[0].model_id, id_b);
    assert_ne!(adapter.detail().unwrap(), detail_a, "server was restarted");

    adapter.unload_model(&id_b).await.unwrap();
}

#[tokio::test]
async fn rejects_a_model_whose_directory_is_missing() {
    let (_db, adapter, model_id) = adapter_for(Path::new("Z:\\definitely\\missing")).await;
    let err = adapter.load_model(&model_id, 0).await.unwrap_err();
    assert!(err.to_string().contains("missing"), "got: {err}");
    assert!(adapter.loaded_models().is_empty());
}
