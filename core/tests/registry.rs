//! `HuggingFaceSource` + `Registry` driven against `aiwm-fake-hfhub`: a real
//! child process, real loopback HTTP, real JSON parsing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use aiwm_core::registry::{Gated, RemoteFormat};
use aiwm_core::{Freshness, HuggingFaceSource, ModelSource, Registry, SearchQuery, SearchSort};

/// The fixture process + the base URL it bound.
struct FakeHub {
    child: Child,
    base: String,
}

impl Drop for FakeHub {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_fake_hub() -> FakeHub {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aiwm-fake-hfhub"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn aiwm-fake-hfhub");

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
    FakeHub {
        child,
        base: format!("http://{addr}"),
    }
}

fn source(base: &str) -> HuggingFaceSource {
    HuggingFaceSource::with_base_url(base).unwrap()
}

#[tokio::test]
async fn search_parses_the_expanded_hub_fields() {
    let hub = start_fake_hub();
    let src = source(&hub.base);

    let hits = src
        .search(&SearchQuery {
            text: Some("coder".into()),
            gguf_only: true,
            sort: SearchSort::Downloads,
            limit: 10,
            ..SearchQuery::default()
        })
        .await
        .unwrap();

    assert_eq!(hits.len(), 1, "the gguf filter drops the base repo");
    let m = &hits[0];
    assert_eq!(m.id, "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF");
    assert_eq!(m.downloads, 1_780_676, "downloadsAllTime wins");
    assert_eq!(m.param_count, Some(7_615_616_512));
    assert_eq!(m.ctx_max, Some(131_072));
    assert_eq!(m.arch.as_deref(), Some("qwen2"));
    assert_eq!(m.license.as_deref(), Some("apache-2.0"));
    assert_eq!(
        m.base_model.as_deref(),
        Some("Qwen/Qwen2.5-Coder-7B-Instruct")
    );
    assert_eq!(m.format, RemoteFormat::Gguf);
    assert_eq!(m.gated, Gated::No);
}

#[tokio::test]
async fn search_by_base_model_finds_the_quant_re_uploads() {
    let hub = start_fake_hub();
    let src = source(&hub.base);

    let hits = src
        .search(&SearchQuery {
            base_model: Some("Qwen/Qwen2.5-Coder-7B-Instruct".into()),
            limit: 10,
            ..SearchQuery::default()
        })
        .await
        .unwrap();

    let ids: Vec<_> = hits.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["Qwen/Qwen2.5-Coder-7B-Instruct-GGUF"]);
}

#[tokio::test]
async fn details_lists_files_with_the_sha256_from_lfs_oid() {
    let hub = start_fake_hub();
    let src = source(&hub.base);

    let d = src
        .details("Qwen/Qwen2.5-Coder-7B-Instruct-GGUF")
        .await
        .unwrap();

    assert_eq!(d.model.param_count, Some(7_615_616_512));
    let gguf = d
        .files
        .iter()
        .find(|f| f.path.ends_with("q4_k_m.gguf"))
        .unwrap();
    assert_eq!(
        gguf.sha256.as_deref(),
        Some("509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c")
    );
    assert_eq!(gguf.quant.as_deref(), Some("Q4_K_M"));
    assert_eq!(gguf.size, 4_683_073_536);

    let shard = d.files.iter().find(|f| f.shard.is_some()).unwrap();
    assert_eq!(shard.shard, Some((1, 2)));
    assert_eq!(shard.quant.as_deref(), Some("Q8_0"));
    assert_eq!(shard.sha256.as_deref().map(str::len), Some(64));

    // The README has no LFS pointer → listed, no hash.
    let readme = d.files.iter().find(|f| f.path == "README.md").unwrap();
    assert_eq!(readme.sha256, None);
}

#[tokio::test]
async fn the_registry_serves_the_cache_when_offline() {
    let hub = start_fake_hub();
    let cache_dir = tempfile::tempdir().unwrap();
    let offline = Arc::new(AtomicBool::new(false));
    let reg = Registry::new(
        Box::new(source(&hub.base)),
        cache_dir.path().to_path_buf(),
        offline.clone(),
    );
    let q = SearchQuery {
        text: Some("coder".into()),
        limit: 10,
        ..SearchQuery::default()
    };

    let live = reg.search(&q).await.unwrap();
    assert_eq!(live.freshness, Freshness::Live);

    // Kill the fixture, go offline: the same query still answers, from disk.
    drop(hub);
    offline.store(true, std::sync::atomic::Ordering::Relaxed);
    let cached = reg.search(&q).await.unwrap();
    assert!(matches!(cached.freshness, Freshness::Offline { .. }));
    assert_eq!(cached.data[0].id, "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF");

    // An uncached query offline is a clean refusal.
    let other = SearchQuery {
        text: Some("never-searched".into()),
        ..SearchQuery::default()
    };
    assert!(reg.search(&other).await.is_err());
}

/// Hits the real Hub. `cargo test -p aiwm-core --test registry -- --ignored`
/// to calibrate the parser against live data.
#[tokio::test]
#[ignore = "network — real huggingface.co"]
async fn real_hub_search_and_details() {
    let src = HuggingFaceSource::new().unwrap();
    let hits = src
        .search(&SearchQuery {
            text: Some("Qwen2.5-Coder-7B-Instruct-GGUF".into()),
            gguf_only: true,
            limit: 3,
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert!(!hits.is_empty());
    let first = &hits[0];
    assert!(
        first.param_count.is_some(),
        "expand[]=gguf should populate params"
    );

    let d = src.details(&first.id).await.unwrap();
    assert!(
        d.files.iter().any(|f| f.sha256.is_some()),
        "at least one file must carry an lfs.oid sha256"
    );
}

#[tokio::test]
async fn a_dead_source_falls_back_to_a_stale_cache() {
    let hub = start_fake_hub();
    let cache_dir = tempfile::tempdir().unwrap();
    let reg = Registry::new(
        Box::new(source(&hub.base)),
        cache_dir.path().to_path_buf(),
        Arc::new(AtomicBool::new(false)),
    );
    let q = SearchQuery::default();

    reg.search(&q).await.unwrap(); // seed
    drop(hub); // the source is now unreachable, but we are not "offline"

    tokio::time::sleep(Duration::from_millis(50)).await;
    let stale = reg.search(&q).await.unwrap();
    assert!(matches!(stale.freshness, Freshness::Stale { .. }));
}
