//! `core::recommend` — tag/vibe-based model discovery: "given this free-text
//! description, what should I install?"
//!
//! Sibling to [`crate::upgrade`] (Phase 6.7), reusing the same shape and the
//! same anti-hallucination discipline, but framed as a fresh suggestion from
//! a description rather than "is there something better than what I already
//! have": search Hugging Face for the free text → drop spam and anything
//! that will not fit this machine → rank the survivors by objective signals
//! (recency, downloads/likes) → let the local LLM re-order the top few and
//! write a one-line reason each, **validating every id it returns against
//! the real candidate list** so it cannot invent a result. The LLM step is
//! best-effort: a bad answer falls back to the objective order.
//!
//! "Recommended" is never a quality claim (see `crate::upgrade`'s ADR-024 /
//! ADR-025 note) — only "matches what you described, is real, and fits."

use std::collections::BTreeSet;

use serde::Serialize;

use crate::compat::{self, FitVerdict};
use crate::registry::{Freshness, Registry, RemoteFormat, RemoteModel, SearchQuery, SearchSort};
use crate::upgrade::{fit_of, looks_like_spam, Reasoner};
use crate::{CoreError, Result};

/// Candidates kept for the LLM prompt / the final report.
const SHORTLIST: usize = 8;
/// Per HF search call.
const SEARCH_LIMIT: u32 = 40;
/// Max tags carried into the LLM prompt per candidate — enough to signal
/// content/style without bloating a local model's context.
const PROMPT_TAGS: usize = 6;
/// Max tokens for the ranking completion.
const RANK_MAX_TOKENS: i32 = 700;

fn recommend_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("model recommendation: {msg}"))
}

/// What kind of model the user is asking for — drives the Hugging Face
/// search (GGUF-only makes sense for chat/coding, not image/video weights)
/// and whether the VRAM-fit filter applies at all (a LoRA is a few hundred
/// MB; fit is never what would rule one out).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Chat,
    Coding,
    Image,
    Video,
    Lora,
}

impl MediaKind {
    fn gguf_only(self) -> bool {
        matches!(self, MediaKind::Chat | MediaKind::Coding)
    }

    fn checks_fit(self) -> bool {
        !matches!(self, MediaKind::Lora)
    }

    fn label(self) -> &'static str {
        match self {
            MediaKind::Chat => "a chat model",
            MediaKind::Coding => "a coding model",
            MediaKind::Image => "an image generation model",
            MediaKind::Video => "a video generation model",
            MediaKind::Lora => "a LoRA",
        }
    }
}

/// One proposed model.
#[derive(Debug, Clone, Serialize)]
pub struct RecommendCandidate {
    pub id: String,
    /// One sentence — the LLM's reason, or an objective note.
    pub why: String,
    pub downloads: i64,
    pub likes: i64,
    pub last_modified: Option<String>,
    pub param_count: Option<u64>,
    pub format: RemoteFormat,
    pub gated: bool,
    pub tags: Vec<String>,
    pub fit: FitVerdict,
    /// The LLM put this in its ranking (vs. objective-only).
    pub llm_ranked: bool,
}

/// The search's result — written to `jobs.result` as JSON.
#[derive(Debug, Clone, Serialize)]
pub struct RecommendReport {
    pub query: String,
    pub candidates: Vec<RecommendCandidate>,
    /// How the list was ranked + the quality caveat.
    pub note: String,
    pub freshness: Freshness,
}

