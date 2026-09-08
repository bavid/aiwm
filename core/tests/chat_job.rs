//! End-to-end: a `job_type=chat` job through the real `JobEngine` — scheduler
//! decision, `LlamaCppAdapter` spawns the (fake) server, the chat body streams
//! the answer into `jobs.result`, the job reaches `Completed`.
//!
//! Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use aiwm_core::db::{Database, NewJob, NewModel};
use aiwm_core::orchestrator::{JobEngine, JobOutcome, JobState};
use aiwm_core::runtime::{LlamaCppAdapter, LlamaServerOptions, RuntimeRegistry};
use aiwm_core::scheduler::HybridScheduler;

fn fake_llama_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-llama"))
}

fn chat_job(prompt: &str) -> NewJob {
    let mut job = NewJob::new("chat");
    job.params = serde_json::json!({ "prompt": prompt, "max_tokens": 64 });
    job
}

struct Harness {
    db: Database,
    engine: JobEngine,
    _tmp: tempfile::TempDir,
}

/// Engine wired to the fixture binary. `with_model` registers a `chat`-role model.
async fn harness(with_model: bool) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();

    if with_model {
        let gguf = tmp.path().join("smol.gguf");
        std::fs::write(&gguf, b"GGUF\0fixture").unwrap();
        db.models()
            .insert(NewModel {
                name: "Smol Chat".into(),
                format: "gguf".into(),
                file_path: gguf.to_string_lossy().into_owned(),
                size_bytes: 4096,
                vram_estimate_mb: Some(1500),
                source: "manual".into(),
                roles: vec!["chat".into()],
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
    let scheduler = Arc::new(HybridScheduler::new(registry.clone(), 16_384));
    let engine = JobEngine::new(db.clone(), registry, scheduler, llama);

    Harness {
        db,
        engine,
        _tmp: tmp,
    }
}

#[tokio::test]
async fn auto_chat_job_streams_an_answer_to_completion() {
    let h = harness(true).await;
    let job = h.engine.submit(chat_job("Hello there")).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Completed);
    assert!(stored.model_id.is_some(), "Auto should have bound a model");
    assert!(stored.finished_at.is_some());

    let answer = stored.result.unwrap_or_default();
    assert!(answer.contains("fake-llama"), "answer: {answer:?}");
    assert!(answer.contains("Hello there"), "answer: {answer:?}");

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
    assert!(events.iter().any(|m| m.contains("answered")), "{events:?}");

    assert_eq!(h.db.models().list().await.unwrap()[0].use_count, 1);
}

#[tokio::test]
async fn explicit_model_chat_job_completes() {
    let h = harness(true).await;
    let model_id = h.db.models().list().await.unwrap()[0].id.clone();

    let mut job = chat_job("Ping");
    job = job.on("llamacpp", &model_id, 1500);
    let job = h.engine.submit(job).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(outcome, JobOutcome::Completed { .. }),
        "got {outcome:?}"
    );
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(stored.result.unwrap_or_default().contains("Ping"));
}

#[tokio::test]
async fn chat_job_without_a_prompt_fails() {
    let h = harness(true).await;
    let model_id = h.db.models().list().await.unwrap()[0].id.clone();
    let job = h
        .engine
        .submit(NewJob::new("chat").on("llamacpp", &model_id, 1500))
        .await
        .unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(outcome, JobOutcome::Failed { .. }),
        "got {outcome:?}"
    );
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Failed);
    assert!(stored.error_text.unwrap_or_default().contains("prompt"));
}

#[tokio::test]
async fn auto_chat_job_fails_cleanly_without_a_chat_model() {
    let h = harness(false).await;
    let outcome = h.engine.submit(chat_job("hi")).await.unwrap();
    let _ = outcome;

    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => assert!(error.contains("no chat model"), "{error}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}
