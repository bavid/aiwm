//! `core::upgrade` — the "Is there something better?" check (Phase 6.7).
//!
//! Flow: ask Hugging Face for newer / more popular models of the same
//! **family + role** → drop the ones that will not fit this machine's VRAM
//! budget (`core::compat`) and the obvious spam → rank the survivors by
//! **objective, retrievable** signals (recency, downloads/likes, params) → let
//! the local LLM re-order the top few and write a one-line reason, **validating
//! every id it returns against the real candidate list** so it cannot invent a
//! model. The LLM step is best-effort: a bad answer falls back to the objective
//! order.
//!
//! "Better" here is never a quality claim (ADR-024 / ADR-025) — only "newer,
//! bigger, better-supported, and it fits".

use std::collections::BTreeSet;

use async_trait::async_trait;
use serde::Serialize;

use crate::compat::{self, FitVerdict, ModelDims};
use crate::registry::{Freshness, Registry, RemoteFormat, RemoteModel, SearchQuery, SearchSort};
use crate::{CoreError, Result};

/// Candidates kept for the LLM prompt / the final report.
const SHORTLIST: usize = 8;
/// Per HF search call.
const SEARCH_LIMIT: u32 = 40;
/// A candidate needs at least this much traction to be taken seriously (filters
/// brand-new spam re-uploads — R9).
const MIN_DOWNLOADS: i64 = 80;
const MIN_LIKES: i64 = 3;
/// Rough KV + runtime overhead added to the estimated weights (MB).
const RUNTIME_PAD_MB: u64 = 1_400;
/// Max tokens for the ranking completion.
const RANK_MAX_TOKENS: i32 = 700;

fn upgrade_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("upgrade check: {msg}"))
}

/// Something that can run one text completion — the local `chat` / `coding`
/// model. Abstracted so the flow is testable without a server.
#[async_trait]
pub trait Reasoner: Send + Sync {
    async fn think(&self, prompt: &str, max_tokens: i32) -> Result<String>;
}

#[async_trait]
impl Reasoner for crate::runtime::LlamaCppAdapter {
    async fn think(&self, prompt: &str, max_tokens: i32) -> Result<String> {
        self.complete(prompt, max_tokens).await
    }
}

/// What is being checked — a specific installed model, or a whole role.
#[derive(Debug, Clone)]
pub struct UpgradeTarget {
    /// For the report + the prompt, e.g. `"Qwen2.5 7B Instruct"` or
    /// `"role: chat"`.
    pub label: String,
    /// Free-text family hint for the HF search (`family`, else `arch`, else the
    /// first name tokens).
    pub family: String,
    /// `param_count` of the installed model, when known — the yardstick for
    /// "bigger".
    pub params: Option<u64>,
    /// `true` for `chat` / `coding` (search GGUF; the LLM knows LLMs); `false`
    /// for image / video roles.
    pub is_llm: bool,
    /// Repo ids already in the library — flagged, never proposed.
    pub installed_ids: BTreeSet<String>,
}

/// One proposed model.
#[derive(Debug, Clone, Serialize)]
pub struct UpgradeCandidate {
    pub id: String,
    /// One sentence — the LLM's reason, or an objective note.
    pub why: String,
    pub downloads: i64,
    pub likes: i64,
    pub last_modified: Option<String>,
    pub param_count: Option<u64>,
    pub format: RemoteFormat,
    pub gated: bool,
    pub fit: FitVerdict,
    /// Already installed (an exact repo-id match).
    pub installed: bool,
    /// The LLM put this in its ranking (vs. objective-only).
    pub llm_ranked: bool,
}

/// The check's result — written to `jobs.result` as JSON.
#[derive(Debug, Clone, Serialize)]
pub struct UpgradeReport {
    pub target: String,
    /// The family text the search used.
    pub query: String,
    pub candidates: Vec<UpgradeCandidate>,
    /// How the list was ranked + the quality caveat.
    pub note: String,
    pub freshness: Freshness,
}

