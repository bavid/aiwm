//! The Civitai source (civitai.com) — image/video checkpoints and LoRAs
//! (Phase 6.1 extension).
//!
//! Read-only against two endpoints, anonymous by default (public browsing
//! needs no key; an optional API key — the `CIVITAI_TOKEN`-equivalent, stored
//! the same machine-local-file way as Hugging Face's `HF_TOKEN`, see
//! `AppPaths::civitai_token_file` — lifts gated/early-access content the same
//! way `HF_TOKEN` lifts Hugging Face's own gate):
//!
//! - `GET /api/v1/models` — search; the response wraps hits in `items[]`
//!   (verified against a real, unauthenticated request — this is *not* a bare
//!   array the way Hugging Face's `/api/models` is).
//! - `GET /api/v1/models/{id}` — one model, same per-model shape as a search
//!   item, with every `modelVersions[].files[]` entry carrying its own
//!   `hashes`, `downloadUrl`, and — because Civitai is an open, anonymous
//!   upload platform with a real history of malicious uploads, unlike Hugging
//!   Face's more curated ecosystem — its own `pickleScanResult` /
//!   `virusScanResult`.
//!
//! # Security posture (read this before touching the parsing below)
//!
//! Three things this module deliberately does **not** do, each backed by a
//! real, documented reason:
//!
//! 1. **It never marks a Pickle-format file safe to auto-estimate.**
//!    [`format_of_version`] reports [`RemoteFormat::Safetensors`] only when
//!    *every* real weight file (`type: "Model"` / `"Pruned Model"`) in a
//!    version has `metadata.format == "SafeTensor"`; anything else —
//!    `"PickleTensor"`, `"Other"`, or missing — falls back to
//!    [`RemoteFormat::Other`]. That only affects the cosmetic VRAM-fit
//!    estimate the Discover UI shows, though: the *real* enforcement is
//!    `model::import::resolve_kind`'s Pickle-format guard, which runs
//!    unconditionally on every import, driven by the actual file extension on
//!    disk after download — never by what this module or Civitai's own
//!    `pickleScanResult` claims. There is no code path here that could bypass
//!    that guard.
//! 2. **It never treats Civitai's reported hash as verified.** `hashes.SHA256`
//!    is surfaced on [`RemoteFile::sha256`] purely as a pre-download sanity
//!    check / dedup key — the same role `lfs.oid` plays for Hugging Face.
//!    AIWM's own download manager (`download::verify`) always re-hashes the
//!    bytes it actually received and treats *that* as the source of truth;
//!    nothing in this module or its callers substitutes Civitai's claim for
//!    that check.
//! 3. **It never defaults to showing NSFW content.** [`SearchQuery::nsfw`]
//!    must be explicitly set `true` by the caller (the Discover UI's opt-in
//!    toggle) — [`CivitaiSource::search`] always sends an explicit
//!    `nsfw=<bool>` query parameter (verified against a real request; Civitai
//!    honours it), never omitting it and relying on whatever an anonymous
//!    request's own implicit default happens to be. A model's own `nsfw` flag
//!    is still copied onto [`RemoteModel::nsfw`] regardless, as defense in
//!    depth for the rare mislabelled result.

mod checkpoints;

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::Value;

use super::{
    CheckpointCandidate, Gated, ModelSource, RegistryStatus, RemoteFile, RemoteFormat, RemoteModel,
    RemoteModelDetails, RemotePreview, RemoteVersion, SearchQuery, SearchSort, MAX_PREVIEWS,
};
use crate::config::CivitaiFrontDoor;
use crate::db::now_rfc3339;
use crate::{CoreError, Result};

/// When a `429` carries no usable `Retry-After`, back off this long. Civitai
/// documents no IETF-draft `RateLimit` header the way Hugging Face does, so
/// this is the one thing we fall back to rather than inventing a header
/// contract Civitai doesn't publish.
const DEFAULT_BACKOFF_SECS: i64 = 60;

/// The real service. Overridden in tests with [`CivitaiSource::with_base_url`].
const DEFAULT_BASE: &str = "https://civitai.com";

