//! `core::bench` — local micro-benchmarks (Phase 6.5).
//!
//! A "Test model" job loads the model, runs a short fixed prompt through it a
//! few times, and records what is honestly measurable on *this* machine:
//! generation + prompt tokens/sec, cold load time, VRAM/RAM peak, and how
//! consistent the tokens/sec were across the runs.
//!
//! There is **no local quality benchmark** (ADR-024). [`overall_score`] is an
//! openly-declared weighted heuristic over speed, fit and stability — a
//! ranking aid, not a claim about how good the model's answers are.
//!
//! A request may name a versioned [`suites::Suite`] instead of the single
//! default prompt; the job then runs every prompt of that suite `runs` times and
//! the report carries a per-prompt breakdown ([`PromptResult`]) next to the
//! overall means. Without a suite the behaviour is exactly what it always was.

pub mod suites;

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::{mpsc, watch};

use crate::compat::{self, FitVerdict};
use crate::db::{Database, EventLevel, Model, NewBenchmark};
use crate::runtime::{GenerationEvent, LlamaCppAdapter};
use crate::telemetry::{GpuStatus, SystemTelemetry};
use crate::{CoreError, Result};

/// A neutral prompt that produces a few sentences of output — long enough for a
/// meaningful generation-rate sample, short enough to keep a run quick.
pub const DEFAULT_PROMPT: &str =
    "In two or three short paragraphs, explain what a CPU cache is, why it exists, \
     and how it affects everyday program performance.";
const DEFAULT_RUNS: u32 = 3;
/// Default passes *per prompt* when a suite is named: a suite has three prompts,
/// so two passes each already means six generations — enough to average without
/// making a run take minutes.
const DEFAULT_SUITE_RUNS: u32 = 2;
const MAX_RUNS: u32 = 10;
const DEFAULT_MAX_TOKENS: i32 = 128;
const MIN_MAX_TOKENS: i32 = 16;
const MAX_MAX_TOKENS: i32 = 512;

/// tokens/sec that maps to a full speed score. A well-quantised 7-8B model on a
/// 16 GB desktop card generates roughly this fast; bigger / less-quantised
/// models trail it. Calibrated by feel — see `docs/HARDWARE.md`.
const SPEED_REF_TPS: f64 = 80.0;

fn bench_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "bench".into(),
        message: msg.to_string(),
    }
}

/// What a "Test model" job asks for, from its `params`.
#[derive(Debug, Clone)]
pub struct BenchRequest {
    /// Used only when no `suite` is named — the suite brings its own prompts.
    pub prompt: String,
    /// Generation passes to average, *per prompt* (clamped to `1..=10`).
    pub runs: u32,
    /// Tokens to generate per pass (clamped to `16..=512`). A suite overrides it.
    pub max_tokens: i32,
    /// Id of a [`suites`] suite to run instead of `prompt`; `None` = the legacy
    /// single-prompt "quick test".
    pub suite: Option<String>,
}

impl Default for BenchRequest {
    fn default() -> Self {
        Self {
            prompt: DEFAULT_PROMPT.to_string(),
            runs: DEFAULT_RUNS,
            max_tokens: DEFAULT_MAX_TOKENS,
            suite: None,
        }
    }
}

impl BenchRequest {
    pub fn from_params(params: &serde_json::Value) -> Self {
        let d = Self::default();
        let prompt = params
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map_or(d.prompt, str::to_string);
        let suite = params
            .get("suite")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // A suite multiplies the passes by its prompt count, so it starts lower.
        let default_runs = if suite.is_some() {
            DEFAULT_SUITE_RUNS
        } else {
            d.runs
        };
        let runs = params
            .get("runs")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(default_runs)
            .clamp(1, MAX_RUNS);
        let max_tokens = params
            .get("max_tokens")
            .and_then(serde_json::Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .unwrap_or(d.max_tokens)
            .clamp(MIN_MAX_TOKENS, MAX_MAX_TOKENS);
        Self {
            prompt,
            runs,
            max_tokens,
            suite,
        }
    }
}

/// What one suite prompt contributed, averaged over its own passes.
#[derive(Debug, Clone, Serialize)]
pub struct PromptResult {
    /// [`suites::SuitePrompt::id`].
    pub prompt_id: String,
    /// Mean generated tokens per pass, rounded.
    pub tokens: u64,
    pub gen_tps: Option<f64>,
    pub prompt_tps: Option<f64>,
}

/// The numbers a finished benchmark produced.
#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    /// Total generation passes: `prompts × runs` for a suite, `runs` otherwise.
    pub runs: u32,
    pub prompt_tps: Option<f64>,
    pub gen_tps: Option<f64>,
    pub load_ms: Option<u64>,
    pub vram_peak_mb: Option<u64>,
    pub ram_peak_mb: Option<u64>,
    pub stability_score: f64,
    pub overall_score: u8,
    pub notes: String,
    /// The suite that was run, if any.
    pub suite: Option<String>,
    /// Per-prompt breakdown, in suite order; empty for a single-prompt run.
    pub detail: Vec<PromptResult>,
}

