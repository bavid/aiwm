//! End-to-end: a `job_type=video` job through the real `JobEngine` — scheduler
//! decision, `ComfyUiAdapter` starts the (fake) server, the video body posts the
//! Wan workflow, polls `/history`, fetches the clip via `/view`, writes it to
//! the outputs dir, the job reaches `Completed` with an `.mp4` `output_path`.
//!
//! Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aiwm_core::db::{Database, NewJob, NewModel};
use aiwm_core::orchestrator::{JobEngine, JobOutcome, JobState};
use aiwm_core::runtime::{
    ComfyDirs, ComfyLaunch, ComfyUiAdapter, LlamaCppAdapter, RuntimeRegistry,
};
use aiwm_core::scheduler::HybridScheduler;

fn fake_comfy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-comfy"))
}

fn video_job(prompt: &str) -> NewJob {
    let mut j = NewJob::new("video");
    j.params = serde_json::json!({ "prompt": prompt, "width": 512, "height": 288, "length": 5, "steps": 2 });
    j
}

struct Harness {
    db: Database,
    engine: Arc<JobEngine>,
    outputs: PathBuf,
    tmp: tempfile::TempDir,
}

impl Harness {
    async fn add_model(&self, name: &str, family: Option<&str>, role: &str) -> String {
        let path = self.tmp.path().join(name);
        std::fs::write(&path, b"fixture").unwrap();
        self.db
            .models()
            .insert(NewModel {
                name: name.into(),
                family: family.map(str::to_string),
                format: "safetensors".into(),
                file_path: path.to_string_lossy().into_owned(),
                size_bytes: 1_000,
                vram_estimate_mb: Some(11_000),
                source: "manual".into(),
                roles: vec![role.into()],
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id
    }
}

async fn harness() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let outputs = tmp.path().join("outputs");

    let registry = RuntimeRegistry::new();
    let llama = Arc::new(LlamaCppAdapter::with_binary(db.clone(), None));
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
            output: outputs.clone(),
            models_store: tmp.path().join("store"),
        },
    ));
    registry.register(comfyui.clone());

    let scheduler = Arc::new(HybridScheduler::new(registry.clone(), 16_384));
    let engine = Arc::new(JobEngine::new(
        db.clone(),
        registry,
        scheduler,
        llama,
        comfyui,
        outputs.clone(),
    ));

    Harness {
        db,
        engine,
        outputs,
        tmp,
    }
}

fn is_mp4(path: &Path) -> bool {
    std::fs::read(path)
        .map(|b| b.len() >= 8 && &b[4..8] == b"ftyp")
        .unwrap_or(false)
}

#[tokio::test]
async fn auto_video_job_renders_an_mp4_and_writes_the_file() {
    let h = harness().await;
    h.add_model("wan2.2_ti2v_5B_fp16.safetensors", Some("wan"), "base_video")
        .await;
    h.add_model(
        "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
        None,
        "text_encoder",
    )
    .await;
    h.add_model("wan2.2_vae.safetensors", Some("wan"), "vae")
        .await;

    let job = h
        .engine
        .submit(video_job("a boat on a calm sea"))
        .await
        .unwrap();
    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Completed);
    assert_eq!(stored.runtime_id.as_deref(), Some("comfyui"));

    let out = stored.output_path.expect("output_path set");
    assert_eq!(out, h.outputs.join(format!("{}.mp4", job.id)));
    assert!(is_mp4(Path::new(&out)), "a real MP4 was written");

    // The Wan grid snapped length 5 → 5, and the seed is pinned.
    assert_eq!(stored.params["length"], 5);
    assert!(stored.params["seed"].as_i64().unwrap() >= 0);

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(
        events.iter().any(|m| m.contains("auto-selected")),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|m| m.contains("Wan") && m.contains("takes several minutes")),
        "{events:?}"
    );
    assert!(
        events.iter().any(|m| m.contains("video ready")),
        "{events:?}"
    );
}

#[tokio::test]
async fn a_video_job_without_the_encoder_fails_with_a_clear_message() {
    let h = harness().await;
    h.add_model("wan2.2_ti2v_5B_fp16.safetensors", Some("wan"), "base_video")
        .await;
    h.add_model("wan2.2_vae.safetensors", Some("wan"), "vae")
        .await;

    let job = h.engine.submit(video_job("x")).await.unwrap();
    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => {
            assert!(
                error.contains("umt5") && error.contains("Models tab"),
                "{error}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(h
        .db
        .jobs()
        .get(&job.id)
        .await
        .unwrap()
        .unwrap()
        .output_path
        .is_none());
}

#[tokio::test]
async fn auto_video_job_fails_cleanly_without_a_video_model() {
    let h = harness().await;
    h.engine.submit(video_job("hi")).await.unwrap();
    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => assert!(error.contains("no video model"), "{error}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}
