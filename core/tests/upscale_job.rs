//! End-to-end: a `job_type=upscale` job through the real `JobEngine` —
//! scheduler decision (no library `Model`, unlike image/video), the fake
//! ComfyUI server posts the RTX Video Super Resolution workflow, polls
//! `/history`, fetches the result via `/view`, writes it to the outputs dir,
//! the job reaches `Completed` with a new `output_path`.
//!
//! Sources an already-finished `image`/`video` job's own output — the
//! engine's own `run_next` is used twice per test: once to actually render
//! the source job for real (through the same fake ComfyUI), then again for
//! the upscale job that references it. This exercises the full plumbing
//! (job dispatch, `capability::media` staging, the pipeline graph builder,
//! the ComfyUI HTTP round trip) even though the fake fixture — like
//! `image_job.rs`/`video_job.rs` — only cares about the `SaveImage` /
//! `SaveVideo` / `LoadImage` node shapes, not the RTX node's own internals
//! (there is no real ComfyUI + the RTX node installed in this environment;
//! see `capability::upscale` / `pipeline::rtx_upscale_image`'s doc comments
//! for how that part was verified instead).
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

fn video_job(prompt: &str) -> NewJob {
    let mut job = NewJob::new("video");
    job.params = serde_json::json!({ "prompt": prompt, "width": 512, "height": 288, "length": 5, "steps": 2 });
    job
}

fn upscale_job(source: &str) -> NewJob {
    let mut job = NewJob::new("upscale");
    job.params = serde_json::json!({ "source": source });
    job
}

struct Harness {
    db: Database,
    engine: Arc<JobEngine>,
    outputs: PathBuf,
    _tmp: tempfile::TempDir,
}

async fn harness() -> Harness {
    harness_with(&[]).await
}

/// `extra_args` are appended to the fake ComfyUI fixture's command line (e.g.
/// `--fake-render-ms` for the cancel test).
async fn harness_with(extra_args: &[&str]) -> Harness {
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
        outputs.join("datasets"),
    ));

    Harness {
        db,
        engine,
        outputs,
        _tmp: tmp,
    }
}

