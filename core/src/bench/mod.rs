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
    pub prompt: String,
    /// Generation passes to average (clamped to `1..=10`).
    pub runs: u32,
    /// Tokens to generate per pass (clamped to `16..=512`).
    pub max_tokens: i32,
}

impl Default for BenchRequest {
    fn default() -> Self {
        Self {
            prompt: DEFAULT_PROMPT.to_string(),
            runs: DEFAULT_RUNS,
            max_tokens: DEFAULT_MAX_TOKENS,
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
        let runs = params
            .get("runs")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(d.runs)
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
        }
    }
}

/// The numbers a finished benchmark produced.
#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    pub runs: u32,
    pub prompt_tps: Option<f64>,
    pub gen_tps: Option<f64>,
    pub load_ms: Option<u64>,
    pub vram_peak_mb: Option<u64>,
    pub ram_peak_mb: Option<u64>,
    pub stability_score: f64,
    pub overall_score: u8,
    pub notes: String,
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

/// One generation pass's timing.
#[derive(Debug, Clone, Copy)]
struct Pass {
    prompt_tps: f64,
    gen_tps: f64,
}

/// Run the benchmark against the resident model. `load` is the time the engine
/// spent loading it (`None` when it was already resident). Samples the telemetry
/// peak while it works; writes one [`crate::db::Benchmark`] row on success.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    db: &Database,
    llama: &Arc<LlamaCppAdapter>,
    mut telemetry: watch::Receiver<SystemTelemetry>,
    job_id: &str,
    model: &Model,
    req: BenchRequest,
    load: Option<std::time::Duration>,
    vram_budget_mb: u64,
    mut cancel: watch::Receiver<bool>,
) -> Result<BenchOutcome> {
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "benchmarking \u{201c}{}\u{201d} — {} run(s), {} tokens each",
                model.name, req.runs, req.max_tokens
            ),
        )
        .await?;

    let mut vram_peak = TelemetryPeak::default();
    vram_peak.sample(&telemetry.borrow_and_update());

    let mut passes: Vec<Pass> = Vec::with_capacity(req.runs as usize);
    for i in 1..=req.runs {
        if *cancel.borrow_and_update() {
            return Ok(BenchOutcome::Cancelled);
        }
        let pass = one_pass(llama, &req).await?;
        vram_peak.sample(&telemetry.borrow_and_update());
        db.jobs()
            .append_event(
                job_id,
                EventLevel::Info,
                &format!(
                    "run {i}/{}: {:.1} tok/s generation, {:.0} tok/s prompt",
                    req.runs, pass.gen_tps, pass.prompt_tps
                ),
            )
            .await?;
        passes.push(pass);
    }

    let report = summarise(model, &req, &passes, load, vram_budget_mb, &vram_peak);
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

