//! The Hugging Face Hub source (Phase 6.1, ADR-022).
//!
//! Read-only against three endpoints, anonymous by default (500 API calls / 5
//! min / IP is plenty; an optional `HF_TOKEN` lifts that and reaches gated
//! repos):
//!
//! - `GET /api/models?…&expand[]=…` — one call yields param count, context
//!   length, precision, licence and `base_model` without a download.
//! - `GET /api/models/{id}?expand[]=…` — the same, for one repo.
//! - `GET /api/models/{id}/tree/{rev}?recursive=true` — every file with its
//!   size and **SHA-256** (`lfs.oid`; **not** `xetHash`).

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::Value;

use super::{
    Gated, ModelSource, RegistryStatus, RemoteFile, RemoteFormat, RemoteModel, RemoteModelDetails,
    SearchQuery, SearchSort,
};
use crate::db::now_rfc3339;
use crate::{CoreError, Result};

/// When a `429` carries no usable reset hint, back off this long.
const DEFAULT_BACKOFF_SECS: i64 = 90;

/// Live rate-limit / last-fetch bookkeeping (Phase 6.9).
#[derive(Debug, Default)]
struct HubState {
    last_fetch: Option<String>,
    remaining: Option<i64>,
    /// Unix seconds; while `now < limited_until` every request fails fast.
    limited_until: Option<i64>,
}

/// The real Hub. Overridden in tests with [`HuggingFaceSource::with_base_url`].
const DEFAULT_BASE: &str = "https://huggingface.co";

/// `expand[]` values requested on every list + detail call.
const EXPAND: &[&str] = &[
    "gguf",
    "safetensors",
    "gated",
    "downloadsAllTime",
    "lastModified",
    "createdAt",
    "trendingScore",
    "cardData",
];

#[derive(Debug)]
pub struct HuggingFaceSource {
    base: String,
    token: Option<String>,
    client: reqwest::Client,
    state: Mutex<HubState>,
}

impl HuggingFaceSource {
    /// Anonymous client against the real Hub.
    pub fn new() -> Result<Self> {
        Self::build(DEFAULT_BASE.to_string(), None)
    }

    /// Point at a fixture / mirror.
    pub fn with_base_url(base: impl Into<String>) -> Result<Self> {
        Self::build(base.into(), None)
    }

