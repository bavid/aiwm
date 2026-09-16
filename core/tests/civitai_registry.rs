//! `CivitaiSource` + `Registry` driven against `aiwm-fake-civitai`: a real
//! child process, real loopback HTTP, real JSON parsing — mirrors
//! `tests/registry.rs`'s Hugging Face coverage.
//!
//! These tests exist specifically to pin down the three safety requirements
//! from the Civitai integration brief:
//! 1. a Pickle/`"Danger"`-scanned file is never hidden and never reported as
//!    a safe safetensors weight file;
//! 2. Civitai's own `hashes.SHA256` is surfaced as data, never treated as a
//!    verified hash by anything in this crate (verified by inspection — see
//!    the module doc on `registry::civitai` — this file just pins the value
//!    flows through unmodified so a future change can't quietly start
//!    "trusting" it further up the stack);
//! 3. an NSFW-flagged model never appears in a default (`nsfw=false`) search.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::net::{Ipv4Addr, SocketAddr};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use aiwm_core::registry::RemoteFormat;
use aiwm_core::{
    ApiServer, AppPaths, CivitaiSource, Freshness, ModelSource, Registry, SearchQuery,
};

/// The fixture process + the base URL it bound.
struct FakeCivitai {
    child: Child,
    base: String,
}

impl Drop for FakeCivitai {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_fake_civitai() -> FakeCivitai {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aiwm-fake-civitai"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn aiwm-fake-civitai");

    let stdout = child.stdout.take().unwrap();
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("fixture should announce its address");
    let addr = line
        .rsplit("listening on ")
        .next()
        .unwrap()
        .trim()
        .to_string();
    assert!(!addr.is_empty(), "no address in {line:?}");
    FakeCivitai {
        child,
        base: format!("http://{addr}"),
    }
}

fn source(base: &str) -> CivitaiSource {
    CivitaiSource::with_base_url(base).unwrap()
}