/// Run the search. `vram_budget_mb` / `free_ram_mb` drive the fit filter for
/// every [`MediaKind`] except [`MediaKind::Lora`], which is never filtered
/// on fit.
pub async fn run(
    registry: &Registry,
    reasoner: &dyn Reasoner,
    query: &str,
    kind: MediaKind,
    vram_budget_mb: u64,
    free_ram_mb: u64,
) -> Result<RecommendReport> {
    let query = query.trim();
    if query.is_empty() {
        return Err(recommend_err("no search text"));
    }

    // Two passes: recency-weighted popularity, then raw downloads. Merge.
    let mut freshness = Freshness::Live;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut pool: Vec<RemoteModel> = Vec::new();
    for sort in [SearchSort::Trending, SearchSort::Downloads] {
        let fetched = registry
            .search(&SearchQuery {
                text: Some(query.to_string()),
                gguf_only: kind.gguf_only(),
                sort,
                limit: SEARCH_LIMIT,
                ..SearchQuery::default()
            })
            .await?;
        freshness = stalest(freshness, fetched.freshness);
        for m in fetched.data {
            if seen.insert(m.id.clone()) {
                pool.push(m);
            }
        }
    }

    let ctx = compat::effective_ctx(None);
    let mut candidates: Vec<RecommendCandidate> = pool
        .into_iter()
        .filter(|m| !looks_like_spam(m))
        .map(|m| {
            let fit = if kind.checks_fit() {
                fit_of(&m, ctx, vram_budget_mb, free_ram_mb)
            } else {
                FitVerdict::Green
            };
            RecommendCandidate {
                why: objective_why(&m),
                id: m.id,
                downloads: m.downloads,
                likes: m.likes,
                last_modified: m.last_modified,
                param_count: m.param_count,
                format: m.format,
                gated: m.gated.is_gated(),
                tags: m.tags,
                fit,
                llm_ranked: false,
            }
        })
        .filter(|c| !matches!(c.fit, FitVerdict::Red { .. }))
        .collect();

    candidates.sort_by(|a, b| objective_score(b).total_cmp(&objective_score(a)));
    candidates.truncate(SHORTLIST);

    if candidates.is_empty() {
        return Ok(RecommendReport {
            query: query.to_string(),
            candidates,
            note: "Nothing on Hugging Face matched that and fit your hardware — \
                   try broader wording, or drop the GGUF-only restriction from Discover."
                .to_string(),
            freshness,
        });
    }

    // Best-effort LLM re-ranking.
    let valid: BTreeSet<String> = candidates.iter().map(|c| c.id.clone()).collect();
    let note = match reasoner
        .think(&build_prompt(query, kind, &candidates), RANK_MAX_TOKENS)
        .await
    {
        Ok(raw) => {
            let ranking = parse_recommendations(&raw, &valid);
            if ranking.is_empty() {
                objective_note()
            } else {
                apply_ranking(&mut candidates, ranking);
                "Ranked by your local model against what you described. Quality and \
                 content claims (e.g. \u{201c}uncensored\u{201d}) are the publisher's own — \
                 not locally verified."
                    .to_string()
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "model recommendation: LLM ranking failed — objective order");
            objective_note()
        }
    };

    Ok(RecommendReport {
        query: query.to_string(),
        candidates,
        note,
        freshness,
    })
}

fn objective_note() -> String {
    "Ranked by objective signals only (downloads/likes, recency, fit) — the local \
     model did not return a usable ranking. Quality is not locally verifiable."
        .to_string()
}

/// Reorder `candidates` so the LLM's picks come first, in its order, carrying
/// its reason; the rest keep their objective order.
fn apply_ranking(candidates: &mut Vec<RecommendCandidate>, ranking: Vec<(String, String)>) {
    let mut picked: Vec<RecommendCandidate> = Vec::new();
    for (id, why) in ranking {
        if let Some(pos) = candidates.iter().position(|c| c.id == id) {
            let mut c = candidates.remove(pos);
            c.why = why;
            c.llm_ranked = true;
            picked.push(c);
        }
    }
    picked.append(candidates);
    *candidates = picked;
}

fn stalest(a: Freshness, b: Freshness) -> Freshness {
    let rank = |f: &Freshness| match f {
        Freshness::Live => 0,
        Freshness::Stale { .. } => 1,
        Freshness::Offline { .. } => 2,
    };
    if rank(&b) > rank(&a) {
        b
    } else {
        a
    }
}

