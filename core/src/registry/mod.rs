//! Online model discovery (Phase 6.1, ADR-022).
//!
//! A [`ModelSource`] is a read-only view of a remote model index. The only MVP
//! implementation is [`HuggingFaceSource`] (the Hugging Face Hub); an Ollama
//! adapter is a later best-effort addition behind the same trait.
//!
//! [`Registry`] wraps a source with a disposable on-disk TTL cache
//! (`<data>/cache/registry/`) so `search` / `details` still answer when the
//! network is flaky (`Freshness::Stale`) and refuse cleanly under the global
//! `offline_mode` switch (ADR-009) — unless the cache can serve the answer.

mod cache;
mod huggingface;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use huggingface::HuggingFaceSource;

use crate::{CoreError, Result};

/// Whether a repo's files are behind an access gate on the Hub. Discovery still
/// works; only the *download* needs an accepted licence + token (handled in the
/// download manager, 6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gated {
    #[default]
    No,
    /// Instant click-through on the Hub.
    Auto,
    /// A human at the publisher approves each request.
    Manual,
}

impl Gated {
    pub fn is_gated(self) -> bool {
        !matches!(self, Gated::No)
    }
}

/// The file format that decides which runtime can load the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteFormat {
    Gguf,
    Safetensors,
    #[default]
    Other,
}

/// One search hit — enough for a discovery card without a second call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteModel {
    /// `owner/repo`, e.g. `Qwen/Qwen2.5-Coder-7B-Instruct-GGUF`.
    pub id: String,
    pub author: Option<String>,
    pub downloads: i64,
    pub likes: i64,
    pub trending_score: Option<i64>,
    /// RFC 3339, from `createdAt`.
    pub created_at: Option<String>,
    /// RFC 3339, from `lastModified`.
    pub last_modified: Option<String>,
    pub pipeline_tag: Option<String>,
    pub library_name: Option<String>,
    pub gated: Gated,
    /// Licence slug, parsed from the `license:<slug>` tag or `cardData.license`.
    pub license: Option<String>,
    /// The upstream repo this is a quant / fine-tune of, from the
    /// `base_model:<id>` tag — the thread the upgrade-check (6.7) pulls.
    pub base_model: Option<String>,
    pub tags: Vec<String>,
    /// Total parameters, from `expand[]=gguf` / `expand[]=safetensors`.
    pub param_count: Option<u64>,
    /// Architecture id from the GGUF metadata, e.g. `qwen2`.
    pub arch: Option<String>,
    /// Trained context length from the GGUF metadata.
    pub ctx_max: Option<u32>,
    /// Weight precision — a GGUF quant label (`Q4_K_M`) or a safetensors dtype
    /// key (`BF16`, `F16`, `F8_E4M3`).
    pub precision: Option<String>,
    pub format: RemoteFormat,
}

/// One downloadable file in a repo revision, with the size and **SHA-256**
/// (`lfs.oid`) the download manager verifies against — no download needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFile {
    pub path: String,
    pub size: u64,
    pub sha256: Option<String>,
    /// Quant label parsed from the filename (`Q4_K_M`, `fp16`, `iq4_xs`).
    pub quant: Option<String>,
    /// `Some((index, total))` for a split file `…-00001-of-00003.gguf`.
    pub shard: Option<(u32, u32)>,
}

/// A repo's full detail, including every file with size + hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteModelDetails {
    #[serde(flatten)]
    pub model: RemoteModel,
    pub revision: String,
    pub files: Vec<RemoteFile>,
}

/// How to order search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchSort {
    #[default]
    Downloads,
    Likes,
    /// `likes7d` — recency-weighted popularity.
    Trending,
    /// `lastModified` desc.
    RecentlyUpdated,
    /// `createdAt` desc.
    RecentlyCreated,
}

/// A discovery query. All fields optional; an empty query is "most-downloaded".
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub text: Option<String>,
    /// `filter=base_model:<owner/repo>` — every quant / derivative of one base.
    pub base_model: Option<String>,
    /// Restrict to repos carrying the `gguf` tag.
    pub gguf_only: bool,
    pub sort: SearchSort,
    /// 1..=100; clamped.
    pub limit: u32,
}