/// Live rate-limit / last-fetch bookkeeping.
#[derive(Debug, Default)]
struct CivitaiState {
    last_fetch: Option<String>,
    /// Unix seconds; while `now < limited_until` every request fails fast.
    limited_until: Option<i64>,
}

#[derive(Debug)]
pub struct CivitaiSource {
    base: String,
    token: Option<String>,
    client: reqwest::Client,
    state: Mutex<CivitaiState>,
}

impl CivitaiSource {
    /// Anonymous client against the real service.
    pub fn new() -> Result<Self> {
        Self::build(DEFAULT_BASE.to_string(), None)
    }

    /// Point at a fixture (tests) or any other base URL.
    pub fn with_base_url(base: impl Into<String>) -> Result<Self> {
        Self::build(base.into(), None)
    }

    /// The real service on the configured front door — `civitai.com` (the
    /// safe-for-work catalogue) or `civitai.red` (Civitai's own domain that
    /// also carries adult models). Both serve the same API; the download URLs
    /// a search returns point at whichever host was asked, and what comes back
    /// is still governed by [`SearchQuery::nsfw`].
    pub fn for_front_door(front_door: CivitaiFrontDoor) -> Result<Self> {
        Self::build(front_door.base_url().to_string(), None)
    }

    /// Add an API key for gated/early-access content or higher limits.
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token.filter(|t| !t.trim().is_empty());
        self
    }

    fn build(base: String, token: Option<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("aiwm/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| CoreError::Config(format!("registry: civitai: http client: {e}")))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token,
            client,
            state: Mutex::new(CivitaiState::default()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CivitaiState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("registry: civitai: {msg}"))
}

impl CivitaiSource {
    async fn get_json(&self, path: &str, params: &[(&str, String)]) -> Result<Value> {
        // Fast-fail while we know we're rate-limited — don't spend a call.
        if let Some(until) = self.lock().limited_until {
            let wait = until - unix_now();
            if wait > 0 {
                return Err(err(format!("Civitai rate limit — try again in {wait}s")));
            }
        }

        let url = format!("{}{path}", self.base);
        let url = reqwest::Url::parse_with_params(&url, params)
            .map_err(|e| err(format!("build url {url}: {e}")))?;
        let mut req = self.client.get(url.clone());
        // The conventional REST bearer form, confirmed against a real
        // (unauthenticated) request plus Civitai's own download-API guide —
        // the same shape `HuggingFaceSource::get_json` uses for `HF_TOKEN`.
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| err(format!("GET {url}: {e}")))?;
        let status = resp.status();

        if status.as_u16() == 429 {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<i64>().ok());
            let backoff = retry_after
                .filter(|s| *s > 0)
                .unwrap_or(DEFAULT_BACKOFF_SECS);
            self.lock().limited_until = Some(unix_now() + backoff);
            return Err(err(format!(
                "Civitai rate limit hit — backing off for {backoff}s"
            )));
        }
        if !status.is_success() {
            return Err(err(format!("Civitai returned {status}")));
        }
        self.lock().last_fetch = Some(now_rfc3339());
        resp.json().await.map_err(|e| err(format!("decode: {e}")))
    }
}

