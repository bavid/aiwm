//! Unit tests for [`crate::bench`] — the pure scoring/aggregation helpers plus
//! [`super::run`] against a stand-in llama-server.

use super::*;

#[test]
fn stability_of_identical_samples_is_one() {
    assert_eq!(stability_score(&[60.0, 60.0, 60.0]), 1.0);
    assert_eq!(stability_score(&[42.0]), 1.0);
    assert_eq!(stability_score(&[]), 1.0);
}

#[test]
fn stability_drops_as_samples_spread() {
    let tight = stability_score(&[60.0, 61.0, 59.0]);
    let loose = stability_score(&[60.0, 30.0, 90.0]);
    assert!(tight > 0.9, "{tight}");
    assert!(loose < 0.6, "{loose}");
    assert!(tight > loose);
}

#[test]
fn overall_score_rewards_speed_fit_and_stability() {
    let fast_fit = overall_score(80.0, 1.0, &FitVerdict::Green);
    let slow_fit = overall_score(10.0, 1.0, &FitVerdict::Green);
    let fast_tight = overall_score(
        80.0,
        1.0,
        &FitVerdict::Yellow {
            reason: "tight".into(),
        },
    );
    let fast_nofit = overall_score(
        80.0,
        1.0,
        &FitVerdict::Red {
            reason: "over budget".into(),
        },
    );

    assert!(fast_fit > slow_fit);
    assert!(fast_fit > fast_tight);
    assert!(fast_tight > fast_nofit);
    assert_eq!(fast_fit, 100);
    // A model that won't fit is capped low even when it's fast.
    assert!(fast_nofit < 45, "{fast_nofit}");
}

#[test]
fn overall_score_clamps_a_wild_tps() {
    assert_eq!(overall_score(100_000.0, 1.0, &FitVerdict::Green), 100);
    assert_eq!(overall_score(-5.0, 0.0, &FitVerdict::Unknown), 0);
}

#[test]
fn request_from_params_defaults_and_clamps() {
    let d = BenchRequest::from_params(&serde_json::json!({}));
    assert_eq!(d.runs, DEFAULT_RUNS);
    assert_eq!(d.max_tokens, DEFAULT_MAX_TOKENS);
    assert_eq!(d.prompt, DEFAULT_PROMPT);

    let c = BenchRequest::from_params(&serde_json::json!({
        "prompt": "  hi  ", "runs": 99, "max_tokens": 4
    }));
    assert_eq!(c.prompt, "hi");
    assert_eq!(c.runs, MAX_RUNS);
    assert_eq!(c.max_tokens, MIN_MAX_TOKENS);
}

#[test]
fn request_picks_up_a_suite_and_halves_the_default_runs() {
    let r = BenchRequest::from_params(&serde_json::json!({ "suite": "  chat-v1  " }));
    assert_eq!(r.suite.as_deref(), Some("chat-v1"));
    assert_eq!(r.runs, DEFAULT_SUITE_RUNS);
    // The prompt/max_tokens fields stay at their defaults; the suite supplies both.
    assert_eq!(r.max_tokens, DEFAULT_MAX_TOKENS);

    let explicit = BenchRequest::from_params(&serde_json::json!({
        "suite": "coding-v1", "runs": 5
    }));
    assert_eq!(explicit.runs, 5);
    assert_eq!(
        BenchRequest::from_params(&serde_json::json!({ "suite": "coding-v1", "runs": 99 })).runs,
        MAX_RUNS
    );
}

#[test]
fn a_blank_suite_is_no_suite_at_all() {
    let blank = BenchRequest::from_params(&serde_json::json!({ "suite": "   " }));
    assert!(blank.suite.is_none());
    assert_eq!(blank.runs, DEFAULT_RUNS);
    assert!(
        BenchRequest::from_params(&serde_json::json!({ "suite": 7 }))
            .suite
            .is_none()
    );
    assert!(BenchRequest::default().suite.is_none());
}

// --- the pass plan and the pure aggregation ------------------------------