/// Objective ranking score — higher is better. Recency + popularity (log) +
/// a small bonus for a known-good fit.
fn objective_score(c: &RecommendCandidate) -> f64 {
    let recency = c
        .last_modified
        .as_deref()
        .map(recency_points)
        .unwrap_or(0.0);
    let popularity =
        ((c.downloads.max(0) as f64 + 1.0).ln() * 0.6) + ((c.likes.max(0) as f64 + 1.0).ln() * 1.2);
    let fit_bonus = match c.fit {
        FitVerdict::Green => 2.0,
        FitVerdict::Yellow { .. } => 0.5,
        _ => 0.0,
    };
    recency + popularity + fit_bonus
}

fn recency_points(last_modified: &str) -> f64 {
    match last_modified.get(..4).and_then(|y| y.parse::<i64>().ok()) {
        Some(y) if y >= 2026 => 6.0,
        Some(2025) => 4.0,
        Some(2024) => 2.0,
        Some(_) => 0.5,
        None => 0.0,
    }
}

fn objective_why(m: &RemoteModel) -> String {
    let mut bits: Vec<String> = Vec::new();
    if let Some(lm) = &m.last_modified {
        if let Some(y) = lm.get(..7) {
            bits.push(format!("updated {y}"));
        }
    }
    if m.downloads > 0 {
        bits.push(format!("{} downloads", human_count(m.downloads)));
    }
    if bits.is_empty() {
        "matched your search on Hugging Face".to_string()
    } else {
        bits.join(", ")
    }
}

fn human_params(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.0}B", n as f64 / 1e9)
    } else {
        format!("{:.0}M", n as f64 / 1e6)
    }
}

fn human_count(n: i64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.0}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

/// The ranking prompt. Deterministic layout so the parser has a stable
/// target; each candidate carries a few of its own Hugging Face tags so the
/// model can weigh content/style descriptors (e.g. "uncensored", "anime")
/// that plain popularity/recency say nothing about.
pub fn build_prompt(query: &str, kind: MediaKind, candidates: &[RecommendCandidate]) -> String {
    let mut p = String::new();
    p.push_str(&format!(
        "You help find {} for a local AI tool from real Hugging Face search results. \
         The user described what they want as: \"{query}\"\n\nCandidates:\n",
        kind.label()
    ));
    for c in candidates {
        let tags: Vec<&str> = c
            .tags
            .iter()
            .take(PROMPT_TAGS)
            .map(String::as_str)
            .collect();
        p.push_str(&format!(
            "- {} | {} | {} downloads | {} likes | updated {} | tags: {}{}\n",
            c.id,
            c.param_count
                .map(human_params)
                .unwrap_or_else(|| "?".into()),
            human_count(c.downloads),
            human_count(c.likes),
            c.last_modified.as_deref().unwrap_or("?"),
            if tags.is_empty() {
                "none".to_string()
            } else {
                tags.join(", ")
            },
            if c.gated {
                " | GATED (needs an HF license click-through)"
            } else {
                ""
            },
        ));
    }
    p.push_str(
        "\nPick up to 4 that best match what the user described. Reply with ONLY a \
         JSON array, no prose:\n\
         [{\"id\": \"<exact id from the list above>\", \"why\": \"<one short sentence, \
         call out anything the tags/license contradict about what was asked for>\"}]\n\
         Use only ids from the list. If none is a good match, reply [].\n",
    );
    p
}