/// How a benchmark body came to rest.
#[derive(Debug, Clone)]
pub enum BenchOutcome {
    Done(BenchReport),
    Cancelled,
}

/// Consistency of the tokens/sec samples: `1.0` = identical every run, falling
/// towards `0.0` as they spread out (`1 - coefficient of variation`, clamped).
/// Fewer than two samples → `1.0` (nothing to disagree).
pub fn stability_score(samples: &[f64]) -> f64 {
    let usable: Vec<f64> = samples.iter().copied().filter(|s| *s > 0.0).collect();
    if usable.len() < 2 {
        return 1.0;
    }
    let n = usable.len() as f64;
    let mean = usable.iter().sum::<f64>() / n;
    if mean <= 0.0 {
        return 0.0;
    }
    let variance = usable.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / n;
    let cv = variance.sqrt() / mean;
    (1.0 - cv).clamp(0.0, 1.0)
}

/// Openly-declared ranking heuristic in `0..=100`. **Not** a quality score
/// (ADR-024).
///
/// The run itself scores as `0.65·speed + 0.35·stability`, where `speed` is
/// `gen_tps` against [`SPEED_REF_TPS`] (clamped). Fit then scales the whole
/// thing: a model that will not fit this machine's VRAM budget is heavily
/// capped no matter how fast it looked, because you cannot actually run it.
pub fn overall_score(gen_tps: f64, stability: f64, fit: &FitVerdict) -> u8 {
    let speed = (gen_tps / SPEED_REF_TPS).clamp(0.0, 1.0);
    let run = 0.65 * speed + 0.35 * stability.clamp(0.0, 1.0);
    let fit_factor = match fit {
        FitVerdict::Green => 1.0,
        FitVerdict::Yellow { .. } => 0.85,
        FitVerdict::Unknown => 0.8,
        FitVerdict::Red { .. } => 0.35,
    };
    (run * fit_factor * 100.0).round().clamp(0.0, 100.0) as u8
}

/// One generation pass's timing, tagged with the suite prompt it came from
/// (`None` for the legacy single-prompt run).
#[derive(Debug, Clone, Copy)]
struct Pass {
    prompt_id: Option<&'static str>,
    tokens: u64,
    prompt_tps: f64,
    gen_tps: f64,
}

/// One prompt of the plan: what to send, how long to let it generate, and how to
/// label it on the job's event trail.
#[derive(Debug, Clone)]
struct Step {
    prompt_id: Option<&'static str>,
    /// `"chat-v1 · Explain a concept"`; `None` for the legacy run.
    label: Option<String>,
    text: String,
    max_tokens: i32,
}

/// The prompts this request wants, in order. Errors on an unknown suite id —
/// there is deliberately no silent fallback to the default prompt, because the
/// numbers would then quietly not mean what the caller asked for.
fn plan(req: &BenchRequest) -> Result<Vec<Step>> {
    let Some(id) = req.suite.as_deref() else {
        return Ok(vec![Step {
            prompt_id: None,
            label: None,
            text: req.prompt.clone(),
            max_tokens: req.max_tokens,
        }]);
    };
    let suite = suites::find(id)
        .ok_or_else(|| bench_err(format!("unknown benchmark suite \u{201c}{id}\u{201d}")))?;
    Ok(suite
        .prompts
        .iter()
        .map(|p| Step {
            prompt_id: Some(p.id),
            label: Some(format!("{} \u{b7} {}", suite.id, p.title)),
            text: p.text.to_string(),
            max_tokens: suite.max_tokens,
        })
        .collect())
}

