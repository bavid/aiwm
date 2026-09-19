//! The Phase-3 DONE criterion, as one test: a chat model is resident on the GPU,
//! an image job needs the VRAM, and the engine swaps ComfyUI in for llama.cpp by
//! itself — then a second chat job swaps it back. No human in the loop.
//!
//! Real `JobEngine` + real `LlamaCppAdapter` + real `ComfyUiAdapter` against the
//! two fake servers. Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use aiwm_core::db::{Database, NewJob, NewModel};
use aiwm_core::orchestrator::{JobEngine, JobOutcome, JobState};
use aiwm_core::runtime::{
    ComfyDirs, ComfyLaunch, ComfyUiAdapter, LlamaCppAdapter, LlamaServerOptions, RuntimeAdapter,
    RuntimeRegistry,
};
use aiwm_core::scheduler::HybridScheduler;

fn fake_llama_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-llama"))
}
fn fake_comfy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-comfy"))
}

struct Harness {
    db: Database,
    engine: Arc<JobEngine>,
    llama: Arc<LlamaCppAdapter>,
    comfyui: Arc<ComfyUiAdapter>,
    _tmp: tempfile::TempDir,
}

/// `budget_mb` small enough that a ~7 GB chat model and a ~7 GB image checkpoint
/// cannot coexist.
async fn harness(budget_mb: u64) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();

    // A chat model (rough KV estimate ≈ 6.9 GB) …
    let gguf = tmp.path().join("chat.gguf");
    std::fs::write(&gguf, b"GGUF\0fixture").unwrap();
    db.models()
        .insert(NewModel {
            name: "Chat 8B".into(),
            format: "gguf".into(),
            file_path: gguf.to_string_lossy().into_owned(),
            size_bytes: 5_000 * 1024 * 1024,
            source: "manual".into(),
            roles: vec!["chat".into()],
            ..NewModel::default()
        })
        .await
        .unwrap();

    // … and an SDXL checkpoint reserving 7 GB.
    let ckpt = tmp.path().join("sd_xl_base_1.0.safetensors");
    std::fs::write(&ckpt, b"safetensors fixture").unwrap();
    db.models()
        .insert(NewModel {
            name: "SDXL Base".into(),
            family: Some("sdxl".into()),
            format: "safetensors".into(),
            file_path: ckpt.to_string_lossy().into_owned(),
            size_bytes: 6_000 * 1024 * 1024,
            vram_estimate_mb: Some(7_000),
            source: "manual".into(),
            roles: vec!["base_diffusion".into()],
            ..NewModel::default()
        })
        .await
        .unwrap();

    let registry = RuntimeRegistry::new();
    let llama = Arc::new(
        LlamaCppAdapter::with_binary(db.clone(), Some(fake_llama_bin())).with_options(
            LlamaServerOptions {
                flash_attention: false,
                ..LlamaServerOptions::default()
            },
        ),
    );
    registry.register(llama.clone());
    let comfyui = Arc::new(ComfyUiAdapter::with_launch(
        db.clone(),
        Some(ComfyLaunch {
            program: fake_comfy_bin(),
            main: None,
            extra_args: Vec::new(),
        }),
        ComfyDirs {
            base: tmp.path().join("comfyui-data"),
            output: tmp.path().join("outputs"),
            models_store: tmp.path().join("store"),
        },
    ));
    registry.register(comfyui.clone());

    let scheduler = Arc::new(HybridScheduler::new(registry.clone(), budget_mb));
    let engine = Arc::new(JobEngine::new(
        db.clone(),
        registry,
        scheduler,
        llama.clone(),
        comfyui.clone(),
        tmp.path().join("outputs"),
        tmp.path().join("datasets"),
    ));

    Harness {
        db,
        engine,
        llama,
        comfyui,
        _tmp: tmp,
    }
}

fn chat_job(prompt: &str) -> NewJob {
    let mut j = NewJob::new("chat");
    j.params = serde_json::json!({ "prompt": prompt, "max_tokens": 16 });
    j
}
fn image_job(prompt: &str) -> NewJob {
    let mut j = NewJob::new("image");
    j.params = serde_json::json!({ "prompt": prompt, "width": 512, "height": 512, "steps": 4 });
    j
}

async fn run_ok(engine: &JobEngine) -> String {
    match engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Completed { job_id } => job_id,
        other => panic!("expected Completed, got {other:?}"),
    }
}

async fn events(db: &Database, job_id: &str) -> Vec<String> {
    db.jobs()
        .events(job_id)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.message)
        .collect()
}

#[tokio::test]
async fn the_engine_swaps_llama_for_comfyui_and_back_around_the_vram_budget() {
    // 12 000 − 1024 driver − 512 headroom = 10 464 usable: one ~7 GB model fits,
    // two do not.
    let h = harness(12_000).await;

    // 1. Chat — llama.cpp becomes resident.
    let c1 = h.engine.submit(chat_job("hello")).await.unwrap();
    run_ok(&h.engine).await;
    assert_eq!(h.llama.loaded_models().len(), 1);
    assert!(h.comfyui.loaded_models().is_empty());

    // 2. Image — the engine must evict the LLM and reserve ComfyUI on its own.
    let img = h.engine.submit(image_job("a red fox")).await.unwrap();
    run_ok(&h.engine).await;

    assert!(
        h.llama.loaded_models().is_empty(),
        "the chat model was evicted"
    );
    let comfy_loaded: Vec<_> = h
        .comfyui
        .loaded_models()
        .into_iter()
        .map(|m| m.model_id)
        .collect();
    assert_eq!(comfy_loaded.len(), 1, "ComfyUI holds the checkpoint slot");

    let img_events = events(&h.db, &img.id).await;
    assert!(
        img_events
            .iter()
            .any(|m| m.contains("made room on the GPU") && m.contains("Chat 8B")),
        "the eviction is on the image job's trail: {img_events:?}"
    );
    let stored = h.db.jobs().get(&img.id).await.unwrap().unwrap();
    assert!(stored.output_path.is_some());

    // 3. Chat again — now ComfyUI's reservation is the one that gets evicted.
    let c2 = h.engine.submit(chat_job("and hello again")).await.unwrap();
    run_ok(&h.engine).await;

    assert!(
        h.comfyui.loaded_models().is_empty(),
        "ComfyUI's slot was freed"
    );
    assert_eq!(h.llama.loaded_models().len(), 1, "llama.cpp is back");

    let c2_events = events(&h.db, &c2.id).await;
    assert!(
        c2_events.iter().any(|m| m.contains("made room on the GPU")),
        "{c2_events:?}"
    );

    // Every job finished cleanly — no human, no `blocked`.
    for id in [&c1.id, &img.id, &c2.id] {
        assert_eq!(
            h.db.jobs().get(id).await.unwrap().unwrap().state,
            JobState::Completed
        );
    }
}
