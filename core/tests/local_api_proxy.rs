//! The unified local API endpoint (`ANY /v1/*`) driven end-to-end: a real
//! `aiwm-fake-llama` child process stands in for llama.cpp, `App::attach`
//! marks it resident (no installer, no supervised spawn needed — the proxy
//! only cares that *something* is loaded), and a real HTTP round trip proves
//! requests really reach it and stream back.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use aiwm_core::api::handlers;
use aiwm_core::{ApiServer, App, AppPaths};

fn fake_llama_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-llama"))
}

/// Reserve a loopback port the same way `runtime::mod` does: bind ephemeral,
/// read the port back, then drop the listener so the child process can bind it.
async fn reserve_port() -> u16 {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    listener.local_addr().unwrap().port()
}

/// Spawn the fixture on `port` and wait until it answers `/health`.
async fn spawn_fake_llama(port: u16) -> tokio::process::Child {
    let child = tokio::process::Command::new(fake_llama_bin())
        .arg("--port")
        .arg(port.to_string())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let url = format!("http://127.0.0.1:{port}/health");
    for _ in 0..100 {
        if reqwest::get(&url)
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return child;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("aiwm-fake-llama never became healthy on port {port}");
}

/// A live `App` with the fake server attached as the resident model and a
/// local API token already configured. The returned `TempDir` must outlive
/// the test (it backs `App`'s data directory).
async fn app_with_resident_fake(
    token: &str,
) -> (
    std::sync::Arc<App>,
    tempfile::TempDir,
    tokio::process::Child,
) {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::rooted(tmp.path());
    let app = std::sync::Arc::new(App::load(paths).await.unwrap());

    let port = reserve_port().await;
    let child = spawn_fake_llama(port).await;
    app.llama.attach(port, "fixture-model", 4096).await.unwrap();
    handlers::set_local_api_token(&app, token).unwrap();
    (app, tmp, child)
}

#[tokio::test]
async fn forwards_a_real_chat_completion_and_streams_it_back() {
    let (app, _tmp, _fake) = app_with_resident_fake("sk-test-token").await;
    let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("http://{}/v1/chat/completions", server.addr))
        .bearer_auth("sk-test-token")
        .json(&serde_json::json!({
            "messages": [{ "role": "user", "content": "hello proxy" }],
            "stream": true,
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/event-stream"),
        "got: {content_type}"
    );

    // The fixture echoes the prompt back one word per SSE chunk, so "hello"
    // and "proxy" land in separate JSON deltas rather than as one substring.
    let body = resp.text().await.unwrap();
    assert!(body.contains("fake-llama"), "got: {body}");
    assert!(body.contains("\"hello \""), "got: {body}");
    assert!(body.contains("\"proxy "), "got: {body}");
    assert!(body.trim_end().ends_with("data: [DONE]"), "got: {body}");
}

#[tokio::test]
async fn forwards_upstream_error_status_codes_verbatim() {
    let (app, _tmp, _fake) = app_with_resident_fake("sk-test-token").await;
    let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();

    // The fixture has no `/v1/models` route -- proves the proxy relays a real
    // upstream 404 rather than swallowing it into a generic error shape.
    let resp = reqwest::Client::new()
        .get(format!("http://{}/v1/models", server.addr))
        .bearer_auth("sk-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