/// Run the benchmark against the resident model. `load` is the time the engine
/// spent loading it (`None` when it was already resident). Samples the telemetry
/// peak while it works; writes one [`crate::db::Benchmark`] row on success.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    db: &Database,
    llama: &Arc<LlamaCppAdapter>,
    telemetry: watch::Receiver<SystemTelemetry>,
    job_id: &str,
    model: &Model,
    req: BenchRequest,
    load: Option<std::time::Duration>,
    vram_budget_mb: u64,
    cancel: watch::Receiver<bool>,
) -> Result<BenchOutcome> {
    let steps = plan(&req)?;
    db.jobs()
        .append_event(job_id, EventLevel::Info, &opening_line(model, &req, &steps))
        .await?;

    let mut runner = Runner {
        db,
        llama,
        job_id,
        telemetry,
        cancel,
        peak: TelemetryPeak::default(),
        passes: Vec::with_capacity(steps.len() * req.runs as usize),
    };
    runner.sample_peak();
    for step in &steps {
        if !runner.run_passes(step, req.runs).await? {
            return Ok(BenchOutcome::Cancelled);
        }
    }

    let report = summarise(
        model,
        &req,
        &runner.passes,
        load,
        vram_budget_mb,
        &runner.peak,
    );
    db.benchmarks()
        .insert(NewBenchmark {
            model_id: model.id.clone(),
            job_id: Some(job_id.to_string()),
            kind: "llm".into(),
            runs: report.runs,
            prompt_tps: report.prompt_tps,
            gen_tps: report.gen_tps,
            load_ms: report.load_ms,
            vram_peak_mb: report.vram_peak_mb,
            ram_peak_mb: report.ram_peak_mb,
            stability_score: report.stability_score,
            overall_score: report.overall_score,
            notes: Some(report.notes.clone()),
            suite: report.suite.clone(),
            detail_json: detail_json(&report.detail),
        })
        .await?;

    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "score {} — {:.1} tok/s gen, {:.0} tok/s prompt, stability {:.2}",
                report.overall_score,
                report.gen_tps.unwrap_or(0.0),
                report.prompt_tps.unwrap_or(0.0),
                report.stability_score,
            ),
        )
        .await?;

    Ok(BenchOutcome::Done(report))
}

/// The opening line on the job's event trail.
fn opening_line(model: &Model, req: &BenchRequest, steps: &[Step]) -> String {
    let tokens = steps.first().map_or(req.max_tokens, |s| s.max_tokens);
    match &req.suite {
        Some(suite) => format!(
            "benchmarking \u{201c}{}\u{201d} with suite {suite} — {} prompt(s) \u{d7} {} run(s), \
             {tokens} tokens each",
            model.name,
            steps.len(),
            req.runs,
        ),
        None => format!(
            "benchmarking \u{201c}{}\u{201d} — {} run(s), {tokens} tokens each",
            model.name, req.runs
        ),
    }
}

/// The moving parts `run` carries from step to step.
#[derive(Debug)]
struct Runner<'a> {
    db: &'a Database,
    llama: &'a Arc<LlamaCppAdapter>,
    job_id: &'a str,
    telemetry: watch::Receiver<SystemTelemetry>,
    cancel: watch::Receiver<bool>,
    peak: TelemetryPeak,
    passes: Vec<Pass>,
}

impl Runner<'_> {
    fn sample_peak(&mut self) {
        self.peak.sample(&self.telemetry.borrow_and_update());
    }

    /// Run one step `runs` times, one event per pass. `Ok(false)` means a cancel
    /// was seen before a pass started.
    async fn run_passes(&mut self, step: &Step, runs: u32) -> Result<bool> {
        for i in 1..=runs {
            if *self.cancel.borrow_and_update() {
                return Ok(false);
            }
            let pass = one_pass(self.llama, step).await?;
            self.sample_peak();
            self.db
                .jobs()
                .append_event(
                    self.job_id,
                    EventLevel::Info,
                    &pass_line(step, i, runs, &pass),
                )
                .await?;
            self.passes.push(pass);
        }
        Ok(true)
    }
}

fn pass_line(step: &Step, i: u32, runs: u32, pass: &Pass) -> String {
    match &step.label {
        Some(label) => format!("{label} — pass {i}/{runs}: {:.1} tok/s", pass.gen_tps),
        None => format!(
            "run {i}/{runs}: {:.1} tok/s generation, {:.0} tok/s prompt",
            pass.gen_tps, pass.prompt_tps
        ),
    }
}

/// One `stream_completion` pass; returns its prompt + generation rates.
async fn one_pass(llama: &Arc<LlamaCppAdapter>, step: &Step) -> Result<Pass> {
    let (tx, mut rx) = mpsc::channel::<GenerationEvent>(64);
    let stream = tokio::spawn({
        let llama = Arc::clone(llama);
        let prompt = step.text.clone();
        let max_tokens = step.max_tokens;
        async move { llama.stream_completion(&prompt, max_tokens, tx).await }
    });

    let mut done: Option<Pass> = None;
    while let Some(ev) = rx.recv().await {
        if let GenerationEvent::Done {
            tokens,
            tokens_per_second,
            prompt_tokens_per_second,
        } = ev
        {
            done = Some(Pass {
                prompt_id: step.prompt_id,
                tokens: u64::from(tokens),
                prompt_tps: prompt_tokens_per_second,
                gen_tps: tokens_per_second,
            });
        }
    }
    stream
        .await
        .map_err(|e| bench_err(format!("benchmark stream task panicked: {e}")))??;

    done.ok_or_else(|| bench_err("the model produced no timing data — is it really loaded?"))
}

