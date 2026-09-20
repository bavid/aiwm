//! Online model discovery (Phase 6.1, ADR-022).
//!
//! A [`ModelSource`] is a read-only view of a remote model index. The two
//! implementations are [`HuggingFaceSource`] (the Hugging Face Hub) and
//! [`CivitaiSource`] (civitai.com, image/video checkpoints + LoRAs); an Ollama
//! adapter is a later best-effort addition behind the same trait.
//!
//! [`Registry`] wraps a source with a disposable on-disk TTL cache
//! (`<data>/cache/registry/`) so `search` / `details` still answer when the
//! network is flaky (`Freshness::Stale`) and refuse cleanly under the global
//! `offline_mode` switch (ADR-009) — unless the cache can serve the answer.
//! Each source gets its own [`Registry`] (own cache subdirectory, own
//! `offline` check) — see [`crate::App::registry`] / `civitai_registry`.

mod cache;
mod civitai;
mod huggingface;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use civitai::CivitaiSource;
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

/// How many sample items of a model's gallery a search result carries. The
/// primary version of a popular Civitai model has 15–20; more than this adds
/// response size for a strip nobody scrolls that far.
pub const MAX_PREVIEWS: usize = 12;

/// One item of a model's sample gallery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePreview {
    pub url: String,
    /// Civitai serves short mp4 clips as samples for video models, so a
    /// gallery is not all `<img>`.
    #[serde(default)]
    pub is_video: bool,
    /// Civitai's own rating for this single image, `1` = safe. A model whose
    /// own `nsfw` flag is false can still carry spicier samples, so the client
    /// hides anything above `1` unless the user asked to see adult content.
    #[serde(default)]
    pub nsfw_level: i64,
}

/// One search hit — enough for a discovery card without a second call.
///
/// Shared by every [`ModelSource`]. A source that has no concept of a given
/// field (e.g. Hugging Face has no NSFW flag; Civitai has no licence slug)
/// leaves it at its default rather than fabricating a value — see each
/// field's doc comment for which sources actually populate it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteModel {
    /// The source's own id — Hugging Face's `owner/repo`, or Civitai's
    /// numeric model id (as a string). Round-tripped into `details(id)`.
    pub id: String,
    /// A separate human-readable title, for sources whose `id` isn't already
    /// one (Civitai's `name`, e.g. "Pony Diffusion V6 XL"). `None` for
    /// Hugging Face, where `id` already reads as a title.
    #[serde(default)]
    pub name: Option<String>,
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
    /// Licence slug, parsed from the `license:<slug>` tag or `cardData.license`
    /// (Hugging Face only — Civitai has no slug; see `allow_commercial_use`).
    pub license: Option<String>,
    /// The upstream repo this is a quant / fine-tune of, from the
    /// `base_model:<id>` tag — the thread the upgrade-check (6.7) pulls.
    /// Hugging Face only.
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
    /// Civitai's own `nsfw` flag on the model. Always `false` for a source
    /// with no such concept (Hugging Face). The UI defaults every Civitai
    /// search to `SearchQuery::nsfw = false` (excluded) regardless of this
    /// flag; it is still surfaced per-result as defense in depth.
    #[serde(default)]
    pub nsfw: bool,
    /// A representative preview image (Civitai's first sample image on the
    /// primary version). `None` when the source has none (Hugging Face).
    /// The same url as `previews[0].url` when there is one — kept as its own
    /// field because a result row shows exactly one thumbnail until the user
    /// asks for the rest.
    #[serde(default)]
    pub preview_image_url: Option<String>,
    /// The primary version's sample gallery, in the source's own order and
    /// capped at [`MAX_PREVIEWS`]. Empty when the source has none. Nothing is
    /// fetched to build this — these are urls the search response already
    /// carried; the client decides which of them it ever loads.
    #[serde(default)]
    pub previews: Vec<RemotePreview>,
    /// Civitai's `allowCommercialUse` flags (e.g. `["Image", "Sell"]`) —
    /// empty when the source has no such concept. This, not `license`, is
    /// how a Civitai model's commercial terms are surfaced: Civitai has no
    /// licence-slug concept, only these per-use flags.
    #[serde(default)]
    pub allow_commercial_use: Vec<String>,
    /// A source-provided hint at the AIWM [`crate::model::ModelKind`] to
    /// import a file from this model as — Civitai's own `type`
    /// (`"Checkpoint"`, `"LORA"`, …), verbatim. `None` for a source with no
    /// such concept (Hugging Face relies on `format` + tags instead, via the
    /// UI's existing `importTypeFor` guess).
    #[serde(default)]
    pub model_kind_hint: Option<String>,
    /// Which foundation model family this targets — Civitai's `baseModels`
    /// (joined) or its primary version's `baseModel` (e.g. `"SDXL 1.0"`,
    /// `"Pony"`, `"Flux.1 D"`). Distinct from `base_model`: this is a family
    /// label, not an upstream repo id. `None` for a source with no such
    /// concept (Hugging Face).
    #[serde(default)]
    pub base_model_family: Option<String>,
}

