//! End-to-end: a `job_type=bench` job through the real `JobEngine` — the
//! scheduler loads the (fake) llama server, [`aiwm_core::bench`] runs the prompt
//! a few times, a `benchmarks` row lands, the job reaches `Completed`.
//!
//! Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use aiwm_core::bench::suites;
use aiwm_core::db::{Database, NewJob, NewModel};
use aiwm_core::orchestrator::{JobEngine, JobOutcome, JobState};
use aiwm_core::runtime::{
    ComfyDirs, ComfyUiAdapter, LlamaCppAdapter, LlamaServerOptions, RuntimeRegistry,
};
use aiwm_core::scheduler::HybridScheduler;
use aiwm_core::telemetry::{GpuInfo, GpuStatus, HostStatus, SystemTelemetry};
use tokio::sync::watch;

fn fake_llama_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-llama"))
}

struct Harness {
    db: Database,
    engine: Arc<JobEngine>,
    llama: Arc<LlamaCppAdapter>,
    model_id: String,
    _tel_tx: watch::Sender<SystemTelemetry>,
    _tmp: tempfile::TempDir,
}

async fn harness() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();

    let gguf = tmp.path().join("smol.gguf");
    std::fs::write(&gguf, b"GGUF\0fixture").unwrap();
    let model_id = db
        .models()
        .insert(NewModel {
            name: "Smol Chat".into(),
            format: "gguf".into(),
            file_path: gguf.to_string_lossy().into_owned(),
            size_bytes: 4_500 * 1024 * 1024,
            ctx_max: Some(32_768),
            n_layers: Some(28),
            n_embd: Some(3_584),
            n_heads: Some(28),
            n_kv_heads: Some(4),
            vram_estimate_mb: Some(6_000),
            source: "manual".into(),
            roles: vec!["chat".into()],
            ..NewModel::default()
        })
        .await
        .unwrap()
        .id;

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
    let probe = llama.clone();
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

    let (tel_tx, tel_rx) = watch::channel(SystemTelemetry {
        captured_at_ms: 1,
        gpu: GpuStatus::Available(GpuInfo {
            name: "Test GPU".into(),
            vram_total_mb: 16_376,
            vram_used_mb: 6_200,
            vram_free_mb: 10_176,
            utilization_pct: 30,
            temperature_c: 50,
            processes: vec![],
        }),
        host: HostStatus {
            ram_total_mb: 32_000,
            ram_used_mb: 15_500,
            cpu_total_pct: 12,
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
        )
        .with_telemetry(tel_rx),
    );

    Harness {
        db,
        engine,
        llama: probe,
        model_id,
        _tel_tx: tel_tx,
        _tmp: tmp,
    }
}

fn bench_job(model_id: &str) -> NewJob {
    let mut job = NewJob::new("bench").on("llamacpp", model_id, 6_000);
    job.params["runs"] = 2.into();
    job.params["max_tokens"] = 32.into();
    job
}

fn suite_job(model_id: &str, suite: &str) -> NewJob {
    let mut job = NewJob::new("bench").on("llamacpp", model_id, 6_000);
    job.params["suite"] = suite.into();
    job
}