fn mean(vals: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = vals.fold((0.0, 0u32), |(s, c), v| (s + v, c + 1));
    (n > 0).then(|| sum / f64::from(n))
}

/// Overall and per-prompt means over the passes of one benchmark. Pure, so the
/// arithmetic is testable without a model.
#[derive(Debug)]
struct Aggregate {
    /// Passes actually executed.
    runs: u32,
    gen_tps: Option<f64>,
    prompt_tps: Option<f64>,
    stability: f64,
    /// One entry per tagged prompt, in first-seen order; empty for a legacy run.
    detail: Vec<PromptResult>,
}

fn aggregate(passes: &[Pass]) -> Aggregate {
    let gen_samples: Vec<f64> = passes.iter().map(|p| p.gen_tps).collect();
    let mut detail: Vec<PromptResult> = Vec::new();
    for id in passes.iter().filter_map(|p| p.prompt_id) {
        if detail.iter().any(|d| d.prompt_id == id) {
            continue;
        }
        let mine = || passes.iter().filter(move |p| p.prompt_id == Some(id));
        detail.push(PromptResult {
            prompt_id: id.to_string(),
            tokens: mean(mine().map(|p| p.tokens as f64)).map_or(0, |t| t.round() as u64),
            gen_tps: mean(mine().map(|p| p.gen_tps)),
            prompt_tps: mean(mine().map(|p| p.prompt_tps)),
        });
    }

    Aggregate {
        runs: u32::try_from(passes.len()).unwrap_or(u32::MAX),
        gen_tps: mean(gen_samples.iter().copied()),
        prompt_tps: mean(passes.iter().map(|p| p.prompt_tps)),
        stability: stability_score(&gen_samples),
        detail,
    }
}

/// The per-prompt breakdown as stored JSON; `None` for a legacy run.
fn detail_json(detail: &[PromptResult]) -> Option<String> {
    (!detail.is_empty())
        .then(|| serde_json::to_string(detail).ok())
        .flatten()
}

fn summarise(
    model: &Model,
    req: &BenchRequest,
    passes: &[Pass],
    load: Option<std::time::Duration>,
    vram_budget_mb: u64,
    vram_peak: &TelemetryPeak,
) -> BenchReport {
    let agg = aggregate(passes);

    let ctx = compat::effective_ctx(model.ctx_max.and_then(|v| u32::try_from(v).ok()));
    let free_ram_mb = vram_peak
        .ram_total_mb
        .saturating_sub(vram_peak.ram_used_mb.min(vram_peak.ram_total_mb));
    let fit = compat::verdict(&model.vram_dims(), ctx, vram_budget_mb, free_ram_mb);
    let overall = overall_score(agg.gen_tps.unwrap_or(0.0), agg.stability, &fit);

    BenchReport {
        runs: agg.runs,
        prompt_tps: agg.prompt_tps,
        gen_tps: agg.gen_tps,
        load_ms: load.map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64),
        vram_peak_mb: vram_peak.vram_used_mb,
        ram_peak_mb: (vram_peak.ram_used_mb > 0).then_some(vram_peak.ram_used_mb),
        stability_score: agg.stability,
        overall_score: overall,
        notes: notes(req, agg.runs, load),
        suite: req.suite.clone(),
        detail: agg.detail,
    }
}

fn notes(req: &BenchRequest, runs: u32, load: Option<std::time::Duration>) -> String {
    let load_note = match load {
        Some(_) => "cold load",
        None => "model already resident (load time not measured)",
    };
    match &req.suite {
        Some(suite) => format!("{load_note}; suite {suite}, {runs} pass(es) averaged"),
        None => format!("{load_note}; {runs} run(s) averaged"),
    }
}

/// Running maxima pulled from the telemetry stream during a run. The sampler
/// ticks once a second, so this is a coarse peak — good enough for a heuristic.
#[derive(Debug, Default)]
struct TelemetryPeak {
    vram_used_mb: Option<u64>,
    ram_used_mb: u64,
    ram_total_mb: u64,
}

impl TelemetryPeak {
    fn sample(&mut self, t: &SystemTelemetry) {
        if let GpuStatus::Available(gpu) = &t.gpu {
            self.vram_used_mb = Some(self.vram_used_mb.unwrap_or(0).max(gpu.vram_used_mb));
        }
        self.ram_used_mb = self.ram_used_mb.max(t.host.ram_used_mb);
        self.ram_total_mb = self.ram_total_mb.max(t.host.ram_total_mb);
    }
}

#[cfg(test)]
mod tests;