/// One downloadable file in a repo revision, with the size and **SHA-256**
/// the download manager verifies against — no download needed. The SHA-256
/// here is only ever what the source itself reports (Hugging Face's
/// `lfs.oid`, Civitai's `hashes.SHA256`) — a pre-download sanity check /
/// dedup key. AIWM's own import pipeline (`download::verify`) always
/// re-hashes the actually-downloaded bytes and treats that as the source of
/// truth; this field is never substituted for that check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFile {
    pub path: String,
    pub size: u64,
    pub sha256: Option<String>,
    /// Quant / precision label (`Q4_K_M`, `F16`, Civitai's `metadata.fp`).
    pub quant: Option<String>,
    /// `Some((index, total))` for a split file `…-00001-of-00003.gguf`.
    /// Hugging Face only — Civitai never shards a file.
    pub shard: Option<(u32, u32)>,
    /// The file's own absolute download URL, when the source hands one back
    /// directly (Civitai's `downloadUrl`). `None` for Hugging Face, whose
    /// `/resolve/<rev>/<path>` URL is instead built by the caller (see
    /// `api::handlers::enrich_file`) from the model id + revision + path.
    #[serde(default)]
    pub download_url: Option<String>,
    /// Civitai's own malware-scan verdict for this file (`"Success"`,
    /// `"Danger"`, `"Pending"`, `"Error"`, …), surfaced as-is. `None` for a
    /// source with no such concept. This is informational only — never a
    /// substitute for AIWM's own Pickle-format import guard
    /// (`model::import::resolve_kind`), which runs unconditionally on every
    /// import regardless of what a source claims.
    #[serde(default)]
    pub pickle_scan_result: Option<String>,
    /// Civitai's own antivirus-scan verdict for this file, surfaced as-is.
    /// `None` for a source with no such concept.
    #[serde(default)]
    pub virus_scan_result: Option<String>,
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
/// Shared by every [`ModelSource`] — a field a given source has no filter for
/// is simply ignored by that source (e.g. Civitai ignores `gguf_only` /
/// `base_model`; Hugging Face ignores `nsfw` / `media_types`).
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub text: Option<String>,
    /// `filter=base_model:<owner/repo>` — every quant / derivative of one base.
    /// Hugging Face only.
    pub base_model: Option<String>,
    /// Restrict to repos carrying the `gguf` tag. Hugging Face only.
    pub gguf_only: bool,
    pub sort: SearchSort,
    /// 1..=100; clamped (each source clamps to its own real ceiling).
    pub limit: u32,
    /// Include NSFW-flagged results. Civitai only; defaults to `false`
    /// (excluded) — the caller (the Discover UI) must explicitly opt in.
    pub nsfw: bool,
    /// Restrict to these Civitai `types` (`"Checkpoint"`, `"LORA"`, …), verbatim
    /// as Civitai spells them. Empty = every type. Civitai only.
    pub media_types: Vec<String>,
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
    /// The host this source talks to, e.g. `"https://civitai.red"` — so the UI
    /// can link a result to the front door the app is actually using instead
    /// of guessing one. Empty for a source with a single fixed host.
    #[serde(default)]
    pub base_url: String,
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
        "{}|{}|{}|{:?}|{}|{}|{}",
        q.text.as_deref().unwrap_or(""),
        q.base_model.as_deref().unwrap_or(""),
        q.gguf_only,
        q.sort,
        q.limit.clamp(1, 100),
        q.nsfw,
        q.media_types.join(","),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn model(id: &str) -> RemoteModel {
        RemoteModel {
            id: id.into(),
            name: None,
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
            nsfw: false,
            preview_image_url: None,
            previews: Vec::new(),
            allow_commercial_use: vec![],
            model_kind_hint: None,
            base_model_family: None,
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