/// Run the check. `vram_budget_mb` / `free_ram_mb` drive the fit filter; `0`
/// budget → the fit column is `Unknown` and nothing is filtered on it.
pub async fn run(
    registry: &Registry,
    reasoner: &dyn Reasoner,
    target: &UpgradeTarget,
    vram_budget_mb: u64,
    free_ram_mb: u64,
) -> Result<UpgradeReport> {
    let family = target.family.trim();
    if family.is_empty() {
        return Err(upgrade_err("no family to search for"));
    }

    // Two passes: recency-weighted popularity, then recently-updated. Merge.
    let mut freshness = Freshness::Live;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut pool: Vec<RemoteModel> = Vec::new();
    for sort in [SearchSort::Trending, SearchSort::RecentlyUpdated] {
        let fetched = registry
            .search(&SearchQuery {
                text: Some(family.to_string()),
                gguf_only: target.is_llm,
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
    let mut candidates: Vec<UpgradeCandidate> = pool
        .into_iter()
        .filter(|m| !looks_like_spam(m))
        .map(|m| {
            let fit = fit_of(&m, ctx, vram_budget_mb, free_ram_mb);
            let installed = target.installed_ids.contains(&m.id);
            UpgradeCandidate {
                why: objective_why(target, &m),
                id: m.id,
                downloads: m.downloads,
                likes: m.likes,
                last_modified: m.last_modified,
                param_count: m.param_count,
                format: m.format,
                gated: m.gated.is_gated(),
                fit,
                installed,
                llm_ranked: false,
            }
        })
        .filter(|c| !matches!(c.fit, FitVerdict::Red { .. }))
        .collect();

    // Objective pre-rank; installed models sink (they are context, not a pick).
    candidates.sort_by(|a, b| {
        a.installed
            .cmp(&b.installed)
            .then(objective_score(b).total_cmp(&objective_score(a)))
    });
    candidates.truncate(SHORTLIST);

    if candidates.is_empty() {
        return Ok(UpgradeReport {
            target: target.label.clone(),
            query: family.to_string(),
            candidates,
            note: "Nothing on Hugging Face matched that fits your VRAM budget — \
                   widen the search from the Discover panel."
                .to_string(),
            freshness,
        });
    }

    // Best-effort LLM re-ranking.
    let valid: BTreeSet<String> = candidates.iter().map(|c| c.id.clone()).collect();
    let note = match reasoner
        .think(&build_prompt(target, &candidates), RANK_MAX_TOKENS)
        .await
    {
        Ok(raw) => {
            let ranking = parse_ranking(&raw, &valid);
            if ranking.is_empty() {
                objective_note()
            } else {
                apply_ranking(&mut candidates, ranking);
                "Ranked by your local model over the objective signals \
                 (release date, downloads, size, fit). Quality is not locally \
                 verifiable."
                    .to_string()
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "upgrade-check LLM ranking failed — objective order");
            objective_note()
        }
    };

    Ok(UpgradeReport {
        target: target.label.clone(),
        query: family.to_string(),
        candidates,
        note,
        freshness,
    })
}

fn objective_note() -> String {
    "Ranked by objective signals only (release date, downloads/likes, size, \
     fit) — the local model did not return a usable ranking. Quality is not \
     locally verifiable."
        .to_string()
}

/// Reorder `candidates` so the LLM's picks come first, in its order, carrying
/// its reason; the rest keep their objective order.
fn apply_ranking(candidates: &mut Vec<UpgradeCandidate>, ranking: Vec<(String, String)>) {
    let mut picked: Vec<UpgradeCandidate> = Vec::new();
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

/// Merge two `Freshness` values, keeping the more degraded one.
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

/// Rough VRAM estimate for a remote model from its param count + precision, run
/// through [`compat::verdict`]. `pub(crate)` — also used by `crate::recommend`,
/// which shares the same "does this fit?" question for a freshly-searched
/// candidate.
pub(crate) fn fit_of(m: &RemoteModel, ctx: u32, budget_mb: u64, free_ram_mb: u64) -> FitVerdict {
    let Some(params) = m.param_count.filter(|p| *p > 0) else {
        return FitVerdict::Unknown;
    };
    let bpp = bytes_per_param(m.precision.as_deref());
    let weights_mb = ((params as f64 * bpp) / (1024.0 * 1024.0)) as u64;
    let size_bytes = (weights_mb + RUNTIME_PAD_MB) * 1024 * 1024;
    compat::verdict(
        &ModelDims {
            size_bytes,
            ctx_max: m.ctx_max,
            param_count: Some(params),
            ..ModelDims::default()
        },
        ctx,
        budget_mb,
        free_ram_mb,
    )
}

/// Bytes per weight for a quant / dtype label. Falls back to a Q4-ish 0.6.
fn bytes_per_param(precision: Option<&str>) -> f64 {
    let Some(p) = precision else { return 0.6 };
    let p = p.to_ascii_uppercase();
    if p.contains("Q2") || p.contains("IQ1") || p.contains("IQ2") {
        0.36
    } else if p.contains("Q3") || p.contains("IQ3") {
        0.44
    } else if p.contains("Q4") || p.contains("IQ4") {
        0.56
    } else if p.contains("Q5") {
        0.66
    } else if p.contains("Q6") {
        0.78
    } else if p.contains("Q8") || p.contains("F8") || p.contains("FP8") {
        1.08
    } else if p.contains("F16") || p.contains("BF16") || p.contains("FP16") {
        2.0
    } else if p.contains("F32") || p.contains("FP32") {
        4.0
    } else {
        0.6
    }
}

/// `true` for repos that look like content-farm spam (R9): no traction, or a
/// hallucinated-looking version bump with impossible popularity.
pub fn looks_like_spam(m: &RemoteModel) -> bool {
    if m.downloads < MIN_DOWNLOADS && m.likes < MIN_LIKES {
        return true;
    }
    // Brand-new repo (< ~7 days) already claiming huge downloads is the tell.
    let recent = m
        .created_at
        .as_deref()
        .map(|c| c >= "2099")
        .unwrap_or(false);
    recent && m.downloads > 50_000 && m.likes < 20
}

/// Objective ranking score — higher is better. Recency + popularity (log) +
/// a bonus for being bigger than what is installed.
fn objective_score(c: &UpgradeCandidate) -> f64 {
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

/// `lastModified` (RFC 3339) → points, newer is more. Anything in the last year
/// scores well; older tails off.
fn recency_points(last_modified: &str) -> f64 {
    match last_modified.get(..4).and_then(|y| y.parse::<i64>().ok()) {
        Some(y) if y >= 2026 => 6.0,
        Some(2025) => 4.0,
        Some(2024) => 2.0,
        Some(_) => 0.5,
        None => 0.0,
    }
}

fn objective_why(target: &UpgradeTarget, m: &RemoteModel) -> String {
    let mut bits: Vec<String> = Vec::new();
    if let (Some(theirs), Some(ours)) = (m.param_count, target.params) {
        if theirs > ours + ours / 20 {
            bits.push(format!(
                "~{} params vs your ~{}",
                human_params(theirs),
                human_params(ours)
            ));
        }
    }
    if let Some(lm) = &m.last_modified {
        if let Some(y) = lm.get(..7) {
            bits.push(format!("updated {y}"));
        }
    }
    if m.downloads > 0 {
        bits.push(format!("{} downloads", human_count(m.downloads)));
    }
    if bits.is_empty() {
        "same family, popular on Hugging Face".to_string()
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

/// The ranking prompt. Deterministic layout so the parser has a stable target.
pub fn build_prompt(target: &UpgradeTarget, candidates: &[UpgradeCandidate]) -> String {
    let mut p = String::new();
    p.push_str(
        "You help choose a better local AI model. Below is the model in use and a \
         list of real candidates from Hugging Face that already fit the user's \
         hardware.\n\n",
    );
    p.push_str(&format!("In use: {}", target.label));
    if let Some(params) = target.params {
        p.push_str(&format!(" (~{} params)", human_params(params)));
    }
    p.push_str("\n\nCandidates:\n");
    for c in candidates {
        p.push_str(&format!(
            "- {} | {} | {} downloads | {} likes | updated {}{}\n",
            c.id,
            c.param_count
                .map(human_params)
                .unwrap_or_else(|| "?".into()),
            human_count(c.downloads),
            human_count(c.likes),
            c.last_modified.as_deref().unwrap_or("?"),
            if c.installed {
                " | ALREADY INSTALLED"
            } else {
                ""
            },
        ));
    }
    p.push_str(
        "\nPick up to 4 that are genuine upgrades (newer, larger, or clearly \
         better supported). Ignore ones already installed. Reply with ONLY a \
         JSON array, no prose:\n\
         [{\"id\": \"<exact id from the list above>\", \"why\": \"<one short sentence>\"}]\n\
         Use only ids from the list. If none is a clear upgrade, reply [].\n",
    );
    p
}

/// Extract `[{id, why}, …]` from a completion, keeping only ids in `valid` and
/// trimming each reason. Tolerant of surrounding prose / code fences.
pub fn parse_ranking(raw: &str, valid: &BTreeSet<String>) -> Vec<(String, String)> {
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
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn rm(id: &str, downloads: i64, likes: i64, params: u64, updated: &str) -> RemoteModel {
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
            tags: vec!["gguf".into()],
            param_count: Some(params),
            arch: Some("qwen2".into()),
            ctx_max: Some(32_768),
            precision: Some("Q4_K_M".into()),
            format: RemoteFormat::Gguf,
            nsfw: false,
            preview_image_url: None,
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
            Err(upgrade_err("no model loaded"))
        }
    }

    fn target() -> UpgradeTarget {
        UpgradeTarget {
            label: "Qwen2.5 7B Instruct".into(),
            family: "qwen2".into(),
            params: Some(7_600_000_000),
            is_llm: true,
            installed_ids: BTreeSet::new(),
        }
    }

    #[test]
    fn bytes_per_param_reads_the_quant_label() {
        assert!(bytes_per_param(Some("Q4_K_M")) < bytes_per_param(Some("Q8_0")));
        assert!(bytes_per_param(Some("Q8_0")) < bytes_per_param(Some("BF16")));
        assert_eq!(bytes_per_param(None), 0.6);
    }

    #[test]
    fn spam_guard_drops_no_traction_and_impossible_repos() {
        assert!(looks_like_spam(&rm(
            "x/nobody",
            3,
            0,
            7_000_000_000,
            "2025-06"
        )));
        let mut fake = rm("x/hype", 900_000, 2, 7_000_000_000, "2026-01");
        fake.created_at = Some("2099-01-01".into());
        assert!(looks_like_spam(&fake));
        assert!(!looks_like_spam(&rm(
            "Qwen/real",
            500_000,
            300,
            7_000_000_000,
            "2026-02"
        )));
    }

    #[test]
    fn parse_ranking_keeps_only_known_ids_and_trims() {
        let valid: BTreeSet<String> = ["a/one", "b/two"].iter().map(|s| s.to_string()).collect();
        let raw = "sure!\n```json\n[{\"id\":\"a/one\",\"why\":\"newer\"},\
                   {\"id\":\"ghost/invented\",\"why\":\"no\"},\
                   {\"id\":\"b/two\",\"why\":\"\"},\
                   {\"id\":\"a/one\",\"why\":\"dup\"}]\n```";
        let got = parse_ranking(raw, &valid);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], ("a/one".into(), "newer".into()));
        assert_eq!(got[1].0, "b/two");
        assert!(!got[1].1.is_empty());
    }

    #[test]
    fn parse_ranking_handles_junk() {
        let valid: BTreeSet<String> = ["a/one"].iter().map(|s| s.to_string()).collect();
        assert!(parse_ranking("no json here", &valid).is_empty());
        assert!(parse_ranking("[not json]", &valid).is_empty());
        assert!(parse_ranking("[]", &valid).is_empty());
    }

    #[tokio::test]
    async fn run_filters_ranks_and_applies_the_llm_order() {
        let (reg, _d) = registry(vec![
            vec![
                rm("Qwen/big-70b", 200_000, 400, 70_000_000_000, "2026-03"), // won't fit
                rm("Qwen/coder-14b", 300_000, 250, 14_000_000_000, "2026-02"),
                rm("spam/nobody", 2, 0, 7_000_000_000, "2026-01"), // spam
            ],
            vec![rm(
                "Qwen/coder-7b-v2",
                900_000,
                500,
                7_600_000_000,
                "2026-04",
            )],
        ]);
        let reasoner = CannedReasoner(
            "[{\"id\":\"Qwen/coder-7b-v2\",\"why\":\"same size, newer, far more downloads\"}]"
                .into(),
        );

        let report = run(&reg, &reasoner, &target(), 16_000, 20_000)
            .await
            .unwrap();

        let ids: Vec<&str> = report.candidates.iter().map(|c| c.id.as_str()).collect();
        assert!(
            !ids.contains(&"Qwen/big-70b"),
            "70B filtered on fit: {ids:?}"
        );
        assert!(!ids.contains(&"spam/nobody"), "spam filtered: {ids:?}");
        assert_eq!(report.candidates[0].id, "Qwen/coder-7b-v2");
        assert!(report.candidates[0].llm_ranked);
        assert!(report.candidates[0].why.contains("more downloads"));
        assert!(report.note.contains("local model"));
    }

    #[tokio::test]
    async fn run_falls_back_to_objective_order_when_the_llm_fails() {
        let (reg, _d) = registry(vec![vec![
            rm("Qwen/newer", 500_000, 400, 7_600_000_000, "2026-05"),
            rm("Qwen/older", 500_000, 400, 7_600_000_000, "2024-01"),
        ]]);
        let report = run(&reg, &BrokenReasoner, &target(), 16_000, 20_000)
            .await
            .unwrap();
        assert_eq!(report.candidates[0].id, "Qwen/newer"); // recency wins
        assert!(!report.candidates[0].llm_ranked);
        assert!(report.note.contains("objective signals only"));
    }

    #[tokio::test]
    async fn run_returns_a_helpful_note_when_nothing_fits() {
        let (reg, _d) = registry(vec![vec![rm(
            "Qwen/huge",
            999_999,
            999,
            180_000_000_000,
            "2026-01",
        )]]);
        let report = run(&reg, &BrokenReasoner, &target(), 16_000, 20_000)
            .await
            .unwrap();
        assert!(report.candidates.is_empty());
        assert!(report.note.to_lowercase().contains("nothing"));
    }
}
