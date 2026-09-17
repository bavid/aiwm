//! Tests that drive [`crate::bench::run`] against a stand-in llama-server,
//! plus the stand-in servers themselves. Split out of `tests/mod.rs` to keep
//! that file under the repo's line limit — a pure move, no behaviour change.

use super::super::*;

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

/// A stand-in that streams one token every `token_ms` and never finishes within
/// a test's patience — used to cancel a pass *while it generates*.
async fn paced_llama(token_ms: u64) -> u16 {
    use axum::response::sse::{Event, Sse};
    use futures_util::stream::{self, StreamExt};

    let router = Router::new()
        .route("/health", get(|| async { axum::http::StatusCode::OK }))
        .route(
            "/v1/chat/completions",
            post(move || async move {
                let tokens = stream::iter(0..10_000).then(move |i| async move {
                    tokio::time::sleep(std::time::Duration::from_millis(token_ms)).await;
                    Ok::<_, std::convert::Infallible>(
                        Event::default().data(
                            serde_json::json!({
                                "choices": [{ "delta": { "content": format!("w{i} ") },
                                              "finish_reason": null }]
                            })
                            .to_string(),
                        ),
                    )
                });
                Sse::new(tokens)
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

/// A stand-in that sends one token and then nothing at all, without closing the
/// connection — the shape that parks the client in `chunk().await` forever. Used
/// to prove a cancel does not wait for a stalled server.
async fn stalling_llama() -> u16 {
    use axum::response::sse::{Event, Sse};
    use futures_util::stream::{self, StreamExt};

    let router = Router::new()
        .route("/health", get(|| async { axum::http::StatusCode::OK }))
        .route(
            "/v1/chat/completions",
            post(|| async {
                let first = stream::once(async {
                    Ok::<_, std::convert::Infallible>(
                        Event::default().data(
                            serde_json::json!({
                                "choices": [{ "delta": { "content": "w " },
                                              "finish_reason": null }]
                            })
                            .to_string(),
                        ),
                    )
                });
                Sse::new(first.chain(stream::pending()))
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

/// Like [`mock_llama`], but each pass takes `delay_ms` — long enough for a test
/// to flip a cancel between two prompts of a suite.
async fn delayed_llama(delay_ms: u64) -> u16 {
    let router = Router::new()
        .route("/health", get(|| async { axum::http::StatusCode::OK }))
        .route(
            "/v1/chat/completions",
            post(move || async move {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
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
    assert!(report.suite.is_none(), "the quick test has no suite");
    assert!(report.detail.is_empty(), "and no per-prompt breakdown");
    assert_eq!(report.notes, "cold load; 3 run(s) averaged");

    // Persisted, and it is the latest for the model.
    let stored = db
        .benchmarks()
        .latest_for(&model.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.job_id.as_deref(), Some(job.id.as_str()));
    assert!(stored.suite.is_none());
    assert!(stored.detail.is_none());
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

/// A tiny model + an attached stand-in on `port`, for the cancel tests.
async fn cancellable_fixture(db: &Database, port: u16) -> (Model, String, Arc<LlamaCppAdapter>) {
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
    let llama = Arc::new(LlamaCppAdapter::with_binary(db.clone(), None));
    llama.attach(port, &model.id, 1_000).await.unwrap();
    (model, job.id, llama)
}

#[tokio::test]
async fn a_cancel_mid_generation_does_not_wait_for_the_pass_to_finish() {
    let db = Database::connect_in_memory().await.unwrap();
    let port = paced_llama(20).await; // never reaches `finish_reason`
    let (model, job_id, llama) = cancellable_fixture(&db, port).await;

    let (_tel_tx, tel_rx) = watch::channel(telemetry(1_000, 10_000));
    let (cancel_tx, cancel_rx) = watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        let _ = cancel_tx.send(true);
    });

    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run(
            &db,
            &llama,
            tel_rx,
            &job_id,
            &model,
            BenchRequest::default(),
            None,
            16_376,
            cancel_rx,
        ),
    )
    .await
    .expect("a cancel must not wait for the generation to end")
    .unwrap();

    assert!(matches!(outcome, BenchOutcome::Cancelled), "{outcome:?}");
    assert!(
        db.benchmarks()
            .latest_for(&model.id)
            .await
            .unwrap()
            .is_none(),
        "a cancelled benchmark records nothing"
    );
}

/// The client only notices the dropped receiver when the *next* chunk arrives,
/// so a server that goes quiet mid-body would otherwise pin the pass forever.
#[tokio::test]
async fn a_cancel_is_not_blocked_by_a_server_that_stalls_mid_body() {
    let db = Database::connect_in_memory().await.unwrap();
    let port = stalling_llama().await;
    let (model, job_id, llama) = cancellable_fixture(&db, port).await;

    let (_tel_tx, tel_rx) = watch::channel(telemetry(1_000, 10_000));
    let (cancel_tx, cancel_rx) = watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        let _ = cancel_tx.send(true);
    });

    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run(
            &db,
            &llama,
            tel_rx,
            &job_id,
            &model,
            BenchRequest::default(),
            None,
            16_376,
            cancel_rx,
        ),
    )
    .await
    .expect("a stalled stream must not hold the cancel open")
    .unwrap();

    assert!(matches!(outcome, BenchOutcome::Cancelled), "{outcome:?}");
    assert!(
        db.benchmarks()
            .latest_for(&model.id)
            .await
            .unwrap()
            .is_none(),
        "a cancelled benchmark records nothing"
    );
}

#[tokio::test]
async fn a_cancel_between_two_suite_prompts_stops_the_run() {
    let db = Database::connect_in_memory().await.unwrap();
    let port = delayed_llama(150).await;
    let (model, job_id, llama) = cancellable_fixture(&db, port).await;

    let (_tel_tx, tel_rx) = watch::channel(telemetry(1_000, 10_000));
    let (cancel_tx, cancel_rx) = watch::channel(false);
    // Flip the cancel as soon as the first prompt has reported a pass.
    let watcher = {
        let db = db.clone();
        let job_id = job_id.clone();
        tokio::spawn(async move {
            loop {
                let events = db.jobs().events(&job_id).await.unwrap();
                if events.iter().any(|e| e.message.contains("pass 1/1")) {
                    let _ = cancel_tx.send(true);
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        })
    };

    let outcome = run(
        &db,
        &llama,
        tel_rx,
        &job_id,
        &model,
        BenchRequest {
            runs: 1,
            suite: Some("chat-v1".into()),
            ..BenchRequest::default()
        },
        None,
        16_376,
        cancel_rx,
    )
    .await
    .unwrap();
    watcher.abort();

    assert!(matches!(outcome, BenchOutcome::Cancelled), "{outcome:?}");
    assert!(
        db.benchmarks()
            .latest_for(&model.id)
            .await
            .unwrap()
            .is_none(),
        "a half-finished suite records nothing"
    );
}
