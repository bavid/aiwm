//! End-to-end: a real `job_type=colibri` job through the real `JobEngine` —
//! scheduler decision (always `RunNow`, since Colibri never charges VRAM),
//! `ColibriAdapter` spawns the (fake) server, the chat body streams the
//! answer into `jobs.result`, the job reaches `Completed`.
//!
//! Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use aiwm_core::db::{Database, NewJob, NewModel};
use aiwm_core::orchestrator::{JobEngine, JobOutcome, JobState};
use aiwm_core::runtime::{
    ColibriAdapter, ComfyDirs, ComfyUiAdapter, LlamaCppAdapter, RuntimeRegistry,
};
use aiwm_core::scheduler::HybridScheduler;

fn fake_colibri_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-colibri"))
}

fn colibri_job(prompt: &str, model_id: &str) -> NewJob {
    let mut job = NewJob::new("colibri").on("colibri", model_id, 0);
    job.params = serde_json::json!({ "prompt": prompt, "max_tokens": 32 });
    job
}

struct Harness {
    db: Database,
    engine: Arc<JobEngine>,
    colibri: Arc<ColibriAdapter>,
    _tmp: tempfile::TempDir,
}

impl Harness {
    /// The body of the last `/v1/chat/completions` the fixture served —
    /// fake-colibri's test-only `GET /__test/last_request`.
    async fn last_request(&self) -> serde_json::Value {
        let base = self.colibri.base_url().expect("a loaded fake server");
        reqwest::get(format!("{base}/__test/last_request"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

/// The chat messages the fixture last received, as `(role, content)` pairs.
fn messages(body: &serde_json::Value) -> Vec<(String, String)> {
    body["messages"]
        .as_array()
        .expect("a messages array")
        .iter()
        .map(|m| {
            (
                m["role"].as_str().unwrap_or_default().to_string(),
                m["content"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

async fn harness() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();

    let model_dir = tmp.path().join("qwen36-colibri");
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::write(model_dir.join("config.json"), b"{}").unwrap();
    db.models()
        .insert(NewModel {
            name: "Qwen3.6-35B-A3B".into(),
            format: "colibri".into(),
            file_path: model_dir.to_string_lossy().into_owned(),
            size_bytes: 20_000_000_000,
            ram_estimate_mb: Some(24_576),
            source: "manual".into(),
            roles: vec!["chat".into()],
            ..NewModel::default()
        })
        .await
        .unwrap();

    let registry = RuntimeRegistry::new();
    // llama.cpp / ComfyUI are wired unused (JobEngine::new still wants them) —
    // no binary needed since no job in this file targets them.
    let llama = Arc::new(LlamaCppAdapter::with_binary(db.clone(), None));
    registry.register(llama.clone());
    let comfyui = Arc::new(ComfyUiAdapter::with_launch(
        db.clone(),
        None,
        ComfyDirs {
            base: tmp.path().join("comfyui-data"),
            output: tmp.path().join("outputs"),
            models_store: tmp.path().join("store"),
        },
    ));
    registry.register(comfyui.clone());
    let colibri = Arc::new(ColibriAdapter::with_binary(
        db.clone(),
        Some(fake_colibri_bin()),
    ));
    let probe = colibri.clone();
    registry.register(colibri.clone());

    let scheduler = Arc::new(HybridScheduler::new(registry.clone(), 16_384));
    let engine = Arc::new(
        JobEngine::new(
            db.clone(),
            registry,
            scheduler,
            llama,
            comfyui,
            tmp.path().join("outputs"),
            tmp.path().join("datasets"),
        )
        .with_colibri(colibri),
    );

    Harness {
        db,
        engine,
        colibri: probe,
        _tmp: tmp,
    }
}

/// Personas are not a llama.cpp-only feature: a Colibri chat gets the same
/// system message in front of the user message, and the same `persona` record
/// in its job params.
#[tokio::test]
async fn a_global_persona_reaches_a_colibri_chat() {
    let h = harness().await;
    let model_id = h.db.models().list().await.unwrap()[0].id.clone();
    let p = aiwm_core::persona::create(&h.db, "Blunt", "🪓", "Answer in at most three sentences.")
        .await
        .unwrap();
    aiwm_core::persona::set_active(&h.db, Some(&p.id))
        .await
        .unwrap();

    let job = h
        .engine
        .submit(colibri_job("Hello there", &model_id))
        .await
        .unwrap();
    h.engine.run_next().await.unwrap().unwrap();

    assert_eq!(
        messages(&h.last_request().await),
        [
            (
                "system".to_string(),
                "Answer in at most three sentences.".to_string()
            ),
            ("user".to_string(), "Hello there".to_string()),
        ],
        "the system message must come first"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.params["persona"]["id"], p.id);
    assert_eq!(stored.params["persona"]["name"], "Blunt");
    assert_eq!(stored.params["persona"]["icon"], "🪓");
}

#[tokio::test]
async fn colibri_job_streams_an_answer_to_completion() {
    let h = harness().await;
    let model_id = h.db.models().list().await.unwrap()[0].id.clone();
    let job = h
        .engine
        .submit(colibri_job("Hello there", &model_id))
        .await
        .unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Completed);
    assert!(stored.finished_at.is_some());

    let answer = stored.result.unwrap_or_default();
    assert!(answer.contains("fake-colibri"), "answer: {answer:?}");
    assert!(answer.contains("Hello there"), "answer: {answer:?}");

    assert_eq!(h.db.models().list().await.unwrap()[0].use_count, 1);
}

#[tokio::test]
async fn colibri_job_without_a_prompt_fails() {
    let h = harness().await;
    let model_id = h.db.models().list().await.unwrap()[0].id.clone();
    let mut job = NewJob::new("colibri").on("colibri", &model_id, 0);
    job.params = serde_json::json!({});
    h.engine.submit(job).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(outcome, JobOutcome::Failed { .. }),
        "got {outcome:?}"
    );
}