impl Harness {
    async fn add_model(&self, name: &str, family: Option<&str>, role: &str) -> String {
        let path = self._tmp.path().join(name);
        std::fs::write(&path, b"fixture").unwrap();
        self.db
            .models()
            .insert(NewModel {
                name: name.into(),
                family: family.map(str::to_string),
                format: "safetensors".into(),
                file_path: path.to_string_lossy().into_owned(),
                size_bytes: 1_000,
                vram_estimate_mb: Some(9_000),
                source: "manual".into(),
                roles: vec![role.into()],
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id
    }

    /// Render a real `image` job through the engine (needs a `base_diffusion`
    /// checkpoint) and return its id — a finished job the upscale tests can
    /// reference as `source`.
    async fn render_image(&self) -> String {
        self.add_model("sd_xl_base_1.0.safetensors", Some("sdxl"), "base_diffusion")
            .await;
        let job = self
            .engine
            .submit(image_job("a red fox in the snow"))
            .await
            .unwrap();
        let outcome = self.engine.run_next().await.unwrap().unwrap();
        assert!(
            matches!(outcome, JobOutcome::Completed { .. }),
            "{outcome:?}"
        );
        job.id
    }

    /// Same, for a `video` job (needs Wan's three companion files).
    async fn render_video(&self) -> String {
        self.add_model("wan2.2_ti2v_5B_fp16.safetensors", Some("wan"), "base_video")
            .await;
        self.add_model(
            "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
            None,
            "text_encoder",
        )
        .await;
        self.add_model("wan2.2_vae.safetensors", Some("wan"), "vae")
            .await;
        let job = self
            .engine
            .submit(video_job("a boat on a calm sea"))
            .await
            .unwrap();
        let outcome = self.engine.run_next().await.unwrap().unwrap();
        assert!(
            matches!(outcome, JobOutcome::Completed { .. }),
            "{outcome:?}"
        );
        job.id
    }
}

fn is_png(path: &Path) -> bool {
    std::fs::read(path)
        .map(|b| b.starts_with(&[0x89, 0x50, 0x4e, 0x47]))
        .unwrap_or(false)
}

fn is_mp4(path: &Path) -> bool {
    std::fs::read(path)
        .map(|b| b.len() >= 8 && &b[4..8] == b"ftyp")
        .unwrap_or(false)
}

#[tokio::test]
async fn upscaling_a_finished_image_job_completes_with_a_new_png() {
    let h = harness().await;
    let source_id = h.render_image().await;

    let job = h.engine.submit(upscale_job(&source_id)).await.unwrap();
    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Completed);
    assert_eq!(stored.runtime_id.as_deref(), Some("comfyui"));
    assert!(stored.finished_at.is_some());

    let out = stored.output_path.expect("output_path set");
    assert_eq!(out, h.outputs.join(format!("{}.png", job.id)));
    assert_ne!(out, h.outputs.join(format!("{source_id}.png")));
    assert!(is_png(Path::new(&out)), "a real PNG was written");

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(events.iter().any(|m| m.contains("upscaling")), "{events:?}");
    assert!(
        events.iter().any(|m| m.contains("upscale ready")),
        "{events:?}"
    );
}

#[tokio::test]
async fn upscaling_a_finished_video_job_completes_with_a_new_mp4() {
    let h = harness().await;
    let source_id = h.render_video().await;

    let job = h.engine.submit(upscale_job(&source_id)).await.unwrap();
    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    let out = stored.output_path.expect("output_path set");
    assert_eq!(out, h.outputs.join(format!("{}.mp4", job.id)));
    assert!(is_mp4(Path::new(&out)), "a real MP4 was written");
}

#[tokio::test]
async fn upscale_job_without_a_source_fails_cleanly() {
    let h = harness().await;
    h.engine.submit(NewJob::new("upscale")).await.unwrap();

    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => assert!(error.contains("source"), "{error}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn upscale_job_reports_a_clear_error_for_a_missing_source_file() {
    let h = harness().await;
    h.engine
        .submit(upscale_job("C:\\nope\\gone.png"))
        .await
        .unwrap();

    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => assert!(error.contains("not found"), "{error}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn upscale_job_reports_a_clear_error_when_the_source_job_never_rendered() {
    let h = harness().await;
    let empty_job = h.engine.submit(image_job("never run")).await.unwrap();

    h.engine.submit(upscale_job(&empty_job.id)).await.unwrap();
    // The image job above is still `queued` — run it (fails: no checkpoint in
    // the library), then the upscale job that references its (nonexistent)
    // output.
    let _ = h.engine.run_next().await.unwrap().unwrap();
    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => assert!(error.contains("no image output"), "{error}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn cancel_stops_a_running_upscale_job_mid_pass() {
    // /history reports "pending" for 4 s — plenty of time to cancel.
    let h = harness_with(&["--fake-render-ms", "4000"]).await;
    // Render the source image with a *separate*, instant harness so only the
    // upscale job itself hits the slow fixture.
    let fast = harness().await;
    let source_id = fast.render_image().await;
    // Copy the rendered PNG into this harness' own outputs dir under the same
    // job id so `source_id` still resolves via a bare path... simpler: just
    // reuse `fast`'s db, pointed at the slow ComfyUI adapter instead.
    let job = h
        .engine
        .submit(upscale_job(
            fast.outputs
                .join(format!("{source_id}.png"))
                .to_str()
                .unwrap(),
        ))
        .await
        .unwrap();

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
    assert!(stored.output_path.is_none(), "no output on a cancel");

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
            .any(|m| m.contains("cancelled while upscaling")),
        "{events:?}"
    );
}