    /// Add an `HF_TOKEN` for gated repos / higher limits.
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token.filter(|t| !t.trim().is_empty());
        self
    }

    fn build(base: String, token: Option<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("aiwm/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| CoreError::Config(format!("registry: http client: {e}")))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token,
            client,
            state: Mutex::new(HubState::default()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HubState> {
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

/// Pull `RateLimit-Remaining` and a reset hint out of the response headers. HF
/// sends the IETF draft `RateLimit: "…";r=<remaining>;t=<seconds-to-reset>`, and
/// `Retry-After: <seconds>` on a `429`.
fn parse_rate_headers(headers: &reqwest::header::HeaderMap) -> (Option<i64>, Option<i64>) {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());

    let mut remaining = get("ratelimit-remaining").and_then(|v| v.trim().parse().ok());
    let mut reset_in = None;

    if let Some(rl) = get("ratelimit") {
        for part in rl.split(';') {
            let part = part.trim();
            if let Some(r) = part.strip_prefix("r=") {
                remaining = r.trim().parse().ok().or(remaining);
            } else if let Some(t) = part.strip_prefix("t=") {
                reset_in = t.trim().parse().ok();
            }
        }
    }
    if reset_in.is_none() {
        reset_in = get("retry-after").and_then(|v| v.trim().parse().ok());
    }
    (remaining, reset_in)
}

impl HuggingFaceSource {
    async fn get_json(&self, url: &str, params: &[(&str, String)]) -> Result<Value> {
        // Fast-fail while we know we're rate-limited — don't spend a call.
        if let Some(until) = self.lock().limited_until {
            let wait = until - unix_now();
            if wait > 0 {
                return Err(err(format!(
                    "Hugging Face rate limit — try again in {wait}s"
                )));
            }
        }

        let url = reqwest::Url::parse_with_params(url, params)
            .map_err(|e| err(format!("build url {url}: {e}")))?;
        let mut req = self.client.get(url.clone());
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| err(format!("GET {url}: {e}")))?;
        let status = resp.status();
        let (remaining, reset_in) = parse_rate_headers(resp.headers());

        {
            let mut st = self.lock();
            st.remaining = remaining.or(st.remaining);
            if status.as_u16() == 429 {
                let backoff = reset_in.filter(|s| *s > 0).unwrap_or(DEFAULT_BACKOFF_SECS);
                st.limited_until = Some(unix_now() + backoff);
            } else if status.is_success() {
                st.limited_until = None;
                st.last_fetch = Some(now_rfc3339());
            }
        }

        if status.as_u16() == 429 {
            let wait = reset_in.filter(|s| *s > 0).unwrap_or(DEFAULT_BACKOFF_SECS);
            return Err(err(format!(
                "Hugging Face rate limit hit — backing off for {wait}s"
            )));
        }
        if !status.is_success() {
            return Err(err(format!("Hugging Face returned {status}")));
        }
        resp.json().await.map_err(|e| err(format!("decode: {e}")))
    }
}

#[async_trait]
impl ModelSource for HuggingFaceSource {
    fn id(&self) -> &'static str {
        "huggingface"
    }

    fn status(&self) -> RegistryStatus {
        let st = self.lock();
        let rate_limited_secs = st
            .limited_until
            .map(|until| until - unix_now())
            .filter(|s| *s > 0);
        RegistryStatus {
            source_id: "huggingface".to_string(),
            last_fetch: st.last_fetch.clone(),
            rate_limit_remaining: st.remaining,
            rate_limited_secs,
            token_set: self.token.is_some(),
            cache_entries: 0,
        }
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<RemoteModel>> {
        let (sort, direction) = sort_params(query.sort);
        let mut params: Vec<(&str, String)> = vec![
            ("limit", query.limit.clamp(1, 100).to_string()),
            ("sort", sort.to_string()),
            ("direction", direction.to_string()),
            ("full", "true".to_string()),
        ];
        params.extend(EXPAND.iter().map(|e| ("expand[]", (*e).to_string())));
        if let Some(text) = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            params.push(("search", text.to_string()));
        }
        if query.gguf_only {
            params.push(("filter", "gguf".to_string()));
        }
        if let Some(bm) = query.base_model.as_deref().filter(|b| !b.is_empty()) {
            params.push(("filter", format!("base_model:{bm}")));
        }

        let url = format!("{}/api/models", self.base);
        let body = self.get_json(&url, &params).await?;
        let entries = body
            .as_array()
            .ok_or_else(|| err("search: expected a JSON array"))?;
        Ok(entries.iter().filter_map(parse_model).collect())
    }

    async fn details(&self, id: &str) -> Result<RemoteModelDetails> {
        let id = id.trim().trim_matches('/');
        if id.is_empty() {
            return Err(err("details: empty model id"));
        }
        let expand: Vec<(&str, String)> = EXPAND
            .iter()
            .map(|e| ("expand[]", (*e).to_string()))
            .collect();
        let meta = self
            .get_json(&format!("{}/api/models/{id}", self.base), &expand)
            .await?;
        let model = parse_model(&meta).ok_or_else(|| err("details: no model in the response"))?;
        let revision = meta
            .get("sha")
            .and_then(Value::as_str)
            .unwrap_or("main")
            .to_string();

        let tree = self
            .get_json(
                &format!("{}/api/models/{id}/tree/main", self.base),
                &[("recursive", "true".to_string())],
            )
            .await?;
        Ok(RemoteModelDetails {
            model,
            revision,
            files: parse_tree(&tree),
        })
    }
}

fn err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("registry: {msg}"))
}

// --- pure parsing (unit-tested against canned JSON) ----------------------

/// Known `base_model:` sub-kinds the Hub prefixes onto the id.
const BASE_MODEL_KINDS: &[&str] = &["quantized", "finetune", "adapter", "merge"];

