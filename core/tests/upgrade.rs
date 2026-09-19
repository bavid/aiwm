//! End-to-end: a `job_type=upgrade_check` job through the real `JobEngine` — the
//! scheduler loads an `Auto`-picked reasoning model, `core::upgrade` queries a
//! stand-in registry and writes an `UpgradeReport` into `jobs.result`.
//!
//! `aiwm-fake-llama` returns prose, not JSON, so this exercises the
//! objective-order fallback (the LLM-ranked path is covered by unit tests).
//!
//! Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use aiwm_core::db::{Database, NewJob, NewModel};
use aiwm_core::orchestrator::{JobEngine, JobOutcome, JobState};
use aiwm_core::registry::{
    Gated, ModelSource, Registry, RemoteFormat, RemoteModel, RemoteModelDetails, SearchQuery,
};
use aiwm_core::runtime::{
    ComfyDirs, ComfyUiAdapter, LlamaCppAdapter, LlamaServerOptions, RuntimeRegistry,
};
use aiwm_core::scheduler::HybridScheduler;
use aiwm_core::telemetry::{GpuInfo, GpuStatus, HostStatus, SystemTelemetry};
use async_trait::async_trait;
use tokio::sync::watch;

fn fake_llama_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-llama"))
}

fn rm(id: &str, downloads: i64, params: u64, updated: &str) -> RemoteModel {
    RemoteModel {
        id: id.into(),
        author: id.split('/').next().map(str::to_string),
        downloads,
        likes: downloads / 1000,
        trending_score: None,
        created_at: Some("2025-01-01".into()),
        last_modified: Some(updated.into()),
        pipeline_tag: Some("text-generation".into()),
        library_name: None,
        gated: Gated::No,
        license: Some("apache-2.0".into()),
        base_model: None,
        tags: vec!["gguf".into()],
        param_count: Some(params),
        arch: Some("qwen2".into()),
        ctx_max: Some(32_768),
        precision: Some("Q4_K_M".into()),
        format: RemoteFormat::Gguf,
        name: None,
        nsfw: false,
        preview_image_url: None,
        allow_commercial_use: vec![],
        model_kind_hint: None,
        base_model_family: None,
    }
}

#[derive(Debug)]
struct StubSource(Mutex<Vec<Vec<RemoteModel>>>);
#[async_trait]
impl ModelSource for StubSource {
    fn id(&self) -> &'static str {
        "stub"
    }
    async fn search(&self, _q: &SearchQuery) -> aiwm_core::Result<Vec<RemoteModel>> {
        let mut q = self.0.lock().unwrap();
        Ok(if q.is_empty() {
            Vec::new()
        } else {
            q.remove(0)
        })
    }
    async fn details(&self, _id: &str) -> aiwm_core::Result<RemoteModelDetails> {
        unreachable!()
    }
}

struct Harness {
    db: Database,
    engine: Arc<JobEngine>,
    target_id: String,
    _tel_tx: watch::Sender<SystemTelemetry>,
    _tmp: tempfile::TempDir,
}

async fn harness() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();

    let gguf = tmp.path().join("qwen.gguf");
    std::fs::write(&gguf, b"GGUF\0fixture").unwrap();
    // The reasoning model (Auto picks it for the `chat` role).
    db.models()
        .insert(NewModel {
            name: "Qwen2.5 7B Instruct".into(),
            format: "gguf".into(),
            family: Some("qwen2".into()),
            file_path: gguf.to_string_lossy().into_owned(),
            size_bytes: 4_500 * 1024 * 1024,
            ctx_max: Some(32_768),
            param_count: Some(7_600_000_000),
            vram_estimate_mb: Some(6_000),
            source: "manual".into(),
            roles: vec!["chat".into(), "coding".into()],
            ..NewModel::default()
        })
        .await
        .unwrap();
    // The model being checked (same one here).
    let target_id = db.models().list().await.unwrap()[0].id.clone();

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
        None,
        ComfyDirs {
            base: tmp.path().join("comfyui-data"),
            output: tmp.path().join("outputs"),
            models_store: tmp.path().join("store"),
        },
    ));
    registry.register(comfyui.clone());
    let scheduler = Arc::new(HybridScheduler::new(registry.clone(), 16_384));

    let model_index = Arc::new(Registry::new(
        Box::new(StubSource(Mutex::new(vec![
            vec![
                rm(
                    "Qwen/Qwen2.5-Coder-14B-GGUF",
                    400_000,
                    14_000_000_000,
                    "2026-03",
                ),
                rm("Qwen/Qwen3-70B-GGUF", 800_000, 70_000_000_000, "2026-04"), // won't fit
                rm("spam/nobody-qwen", 2, 7_000_000_000, "2026-02"),           // spam
            ],
            vec![rm(
                "Qwen/Qwen2.5-7B-Instruct-GGUF-v2",
                1_200_000,
                7_600_000_000,
                "2026-05",
            )],
        ]))),
        tmp.path().join("cache"),
        Arc::new(AtomicBool::new(false)),
    ));

    let (tel_tx, tel_rx) = watch::channel(SystemTelemetry {
        captured_at_ms: 1,
        gpu: GpuStatus::Available(GpuInfo {
            name: "Test GPU".into(),
            vram_total_mb: 16_376,
            vram_used_mb: 6_200,
            vram_free_mb: 10_176,
            utilization_pct: 20,
            temperature_c: 45,
            processes: vec![],
        }),
        host: HostStatus {
            ram_total_mb: 32_000,
            ram_used_mb: 15_000,
            cpu_total_pct: 10,
            cpu_per_core_pct: vec![],
        },
    });

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
        .with_telemetry(tel_rx)
        .with_registry(model_index),
    );

    Harness {
        db,
        engine,
        target_id,
        _tel_tx: tel_tx,
        _tmp: tmp,
    }
}

#[tokio::test]
async fn an_upgrade_check_writes_a_ranked_report_to_the_job_result() {
    let h = harness().await;
    let mut job = NewJob::new("upgrade_check");
    job.params = serde_json::json!({ "target_model_id": h.target_id });
    let job = h.engine.submit(job).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Completed);
    let report: serde_json::Value =
        serde_json::from_str(stored.result.as_deref().unwrap()).expect("result is JSON");

    let ids: Vec<&str> = report["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"Qwen/Qwen2.5-7B-Instruct-GGUF-v2"), "{ids:?}");
    assert!(ids.contains(&"Qwen/Qwen2.5-Coder-14B-GGUF"), "{ids:?}");
    assert!(
        !ids.contains(&"Qwen/Qwen3-70B-GGUF"),
        "70B filtered on fit: {ids:?}"
    );
    assert!(!ids.contains(&"spam/nobody-qwen"), "spam filtered: {ids:?}");
    // fake-llama returns prose → objective order, newest first.
    assert_eq!(ids[0], "Qwen/Qwen2.5-7B-Instruct-GGUF-v2");
    assert!(
        report["note"].as_str().unwrap().contains("objective"),
        "{}",
        report["note"]
    );
}
