//! Benchmark-aware `Auto` model selection (Phase 6.6).
//!
//! [`db::ModelRepo::pick_for_role`](crate::db::ModelRepo::pick_for_role) picks by
//! usage alone. This module layers the 6.5 benchmark data and a VRAM-fit gate on
//! top: a model that fits the budget beats one that does not, then the
//! (preference-steered) benchmark score decides, then usage. With no benchmark
//! data at all it collapses back to the plain usage rule.
//!
//! There is still no quality axis (ADR-024) — [`AutoPreference::Quality`] just
//! re-weights the *measured* signals towards the bigger / heavier model.

use serde::{Deserialize, Serialize};

use crate::db::{Benchmark, Database, Model};
use crate::Result;

/// tokens/sec that count as "full speed" — mirrors [`crate::bench`].
const SPEED_REF_TPS: f64 = 80.0;
/// Parameter count that counts as "full heft" (a 14B model).
const HEFT_REF_PARAMS: f64 = 14e9;

/// How `Auto` should weigh a fast model against a heavier one. From the
/// `[models]` config table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoPreference {
    /// Speed, stability and heft together (the default).
    #[default]
    Balanced,
    /// Favour tokens/sec.
    Fast,
    /// Favour the bigger / less-quantised model that still fits.
    Quality,
}

impl AutoPreference {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Fast => "fast",
            Self::Quality => "quality",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "balanced" | "" => Some(Self::Balanced),
            "fast" => Some(Self::Fast),
            "quality" => Some(Self::Quality),
            _ => None,
        }
    }
}

/// Pick the best model for `role` when the caller asked for `Auto`.
///
/// `vram_budget_mb` of `0` means "unknown" — the fit gate is skipped. Returns
/// `None` when no model carries `role`.
pub async fn pick_for_role(
    db: &Database,
    role: &str,
    vram_budget_mb: u64,
    pref: AutoPreference,
) -> Result<Option<Model>> {
    let candidates = db.models().for_role_with_benchmark(role).await?;
    Ok(rank(candidates, vram_budget_mb, pref).into_iter().next())
}

/// Order `candidates` best-first. Public for testing.
pub fn rank(
    candidates: Vec<(Model, Option<Benchmark>)>,
    vram_budget_mb: u64,
    pref: AutoPreference,
) -> Vec<Model> {
    let mut rows: Vec<(Model, bool, f64)> = candidates
        .into_iter()
        .map(|(m, b)| {
            let fits = fits_budget(&m, vram_budget_mb);
            let score = selection_score(&m, b.as_ref(), pref);
            (m, fits, score)
        })
        .collect();

    rows.sort_by(|a, b| {
        // fits before doesn't-fit; then score; then usage; then name.
        b.1.cmp(&a.1)
            .then_with(|| b.2.total_cmp(&a.2))
            .then_with(|| cmp_opt_desc(&a.0.last_used_at, &b.0.last_used_at))
            .then_with(|| b.0.use_count.cmp(&a.0.use_count))
            .then_with(|| a.0.name.to_lowercase().cmp(&b.0.name.to_lowercase()))
    });

    rows.into_iter().map(|(m, _, _)| m).collect()
}

