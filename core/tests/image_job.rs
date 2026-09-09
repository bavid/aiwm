//! End-to-end: a `job_type=image` job through the real `JobEngine` — scheduler
//! decision, `ComfyUiAdapter` starts the (fake) server, the image body posts the
//! workflow, polls `/history`, fetches the image via `/view`, writes it to the
//! outputs dir, the job reaches `Completed` with `output_path` set.
//!
//! Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use aiwm_core::db::{Database, NewJob, NewModel};
use aiwm_core::orchestrator::{JobEngine, JobOutcome, JobState};
use aiwm_core::runtime::{
    ComfyDirs, ComfyLaunch, ComfyUiAdapter, LlamaCppAdapter, RuntimeRegistry,
};
use aiwm_core::scheduler::HybridScheduler;

fn fake_comfy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-comfy"))
}

fn image_job(prompt: &str) -> NewJob {
    let mut job = NewJob::new("image");
    job.params = serde_json::json!({ "prompt": prompt, "width": 512, "height": 512, "steps": 4 });
    job
}

struct Harness {
    db: Database,
    engine: Arc<JobEngine>,
    outputs: PathBuf,
    _tmp: tempfile::TempDir,
}

async fn harness(with_model: bool) -> Harness {
    harness_with(with_model, &[]).await
}

/// Engine wired to the fake ComfyUI. `with_model` registers a `base_diffusion`
/// checkpoint; `extra_args` are appended to the fixture's command line.
async fn harness_with(with_model: bool, extra_args: &[&str]) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let outputs = tmp.path().join("outputs");

    if with_model {
        let ckpt = tmp.path().join("sd_xl_base_1.0.safetensors");
        std::fs::write(&ckpt, b"safetensors fixture").unwrap();
        db.models()
            .insert(NewModel {
                name: "SDXL Base".into(),
                family: Some("sdxl".into()),
                format: "safetensors".into(),
                file_path: ckpt.to_string_lossy().into_owned(),
                size_bytes: 6_000 * 1024 * 1024,
                vram_estimate_mb: Some(8_000),
                source: "manual".into(),
                roles: vec!["base_diffusion".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
    }

    let registry = RuntimeRegistry::new();
    let llama = Arc::new(LlamaCppAdapter::with_binary(db.clone(), None));
    registry.register(llama.clone());
    let comfyui = Arc::new(ComfyUiAdapter::with_launch(
        db.clone(),
        Some(ComfyLaunch {
            program: fake_comfy_bin(),
            main: None,
            extra_args: extra_args.iter().map(|s| (*s).to_string()).collect(),
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
        _tmp: tmp,
    }
}

fn is_png(path: &Path) -> bool {
    std::fs::read(path)
        .map(|b| b.starts_with(&[0x89, 0x50, 0x4e, 0x47]))
        .unwrap_or(false)
}

#[tokio::test]
async fn auto_image_job_renders_and_writes_the_file() {
    let h = harness(true).await;
    let job = h
        .engine
        .submit(image_job("a red fox in the snow"))
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
    assert!(
        stored.model_id.is_some(),
        "Auto should have bound a checkpoint"
    );
    assert!(stored.finished_at.is_some());

    let out = stored.output_path.expect("output_path set");
    assert_eq!(out, h.outputs.join(format!("{}.png", job.id)));
    assert!(is_png(Path::new(&out)), "a real PNG was written");

    // The random seed was pinned back onto the job.
    let seed = stored.params["seed"].as_i64().expect("seed persisted");
    assert!(seed >= 0);

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
        events.iter().any(|m| m.contains("image ready")),
        "{events:?}"
    );

    assert_eq!(h.db.models().list().await.unwrap()[0].use_count, 1);
}

#[tokio::test]
async fn explicit_checkpoint_image_job_completes() {
    let h = harness(true).await;
    let model_id = h.db.models().list().await.unwrap()[0].id.clone();

    let job = h
        .engine
        .submit(image_job("Ping").on("comfyui", &model_id, 8_000))
        .await
        .unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(outcome, JobOutcome::Completed { .. }),
        "got {outcome:?}"
    );
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(stored.output_path.is_some());
}

#[tokio::test]
async fn image_job_without_a_prompt_fails() {
    let h = harness(true).await;
    let job = h.engine.submit(NewJob::new("image")).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(outcome, JobOutcome::Failed { .. }),
        "got {outcome:?}"
    );
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(stored.error_text.unwrap_or_default().contains("prompt"));
}

#[tokio::test]
async fn auto_image_job_fails_cleanly_without_an_image_model() {
    let h = harness(false).await;
    h.engine.submit(image_job("hi")).await.unwrap();

    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => {
            assert!(error.contains("no image model"), "{error}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn a_comfyui_execution_error_fails_the_job() {
    let h = harness_with(true, &["--fake-history-error"]).await;
    let job = h.engine.submit(image_job("boom")).await.unwrap();

    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => assert!(error.contains("fake render error"), "{error}"),
        other => panic!("expected Failed, got {other:?}"),
    }
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Failed);
}

#[tokio::test]
async fn cancel_stops_a_running_image_job_mid_render() {
    // /history reports "pending" for 4 s — plenty of time to cancel.
    let h = harness_with(true, &["--fake-render-ms", "4000"]).await;
    let job = h.engine.submit(image_job("a long render")).await.unwrap();

    let engine = h.engine.clone();
    let run = tokio::spawn(async move { engine.run_next().await.unwrap().unwrap() });

    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        h.engine.cancel(&job.id).await.unwrap(),
        "cancel should apply"
    );

    let outcome = run.await.unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Cancelled { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Cancelled);
    assert!(stored.finished_at.is_some());
    assert!(stored.output_path.is_none(), "no image on a cancel");

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(
        events
            .iter()
            .any(|m| m.contains("cancelled while rendering")),
        "{events:?}"
    );
}
