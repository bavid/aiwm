//! The benchmark HTTP surface driven end-to-end over a real loopback server:
//! the suite catalogue, the optional body a "Test model" POST may carry, and
//! the cross-model history the Benchmark tab reads. No llama.cpp is involved —
//! a submitted job only ever reaches `queued` here, which is exactly the part
//! the API owns; what the job then measures has its own tests in
//! `core::bench`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use aiwm_core::db::{NewBenchmark, NewModel};
use aiwm_core::{ApiServer, App, AppOptions, AppPaths};

/// A live server over a fresh store plus one GGUF model to benchmark. The
/// `TempDir` must outlive the test (it backs the `App`'s data directory). The
/// training poller is off — nothing here touches training runs, and the loop
/// would only add noise.
async fn fixture() -> (ApiServer, tempfile::TempDir, Arc<App>, String) {
    let tmp = tempfile::tempdir().unwrap();
    let app = Arc::new(
        App::load_with(
            AppPaths::rooted(tmp.path()),
            AppOptions {
                training_poller: false,
            },
        )
        .await
        .unwrap(),
    );
    let model_id = insert_model(&app, "Qwen2.5 7B Instruct").await;
    let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    (server, tmp, app, model_id)
}

async fn insert_model(app: &App, name: &str) -> String {
    app.db
        .models()
        .insert(NewModel {
            name: name.into(),
            format: "gguf".into(),
            file_path: format!("E:\\AI\\models\\llm\\{name}.gguf"),
            size_bytes: 4_000 * 1024 * 1024,
            ctx_max: Some(32_768),
            source: "manual".into(),
            roles: vec!["chat".into()],
            ..NewModel::default()
        })
        .await
        .unwrap()
        .id
}

fn row(model_id: &str, suite: Option<&str>) -> NewBenchmark {
    NewBenchmark {
        model_id: model_id.into(),
        job_id: None,
        kind: "llm".into(),
        runs: 6,
        prompt_tps: Some(420.5),
        gen_tps: Some(61.2),
        load_ms: Some(1_800),
        vram_peak_mb: Some(6_100),
        ram_peak_mb: Some(14_200),
        stability_score: 0.94,
        overall_score: 72,
        notes: None,
        suite: suite.map(str::to_string),
        detail_json: None,
    }
}

#[tokio::test]
async fn suites_list_both_shipped_ids_with_their_prompts() {
    let (server, _tmp, _app, _model) = fixture().await;
    let base = format!("http://{}", server.addr);

    let resp = reqwest::get(format!("{base}/bench/suites")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let suites: serde_json::Value = resp.json().await.unwrap();
    let suites = suites.as_array().unwrap();

    let ids: Vec<&str> = suites.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["chat-v1", "coding-v1"]);
    for suite in suites {
        assert!(!suite["title"].as_str().unwrap().is_empty());
        assert!(!suite["description"].as_str().unwrap().is_empty());
        assert!(suite["max_tokens"].as_i64().unwrap() >= 16);
        let prompts = suite["prompts"].as_array().unwrap();
        assert_eq!(prompts.len(), 3, "{}", suite["id"]);
        assert!(!prompts[0]["id"].as_str().unwrap().is_empty());
        assert!(!prompts[0]["title"].as_str().unwrap().is_empty());
        assert!(prompts[0]["text"].as_str().unwrap().len() > 40);
    }
}