/// `sort` + `direction` query params for a [`SearchSort`].
fn sort_params(sort: SearchSort) -> (&'static str, &'static str) {
    let field = match sort {
        SearchSort::Downloads => "downloads",
        SearchSort::Likes => "likes",
        SearchSort::Trending => "likes7d",
        SearchSort::RecentlyUpdated => "lastModified",
        SearchSort::RecentlyCreated => "createdAt",
    };
    (field, "-1")
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// One entry from `GET /api/models` (or the object from the detail endpoint).
fn parse_model(v: &Value) -> Option<RemoteModel> {
    let id = str_field(v, "id").or_else(|| str_field(v, "modelId"))?;

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

    let card = v.get("cardData");
    let gguf = v.get("gguf");

    let downloads = v
        .get("downloadsAllTime")
        .or_else(|| v.get("downloads"))
        .and_then(Value::as_i64)
        .unwrap_or(0);

    Some(RemoteModel {
        author: str_field(v, "author").or_else(|| id.split('/').next().map(str::to_string)),
        id,
        // Hugging Face's `id` (`owner/repo`) already reads as a title.
        name: None,
        downloads,
        likes: v.get("likes").and_then(Value::as_i64).unwrap_or(0),
        trending_score: v.get("trendingScore").and_then(Value::as_i64),
        created_at: str_field(v, "createdAt"),
        last_modified: str_field(v, "lastModified"),
        pipeline_tag: str_field(v, "pipeline_tag"),
        library_name: str_field(v, "library_name"),
        gated: gated_of(v),
        tags: tags.clone(),
        license: license_from_tags(&tags).or_else(|| card.and_then(|c| str_field(c, "license"))),
        base_model: base_model_from_tags(&tags)
            .or_else(|| card.and_then(|c| str_field(c, "base_model"))),
        param_count: gguf
            .and_then(|g| g.get("total"))
            .or_else(|| v.get("safetensors").and_then(|s| s.get("total")))
            .and_then(Value::as_u64),
        arch: gguf.and_then(|g| str_field(g, "architecture")),
        ctx_max: gguf
            .and_then(|g| g.get("context_length"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        precision: v.get("safetensors").and_then(safetensors_precision),
        format: format_of(&tags, str_field(v, "library_name").as_deref()),
        // Hugging Face has none of these Civitai-only concepts.
        nsfw: false,
        preview_image_url: None,
        allow_commercial_use: Vec::new(),
        model_kind_hint: None,
        base_model_family: None,
    })
}

/// The `tree/{rev}?recursive=true` array → downloadable files.
fn parse_tree(v: &Value) -> Vec<RemoteFile> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("file"))
        .filter_map(|e| {
            let path = str_field(e, "path")?;
            Some(RemoteFile {
                size: e.get("size").and_then(Value::as_u64).unwrap_or(0),
                // The SHA-256 is `lfs.oid` — never the git `oid` or `xetHash`.
                sha256: e
                    .get("lfs")
                    .and_then(|l| str_field(l, "oid"))
                    .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())),
                quant: quant_from_filename(&path),
                shard: shard_from_filename(&path),
                // Hugging Face has no per-file download URL or scan verdicts in
                // this response — the URL is built by the caller from
                // id/revision/path, and Hugging Face runs no malware scan.
                download_url: None,
                pickle_scan_result: None,
                virus_scan_result: None,
                path,
            })
        })
        .collect()
}

/// `Q4_K_M` / `F16` / `IQ4_XS` / `FP8` from a GGUF/safetensors filename.
fn quant_from_filename(name: &str) -> Option<String> {
    let stem = name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(name)
        .to_ascii_lowercase();
    // Drop a `-00001-of-00003` shard suffix before looking at the tail.
    let stem = strip_shard_suffix(&stem);

    // Pass 1: a GGUF quant group (`q4_k_m`, `q8_0`, `iq4_xs`).
    for g in stem.split(['-', '.']) {
        if is_gguf_quant(g) {
            return Some(g.to_ascii_uppercase());
        }
    }
    // Pass 2: a bare precision token (`f16`, `bf16`, `fp8`, `fp8_e4m3fn`).
    for t in stem.split(['-', '.', '_']) {
        match t {
            "f16" | "fp16" => return Some("F16".into()),
            "bf16" => return Some("BF16".into()),
            "f32" | "fp32" => return Some("F32".into()),
            "f8" | "fp8" => return Some("FP8".into()),
            _ => {}
        }
    }
    None
}