/// `Some > None`, and larger strings (later timestamps) first.
fn cmp_opt_desc(a: &Option<String>, b: &Option<String>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => y.cmp(x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// Whether the model's import-time VRAM estimate is within the budget. Unknown
/// estimate or `0` budget → not excluded.
fn fits_budget(m: &Model, budget_mb: u64) -> bool {
    if budget_mb == 0 {
        return true;
    }
    match m.vram_estimate_mb.and_then(|v| u64::try_from(v).ok()) {
        Some(mb) if mb > 0 => mb <= budget_mb,
        _ => true,
    }
}

/// `0..=100`. A benchmarked model scores from its measured speed / stability /
/// heft, weighted by `pref`; a non-benchmarked one is neutral (50), except that
/// [`AutoPreference::Quality`] still nudges towards the bigger model.
fn selection_score(m: &Model, b: Option<&Benchmark>, pref: AutoPreference) -> f64 {
    let heft = m
        .param_count
        .and_then(|n| u64::try_from(n).ok())
        .filter(|p| *p > 0)
        .map_or(0.5, |p| (p as f64 / HEFT_REF_PARAMS).clamp(0.0, 1.0));

    let Some(b) = b else {
        return match pref {
            AutoPreference::Quality => 40.0 + 20.0 * heft,
            _ => 50.0,
        };
    };

    let speed = b
        .gen_tps
        .map_or(0.0, |t| (t / SPEED_REF_TPS).clamp(0.0, 1.0));
    let stability = b.stability_score.clamp(0.0, 1.0);
    let raw = match pref {
        AutoPreference::Balanced => 0.6 * speed + 0.2 * stability + 0.2 * heft,
        AutoPreference::Fast => 0.8 * speed + 0.15 * stability + 0.05 * heft,
        AutoPreference::Quality => 0.3 * speed + 0.15 * stability + 0.55 * heft,
    };
    raw * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{NewBenchmark, NewModel};

    fn model(name: &str, params: Option<i64>, vram_mb: Option<i64>, use_count: i64) -> Model {
        Model {
            id: name.into(),
            publisher: None,
            name: name.into(),
            family: None,
            family_source: None,
            format: "gguf".into(),
            quant: None,
            arch: None,
            param_count: params,
            file_path: format!("E:\\m\\{name}.gguf"),
            sha256: None,
            size_bytes: 4_000 * 1024 * 1024,
            ctx_max: Some(32_768),
            vram_estimate_mb: vram_mb,
            ram_estimate_mb: None,
            source: "manual".into(),
            source_revision: None,
            imported_at: "2026-01-01T00:00:00Z".into(),
            last_used_at: None,
            use_count,
            n_layers: None,
            n_embd: None,
            n_heads: None,
            n_kv_heads: None,
            roles: vec!["chat".into()],
            runtimes: vec![],
        }
    }

    fn bench(gen_tps: f64, stability: f64) -> Benchmark {
        Benchmark {
            id: "b".into(),
            model_id: "m".into(),
            job_id: None,
            kind: "llm".into(),
            runs: 3,
            prompt_tps: Some(300.0),
            gen_tps: Some(gen_tps),
            load_ms: Some(1_500),
            vram_peak_mb: Some(6_000),
            ram_peak_mb: Some(14_000),
            stability_score: stability,
            overall_score: 60,
            notes: None,
            suite: None,
            detail: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn a_model_that_fits_beats_a_faster_one_that_does_not() {
        let fits = model("fits", Some(7_000_000_000), Some(6_000), 0);
        let over = model("over", Some(30_000_000_000), Some(20_000), 0);
        let out = rank(
            vec![
                (over.clone(), Some(bench(90.0, 1.0))),
                (fits.clone(), Some(bench(40.0, 1.0))),
            ],
            16_000,
            AutoPreference::Balanced,
        );
        assert_eq!(out[0].id, "fits");
        assert_eq!(out[1].id, "over");
    }

    #[test]
    fn fast_preference_picks_the_higher_tok_per_sec() {
        let quick = model("quick", Some(7_000_000_000), Some(6_000), 0);
        let big = model("big", Some(14_000_000_000), Some(9_000), 0);
        let cs = vec![
            (big.clone(), Some(bench(35.0, 1.0))),
            (quick.clone(), Some(bench(75.0, 1.0))),
        ];
        assert_eq!(
            rank(cs.clone(), 16_000, AutoPreference::Fast)[0].id,
            "quick"
        );
        // Quality flips it towards the bigger model.
        assert_eq!(rank(cs, 16_000, AutoPreference::Quality)[0].id, "big");
    }

    #[test]
    fn with_no_benchmarks_it_is_the_usage_rule() {
        let mut recent = model("recent", None, Some(6_000), 1);
        recent.last_used_at = Some("2026-02-01T00:00:00Z".into());
        let stale = model("stale", None, Some(6_000), 9);
        let fresh = model("fresh-unused", None, Some(6_000), 0);

        let out = rank(
            vec![
                (fresh.clone(), None),
                (stale.clone(), None),
                (recent.clone(), None),
            ],
            16_000,
            AutoPreference::Balanced,
        );
        // most-recently-used, then most-used, then name.
        assert_eq!(
            out.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["recent", "stale", "fresh-unused"]
        );
    }

    #[test]
    fn budget_of_zero_skips_the_fit_gate() {
        let over = model("over", Some(7_000_000_000), Some(99_999), 0);
        let out = rank(
            vec![(over.clone(), Some(bench(80.0, 1.0)))],
            0,
            AutoPreference::Balanced,
        );
        assert_eq!(out[0].id, "over");
    }

    #[tokio::test]
    async fn pick_for_role_joins_the_latest_benchmark() {
        let db = Database::connect_in_memory().await.unwrap();
        async fn mk(db: &Database, name: &str) -> Model {
            db.models()
                .insert(NewModel {
                    name: name.into(),
                    format: "gguf".into(),
                    file_path: format!("E:\\m\\{name}.gguf"),
                    size_bytes: 4_000 * 1024 * 1024,
                    param_count: Some(7_000_000_000),
                    vram_estimate_mb: Some(6_000),
                    source: "manual".into(),
                    roles: vec!["chat".into()],
                    ..NewModel::default()
                })
                .await
                .unwrap()
        }
        let slow = mk(&db, "slow").await;
        let fast = mk(&db, "fast").await;

        // Only "fast" is benchmarked, and it is quick.
        db.benchmarks()
            .insert(NewBenchmark {
                model_id: fast.id.clone(),
                job_id: None,
                kind: "llm".into(),
                runs: 3,
                prompt_tps: Some(300.0),
                gen_tps: Some(78.0),
                load_ms: Some(1_500),
                vram_peak_mb: Some(6_000),
                ram_peak_mb: Some(14_000),
                stability_score: 0.95,
                overall_score: 80,
                notes: None,
                suite: None,
                detail_json: None,
            })
            .await
            .unwrap();

        let picked = pick_for_role(&db, "chat", 16_000, AutoPreference::Balanced)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(picked.id, fast.id);
        let _ = slow;
    }
}
