//! Which host may see which API key, and that a redirect never carries one
//! along. Split out of `mod.rs` so the download manager itself stays readable.

use std::sync::{Arc, Mutex};

use axum::routing::get;
use axum::Router;

use super::HostTokens;

fn tokens() -> HostTokens {
    HostTokens {
        civitai: Some("civ-key".into()),
        huggingface: Some("hf-key".into()),
    }
}

#[test]
fn each_host_gets_only_its_own_key() {
    let t = tokens();

    assert_eq!(
        t.for_url("https://civitai.com/api/download/models/1"),
        Some("civ-key")
    );
    // The second front door is the same account (verified 2026-09-20: both
    // hosts answer the same API).
    assert_eq!(
        t.for_url("https://civitai.red/api/download/models/1"),
        Some("civ-key")
    );
    assert_eq!(
        t.for_url("https://huggingface.co/x/resolve/main/f.gguf"),
        Some("hf-key")
    );
    assert_eq!(t.for_url("https://cdn-lfs.hf.co/x/f.gguf"), Some("hf-key"));
}

#[test]
fn a_look_alike_or_unrelated_host_never_sees_a_key() {
    let t = tokens();

    for url in [
        "https://civitai.com.evil.test/api/download/models/1",
        "https://notcivitai.com/api/download/models/1",
        "https://civitai.red.evil.test/f",
        "https://example.test/f.safetensors",
        // The signed CDN a Civitai download redirects to: a third party, and
        // the url already carries its own credentials.
        "https://civitai-delivery-worker-prod.abc.r2.cloudflarestorage.com/model/1/x.safetensors",
        "not a url at all",
    ] {
        assert_eq!(t.for_url(url), None, "{url} must not receive a key");
    }
}

#[test]
fn without_a_key_a_known_host_is_still_recognised_for_the_message() {
    let empty = HostTokens::default();

    assert_eq!(
        empty.for_url("https://civitai.com/api/download/models/1"),
        None
    );
    assert!(empty.knows("https://civitai.com/api/download/models/1"));
    assert!(!empty.knows("https://example.test/f"));
    assert_eq!(
        HostTokens::key_name("https://civitai.red/api/download/models/1"),
        "Civitai API key"
    );
    assert_eq!(
        HostTokens::key_name("https://huggingface.co/x/f"),
        "Hugging Face token"
    );
}

/// The header must not survive the hop to the signed CDN url a Civitai
/// download ends at. reqwest drops it itself; this pins that behaviour so a
/// change to the client builder cannot quietly start leaking the key.
#[tokio::test]
async fn the_key_is_not_forwarded_when_a_redirect_leaves_the_host() {
    let seen: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));

    // The "CDN": records whether it was handed an Authorization header.
    let recorder = seen.clone();
    let cdn = Router::new().route(
        "/file",
        get(move |headers: axum::http::HeaderMap| {
            let recorder = recorder.clone();
            async move {
                let got = headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                recorder.lock().unwrap_or_else(|e| e.into_inner()).push(got);
                "payload"
            }
        }),
    );
    let cdn_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let cdn_port = cdn_listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(cdn_listener, cdn).await.unwrap() });

    // The "origin": redirects to the CDN on another host.
    let target = format!("http://localhost:{cdn_port}/file");
    let origin = Router::new().route(
        "/api/download/models/1",
        get(move || {
            let target = target.clone();
            async move { axum::response::Redirect::temporary(&target) }
        }),
    );
    let origin_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin_port = origin_listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(origin_listener, origin).await.unwrap() });

    let body = reqwest::Client::builder()
        .user_agent("aiwm-test")
        .build()
        .unwrap()
        .get(format!(
            "http://127.0.0.1:{origin_port}/api/download/models/1"
        ))
        .bearer_auth("civ-key")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert_eq!(body, "payload", "the redirect was followed");
    let seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(seen.len(), 1, "the CDN was reached exactly once");
    assert_eq!(
        seen[0], None,
        "the Authorization header must not cross to another host"
    );
}
