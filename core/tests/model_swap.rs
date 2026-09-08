//! The Phase-2 DONE criterion, as one test: import two chat models, run a chat
//! on the first, then a chat on the second — and the engine swaps them on the
//! GPU by itself (evict the resident one, load the new one), no human in the
//! loop. Uses the real `JobEngine` + `LlamaCppAdapter` against the fake server.
//!
//! Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use aiwm_core::db::{Database, NewJob, NewModel};
use aiwm_core::orchestrator::{JobEngine, JobOutcome, JobState};
use aiwm_core::runtime::{LlamaCppAdapter, LlamaServerOptions, RuntimeAdapter, RuntimeRegistry};
use aiwm_core::scheduler::HybridScheduler;

fn fake_llama_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-llama"))
}

struct Harness {
    db: Database,
    engine: Arc<JobEngine>,
    llama: Arc<LlamaCppAdapter>,
    tmp: tempfile::TempDir,
}

impl Harness {
    /// A `budget` small enough that two ~2.5 GB models cannot coexist.
    async fn new(budget_mb: u64) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::connect_in_memory().await.unwrap();
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
        let scheduler = Arc::new(HybridScheduler::new(registry.clone(), budget_mb));
        let engine = Arc::new(JobEngine::new(
            db.clone(),
            registry,
            scheduler,
            llama.clone(),
        ));
        Self {
            db,
            engine,
            llama,
            tmp,
        }
    }

    async fn import(&self, name: &str, file: &str) -> String {
        let gguf = self.tmp.path().join(file);
        std::fs::write(&gguf, b"GGUF\0fixture").unwrap();
        self.db
            .models()
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

    fn resident(&self) -> Vec<String> {
        self.llama
            .loaded_models()
            .into_iter()
            .map(|m| m.model_id)
            .collect()
    }

    async fn events(&self, job_id: &str) -> Vec<String> {
        self.db
            .jobs()
            .events(job_id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect()
    }
}

fn chat_on(model_id: &str, prompt: &str) -> NewJob {
    let mut job = NewJob::new("chat").on("llamacpp", model_id, 2_500);
    job.params["prompt"] = prompt.into();
    job.params["max_tokens"] = 32.into();
    job
}

async fn run_to_completion(engine: &JobEngine, job_id: &str) {
    match engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Completed { job_id: done } => assert_eq!(done, job_id),
        other => panic!("expected {job_id} to complete, got {other:?}"),
    }
}

#[tokio::test]
async fn second_model_swaps_the_first_without_manual_vram_management() {
    // 5000 - 1024 driver - 512 headroom = 3464 usable: one 2.5 GB model fits,
    // two do not.
    let h = Harness::new(5_000).await;
    let a = h.import("Coder A", "a.gguf").await;
    let b = h.import("Coder B", "b.gguf").await;

    // Chat on A — it becomes the resident model.
    let ja = h.engine.submit(chat_on(&a, "hello from A")).await.unwrap();
    run_to_completion(&h.engine, &ja.id).await;
    assert_eq!(h.resident(), vec![a.clone()], "A should be loaded");
    assert_eq!(h.llama.vram_used_mb(), 2_500);

    // Chat on B — the engine must evict A and load B on its own.
    let jb = h.engine.submit(chat_on(&b, "hello from B")).await.unwrap();
    run_to_completion(&h.engine, &jb.id).await;
    assert_eq!(h.resident(), vec![b.clone()], "A evicted, B resident");
    assert_eq!(h.llama.vram_used_mb(), 2_500, "VRAM accounting stays clean");

    // Neither job was blocked or needed a human.
    for (id, prompt) in [(&ja.id, "hello from A"), (&jb.id, "hello from B")] {
        let stored = h.db.jobs().get(id).await.unwrap().unwrap();
        assert_eq!(stored.state, JobState::Completed);
        assert!(
            stored.result.unwrap_or_default().contains(prompt),
            "{id} should have a real answer"
        );
    }

    // The swap is visible on job B's trail.
    let ev = h.events(&jb.id).await;
    assert!(
        ev.iter()
            .any(|m| m.contains("made room") && m.contains("Coder A")),
        "{ev:?}"
    );
    assert!(ev.iter().any(|m| m.contains("answered")), "{ev:?}");

    // Both models are registered and each has been used once.
    let models = h.db.models().list().await.unwrap();
    assert_eq!(models.len(), 2);
    assert!(models.iter().all(|m| m.use_count == 1), "{models:?}");
}