/// Extract `[{id, why}, …]` from a completion, keeping only ids in `valid`
/// and trimming each reason. Tolerant of surrounding prose / code fences —
/// same parser shape as `crate::upgrade::parse_ranking`.
pub fn parse_recommendations(raw: &str, valid: &BTreeSet<String>) -> Vec<(String, String)> {
    let Some(start) = raw.find('[') else {
        return Vec::new();
    };
    let Some(end) = raw.rfind(']') else {
        return Vec::new();
    };
    if end <= start {
        return Vec::new();
    }
    let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(&raw[start..=end]) else {
        return Vec::new();
    };

    let mut out: Vec<(String, String)> = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for item in items {
        let Some(id) = item.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let id = id.trim();
        if !valid.contains(id) || !taken.insert(id.to_string()) {
            continue;
        }
        let why = item
            .get("why")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(|w| truncate(w, 200))
            .unwrap_or_else(|| "suggested by your local model".to_string());
        out.push((id.to_string(), why));
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Gated, ModelSource, RemoteModelDetails};
    use async_trait::async_trait;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn rm(id: &str, downloads: i64, likes: i64, tags: &[&str], updated: &str) -> RemoteModel {
        RemoteModel {
            id: id.into(),
            name: None,
            author: id.split('/').next().map(str::to_string),
            downloads,
            likes,
            trending_score: None,
            created_at: Some("2025-01-01".into()),
            last_modified: Some(updated.into()),
            pipeline_tag: Some("text-generation".into()),
            library_name: None,
            gated: Gated::No,
            license: Some("apache-2.0".into()),
            base_model: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            param_count: Some(7_000_000_000),
            arch: Some("qwen2".into()),
            ctx_max: Some(32_768),
            precision: Some("Q4_K_M".into()),
            format: RemoteFormat::Gguf,
            nsfw: false,
            preview_image_url: None,
            previews: Vec::new(),
            allow_commercial_use: vec![],
            model_kind_hint: None,
            base_model_family: None,
        }
    }

    #[derive(Debug)]
    struct ScriptedSource(std::sync::Mutex<Vec<Vec<RemoteModel>>>);
    #[async_trait]
    impl ModelSource for ScriptedSource {
        fn id(&self) -> &'static str {
            "scripted"
        }
        async fn search(&self, _q: &SearchQuery) -> Result<Vec<RemoteModel>> {
            let mut q = self.0.lock().unwrap();
            Ok(if q.is_empty() {
                Vec::new()
            } else {
                q.remove(0)
            })
        }
        async fn details(&self, _id: &str) -> Result<RemoteModelDetails> {
            unreachable!()
        }
    }

    fn registry(passes: Vec<Vec<RemoteModel>>) -> (Registry, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let r = Registry::new(
            Box::new(ScriptedSource(std::sync::Mutex::new(passes))),
            dir.path().to_path_buf(),
            Arc::new(AtomicBool::new(false)),
        );
        (r, dir)
    }

    struct CannedReasoner(String);
    #[async_trait]
    impl Reasoner for CannedReasoner {
        async fn think(&self, _p: &str, _m: i32) -> Result<String> {
            Ok(self.0.clone())
        }
    }
    struct BrokenReasoner;
    #[async_trait]
    impl Reasoner for BrokenReasoner {
        async fn think(&self, _p: &str, _m: i32) -> Result<String> {
            Err(recommend_err("no model loaded"))
        }
    }

    #[test]
    fn parse_recommendations_keeps_only_known_ids_and_trims() {
        let valid: BTreeSet<String> = ["a/one", "b/two"].iter().map(|s| s.to_string()).collect();
        let raw = "sure!\n```json\n[{\"id\":\"a/one\",\"why\":\"matches\"},\
                   {\"id\":\"ghost/invented\",\"why\":\"no\"},\
                   {\"id\":\"b/two\",\"why\":\"\"}]\n```";
        let got = parse_recommendations(raw, &valid);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], ("a/one".into(), "matches".into()));
        assert_eq!(got[1].0, "b/two");
        assert!(!got[1].1.is_empty());
    }

    #[test]
    fn parse_recommendations_handles_junk() {
        let valid: BTreeSet<String> = ["a/one"].iter().map(|s| s.to_string()).collect();
        assert!(parse_recommendations("no json here", &valid).is_empty());
        assert!(parse_recommendations("[]", &valid).is_empty());
    }

    #[test]
    fn media_kind_only_restricts_to_gguf_for_llm_kinds() {
        assert!(MediaKind::Chat.gguf_only());
        assert!(MediaKind::Coding.gguf_only());
        assert!(!MediaKind::Image.gguf_only());
        assert!(!MediaKind::Video.gguf_only());
        assert!(!MediaKind::Lora.gguf_only());
    }

    #[test]
    fn only_loras_skip_the_fit_check() {
        assert!(MediaKind::Chat.checks_fit());
        assert!(MediaKind::Image.checks_fit());
        assert!(!MediaKind::Lora.checks_fit());
    }

    #[tokio::test]
    async fn run_rejects_a_blank_query() {
        let (reg, _d) = registry(vec![vec![]]);
        let err = run(
            &reg,
            &BrokenReasoner,
            "   ",
            MediaKind::Chat,
            16_000,
            20_000,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("no search text"));
    }

    #[tokio::test]
    async fn run_filters_spam_ranks_and_applies_the_llm_order() {
        let (reg, _d) = registry(vec![
            vec![
                rm(
                    "ArliAI/GLM-4.6-Derestricted-v3",
                    18_000,
                    214,
                    &["uncensored", "roleplay", "not-for-all-audiences"],
                    "2026-03",
                ),
                rm("spam/nobody", 2, 0, &[], "2026-01"), // spam
            ],
            vec![rm(
                "some/other-uncensored-model",
                900_000,
                500,
                &["uncensored"],
                "2026-04",
            )],
        ]);
        let reasoner = CannedReasoner(
            "[{\"id\":\"ArliAI/GLM-4.6-Derestricted-v3\",\"why\":\"tagged uncensored and roleplay, matches the ask\"}]"
                .into(),
        );

        let report = run(
            &reg,
            &reasoner,
            "realistic uncensored roleplay",
            MediaKind::Chat,
            16_000,
            20_000,
        )
        .await
        .unwrap();

        let ids: Vec<&str> = report.candidates.iter().map(|c| c.id.as_str()).collect();
        assert!(!ids.contains(&"spam/nobody"), "spam filtered: {ids:?}");
        assert_eq!(report.candidates[0].id, "ArliAI/GLM-4.6-Derestricted-v3");
        assert!(report.candidates[0].llm_ranked);
        assert!(report.candidates[0].why.contains("roleplay"));
        assert!(report.note.contains("local model"));
    }

    #[tokio::test]
    async fn run_falls_back_to_objective_order_when_the_llm_fails() {
        let (reg, _d) = registry(vec![vec![
            rm("x/newer", 500_000, 400, &[], "2026-05"),
            rm("x/older", 500_000, 400, &[], "2024-01"),
        ]]);
        let report = run(
            &reg,
            &BrokenReasoner,
            "anything",
            MediaKind::Chat,
            16_000,
            20_000,
        )
        .await
        .unwrap();
        assert_eq!(report.candidates[0].id, "x/newer"); // recency wins
        assert!(!report.candidates[0].llm_ranked);
        assert!(report.note.contains("objective signals only"));
    }

    #[tokio::test]
    async fn run_returns_a_helpful_note_when_nothing_matches() {
        let (reg, _d) = registry(vec![vec![]]);
        let report = run(
            &reg,
            &BrokenReasoner,
            "something nobody has",
            MediaKind::Chat,
            16_000,
            20_000,
        )
        .await
        .unwrap();
        assert!(report.candidates.is_empty());
        assert!(report.note.to_lowercase().contains("nothing"));
    }

    #[tokio::test]
    async fn loras_are_never_filtered_on_fit() {
        // A LoRA has no param_count reported the way a full model does, so
        // `fit_of` would normally answer `Unknown` -- but MediaKind::Lora
        // skips the fit check entirely and always reports Green.
        let (reg, _d) = registry(vec![vec![rm(
            "someone/cool-lora",
            5_000,
            40,
            &["style"],
            "2026-01",
        )]]);
        let report = run(&reg, &BrokenReasoner, "cool style", MediaKind::Lora, 0, 0)
            .await
            .unwrap();
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].fit, FitVerdict::Green);
    }
}