#[test]
fn without_a_suite_the_plan_is_the_single_request_prompt() {
    let req = BenchRequest {
        prompt: "hello".into(),
        max_tokens: 64,
        ..BenchRequest::default()
    };
    let steps = plan(&req).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].prompt_id, None);
    assert_eq!(steps[0].label, None);
    assert_eq!(steps[0].text, "hello");
    assert_eq!(steps[0].max_tokens, 64);
}

#[test]
fn a_suite_expands_into_its_prompts_at_the_suite_length() {
    let req = BenchRequest {
        suite: Some("coding-v1".into()),
        max_tokens: 16, // ignored — the suite decides
        ..BenchRequest::default()
    };
    let steps = plan(&req).unwrap();
    let suite = suites::find("coding-v1").unwrap();
    assert_eq!(steps.len(), 3);
    for (step, prompt) in steps.iter().zip(suite.prompts) {
        assert_eq!(step.prompt_id, Some(prompt.id));
        assert_eq!(step.text, prompt.text);
        assert_eq!(step.max_tokens, suite.max_tokens);
        assert_eq!(
            step.label.as_deref(),
            Some(format!("{} · {}", suite.id, prompt.title).as_str())
        );
    }
}

#[test]
fn an_unknown_suite_is_a_clear_error() {
    let req = BenchRequest {
        suite: Some("chat-v9".into()),
        ..BenchRequest::default()
    };
    let err = plan(&req).unwrap_err().to_string();
    assert!(
        err.contains("unknown benchmark suite \u{201c}chat-v9\u{201d}"),
        "{err}"
    );
}

fn pass(prompt_id: Option<&'static str>, tokens: u64, gen_tps: f64, prompt_tps: f64) -> Pass {
    Pass {
        prompt_id,
        tokens,
        gen_tps,
        prompt_tps,
    }
}

#[test]
fn aggregating_a_legacy_run_gives_means_and_no_detail() {
    let agg = aggregate(&[
        pass(None, 40, 50.0, 400.0),
        pass(None, 40, 60.0, 500.0),
        pass(None, 40, 70.0, 600.0),
    ]);
    assert_eq!(agg.runs, 3);
    assert!((agg.gen_tps.unwrap() - 60.0).abs() < 1e-9);
    assert!((agg.prompt_tps.unwrap() - 500.0).abs() < 1e-9);
    assert!(agg.detail.is_empty());
    assert!((agg.stability - stability_score(&[50.0, 60.0, 70.0])).abs() < 1e-9);
}

#[test]
fn aggregating_a_suite_run_means_per_prompt_and_overall() {
    let agg = aggregate(&[
        pass(Some("a"), 100, 40.0, 400.0),
        pass(Some("a"), 110, 60.0, 500.0),
        pass(Some("b"), 41, 80.0, 600.0),
        pass(Some("b"), 40, 100.0, 700.0),
    ]);
    assert_eq!(agg.runs, 4);
    assert!((agg.gen_tps.unwrap() - 70.0).abs() < 1e-9, "{agg:?}");
    assert!((agg.prompt_tps.unwrap() - 550.0).abs() < 1e-9);

    // First-seen order, mean tokens rounded to whole tokens.
    let ids: Vec<&str> = agg.detail.iter().map(|d| d.prompt_id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b"]);
    assert_eq!(agg.detail[0].tokens, 105);
    assert!((agg.detail[0].gen_tps.unwrap() - 50.0).abs() < 1e-9);
    assert!((agg.detail[0].prompt_tps.unwrap() - 450.0).abs() < 1e-9);
    assert_eq!(agg.detail[1].tokens, 41, "40.5 rounds up");
    assert!((agg.detail[1].gen_tps.unwrap() - 90.0).abs() < 1e-9);
}

#[test]
fn aggregating_nothing_is_empty_rather_than_a_division_by_zero() {
    let agg = aggregate(&[]);
    assert_eq!(agg.runs, 0);
    assert!(agg.gen_tps.is_none());
    assert!(agg.prompt_tps.is_none());
    assert!(agg.detail.is_empty());
    assert_eq!(agg.stability, 1.0);
}

// --- `run` against a stand-in llama-server -------------------------------

use std::net::Ipv4Addr;

use axum::routing::{get, post};
use axum::Router;

use crate::db::{NewJob, NewModel};
use crate::telemetry::{GpuInfo, GpuStatus, HostStatus, SystemTelemetry};

/// SSE body: a couple of content chunks, then a finish chunk carrying the
/// timing numbers the benchmark reads.
fn sse_completion() -> String {
    let delta = |c: &str| {
        format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{c}\"}},\"finish_reason\":null}}]}}\n\n"
        )
    };
    format!(
        "{}{}data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}],\
         \"timings\":{{\"predicted_n\":40,\"predicted_per_second\":55.0,\"prompt_per_second\":410.0}}}}\n\n\
         data: [DONE]\n\n",
        delta("cache "),
        delta("is fast"),
    )
}