/// The body of the last `/v1/chat/completions` the fixture served — fake-llama's
/// test-only `GET /__test/last_request`.
async fn last_request(h: &Harness) -> serde_json::Value {
    let base = h.llama.base_url().expect("a loaded fake server");
    reqwest::get(format!("{base}/__test/last_request"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_bench_job_measures_the_model_and_records_a_row() {
    let h = harness().await;
    let job = h.engine.submit(bench_job(&h.model_id)).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Completed);
    assert!(stored.finished_at.is_some());

    let bench =
        h.db.benchmarks()
            .latest_for(&h.model_id)
            .await
            .unwrap()
            .expect("a benchmark row");
    assert_eq!(bench.job_id.as_deref(), Some(job.id.as_str()));
    assert_eq!(bench.runs, 2);
    // fake-llama reports 42 tok/s generation, 300 tok/s prompt.
    assert!((bench.gen_tps.unwrap() - 42.0).abs() < 0.001, "{bench:?}");
    assert!(bench.prompt_tps.unwrap() > 0.0);
    assert!(bench.load_ms.is_some(), "cold load was timed");
    assert_eq!(bench.vram_peak_mb, Some(6_200));
    assert!(bench.overall_score > 0 && bench.overall_score <= 100);

    // The model was marked used, and the run left an event trail.
    assert_eq!(
        h.db.models()
            .get(&h.model_id)
            .await
            .unwrap()
            .unwrap()
            .use_count,
        1
    );
    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(events.iter().any(|m| m.contains("run 1/2")), "{events:?}");
    assert!(
        events.iter().any(|m| m.contains("benchmark done")),
        "{events:?}"
    );

    // The quick test keeps sending exactly the request it always sent.
    let body = last_request(&h).await;
    assert_eq!(body["max_tokens"], 32);
    assert_eq!(body["stream"], true);
    for field in ["ignore_eos", "temperature", "seed", "cache_prompt"] {
        assert!(body.get(field).is_none(), "{field} leaked into {body}");
    }
}

#[tokio::test]
async fn a_suite_job_runs_every_prompt_and_stores_the_detail() {
    let h = harness().await;
    let job = h
        .engine
        .submit(suite_job(&h.model_id, "chat-v1"))
        .await
        .unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let bench =
        h.db.benchmarks()
            .latest_for(&h.model_id)
            .await
            .unwrap()
            .expect("a benchmark row");
    assert_eq!(bench.suite.as_deref(), Some("chat-v1"));
    // 3 prompts × the suite default of 2 passes.
    assert_eq!(bench.runs, 6);

    let detail = bench.detail.expect("per-prompt detail");
    let entries = detail.as_array().expect("a JSON array");
    let suite = suites::find("chat-v1").expect("the shipped suite");

    // Every suite pass is fixed-length, deterministic and uncached — otherwise
    // the tok/s of two runs would not be measuring the same work.
    let body = last_request(&h).await;
    assert_eq!(body["max_tokens"], suite.max_tokens);
    assert_eq!(body["ignore_eos"], true);
    assert_eq!(body["temperature"], 0.0);
    assert_eq!(body["seed"], 0);
    assert_eq!(body["cache_prompt"], false);
    assert_eq!(body["messages"][0]["content"], suite.prompts[2].text);

    // …and each prompt really generated its full cap, so no honesty note.
    assert!(entries
        .iter()
        .all(|e| e["tokens"] == i64::from(suite.max_tokens)));
    assert!(entries
        .iter()
        .all(|e| e["max_tokens"] == i64::from(suite.max_tokens)));
    let notes = bench.notes.clone().unwrap_or_default();
    assert!(notes.contains("suite chat-v1"), "{notes}");
    assert!(!notes.contains("stopped early"), "{notes}");
    let ids: Vec<&str> = entries
        .iter()
        .map(|e| e["prompt_id"].as_str().unwrap())
        .collect();
    let expected: Vec<&str> = suite.prompts.iter().map(|p| p.id).collect();
    assert_eq!(ids, expected);
    assert!(entries.iter().all(|e| e["gen_tps"].as_f64().unwrap() > 0.0));

    // It also shows up in the cross-model history, filtered by suite.
    let history =
        h.db.benchmarks()
            .list_all(Some("chat-v1"), 10)
            .await
            .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, bench.id);

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
            .any(|m| m.contains("chat-v1") && m.contains("pass 1/2")),
        "{events:?}"
    );
}

#[tokio::test]
async fn an_unknown_suite_fails_the_job_with_a_clear_message() {
    let h = harness().await;
    let job = h
        .engine
        .submit(suite_job(&h.model_id, "chat-v9"))
        .await
        .unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    let JobOutcome::Failed { error, .. } = &outcome else {
        panic!("expected Failed, got {outcome:?}");
    };
    assert!(
        error.contains("unknown benchmark suite \u{201c}chat-v9\u{201d}"),
        "{error}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Failed);
    assert!(stored
        .error_text
        .as_deref()
        .is_some_and(|e| e.contains("unknown benchmark suite")));
    assert!(
        h.db.benchmarks()
            .latest_for(&h.model_id)
            .await
            .unwrap()
            .is_none(),
        "nothing is recorded for a suite that does not exist"
    );

    // The typo is caught before anything expensive: no model was loaded, and the
    // job never even started running.
    assert!(
        h.llama.base_url().is_none(),
        "a typo must not cost a cold model load"
    );
    assert!(stored.started_at.is_none(), "{stored:?}");
    assert_eq!(
        h.db.models()
            .get(&h.model_id)
            .await
            .unwrap()
            .unwrap()
            .use_count,
        0
    );
}

#[tokio::test]
async fn a_suite_less_job_still_stores_a_legacy_row() {
    let h = harness().await;
    h.engine.submit(bench_job(&h.model_id)).await.unwrap();
    h.engine.run_next().await.unwrap().unwrap();

    let bench =
        h.db.benchmarks()
            .latest_for(&h.model_id)
            .await
            .unwrap()
            .expect("a benchmark row");
    assert!(bench.suite.is_none());
    assert!(bench.detail.is_none());
    assert_eq!(bench.runs, 2);

    // `list_all(None, _)` sees it; a suite filter does not.
    assert_eq!(h.db.benchmarks().list_all(None, 10).await.unwrap().len(), 1);
    assert!(h
        .db
        .benchmarks()
        .list_all(Some("chat-v1"), 10)
        .await
        .unwrap()
        .is_empty());
}