#[tokio::test]
async fn search_defaults_to_excluding_nsfw_results() {
    let hub = start_fake_civitai();
    let src = source(&hub.base);

    // `SearchQuery::default()` has `nsfw: false` — the same default the
    // Discover UI's `CivitaiSearchDto` ships. `limit: 10` so the assertion
    // below is a real filter check, not an accident of a 1-item page.
    let hits = src
        .search(&SearchQuery {
            limit: 10,
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert!(
        hits.iter().all(|m| m.id != "777777"),
        "the nsfw-flagged model must never appear in a default search: {hits:?}"
    );
    assert!(
        hits.iter().all(|m| !m.nsfw),
        "no nsfw result should be nsfw"
    );

    // Explicitly opting in surfaces it.
    let with_nsfw = src
        .search(&SearchQuery {
            nsfw: true,
            limit: 10,
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert!(with_nsfw.iter().any(|m| m.id == "777777"));
}

#[tokio::test]
async fn search_filters_by_query_text_and_type() {
    let hub = start_fake_civitai();
    let src = source(&hub.base);

    let hits = src
        .search(&SearchQuery {
            text: Some("Pony".into()),
            limit: 10,
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "257749");
    assert_eq!(hits[0].model_kind_hint.as_deref(), Some("Checkpoint"));
    assert_eq!(hits[0].base_model_family.as_deref(), Some("Pony, SD 1.5"));
    assert_eq!(hits[0].format, RemoteFormat::Safetensors);

    let loras = src
        .search(&SearchQuery {
            media_types: vec!["LORA".into()],
            limit: 10,
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(loras.len(), 1);
    assert_eq!(loras[0].model_kind_hint.as_deref(), Some("LORA"));
}

#[tokio::test]
async fn details_surfaces_the_pickle_scan_verdict_instead_of_hiding_it() {
    let hub = start_fake_civitai();
    let src = source(&hub.base);

    let clean = src.details("257749").await.unwrap();
    let f = &clean.files[0];
    assert_eq!(f.pickle_scan_result.as_deref(), Some("Success"));
    assert_eq!(f.virus_scan_result.as_deref(), Some("Success"));
    // Civitai's own hash is surfaced verbatim (lowercased) as a pre-download
    // sanity check — never fabricated, never absent when Civitai reports one.
    assert_eq!(
        f.sha256.as_deref(),
        Some("67ab2fd8ec439a89b3fedb15cc65f54336af163c7eb5e4f2acc98f090a29b0b3")
    );
    assert_eq!(
        f.download_url.as_deref(),
        Some("https://civitai.com/api/download/models/290640")
    );

    // The "Suspicious Upload" fixture: a `"Danger"` pickle-scan verdict must
    // come through as-is, not be swallowed or downgraded to "Success".
    let suspicious = src.details("424242").await.unwrap();
    let f = &suspicious.files[0];
    assert_eq!(f.pickle_scan_result.as_deref(), Some("Danger"));
}

#[tokio::test]
async fn the_registry_serves_the_cache_when_offline() {
    let hub = start_fake_civitai();
    let cache_dir = tempfile::tempdir().unwrap();
    let offline = Arc::new(AtomicBool::new(false));
    let reg = Registry::new(
        Box::new(source(&hub.base)),
        cache_dir.path().to_path_buf(),
        offline.clone(),
    );
    let q = SearchQuery {
        text: Some("Pony".into()),
        limit: 10,
        ..SearchQuery::default()
    };

    let live = reg.search(&q).await.unwrap();
    assert_eq!(live.freshness, Freshness::Live);

    drop(hub);
    offline.store(true, std::sync::atomic::Ordering::Relaxed);
    let cached = reg.search(&q).await.unwrap();
    assert!(matches!(cached.freshness, Freshness::Offline { .. }));
    assert_eq!(cached.data[0].id, "257749");
}

#[tokio::test]
async fn a_dead_source_falls_back_to_a_stale_cache() {
    let hub = start_fake_civitai();
    let cache_dir = tempfile::tempdir().unwrap();
    let reg = Registry::new(
        Box::new(source(&hub.base)),
        cache_dir.path().to_path_buf(),
        Arc::new(AtomicBool::new(false)),
    );
    let q = SearchQuery::default();

    reg.search(&q).await.unwrap(); // seed
    drop(hub);

    tokio::time::sleep(Duration::from_millis(50)).await;
    let stale = reg.search(&q).await.unwrap();
    assert!(matches!(stale.freshness, Freshness::Stale { .. }));
}

/// `GET /civitai/search` + `GET /civitai/models/{id}` over the real loopback
/// HTTP server, with the app's Civitai registry pointed at the fixture.
#[tokio::test]
async fn discovery_endpoints_over_http() {
    let hub = start_fake_civitai();
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let reg = Registry::new(
        Box::new(source(&hub.base)),
        cache.path().to_path_buf(),
        Arc::new(AtomicBool::new(false)),
    );
    let app = Arc::new(
        aiwm_core::App::load(AppPaths::rooted(data.path()))
            .await
            .unwrap()
            .with_civitai_registry(reg),
    );
    let server = ApiServer::bind(app, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    let base = format!("http://{}", server.addr);

    // Default search excludes NSFW even over HTTP (no `nsfw=true` sent).
    let search: serde_json::Value = reqwest::get(format!("{base}/civitai/search?q=Model"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(search["freshness"]["kind"], "live");
    let hits = search["data"].as_array().unwrap();
    assert!(
        hits.iter().all(|m| m["id"] != "777777"),
        "nsfw model leaked over the default HTTP search: {hits:?}"
    );

    let details: serde_json::Value = reqwest::get(format!("{base}/civitai/models/257749"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(details["id"], "257749");
    let files = details["files"].as_array().unwrap();
    assert_eq!(files[0]["pickle_scan_result"], "Success");
    assert_eq!(files[0]["virus_scan_result"], "Success");
    assert!(files[0]["sha256"].as_str().unwrap().len() == 64);
    let url = files[0]["download_url"].as_str().unwrap();
    assert!(
        url.starts_with("https://civitai.com/api/download/"),
        "{url}"
    );
}

/// Hits the real service. `cargo test -p aiwm-core --test civitai_registry --
/// --ignored` to calibrate the parser against live data.
#[tokio::test]
#[ignore = "network — real civitai.com"]
async fn real_civitai_search_and_details() {
    let src = CivitaiSource::new().unwrap();
    let hits = src
        .search(&SearchQuery {
            text: Some("Pony Diffusion".into()),
            media_types: vec!["Checkpoint".into()],
            limit: 3,
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert!(!hits.is_empty());
    let first = &hits[0];
    assert!(first.model_kind_hint.is_some());

    let d = src.details(&first.id).await.unwrap();
    assert!(
        d.files.iter().any(|f| f.sha256.is_some()),
        "at least one file must carry a hashes.SHA256"
    );
}