async fn mock_llama() -> u16 {
    let router = Router::new()
        .route("/health", get(|| async { axum::http::StatusCode::OK }))
        .route(
            "/v1/chat/completions",
            post(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    sse_completion(),
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    port
}

fn telemetry(vram_used_mb: u64, ram_used_mb: u64) -> SystemTelemetry {
    SystemTelemetry {
        captured_at_ms: 1,
        gpu: GpuStatus::Available(GpuInfo {
            name: "Test GPU".into(),
            vram_total_mb: 16_376,
            vram_used_mb,
            vram_free_mb: 16_376 - vram_used_mb,
            utilization_pct: 20,
            temperature_c: 45,
            processes: vec![],
        }),
        host: HostStatus {
            ram_total_mb: 32_000,
            ram_used_mb,
            cpu_total_pct: 10,
            cpu_per_core_pct: vec![],
        },
    }
}

#[tokio::test]
async fn run_measures_records_and_reports() {
    let db = Database::connect_in_memory().await.unwrap();
    let model = db
        .models()
        .insert(NewModel {
            name: "Qwen2.5 7B".into(),
            format: "gguf".into(),
            file_path: "E:\\AI\\models\\llm\\qwen\\qwen.gguf".into(),
            size_bytes: 4_500 * 1024 * 1024,
            ctx_max: Some(32_768),
            n_layers: Some(28),
            n_embd: Some(3_584),
            n_heads: Some(28),
            n_kv_heads: Some(4),
            source: "manual".into(),
            ..NewModel::default()
        })
        .await
        .unwrap();
    let job = db.jobs().insert(NewJob::new("bench")).await.unwrap();

    let port = mock_llama().await;
    let llama = Arc::new(LlamaCppAdapter::with_binary(db.clone(), None));
    llama.attach(port, &model.id, 6_000).await.unwrap();

    let (_tel_tx, tel_rx) = watch::channel(telemetry(6_100, 15_000));
    let (_c_tx, cancel_rx) = watch::channel(false);

    let outcome = run(
        &db,
        &llama,
        tel_rx,
        &job.id,
        &model,
        BenchRequest {
            runs: 3,
            max_tokens: 32,
            ..BenchRequest::default()
        },
        Some(std::time::Duration::from_millis(1_750)),
        16_376,
        cancel_rx,
    )
    .await
    .unwrap();

    let BenchOutcome::Done(report) = outcome else {
        panic!("expected Done, got {outcome:?}");
    };
    assert_eq!(report.runs, 3);
    assert!((report.gen_tps.unwrap() - 55.0).abs() < 0.001);
    assert!((report.prompt_tps.unwrap() - 410.0).abs() < 0.001);
    assert_eq!(report.load_ms, Some(1_750));
    assert_eq!(report.vram_peak_mb, Some(6_100));
    assert_eq!(report.ram_peak_mb, Some(15_000));
    assert!(
        (report.stability_score - 1.0).abs() < 0.001,
        "identical runs"
    );
    assert!(report.overall_score > 0);

    // Persisted, and it is the latest for the model.
    let stored = db
        .benchmarks()
        .latest_for(&model.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.job_id.as_deref(), Some(job.id.as_str()));
    assert_eq!(stored.runs, 3);
    assert_eq!(stored.gen_tps, report.gen_tps);
    assert_eq!(stored.overall_score, i64::from(report.overall_score));

    // Per-run + summary lines on the job's event trail.
    let events = db.jobs().events(&job.id).await.unwrap();
    assert!(events.iter().any(|e| e.message.contains("run 1/3")));
    assert!(events.iter().any(|e| e.message.starts_with("score ")));
}

#[tokio::test]
async fn a_suite_run_covers_every_prompt_and_records_the_detail() {
    let db = Database::connect_in_memory().await.unwrap();
    let model = db
        .models()
        .insert(NewModel {
            name: "Qwen2.5 7B".into(),
            format: "gguf".into(),
            file_path: "E:\\AI\\models\\llm\\qwen\\qwen.gguf".into(),
            size_bytes: 4_500 * 1024 * 1024,
            ctx_max: Some(32_768),
            source: "manual".into(),
            ..NewModel::default()
        })
        .await
        .unwrap();
    let job = db.jobs().insert(NewJob::new("bench")).await.unwrap();

    let port = mock_llama().await;
    let llama = Arc::new(LlamaCppAdapter::with_binary(db.clone(), None));
    llama.attach(port, &model.id, 6_000).await.unwrap();

    let (_tel_tx, tel_rx) = watch::channel(telemetry(6_100, 15_000));
    let (_c_tx, cancel_rx) = watch::channel(false);

    let outcome = run(
        &db,
        &llama,
        tel_rx,
        &job.id,
        &model,
        BenchRequest::from_params(&serde_json::json!({ "suite": "chat-v1" })),
        None,
        16_376,
        cancel_rx,
    )
    .await
    .unwrap();

    let BenchOutcome::Done(report) = outcome else {
        panic!("expected Done, got {outcome:?}");
    };
    let suite = suites::find("chat-v1").unwrap();
    assert_eq!(report.suite.as_deref(), Some("chat-v1"));
    assert_eq!(report.runs, 6, "3 prompts × 2 passes");
    let ids: Vec<&str> = report.detail.iter().map(|d| d.prompt_id.as_str()).collect();
    let expected: Vec<&str> = suite.prompts.iter().map(|p| p.id).collect();
    assert_eq!(ids, expected);
    assert!(report.detail.iter().all(|d| d.tokens == 40));
    assert!((report.gen_tps.unwrap() - 55.0).abs() < 0.001);

    let stored = db
        .benchmarks()
        .latest_for(&model.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.suite.as_deref(), Some("chat-v1"));
    assert_eq!(stored.runs, 6);
    let detail = stored.detail.expect("detail json");
    assert_eq!(detail.as_array().unwrap().len(), 3);
    assert_eq!(detail[0]["prompt_id"], suite.prompts[0].id);

    let events: Vec<String> = db
        .jobs()
        .events(&job.id)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert!(
        events
            .iter()
            .any(|m| m.contains(&format!("chat-v1 · {} — pass 2/2", suite.prompts[0].title))),
        "{events:?}"
    );
}

#[tokio::test]
async fn run_stops_when_cancelled_before_the_first_pass() {
    let db = Database::connect_in_memory().await.unwrap();
    let model = db
        .models()
        .insert(NewModel {
            name: "M".into(),
            format: "gguf".into(),
            file_path: "E:\\AI\\models\\llm\\m\\m.gguf".into(),
            size_bytes: 1_000 * 1024 * 1024,
            source: "manual".into(),
            ..NewModel::default()
        })
        .await
        .unwrap();
    let job = db.jobs().insert(NewJob::new("bench")).await.unwrap();
    let port = mock_llama().await;
    let llama = Arc::new(LlamaCppAdapter::with_binary(db.clone(), None));
    llama.attach(port, &model.id, 1_000).await.unwrap();

    let (_tel_tx, tel_rx) = watch::channel(telemetry(1_000, 10_000));
    let (_c_tx, cancel_rx) = watch::channel(true); // already cancelled

    let outcome = run(
        &db,
        &llama,
        tel_rx,
        &job.id,
        &model,
        BenchRequest::default(),
        None,
        16_376,
        cancel_rx,
    )
    .await
    .unwrap();
    assert!(matches!(outcome, BenchOutcome::Cancelled));
    assert!(db
        .benchmarks()
        .latest_for(&model.id)
        .await
        .unwrap()
        .is_none());
}