#[async_trait]
impl ModelSource for CivitaiSource {
    fn id(&self) -> &'static str {
        "civitai"
    }

    fn status(&self) -> RegistryStatus {
        let st = self.lock();
        let rate_limited_secs = st
            .limited_until
            .map(|until| until - unix_now())
            .filter(|s| *s > 0);
        RegistryStatus {
            source_id: "civitai".to_string(),
            last_fetch: st.last_fetch.clone(),
            // Civitai documents no rate-limit-remaining header contract the
            // way Hugging Face's IETF-draft `RateLimit` header does — leave
            // this unset rather than fabricating a number.
            rate_limit_remaining: None,
            rate_limited_secs,
            token_set: self.token.is_some(),
            cache_entries: 0,
            base_url: self.base.clone(),
        }
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<RemoteModel>> {
        let mut params: Vec<(&str, String)> = vec![
            ("limit", query.limit.clamp(1, 200).to_string()),
            // Always explicit — see the module doc's NSFW-default note.
            ("nsfw", query.nsfw.to_string()),
        ];
        if let Some(text) = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            params.push(("query", text.to_string()));
        }
        for t in &query.media_types {
            let t = t.trim();
            if !t.is_empty() {
                params.push(("types", t.to_string()));
            }
        }
        if let Some(sort) = sort_param(query.sort) {
            params.push(("sort", sort.to_string()));
        }
        if let Some(period) = period_param(query.sort) {
            params.push(("period", period.to_string()));
        }

        let body = self.get_json("/api/v1/models", &params).await?;
        let items = body
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| err("search: expected an `items` array"))?;
        Ok(items.iter().filter_map(parse_model_summary).collect())
    }

    async fn details(&self, id: &str) -> Result<RemoteModelDetails> {
        let id = id.trim();
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err(format!("details: {id:?} is not a numeric model id")));
        }
        let body = self.get_json(&format!("/api/v1/models/{id}"), &[]).await?;
        let model =
            parse_model_summary(&body).ok_or_else(|| err("details: no model in the response"))?;

        // Only the primary (first-listed) version's files are surfaced — a
        // documented MVP simplification, the same shape Hugging Face's own
        // `details` has (it only ever shows the `main` revision's tree, not
        // every historical revision).
        let versions = body.get("modelVersions").and_then(Value::as_array);
        let primary = versions.and_then(|a| a.first());
        let revision = primary
            .and_then(|v| v.get("id"))
            .and_then(Value::as_u64)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let files = primary.map(parse_files).unwrap_or_default();

        Ok(RemoteModelDetails {
            model,
            revision,
            files,
            versions: versions.map(|a| parse_versions(a)).unwrap_or_default(),
        })
    }

    async fn checkpoints_for_base(
        &self,
        base_label: &str,
        nsfw: bool,
        limit: usize,
    ) -> Result<Vec<CheckpointCandidate>> {
        checkpoints::search(self, base_label, nsfw, limit).await
    }
}

/// Every listed version with the base it targets.
fn parse_versions(versions: &[Value]) -> Vec<RemoteVersion> {
    versions
        .iter()
        .filter_map(|v| {
            Some(RemoteVersion {
                id: v.get("id").and_then(Value::as_u64)?.to_string(),
                name: str_field(v, "name"),
                base_model: str_field(v, "baseModel"),
            })
        })
        .collect()
}

// --- pure parsing (unit-tested against canned JSON) ----------------------

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// `sort` + `period` query params for a [`SearchSort`]. Civitai's public API
/// has no distinct "recently updated" sort separate from "Newest", and no
/// distinct "trending" sort at all — [`period_param`] approximates trending
/// as "most downloaded this week" rather than silently aliasing it to
/// all-time downloads.
fn sort_param(sort: SearchSort) -> Option<&'static str> {
    Some(match sort {
        SearchSort::Downloads | SearchSort::Trending => "Most Downloaded",
        SearchSort::Likes => "Highest Rated",
        SearchSort::RecentlyCreated | SearchSort::RecentlyUpdated => "Newest",
    })
}

fn period_param(sort: SearchSort) -> Option<&'static str> {
    matches!(sort, SearchSort::Trending).then_some("Week")
}