/// One `stream_completion` pass; returns its prompt + generation rates.
async fn one_pass(llama: &Arc<LlamaCppAdapter>, req: &BenchRequest) -> Result<Pass> {
    let (tx, mut rx) = mpsc::channel::<GenerationEvent>(64);
    let stream = tokio::spawn({
        let llama = Arc::clone(llama);
        let prompt = req.prompt.clone();
        let max_tokens = req.max_tokens;
        async move { llama.stream_completion(&prompt, max_tokens, tx).await }
    });

    let mut done: Option<Pass> = None;
    while let Some(ev) = rx.recv().await {
        if let GenerationEvent::Done {
            tokens_per_second,
            prompt_tokens_per_second,
            ..
        } = ev
        {
            done = Some(Pass {
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

fn summarise(
    model: &Model,
    req: &BenchRequest,
    passes: &[Pass],
    load: Option<std::time::Duration>,
    vram_budget_mb: u64,
    vram_peak: &TelemetryPeak,
) -> BenchReport {
    let gen_samples: Vec<f64> = passes.iter().map(|p| p.gen_tps).collect();
    let gen_tps = mean(gen_samples.iter().copied());
    let prompt_tps = mean(passes.iter().map(|p| p.prompt_tps));
    let stability = stability_score(&gen_samples);

    let ctx = compat::effective_ctx(model.ctx_max.and_then(|v| u32::try_from(v).ok()));
    let free_ram_mb = vram_peak
        .ram_total_mb
        .saturating_sub(vram_peak.ram_used_mb.min(vram_peak.ram_total_mb));
    let fit = compat::verdict(&model.vram_dims(), ctx, vram_budget_mb, free_ram_mb);
    let overall = overall_score(gen_tps.unwrap_or(0.0), stability, &fit);

    let notes = match &load {
        Some(_) => format!("cold load; {} run(s) averaged", req.runs),
        None => format!(
            "model already resident (load time not measured); {} run(s)",
            req.runs
        ),
    };

    BenchReport {
        runs: req.runs,
        prompt_tps,
        gen_tps,
        load_ms: load.map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64),
        vram_peak_mb: vram_peak.vram_used_mb,
        ram_peak_mb: (vram_peak.ram_used_mb > 0).then_some(vram_peak.ram_used_mb),
        stability_score: stability,
        overall_score: overall,
        notes,
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
mod tests {
    use super::*;

    #[test]
    fn stability_of_identical_samples_is_one() {
        assert_eq!(stability_score(&[60.0, 60.0, 60.0]), 1.0);
        assert_eq!(stability_score(&[42.0]), 1.0);
        assert_eq!(stability_score(&[]), 1.0);
    }

    #[test]
    fn stability_drops_as_samples_spread() {
        let tight = stability_score(&[60.0, 61.0, 59.0]);
        let loose = stability_score(&[60.0, 30.0, 90.0]);
        assert!(tight > 0.9, "{tight}");
        assert!(loose < 0.6, "{loose}");
        assert!(tight > loose);
    }

    #[test]
    fn overall_score_rewards_speed_fit_and_stability() {
        let fast_fit = overall_score(80.0, 1.0, &FitVerdict::Green);
        let slow_fit = overall_score(10.0, 1.0, &FitVerdict::Green);
        let fast_tight = overall_score(
            80.0,
            1.0,
            &FitVerdict::Yellow {
                reason: "tight".into(),
            },
        );
        let fast_nofit = overall_score(
            80.0,
            1.0,
            &FitVerdict::Red {
                reason: "over budget".into(),
            },
        );

        assert!(fast_fit > slow_fit);
        assert!(fast_fit > fast_tight);
        assert!(fast_tight > fast_nofit);
        assert_eq!(fast_fit, 100);
        // A model that won't fit is capped low even when it's fast.
        assert!(fast_nofit < 45, "{fast_nofit}");
    }

    #[test]
    fn overall_score_clamps_a_wild_tps() {
        assert_eq!(overall_score(100_000.0, 1.0, &FitVerdict::Green), 100);
        assert_eq!(overall_score(-5.0, 0.0, &FitVerdict::Unknown), 0);
    }

    #[test]
    fn request_from_params_defaults_and_clamps() {
        let d = BenchRequest::from_params(&serde_json::json!({}));
        assert_eq!(d.runs, DEFAULT_RUNS);
        assert_eq!(d.max_tokens, DEFAULT_MAX_TOKENS);
        assert_eq!(d.prompt, DEFAULT_PROMPT);

        let c = BenchRequest::from_params(&serde_json::json!({
            "prompt": "  hi  ", "runs": 99, "max_tokens": 4
        }));
        assert_eq!(c.prompt, "hi");
        assert_eq!(c.runs, MAX_RUNS);
        assert_eq!(c.max_tokens, MIN_MAX_TOKENS);
    }

    // --- `run` against a stand-in llama-server -------------------------------

    use std::net::Ipv4Addr;

    use axum::routing::{get, post};
    use axum::Router;

    use crate::db::{NewJob, NewModel};
    use crate::telemetry::{GpuInfo, GpuStatus, HostStatus, SystemTelemetry};

    /// SSE body: a couple of content chunks, then a finish chunk carrying the
    /// timing numbers the benchmark reads.
    fn sse_completion() -> String {
        let delta = |c: &str| {
            format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{c}\"}},\"finish_reason\":null}}]}}\n\n")
        };
        format!(
            "{}{}data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}],\
             \"timings\":{{\"predicted_n\":40,\"predicted_per_second\":55.0,\"prompt_per_second\":410.0}}}}\n\n\
             data: [DONE]\n\n",
            delta("cache "),
            delta("is fast"),
        )
    }

    async fn mock_llama() -> u16 {
        let router = Router::new()
            .route("/health", get(|| async { axum::http::StatusCode::OK }))
            .route(
                "/v1/chat/completions",
                post(|| async {
                    (
                        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                        sse_completion(),
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        port
    }

    fn telemetry(vram_used_mb: u64, ram_used_mb: u64) -> SystemTelemetry {
        SystemTelemetry {
            captured_at_ms: 1,
            gpu: GpuStatus::Available(GpuInfo {
                name: "Test GPU".into(),
                vram_total_mb: 16_376,
                vram_used_mb,
                vram_free_mb: 16_376 - vram_used_mb,
                utilization_pct: 20,
                temperature_c: 45,
                processes: vec![],
            }),
            host: HostStatus {
                ram_total_mb: 32_000,
                ram_used_mb,
                cpu_total_pct: 10,
                cpu_per_core_pct: vec![],
            },
        }
    }

    #[tokio::test]
    async fn run_measures_records_and_reports() {
        let db = Database::connect_in_memory().await.unwrap();
        let model = db
            .models()
            .insert(NewModel {
                name: "Qwen2.5 7B".into(),
                format: "gguf".into(),
                file_path: "E:\\AI\\models\\llm\\qwen\\qwen.gguf".into(),
                size_bytes: 4_500 * 1024 * 1024,
                ctx_max: Some(32_768),
                n_layers: Some(28),
                n_embd: Some(3_584),
                n_heads: Some(28),
                n_kv_heads: Some(4),
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap();
        let job = db.jobs().insert(NewJob::new("bench")).await.unwrap();

        let port = mock_llama().await;
        let llama = Arc::new(LlamaCppAdapter::with_binary(db.clone(), None));
        llama.attach(port, &model.id, 6_000).await.unwrap();

        let (_tel_tx, tel_rx) = watch::channel(telemetry(6_100, 15_000));
        let (_c_tx, cancel_rx) = watch::channel(false);

        let outcome = run(
            &db,
            &llama,
            tel_rx,
            &job.id,
            &model,
            BenchRequest {
                runs: 3,
                max_tokens: 32,
                ..BenchRequest::default()
            },
            Some(std::time::Duration::from_millis(1_750)),
            16_376,
            cancel_rx,
        )
        .await
        .unwrap();

        let BenchOutcome::Done(report) = outcome else {
            panic!("expected Done, got {outcome:?}");
        };
        assert_eq!(report.runs, 3);
        assert!((report.gen_tps.unwrap() - 55.0).abs() < 0.001);
        assert!((report.prompt_tps.unwrap() - 410.0).abs() < 0.001);
        assert_eq!(report.load_ms, Some(1_750));
        assert_eq!(report.vram_peak_mb, Some(6_100));
        assert_eq!(report.ram_peak_mb, Some(15_000));
        assert!(
            (report.stability_score - 1.0).abs() < 0.001,
            "identical runs"
        );
        assert!(report.overall_score > 0);

        // Persisted, and it is the latest for the model.
        let stored = db
            .benchmarks()
            .latest_for(&model.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.job_id.as_deref(), Some(job.id.as_str()));
        assert_eq!(stored.runs, 3);
        assert_eq!(stored.gen_tps, report.gen_tps);
        assert_eq!(stored.overall_score, i64::from(report.overall_score));

        // Per-run + summary lines on the job's event trail.
        let events = db.jobs().events(&job.id).await.unwrap();
        assert!(events.iter().any(|e| e.message.contains("run 1/3")));
        assert!(events.iter().any(|e| e.message.starts_with("score ")));
    }

    #[tokio::test]
    async fn run_stops_when_cancelled_before_the_first_pass() {
        let db = Database::connect_in_memory().await.unwrap();
        let model = db
            .models()
            .insert(NewModel {
                name: "M".into(),
                format: "gguf".into(),
                file_path: "E:\\AI\\models\\llm\\m\\m.gguf".into(),
                size_bytes: 1_000 * 1024 * 1024,
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap();
        let job = db.jobs().insert(NewJob::new("bench")).await.unwrap();
        let port = mock_llama().await;
        let llama = Arc::new(LlamaCppAdapter::with_binary(db.clone(), None));
        llama.attach(port, &model.id, 1_000).await.unwrap();

        let (_tel_tx, tel_rx) = watch::channel(telemetry(1_000, 10_000));
        let (_c_tx, cancel_rx) = watch::channel(true); // already cancelled

        let outcome = run(
            &db,
            &llama,
            tel_rx,
            &job.id,
            &model,
            BenchRequest::default(),
            None,
            16_376,
            cancel_rx,
        )
        .await
        .unwrap();
        assert!(matches!(outcome, BenchOutcome::Cancelled));
        assert!(db
            .benchmarks()
            .latest_for(&model.id)
            .await
            .unwrap()
            .is_none());
    }
}