/// A read-only remote model index.
#[async_trait]
pub trait ModelSource: Send + Sync + std::fmt::Debug {
    /// Stable id, e.g. `"huggingface"`.
    fn id(&self) -> &'static str;
    async fn search(&self, query: &SearchQuery) -> Result<Vec<RemoteModel>>;
    /// `id` is `owner/repo`.
    async fn details(&self, id: &str) -> Result<RemoteModelDetails>;

    /// Last-fetch / rate-limit / token status for the Diagnostics line.
    fn status(&self) -> RegistryStatus {
        RegistryStatus {
            source_id: self.id().to_string(),
            ..RegistryStatus::default()
        }
    }
}

/// Where an answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Freshness {
    /// Straight from the source.
    Live,
    /// The source failed; this is the last good cached answer.
    Stale { age_secs: u64 },
    /// `offline_mode` is on; this is the cache.
    Offline { age_secs: u64 },
}

/// A source answer plus where it came from.
#[derive(Debug, Clone, Serialize)]
pub struct Fetched<T> {
    pub data: T,
    pub freshness: Freshness,
}

/// Health line for the Diagnostics tab (Phase 6.9).
#[derive(Debug, Clone, Default, Serialize)]
pub struct RegistryStatus {
    /// The source's stable id, e.g. `"huggingface"`.
    pub source_id: String,
    /// RFC 3339 of the last successful fetch, if any this session.
    pub last_fetch: Option<String>,
    /// `RateLimit-Remaining` from the last response.
    pub rate_limit_remaining: Option<i64>,
    /// Seconds until the current rate-limit window clears — `Some` only while
    /// actually limited.
    pub rate_limited_secs: Option<i64>,
    /// A Hugging Face token is configured (never the value).
    pub token_set: bool,
    /// JSON entries in the disposable cache.
    pub cache_entries: u64,
}

/// A [`ModelSource`] plus the disposable TTL cache and the offline switch.
#[derive(Debug)]
pub struct Registry {
    source: Box<dyn ModelSource>,
    cache: cache::Cache,
    offline: Arc<AtomicBool>,
}

impl Registry {
    /// `cache_dir` is `AppPaths::cache_dir().join("registry")`.
    pub fn new(source: Box<dyn ModelSource>, cache_dir: PathBuf, offline: Arc<AtomicBool>) -> Self {
        Self {
            source,
            cache: cache::Cache::new(cache_dir),
            offline,
        }
    }

    fn is_offline(&self) -> bool {
        self.offline.load(Ordering::Relaxed)
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<Fetched<Vec<RemoteModel>>> {
        let key = cache::key(&["search", &search_cache_key(query)]);
        self.resolve(&key, self.source.search(query)).await
    }

    pub async fn details(&self, id: &str) -> Result<Fetched<RemoteModelDetails>> {
        let key = cache::key(&["details", id]);
        self.resolve(&key, self.source.details(id)).await
    }

    /// The Diagnostics health line — the source's status plus the local cache
    /// size.
    pub fn status(&self) -> RegistryStatus {
        RegistryStatus {
            cache_entries: self.cache.count(),
            ..self.source.status()
        }
    }

    /// Offline → serve the cache or refuse. Online → the source, falling back to
    /// the cache (`Stale`) on a transport failure and refreshing it on success.
    async fn resolve<T>(
        &self,
        key: &str,
        fetch: impl std::future::Future<Output = Result<T>>,
    ) -> Result<Fetched<T>>
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        if self.is_offline() {
            return match self.cache.get::<T>(key) {
                Some((data, age_secs)) => Ok(Fetched {
                    data,
                    freshness: Freshness::Offline { age_secs },
                }),
                None => Err(CoreError::Config(
                    "offline mode is on and this query is not cached".into(),
                )),
            };
        }

        match fetch.await {
            Ok(data) => {
                self.cache.put(key, &data);
                Ok(Fetched {
                    data,
                    freshness: Freshness::Live,
                })
            }
            Err(e) => match self.cache.get::<T>(key) {
                Some((data, age_secs)) => Ok(Fetched {
                    data,
                    freshness: Freshness::Stale { age_secs },
                }),
                None => Err(e),
            },
        }
    }
}

