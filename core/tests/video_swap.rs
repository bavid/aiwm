//! Video takes its VRAM slot the same way image does: a chat model is resident,
//! a `job_type=video` job needs the GPU, and the engine evicts llama.cpp and
//! reserves ComfyUI by itself — then a second chat job swaps it back. The
//! scheduler is modality-agnostic, so this is the Phase-4 counterpart of
//! `llm_diffusion_swap.rs`.
//!
//! Real `JobEngine` + real adapters against the two fake servers. Windows only.

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

async fn harness(budget_mb: u64) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();

    // A chat model …
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

    // … and the Wan 2.2 video stack (model reserves ~11 GB).
    for (name, family, role, vram) in [
        (
            "wan2.2_ti2v_5B_fp16.safetensors",
            Some("wan"),
            "base_video",
            Some(11_000),
        ),
        (
            "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
            None,
            "text_encoder",
            None,
        ),
        ("wan2.2_vae.safetensors", Some("wan"), "vae", None),
    ] {
        let path = tmp.path().join(name);
        std::fs::write(&path, b"fixture").unwrap();
        db.models()
            .insert(NewModel {
                name: name.into(),
                family: family.map(str::to_string),
                format: "safetensors".into(),
                file_path: path.to_string_lossy().into_owned(),
                size_bytes: 1_000,
                vram_estimate_mb: vram,
                source: "manual".into(),
                roles: vec![role.into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
    }

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
fn video_job(prompt: &str) -> NewJob {
    let mut j = NewJob::new("video");
    j.params = serde_json::json!({ "prompt": prompt, "width": 512, "height": 288, "length": 5, "steps": 2 });
    j
}

async fn run_ok(engine: &JobEngine) {
    match engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Completed { .. } => {}
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
async fn a_video_job_evicts_the_llm_for_its_vram_slot_and_a_later_chat_swaps_it_back() {
    // 14 000 − 1024 driver − 512 headroom = 12 464 usable: the ~7 GB chat model
    // and the 11 GB video model cannot coexist.
    let h = harness(14_000).await;

    // 1. Chat — llama.cpp resident.
    let c1 = h.engine.submit(chat_job("hi")).await.unwrap();
    run_ok(&h.engine).await;
    assert_eq!(h.llama.loaded_models().len(), 1);
    assert!(h.comfyui.loaded_models().is_empty());

    // 2. Video — the engine evicts the LLM and reserves ComfyUI on its own.
    let vid = h.engine.submit(video_job("a boat at dawn")).await.unwrap();
    run_ok(&h.engine).await;

    assert!(
        h.llama.loaded_models().is_empty(),
        "the chat model was evicted"
    );
    assert_eq!(
        h.comfyui.loaded_models().len(),
        1,
        "ComfyUI holds the video model slot"
    );

    let vid_events = events(&h.db, &vid.id).await;
    assert!(
        vid_events
            .iter()
            .any(|m| m.contains("made room on the GPU") && m.contains("Chat 8B")),
        "the eviction is on the video job's trail: {vid_events:?}"
    );
    let stored = h.db.jobs().get(&vid.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Completed);
    assert!(stored.output_path.unwrap().ends_with(".mp4"));

    // 3. Chat again — ComfyUI's reservation is the one that gets evicted now.
    let c2 = h.engine.submit(chat_job("hi again")).await.unwrap();
    run_ok(&h.engine).await;
    assert!(
        h.comfyui.loaded_models().is_empty(),
        "ComfyUI's slot was freed"
    );
    assert_eq!(h.llama.loaded_models().len(), 1, "llama.cpp is back");

    for id in [&c1.id, &vid.id, &c2.id] {
        assert_eq!(
            h.db.jobs().get(id).await.unwrap().unwrap().state,
            JobState::Completed,
            "no job blocked, no human in the loop"
        );
    }
}