fn is_gguf_quant(g: &str) -> bool {
    let rest = g
        .strip_prefix("iq")
        .or_else(|| g.strip_prefix('q'))
        .unwrap_or("");
    let mut chars = rest.chars();
    matches!(chars.next(), Some(d) if d.is_ascii_digit())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn strip_shard_suffix(stem: &str) -> String {
    match shard_bounds(stem) {
        Some((start, _)) => stem[..start].to_string(),
        None => stem.to_string(),
    }
}

/// `(1, 3)` from `…-00001-of-00003(.ext)`.
fn shard_from_filename(name: &str) -> Option<(u32, u32)> {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let (start, _) = shard_bounds(stem)?;
    let tail = &stem[start + 1..]; // drop the leading '-'
    let (a, b) = tail.split_once("-of-")?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// Byte range of a `-<digits>-of-<digits>` suffix within `stem`.
fn shard_bounds(stem: &str) -> Option<(usize, usize)> {
    let of = stem.rfind("-of-")?;
    let after: &str = &stem[of + 4..];
    if after.is_empty() || !after.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let before = &stem[..of];
    let dash = before.rfind('-')?;
    if !before[dash + 1..].bytes().all(|b| b.is_ascii_digit()) || before[dash + 1..].is_empty() {
        return None;
    }
    Some((dash, stem.len()))
}

/// `license:apache-2.0` → `apache-2.0`.
fn license_from_tags(tags: &[String]) -> Option<String> {
    tags.iter()
        .find_map(|t| t.strip_prefix("license:"))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `base_model:Qwen/Qwen2.5-Coder-7B-Instruct` → that id. Prefers the bare form;
/// falls back to stripping a `quantized:` / `finetune:` / … sub-kind.
fn base_model_from_tags(tags: &[String]) -> Option<String> {
    let raw: Vec<&str> = tags
        .iter()
        .filter_map(|t| t.strip_prefix("base_model:"))
        .filter(|s| !s.is_empty())
        .collect();
    raw.iter()
        .find(|s| {
            !BASE_MODEL_KINDS
                .iter()
                .any(|k| s.starts_with(&format!("{k}:")))
        })
        .map(|s| s.to_string())
        .or_else(|| {
            raw.first().map(|s| {
                BASE_MODEL_KINDS
                    .iter()
                    .find_map(|k| s.strip_prefix(&format!("{k}:")))
                    .unwrap_or(s)
                    .to_string()
            })
        })
}

/// Pick a format from the tags / library.
fn format_of(tags: &[String], library: Option<&str>) -> RemoteFormat {
    let has = |t: &str| tags.iter().any(|x| x == t);
    if has("gguf") {
        RemoteFormat::Gguf
    } else if has("safetensors")
        || matches!(
            library,
            Some("transformers" | "diffusers" | "sentence-transformers" | "timm")
        )
    {
        RemoteFormat::Safetensors
    } else {
        RemoteFormat::Other
    }
}

/// The dominant dtype key of a `safetensors.parameters` map, e.g. `BF16`.
fn safetensors_precision(v: &Value) -> Option<String> {
    v.get("parameters")
        .and_then(Value::as_object)?
        .iter()
        .max_by_key(|(_, n)| n.as_u64().unwrap_or(0))
        .map(|(k, _)| k.clone())
}

fn gated_of(v: &Value) -> Gated {
    match v.get("gated") {
        Some(Value::String(s)) if s == "auto" => Gated::Auto,
        Some(Value::String(s)) if s == "manual" => Gated::Manual,
        Some(Value::Bool(true)) => Gated::Manual,
        _ => Gated::No,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderMap;
    use serde_json::json;

    #[test]
    fn parse_rate_headers_reads_the_draft_and_retry_after_forms() {
        let mut h = HeaderMap::new();
        h.insert("ratelimit", "\"api\";r=487;t=142".parse().unwrap());
        assert_eq!(parse_rate_headers(&h), (Some(487), Some(142)));

        let mut h = HeaderMap::new();
        h.insert("ratelimit-remaining", "12".parse().unwrap());
        h.insert("retry-after", "58".parse().unwrap());
        assert_eq!(parse_rate_headers(&h), (Some(12), Some(58)));

        assert_eq!(parse_rate_headers(&HeaderMap::new()), (None, None));
    }

    #[test]
    fn status_reports_the_token_and_a_live_backoff() {
        let src = HuggingFaceSource::with_base_url("http://x").unwrap();
        assert!(!src.status().token_set);
        assert!(src.status().rate_limited_secs.is_none());

        let src = src.with_token(Some("hf_abc".into()));
        assert!(src.status().token_set);

        src.lock().limited_until = Some(unix_now() + 30);
        let s = src.status();
        assert!(s.rate_limited_secs.unwrap() > 25 && s.rate_limited_secs.unwrap() <= 30);
    }

    #[tokio::test]
    async fn a_known_backoff_fails_fast_without_a_call() {
        let src = HuggingFaceSource::with_base_url("http://127.0.0.1:1").unwrap();
        src.lock().limited_until = Some(unix_now() + 60);
        let err = src.search(&SearchQuery::default()).await.unwrap_err();
        assert!(err.to_string().contains("try again in"), "{err}");
    }

    #[test]
    fn quant_from_filename_reads_the_common_labels() {
        let cases = [
            ("qwen2.5-coder-7b-instruct-q4_k_m.gguf", Some("Q4_K_M")),
            ("flux1-dev-Q8_0.gguf", Some("Q8_0")),
            ("model-IQ4_XS.gguf", Some("IQ4_XS")),
            ("llama-3-8b.f16.gguf", Some("F16")),
            ("t5xxl_fp8_e4m3fn.safetensors", Some("FP8")),
            // shard suffix is stripped before the quant is read
            (
                "qwen2.5-coder-7b-q4_k_m-00001-of-00002.gguf",
                Some("Q4_K_M"),
            ),
            ("sd_xl_base_1.0.safetensors", None),
            ("README.md", None),
        ];
        for (name, want) in cases {
            assert_eq!(quant_from_filename(name).as_deref(), want, "{name}");
        }
    }

    #[test]
    fn shard_from_filename_reads_the_split_suffix() {
        assert_eq!(
            shard_from_filename("qwen-q8_0-00001-of-00003.gguf"),
            Some((1, 3))
        );
        assert_eq!(shard_from_filename("x-00002-of-00002.gguf"), Some((2, 2)));
        assert_eq!(shard_from_filename("plain.gguf"), None);
    }

    #[test]
    fn license_and_base_model_come_from_tags() {
        let tags = vec![
            "gguf".to_string(),
            "license:apache-2.0".to_string(),
            "base_model:Qwen/Qwen2.5-Coder-7B-Instruct".to_string(),
            "base_model:quantized:Qwen/Qwen2.5-Coder-7B-Instruct".to_string(),
        ];
        assert_eq!(license_from_tags(&tags).as_deref(), Some("apache-2.0"));
        assert_eq!(
            base_model_from_tags(&tags).as_deref(),
            Some("Qwen/Qwen2.5-Coder-7B-Instruct")
        );

        // Only the prefixed form present → strip the prefix.
        let only_prefixed = vec!["base_model:quantized:meta-llama/Llama-3.1-8B".to_string()];
        assert_eq!(
            base_model_from_tags(&only_prefixed).as_deref(),
            Some("meta-llama/Llama-3.1-8B")
        );
        assert_eq!(license_from_tags(&["gguf".to_string()]), None);
        assert_eq!(base_model_from_tags(&[]), None);
    }

    #[test]
    fn format_of_prefers_gguf_then_safetensors() {
        assert_eq!(
            format_of(
                &["transformers".into(), "gguf".into()],
                Some("transformers")
            ),
            RemoteFormat::Gguf
        );
        assert_eq!(
            format_of(&["diffusers".into()], Some("diffusers")),
            RemoteFormat::Safetensors
        );
        assert_eq!(
            format_of(&["onnx".into()], Some("onnx")),
            RemoteFormat::Other
        );
    }

    #[test]
    fn safetensors_precision_picks_the_dominant_dtype() {
        assert_eq!(
            safetensors_precision(
                &json!({ "parameters": { "BF16": 7_000_000 }, "total": 7_000_000 })
            )
            .as_deref(),
            Some("BF16")
        );
        assert_eq!(
            safetensors_precision(&json!({ "parameters": { "F16": 90, "F32": 10 } })).as_deref(),
            Some("F16")
        );
        assert_eq!(safetensors_precision(&json!({})), None);
    }

    #[test]
    fn gated_of_maps_the_three_states() {
        assert_eq!(gated_of(&json!({ "gated": false })), Gated::No);
        assert_eq!(gated_of(&json!({ "gated": "auto" })), Gated::Auto);
        assert_eq!(gated_of(&json!({ "gated": "manual" })), Gated::Manual);
        assert_eq!(gated_of(&json!({})), Gated::No);
    }

    #[test]
    fn sort_params_maps_every_variant() {
        assert_eq!(sort_params(SearchSort::Downloads), ("downloads", "-1"));
        assert_eq!(sort_params(SearchSort::Likes), ("likes", "-1"));
        assert_eq!(sort_params(SearchSort::Trending), ("likes7d", "-1"));
        assert_eq!(
            sort_params(SearchSort::RecentlyUpdated),
            ("lastModified", "-1")
        );
        assert_eq!(
            sort_params(SearchSort::RecentlyCreated),
            ("createdAt", "-1")
        );
    }

    /// A trimmed real `GET /api/models?expand[]=…` entry.
    fn list_entry() -> Value {
        json!({
            "id": "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF",
            "author": "Qwen",
            "downloads": 256_578,
            "downloadsAllTime": 1_780_676,
            "likes": 436,
            "trendingScore": 12,
            "private": false,
            "gated": false,
            "createdAt": "2024-09-18T11:40:39.000Z",
            "lastModified": "2024-11-12T07:59:32.000Z",
            "pipeline_tag": "text-generation",
            "library_name": "transformers",
            "tags": [
                "transformers", "gguf", "code", "text-generation",
                "base_model:Qwen/Qwen2.5-Coder-7B-Instruct",
                "base_model:quantized:Qwen/Qwen2.5-Coder-7B-Instruct",
                "license:apache-2.0"
            ],
            "gguf": {
                "total": 7_615_616_512_u64,
                "architecture": "qwen2",
                "context_length": 131_072
            }
        })
    }

    #[test]
    fn parse_model_pulls_the_expanded_fields() {
        let m = parse_model(&list_entry()).unwrap();
        assert_eq!(m.id, "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF");
        assert_eq!(m.author.as_deref(), Some("Qwen"));
        assert_eq!(m.downloads, 1_780_676, "prefers downloadsAllTime");
        assert_eq!(m.likes, 436);
        assert_eq!(m.trending_score, Some(12));
        assert_eq!(m.gated, Gated::No);
        assert_eq!(m.license.as_deref(), Some("apache-2.0"));
        assert_eq!(
            m.base_model.as_deref(),
            Some("Qwen/Qwen2.5-Coder-7B-Instruct")
        );
        assert_eq!(m.param_count, Some(7_615_616_512));
        assert_eq!(m.arch.as_deref(), Some("qwen2"));
        assert_eq!(m.ctx_max, Some(131_072));
        assert_eq!(m.format, RemoteFormat::Gguf);
        assert_eq!(m.last_modified.as_deref(), Some("2024-11-12T07:59:32.000Z"));
    }

    #[test]
    fn parse_model_needs_at_least_an_id() {
        assert!(parse_model(&json!({ "downloads": 5 })).is_none());
    }

    /// A trimmed real `tree/{rev}?recursive=true` array.
    fn tree() -> Value {
        json!([
            { "type": "directory", "path": "subdir" },
            {
                "type": "file", "path": "config.json", "size": 800,
                "oid": "abc"
            },
            {
                "type": "file",
                "path": "qwen2.5-coder-7b-instruct-q4_k_m.gguf",
                "size": 4_683_073_536_u64,
                "oid": "2543d9eb4f39c19ea6b07e3854b561e96921c654",
                "lfs": {
                    "oid": "509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c",
                    "size": 4_683_073_536_u64, "pointerSize": 135
                },
                "xetHash": "DO_NOT_USE"
            },
            {
                "type": "file",
                "path": "qwen2.5-coder-7b-instruct-q8_0-00001-of-00002.gguf",
                "size": 3_980_069_280_u64,
                "lfs": { "oid": "aa".repeat(32), "size": 3_980_069_280_u64 }
            }
        ])
    }

    #[test]
    fn parse_tree_keeps_files_with_sha256_quant_and_shards() {
        let files = parse_tree(&tree());
        assert_eq!(files.len(), 3, "the directory entry is dropped");

        let gguf = files
            .iter()
            .find(|f| f.path.ends_with("q4_k_m.gguf"))
            .unwrap();
        assert_eq!(
            gguf.sha256.as_deref(),
            Some("509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c"),
            "sha256 comes from lfs.oid, never xetHash"
        );
        assert_eq!(gguf.size, 4_683_073_536);
        assert_eq!(gguf.quant.as_deref(), Some("Q4_K_M"));
        assert_eq!(gguf.shard, None);

        let shard = files.iter().find(|f| f.shard.is_some()).unwrap();
        assert_eq!(shard.shard, Some((1, 2)));
        assert_eq!(shard.quant.as_deref(), Some("Q8_0"));

        // A plain git file with no LFS pointer still lists, just without a hash.
        let cfg = files.iter().find(|f| f.path == "config.json").unwrap();
        assert_eq!(cfg.sha256, None);
    }
}