/// A stable string for a query, so two equal queries hit the same cache entry.
fn search_cache_key(q: &SearchQuery) -> String {
    format!(
        "{}|{}|{}|{:?}|{}",
        q.text.as_deref().unwrap_or(""),
        q.base_model.as_deref().unwrap_or(""),
        q.gguf_only,
        q.sort,
        q.limit.clamp(1, 100),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn model(id: &str) -> RemoteModel {
        RemoteModel {
            id: id.into(),
            author: None,
            downloads: 1,
            likes: 0,
            trending_score: None,
            created_at: None,
            last_modified: None,
            pipeline_tag: None,
            library_name: None,
            gated: Gated::No,
            license: None,
            base_model: None,
            tags: vec![],
            param_count: None,
            arch: None,
            ctx_max: None,
            precision: None,
            format: RemoteFormat::Gguf,
        }
    }

    /// A source that answers from a script and counts its calls.
    #[derive(Debug)]
    struct FakeSource {
        answers: Mutex<Vec<Result<Vec<RemoteModel>>>>,
        calls: Mutex<u32>,
    }
    impl FakeSource {
        fn new(answers: Vec<Result<Vec<RemoteModel>>>) -> Self {
            Self {
                answers: Mutex::new(answers),
                calls: Mutex::new(0),
            }
        }
    }
    #[async_trait]
    impl ModelSource for FakeSource {
        fn id(&self) -> &'static str {
            "fake"
        }
        async fn search(&self, _q: &SearchQuery) -> Result<Vec<RemoteModel>> {
            *self.calls.lock().unwrap() += 1;
            self.answers
                .lock()
                .unwrap()
                .remove(0)
                .map_err(|e| CoreError::Config(e.to_string()))
        }
        async fn details(&self, _id: &str) -> Result<RemoteModelDetails> {
            unreachable!()
        }
    }

    fn registry(
        answers: Vec<Result<Vec<RemoteModel>>>,
        offline: bool,
    ) -> (Registry, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let r = Registry::new(
            Box::new(FakeSource::new(answers)),
            dir.path().to_path_buf(),
            Arc::new(AtomicBool::new(offline)),
        );
        (r, dir)
    }

    #[tokio::test]
    async fn a_live_answer_is_returned_and_cached() {
        let (r, _d) = registry(vec![Ok(vec![model("a")])], false);
        let got = r.search(&SearchQuery::default()).await.unwrap();
        assert_eq!(got.freshness, Freshness::Live);
        assert_eq!(got.data[0].id, "a");

        // Flip offline — the same query must now come from the cache.
        r.offline.store(true, Ordering::Relaxed);
        let cached = r.search(&SearchQuery::default()).await.unwrap();
        assert!(matches!(cached.freshness, Freshness::Offline { .. }));
        assert_eq!(cached.data[0].id, "a");
    }

    #[tokio::test]
    async fn a_source_failure_falls_back_to_the_cache_as_stale() {
        let (r, _d) = registry(
            vec![Ok(vec![model("a")]), Err(CoreError::Config("boom".into()))],
            false,
        );
        r.search(&SearchQuery::default()).await.unwrap(); // seed the cache
        let stale = r.search(&SearchQuery::default()).await.unwrap();
        assert!(matches!(stale.freshness, Freshness::Stale { .. }));
        assert_eq!(stale.data[0].id, "a");
    }

    #[tokio::test]
    async fn a_source_failure_with_no_cache_is_an_error() {
        let (r, _d) = registry(vec![Err(CoreError::Config("boom".into()))], false);
        assert!(r.search(&SearchQuery::default()).await.is_err());
    }

    #[tokio::test]
    async fn offline_with_no_cache_refuses_without_calling_the_source() {
        let (r, _d) = registry(vec![], true);
        let err = r.search(&SearchQuery::default()).await.unwrap_err();
        assert!(err.to_string().contains("offline"), "{err}");
    }

    #[test]
    fn the_search_cache_key_ignores_unset_fields_but_tracks_the_real_ones() {
        let a = search_cache_key(&SearchQuery::default());
        let b = search_cache_key(&SearchQuery {
            text: Some("qwen".into()),
            ..SearchQuery::default()
        });
        assert_ne!(a, b);
        assert_eq!(a, search_cache_key(&SearchQuery::default()));
    }
}