/// The Model Library's "Test model" button sends no body at all. That has to
/// stay a plain quick test — same params as before suites existed.
#[tokio::test]
async fn benchmark_without_a_body_queues_a_suiteless_job() {
    let (server, _tmp, _app, model_id) = fixture().await;
    let base = format!("http://{}", server.addr);

    let resp = reqwest::Client::new()
        .post(format!("{base}/models/{model_id}/benchmark"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let job: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(job["job_type"], "bench");
    assert_eq!(job["model_id"], model_id);
    assert!(job["params"].get("suite").is_none(), "{:?}", job["params"]);
    assert!(job["params"].get("runs").is_none(), "{:?}", job["params"]);
    // The scheduler hint the plain button always produced must survive.
    assert!(job["params"]["vram_needed_mb"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn benchmark_with_a_suite_writes_it_into_the_job_params() {
    let (server, _tmp, _app, model_id) = fixture().await;
    let base = format!("http://{}", server.addr);

    let resp = reqwest::Client::new()
        .post(format!("{base}/models/{model_id}/benchmark"))
        .json(&serde_json::json!({ "suite": "chat-v1", "runs": 4 }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let job: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(job["params"]["suite"], "chat-v1");
    assert_eq!(job["params"]["runs"], 4);
}

/// Out-of-range `runs` is clamped by `BenchRequest::from_params`, so the API
/// passes it through untouched rather than refusing a harmless number.
#[tokio::test]
async fn benchmark_passes_an_out_of_range_runs_through_for_the_job_to_clamp() {
    let (server, _tmp, _app, model_id) = fixture().await;
    let base = format!("http://{}", server.addr);

    let resp = reqwest::Client::new()
        .post(format!("{base}/models/{model_id}/benchmark"))
        .json(&serde_json::json!({ "suite": "coding-v1", "runs": 99 }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let job: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(job["params"]["runs"], 99);
}

#[tokio::test]
async fn benchmark_refuses_an_unknown_suite() {
    let (server, _tmp, _app, model_id) = fixture().await;
    let base = format!("http://{}", server.addr);

    let resp = reqwest::Client::new()
        .post(format!("{base}/models/{model_id}/benchmark"))
        .json(&serde_json::json!({ "suite": "chat-v9" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains(r#"unknown benchmark suite "chat-v9""#),
        "{body}"
    );
}

/// A body that *claims* to be JSON but is not must be refused, never quietly
/// treated as "no body at all" (which would silently downgrade a suite run to
/// the single-prompt quick test). The `Option<Json<_>>` extractor only means
/// "no `Content-Type` at all"; everything else has to fail loudly — and
/// nothing may reach the job queue.
#[tokio::test]
async fn benchmark_refuses_a_malformed_body_instead_of_falling_back() {
    let (server, _tmp, _app, model_id) = fixture().await;
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();
    let url = format!("{base}/models/{model_id}/benchmark");

    let post = |content_type: &'static str, body: &'static str| {
        let req = client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body);
        async move { req.send().await.unwrap().status().as_u16() }
    };

    // Truncated JSON and an empty body are both syntax errors.
    assert_eq!(post("application/json", "{").await, 400);
    assert_eq!(post("application/json", "").await, 400);
    // A non-JSON content type is refused before parsing.
    assert_eq!(post("text/plain", "suite=chat-v1").await, 415);
    // Well-formed JSON that cannot become the DTO (`runs` is a `u32`).
    assert_eq!(post("application/json", r#"{"runs": -1}"#).await, 422);

    let jobs: serde_json::Value = reqwest::get(format!("{base}/jobs"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        jobs.as_array().unwrap().is_empty(),
        "a refused body must not queue anything: {jobs}"
    );
}

/// A `limit` that is not a number is a malformed request, not a reason to fall
/// back to the default page size.
#[tokio::test]
async fn history_refuses_a_non_numeric_limit() {
    let (server, _tmp, _app, _model) = fixture().await;
    let base = format!("http://{}", server.addr);

    let resp = reqwest::get(format!("{base}/benchmarks/history?limit=abc"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn history_returns_every_row_and_filters_by_suite() {
    let (server, _tmp, app, first) = fixture().await;
    let second = insert_model(&app, "Hermes 3 8B").await;
    let base = format!("http://{}", server.addr);

    for new in [
        row(&first, None),
        row(&first, Some("chat-v1")),
        row(&second, Some("chat-v1")),
        row(&second, Some("coding-v1")),
    ] {
        app.db.benchmarks().insert(new).await.unwrap();
    }

    let all: serde_json::Value = reqwest::get(format!("{base}/benchmarks/history"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all.as_array().unwrap().len(), 4);
    // Newest first, and the suite column is carried through as-is.
    assert_eq!(all[0]["suite"], "coding-v1");
    assert!(all[3]["suite"].is_null());

    let chat: serde_json::Value = reqwest::get(format!("{base}/benchmarks/history?suite=chat-v1"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let chat = chat.as_array().unwrap();
    assert_eq!(chat.len(), 2);
    assert!(chat.iter().all(|b| b["suite"] == "chat-v1"));
}

#[tokio::test]
async fn history_refuses_an_unknown_suite() {
    let (server, _tmp, _app, _model) = fixture().await;
    let base = format!("http://{}", server.addr);

    let resp = reqwest::get(format!("{base}/benchmarks/history?suite=coding-v9"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains(r#"unknown benchmark suite "coding-v9""#),
        "{body}"
    );
}

/// `limit` is clamped to `1..=200` at the API — a `0` still returns a row and
/// an absurd number cannot drag the whole table into one response.
#[tokio::test]
async fn history_clamps_the_limit_at_both_ends() {
    let (server, _tmp, app, model_id) = fixture().await;
    let base = format!("http://{}", server.addr);

    for _ in 0..205 {
        app.db
            .benchmarks()
            .insert(row(&model_id, Some("chat-v1")))
            .await
            .unwrap();
    }

    let count = |query: &str| {
        let url = format!("{base}/benchmarks/history{query}");
        async move {
            let resp = reqwest::get(url).await.unwrap();
            assert_eq!(resp.status(), 200);
            let rows: serde_json::Value = resp.json().await.unwrap();
            rows.as_array().unwrap().len()
        }
    };

    assert_eq!(count("?limit=0").await, 1);
    assert_eq!(count("?limit=3").await, 3);
    assert_eq!(count("?limit=100000").await, 200);
    // The default when nothing is asked for.
    assert_eq!(count("").await, 50);
}