/// One entry from `GET /api/v1/models`'s `items[]`, or the equivalent object
/// from `GET /api/v1/models/{id}` (same per-model shape, unwrapped).
fn parse_model_summary(v: &Value) -> Option<RemoteModel> {
    let id = v.get("id").and_then(Value::as_u64)?.to_string();
    let versions = v.get("modelVersions").and_then(Value::as_array);
    let primary = versions.and_then(|a| a.first());
    let previews = parse_previews(primary);

    let tags: Vec<String> = v
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let allow_commercial_use: Vec<String> = v
        .get("allowCommercialUse")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    Some(RemoteModel {
        author: v.get("creator").and_then(|c| str_field(c, "username")),
        id,
        name: str_field(v, "name"),
        downloads: v
            .get("stats")
            .and_then(|s| s.get("downloadCount"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        // `thumbsUpCount` is the closest analogue to Hugging Face's "likes" —
        // Civitai's stats object has no separate "favorite" count in the
        // current API.
        likes: v
            .get("stats")
            .and_then(|s| s.get("thumbsUpCount"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        trending_score: None,
        // Civitai exposes no separate "model created" timestamp at this
        // granularity — only a per-version `publishedAt` — so this stays
        // unset rather than reusing that value under the wrong label.
        created_at: None,
        last_modified: primary.and_then(|p| str_field(p, "publishedAt")),
        pipeline_tag: None,
        library_name: None,
        // Civitai's public browsing has no Hugging-Face-style access gate;
        // `allow_commercial_use` is the real licence signal here.
        gated: Gated::No,
        license: None,
        base_model: None,
        tags,
        param_count: None,
        arch: None,
        ctx_max: None,
        precision: None,
        format: primary.map(format_of_version).unwrap_or_default(),
        nsfw: v.get("nsfw").and_then(Value::as_bool).unwrap_or(false),
        preview_image_url: previews.first().map(|p| p.url.clone()),
        previews,
        allow_commercial_use,
        model_kind_hint: str_field(v, "type"),
        base_model_family: base_model_family_of(v, primary),
    })
}

/// The primary version's sample gallery, capped at [`MAX_PREVIEWS`].
///
/// An entry without a `url` is dropped rather than carried as an empty string:
/// the client would render a broken tile for it. `type` is Civitai's own
/// `"image"` / `"video"`; anything else is treated as an image, which is the
/// harmless direction (an `<img>` that fails to load shows nothing, whereas a
/// `<video>` element for a still would show a dead player).
fn parse_previews(primary: Option<&Value>) -> Vec<RemotePreview> {
    primary
        .and_then(|p| p.get("images"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|img| {
                    Some(RemotePreview {
                        url: str_field(img, "url")?,
                        is_video: str_field(img, "type").as_deref() == Some("video"),
                        nsfw_level: img.get("nsfwLevel").and_then(Value::as_i64).unwrap_or(0),
                    })
                })
                .take(MAX_PREVIEWS)
                .collect()
        })
        .unwrap_or_default()
}

/// The model-level `baseModels` facet (joined), falling back to the primary
/// version's own singular `baseModel` when the aggregated array is absent.
fn base_model_family_of(v: &Value, primary: Option<&Value>) -> Option<String> {
    let families: Vec<String> = v
        .get("baseModels")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if !families.is_empty() {
        return Some(families.join(", "));
    }
    primary.and_then(|p| str_field(p, "baseModel"))
}

/// [`RemoteFormat::Safetensors`] only when *every* real weight file
/// (`type: "Model"` / `"Pruned Model"` — not a VAE/config/training-data
/// sidecar) in this version is safetensors-format. A version with no weight
/// file, or with even one Pickle/unknown-format weight file, reports
/// [`RemoteFormat::Other`] — this only ever affects the cosmetic VRAM-fit
/// estimate, never the real import-time Pickle guard (see the module doc).
fn format_of_version(version: &Value) -> RemoteFormat {
    let files = version.get("files").and_then(Value::as_array);
    let weight_files: Vec<&Value> = files
        .into_iter()
        .flatten()
        .filter(|f| {
            matches!(
                str_field(f, "type").as_deref(),
                Some("Model" | "Pruned Model")
            )
        })
        .collect();
    if !weight_files.is_empty() && weight_files.iter().all(|f| is_safetensor(f)) {
        RemoteFormat::Safetensors
    } else {
        RemoteFormat::Other
    }
}

fn is_safetensor(file: &Value) -> bool {
    file.get("metadata")
        .and_then(|m| m.get("format"))
        .and_then(Value::as_str)
        == Some("SafeTensor")
}

/// `modelVersions[N].files[]` → downloadable files, each carrying Civitai's
/// own hash (a pre-download sanity check only — see the module doc) and its
/// own malware-scan verdicts, surfaced verbatim so the UI can show them
/// prominently rather than hiding a non-`"Success"` result.
fn parse_files(version: &Value) -> Vec<RemoteFile> {
    version
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let path = str_field(f, "name")?;
            let size_kb = f.get("sizeKB").and_then(Value::as_f64).unwrap_or(0.0);
            Some(RemoteFile {
                size: (size_kb.max(0.0) * 1024.0).round() as u64,
                sha256: f
                    .get("hashes")
                    .and_then(|h| str_field(h, "SHA256"))
                    .map(|s| s.to_ascii_lowercase())
                    .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())),
                quant: f
                    .get("metadata")
                    .and_then(|m| str_field(m, "fp"))
                    .map(|fp| normalize_precision(&fp)),
                // Civitai never splits one file into numbered shards.
                shard: None,
                download_url: str_field(f, "downloadUrl"),
                pickle_scan_result: str_field(f, "pickleScanResult"),
                virus_scan_result: str_field(f, "virusScanResult"),
                path,
            })
        })
        .collect()
}

/// `fp16` → `F16`, matching the label shape Hugging Face's own
/// `quant_from_filename` already produces, so the UI's one "quant" column
/// reads consistently across sources.
fn normalize_precision(fp: &str) -> String {
    match fp.to_ascii_lowercase().as_str() {
        "fp16" | "f16" => "F16".to_string(),
        "bf16" => "BF16".to_string(),
        "fp32" | "f32" => "F32".to_string(),
        "fp8" | "f8" => "FP8".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_reports_the_token_and_a_live_backoff() {
        let src = CivitaiSource::with_base_url("http://x").unwrap();
        assert!(!src.status().token_set);
        assert!(src.status().rate_limited_secs.is_none());

        let src = src.with_token(Some("civ_abc".into()));
        assert!(src.status().token_set);

        src.lock().limited_until = Some(unix_now() + 30);
        let s = src.status();
        assert!(s.rate_limited_secs.unwrap() > 25 && s.rate_limited_secs.unwrap() <= 30);
    }

    #[tokio::test]
    async fn a_known_backoff_fails_fast_without_a_call() {
        let src = CivitaiSource::with_base_url("http://127.0.0.1:1").unwrap();
        src.lock().limited_until = Some(unix_now() + 60);
        let err = src.search(&SearchQuery::default()).await.unwrap_err();
        assert!(err.to_string().contains("try again in"), "{err}");
    }

    #[tokio::test]
    async fn details_rejects_a_non_numeric_id() {
        let src = CivitaiSource::with_base_url("http://127.0.0.1:1").unwrap();
        let err = src.details("not-a-number").await.unwrap_err();
        assert!(err.to_string().contains("numeric"), "{err}");
    }

    #[test]
    fn sort_and_period_map_every_variant() {
        assert_eq!(sort_param(SearchSort::Downloads), Some("Most Downloaded"));
        assert_eq!(sort_param(SearchSort::Likes), Some("Highest Rated"));
        assert_eq!(sort_param(SearchSort::Trending), Some("Most Downloaded"));
        assert_eq!(sort_param(SearchSort::RecentlyCreated), Some("Newest"));
        assert_eq!(sort_param(SearchSort::RecentlyUpdated), Some("Newest"));

        assert_eq!(period_param(SearchSort::Trending), Some("Week"));
        assert_eq!(period_param(SearchSort::Downloads), None);
    }

    /// A trimmed real (anonymous, unauthenticated) `GET /api/v1/models` item —
    /// field names, casing and nesting verified against a live request.
    fn checkpoint_item() -> Value {
        json!({
            "id": 257_749,
            "name": "Pony Diffusion V6 XL",
            "type": "Checkpoint",
            "nsfw": false,
            "allowCommercialUse": ["Image", "RentCivit"],
            "baseModels": ["Pony", "SD 1.5"],
            "tags": ["western art", "base model"],
            "creator": { "username": "AstraliteHeart", "image": null },
            "stats": { "downloadCount": 1_200_000, "thumbsUpCount": 34_000, "thumbsDownCount": 100, "commentCount": 900 },
            "modelVersions": [
                {
                    "id": 290_640,
                    "baseModel": "Pony",
                    "publishedAt": "2023-07-18T00:00:00.000Z",
                    "images": [{ "url": "https://image.civitai.com/xyz/preview.jpeg" }],
                    "downloadUrl": "https://civitai.com/api/download/models/290640",
                    "files": [
                        {
                            "name": "ponyDiffusionV6XL_v6StartWithThisOne.safetensors",
                            "sizeKB": 6_617_170.5,
                            "type": "Model",
                            "pickleScanResult": "Success",
                            "virusScanResult": "Success",
                            "metadata": { "format": "SafeTensor", "size": "pruned", "fp": "fp16" },
                            "hashes": { "SHA256": "67AB2FD8EC439A89B3FEDB15CC65F54336AF163C7EB5E4F2ACC98F090A29B0B3" },
                            "downloadUrl": "https://civitai.com/api/download/models/290640"
                        },
                        {
                            "name": "sdxl_vae.safetensors",
                            "sizeKB": 334_643.0,
                            "type": "VAE",
                            "pickleScanResult": "Success",
                            "virusScanResult": "Success",
                            "metadata": { "format": "SafeTensor", "size": null, "fp": null },
                            "hashes": { "SHA256": "235745af8d86bf4a4c1b5b4f529868b37019a10f7c0b2e79ad0abca3a22bc6e1" },
                            "downloadUrl": "https://civitai.com/api/download/models/290640?type=VAE&format=SafeTensor"
                        }
                    ]
                }
            ]
        })
    }

    #[test]
    fn parse_model_summary_reads_the_real_field_shape() {
        let m = parse_model_summary(&checkpoint_item()).unwrap();
        assert_eq!(m.id, "257749");
        assert_eq!(m.name.as_deref(), Some("Pony Diffusion V6 XL"));
        assert_eq!(m.author.as_deref(), Some("AstraliteHeart"));
        assert_eq!(m.downloads, 1_200_000);
        assert_eq!(m.likes, 34_000);
        assert!(!m.nsfw);
        assert_eq!(m.model_kind_hint.as_deref(), Some("Checkpoint"));
        assert_eq!(m.base_model_family.as_deref(), Some("Pony, SD 1.5"));
        assert_eq!(m.allow_commercial_use, vec!["Image", "RentCivit"]);
        assert_eq!(m.tags, vec!["western art", "base model"]);
        assert_eq!(
            m.preview_image_url.as_deref(),
            Some("https://image.civitai.com/xyz/preview.jpeg")
        );
        assert_eq!(m.last_modified.as_deref(), Some("2023-07-18T00:00:00.000Z"));
        assert_eq!(m.format, RemoteFormat::Safetensors);
        assert_eq!(m.gated, Gated::No);
        // Concepts Civitai has none of must stay unset, never fabricated.
        assert_eq!(m.license, None);
        assert_eq!(m.base_model, None);
        assert_eq!(m.created_at, None);
        assert_eq!(m.param_count, None);
    }

    #[test]
    fn parse_model_summary_needs_at_least_a_numeric_id() {
        assert!(parse_model_summary(&json!({ "name": "no id" })).is_none());
    }

    #[test]
    fn base_model_family_falls_back_to_the_primary_versions_singular_field() {
        let v = json!({
            "id": 1,
            "modelVersions": [{ "id": 1, "baseModel": "Flux.1 D" }]
        });
        let m = parse_model_summary(&v).unwrap();
        assert_eq!(m.base_model_family.as_deref(), Some("Flux.1 D"));
    }

    #[test]
    fn format_of_version_is_safetensors_only_when_every_weight_file_is() {
        let all_safe = checkpoint_item()["modelVersions"][0].clone();
        assert_eq!(format_of_version(&all_safe), RemoteFormat::Safetensors);

        let mut mixed = all_safe.clone();
        mixed["files"][0]["metadata"]["format"] = json!("PickleTensor");
        assert_eq!(
            format_of_version(&mixed),
            RemoteFormat::Other,
            "one pickle-format weight file must never report as safetensors"
        );

        let no_weight_files =
            json!({ "files": [{ "type": "Config", "metadata": { "format": "SafeTensor" } }] });
        assert_eq!(format_of_version(&no_weight_files), RemoteFormat::Other);

        let missing_metadata = json!({ "files": [{ "type": "Model" }] });
        assert_eq!(format_of_version(&missing_metadata), RemoteFormat::Other);
    }

    #[test]
    fn parse_files_keeps_the_hash_scan_and_download_url_verbatim() {
        let version = &checkpoint_item()["modelVersions"][0];
        let files = parse_files(version);
        assert_eq!(files.len(), 2);

        let model_file = files
            .iter()
            .find(|f| f.path.ends_with("StartWithThisOne.safetensors"))
            .unwrap();
        assert_eq!(
            model_file.sha256.as_deref(),
            Some("67ab2fd8ec439a89b3fedb15cc65f54336af163c7eb5e4f2acc98f090a29b0b3"),
            "sha256 is lowercased for consistency, but is still just Civitai's own claim"
        );
        assert_eq!(model_file.size, 6_775_982_592); // 6_617_170.5 KiB, rounded
        assert_eq!(model_file.quant.as_deref(), Some("F16"));
        assert_eq!(model_file.shard, None);
        assert_eq!(
            model_file.download_url.as_deref(),
            Some("https://civitai.com/api/download/models/290640")
        );
        assert_eq!(model_file.pickle_scan_result.as_deref(), Some("Success"));
        assert_eq!(model_file.virus_scan_result.as_deref(), Some("Success"));

        let vae_file = files
            .iter()
            .find(|f| f.path == "sdxl_vae.safetensors")
            .unwrap();
        assert_eq!(vae_file.quant, None, "no fp on a file with no metadata.fp");
    }

    #[test]
    fn parse_files_drops_a_malformed_hash_instead_of_passing_it_through() {
        let version = json!({
            "files": [{
                "name": "bad-hash.safetensors",
                "sizeKB": 10.0,
                "hashes": { "SHA256": "not-64-hex-chars" },
                "downloadUrl": "https://civitai.com/api/download/models/1"
            }]
        });
        let files = parse_files(&version);
        assert_eq!(files[0].sha256, None);
    }

    #[test]
    fn normalize_precision_matches_hugging_faces_label_shape() {
        assert_eq!(normalize_precision("fp16"), "F16");
        assert_eq!(normalize_precision("bf16"), "BF16");
        assert_eq!(normalize_precision("fp32"), "F32");
        assert_eq!(normalize_precision("fp8"), "FP8");
        assert_eq!(normalize_precision("nf4"), "NF4");
    }

    #[test]
    fn the_gallery_carries_every_sample_capped_with_its_kind_and_rating() {
        // Shaped like a real response (verified 2026-09-20: a popular model's
        // primary version carries 15–20 images, `type` is "image" or "video",
        // `nsfwLevel` 1 = safe).
        let images: Vec<Value> = (0..MAX_PREVIEWS + 5)
            .map(|i| {
                serde_json::json!({
                    "url": format!("https://image.civitai.com/x/{i}.jpeg"),
                    "type": if i == 1 { "video" } else { "image" },
                    "nsfwLevel": if i == 2 { 4 } else { 1 },
                })
            })
            .collect();
        let primary = serde_json::json!({ "images": images });

        let previews = parse_previews(Some(&primary));

        assert_eq!(previews.len(), MAX_PREVIEWS, "capped, in order");
        assert_eq!(previews[0].url, "https://image.civitai.com/x/0.jpeg");
        assert!(!previews[0].is_video);
        assert!(
            previews[1].is_video,
            "a video sample is flagged, not dropped"
        );
        assert_eq!(previews[2].nsfw_level, 4, "the per-image rating is carried");
    }

    #[test]
    fn a_sample_without_a_url_is_dropped_and_a_model_without_images_has_none() {
        let primary = serde_json::json!({
            "images": [
                { "type": "image" },
                { "url": "https://image.civitai.com/x/ok.jpeg" },
            ]
        });

        let previews = parse_previews(Some(&primary));

        assert_eq!(previews.len(), 1, "the entry with no url is dropped");
        assert_eq!(previews[0].url, "https://image.civitai.com/x/ok.jpeg");
        assert_eq!(
            previews[0].nsfw_level, 0,
            "an absent rating is not invented"
        );
        assert!(parse_previews(None).is_empty());
        assert!(parse_previews(Some(&serde_json::json!({}))).is_empty());
    }
}
