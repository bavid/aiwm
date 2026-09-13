//! The job engine: pull a runnable job, resolve its target model (explicit or
//! `Auto`), ask the scheduler where that model goes, act on the decision, and
//! drive the job through the state machine — recording every transition and
//! event.
//!
//! `job_type == "chat"` streams from llama.cpp ([`crate::capability::chat`]);
//! `job_type == "image"` / `"video"` run a fixed workflow on ComfyUI
//! ([`crate::capability::image`] / [`crate::capability::video`]);
//! `job_type == "bench"` runs a local micro-benchmark ([`crate::bench`]); every
//! other type is still a no-op placeholder.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::watch;

use super::JobState;
use crate::bench::{self, BenchOutcome};
use crate::capability;
use crate::capability::chat::{self, ChatOutcome};
use crate::capability::colibri::ColibriOutcome;
use crate::capability::image::{self, ImageOutcome, ImageRequest};
use crate::capability::video::{self, VideoOutcome, VideoRequest};
use crate::compat::{self, VramEstimate};
use crate::db::{EventLevel, Job, JobPatch, Model, NewJob};
use crate::registry::Registry;
use crate::runtime::{ColibriAdapter, ComfyUiAdapter, LlamaCppAdapter, RuntimeRegistry};
use crate::scheduler::{Decision, PlanRequest, Scheduler};
use crate::telemetry::{GpuStatus, SystemTelemetry};
use crate::{recommend, upgrade, CoreError, Database, Result};

const CANCEL_REASON: &str = "cancelled by user";
/// Runtime ids the engine wires capability bodies to.
const LLAMACPP: &str = "llamacpp";
const COMFYUI: &str = "comfyui";
const COLIBRI: &str = "colibri";
/// Fallback VRAM reservation for a ComfyUI model whose import estimate is
/// missing — enough for SDXL on a 16 GB card.
const IMAGE_VRAM_FALLBACK_MB: u64 = 8192;
/// Same, for a video model (Wan 2.2 5B is ~10 GB of weights).
const VIDEO_VRAM_FALLBACK_MB: u64 = 11_264;
/// The job "shape" `media_headroom_mb`'s per-family constants were sized for
/// (matches the UI's own "getting heavy" cutoff — `EASY_PIXELS`/`EASY_FRAMES`
/// in `ui/src/features/video/Video.tsx`, and `Image.tsx`'s 1024×1024 default).
/// A request below this needs proportionally less sampler/VAE-decode memory,
/// a bigger one needs more — a flat per-model number can't tell a small test
/// clip from a full one apart and blocked *every* video job identically on a
/// 16 GB card, no matter what was actually asked for.
const REFERENCE_IMAGE_PIXELS: u64 = 1024 * 1024;
const REFERENCE_VIDEO_PIXEL_FRAMES: u64 = 832 * 480 * 81;
/// Never scale the headroom below this fraction of the per-family constant —
/// some fixed sampler/VAE-decode cost remains no matter how small the ask.
const MIN_HEADROOM_RATIO: f64 = 0.4;

/// How a job came to rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum JobOutcome {
    Completed { job_id: String },
    Blocked { job_id: String, reason: String },
    Cancelled { job_id: String },
    Failed { job_id: String, error: String },
}

/// Where a job's model should run, after `Auto` resolution.
struct Target {
    runtime_id: String,
    model_id: String,
    /// Display name for messages; the id when the model isn't in the library.
    model_name: String,
    /// VRAM to plan against: the larger of the job's explicit ask and the
    /// compatibility estimate.
    vram_mb: u64,
    /// The fit estimate, when the model is in the library.
    estimate: Option<VramEstimate>,
}

#[derive(Debug)]
pub struct JobEngine {
    db: Database,
    registry: RuntimeRegistry,
    scheduler: Arc<dyn Scheduler>,
    llama: Arc<LlamaCppAdapter>,
    comfyui: Arc<ComfyUiAdapter>,
    /// Set only when Colibri is wired up (`with_colibri`) — unlike llama.cpp
    /// and ComfyUI, it's not a required part of every app (CPU-only,
    /// optional, RAM-hungry). A `job_type=colibri` job without one is a clear
    /// config error, not a panic.
    colibri: Option<Arc<ColibriAdapter>>,
    /// Where image jobs write their output (`<job_id>.png`).
    outputs_dir: PathBuf,
    /// Latest system reading — a `bench` job samples the VRAM / RAM peak from it.
    telemetry: watch::Receiver<SystemTelemetry>,
    /// How `Auto` weighs speed vs heft (`[models]` config, 6.6).
    auto_preference: crate::select::AutoPreference,
    /// The online model index — an `upgrade_check` job queries it (6.7).
    model_index: Option<Arc<Registry>>,
    /// Cancel signals for jobs the engine is actively driving right now.
    cancels: Mutex<HashMap<String, watch::Sender<bool>>>,
}

impl JobEngine {
    pub fn new(
        db: Database,
        registry: RuntimeRegistry,
        scheduler: Arc<dyn Scheduler>,
        llama: Arc<LlamaCppAdapter>,
        comfyui: Arc<ComfyUiAdapter>,
        outputs_dir: PathBuf,
    ) -> Self {
        Self {
            db,
            registry,
            scheduler,
            llama,
            comfyui,
            colibri: None,
            outputs_dir,
            telemetry: frozen_telemetry(),
            auto_preference: crate::select::AutoPreference::default(),
            model_index: None,
            cancels: Mutex::new(HashMap::new()),
        }
    }

    /// Feed the engine live system readings so a `bench` job can sample the
    /// VRAM / RAM peak. Without this it uses a static "no GPU" reading.
    #[must_use]
    pub fn with_telemetry(mut self, telemetry: watch::Receiver<SystemTelemetry>) -> Self {
        self.telemetry = telemetry;
        self
    }

    /// Set the `Auto` selection preference (`[models].auto_preference`).
    #[must_use]
    pub fn with_auto_preference(mut self, pref: crate::select::AutoPreference) -> Self {
        self.auto_preference = pref;
        self
    }

    /// Give the engine the model index for `upgrade_check` jobs.
    #[must_use]
    pub fn with_registry(mut self, registry: Arc<Registry>) -> Self {
        self.model_index = Some(registry);
        self
    }

    /// Wire up Colibri so `job_type=colibri` jobs can run. Optional — an app
    /// without it just can't run that job type (a clear config error, not a
    /// panic, if one is ever submitted).
    #[must_use]
    pub fn with_colibri(mut self, colibri: Arc<ColibriAdapter>) -> Self {
        self.colibri = Some(colibri);
        self
    }

    /// Re-point the model index (test helper — [`crate::App::with_registry`]).
    pub fn set_registry(&mut self, registry: Arc<Registry>) {
        self.model_index = Some(registry);
    }

    fn cancels(&self) -> std::sync::MutexGuard<'_, HashMap<String, watch::Sender<bool>>> {
        self.cancels.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ask a job to stop. `Queued` / `Blocked` jobs are cancelled outright; a job
    /// the engine is currently driving is signalled and unwinds itself. Returns
    /// `false` when the job is already finished or in a transient step with no
    /// cancel hook (the caller may retry once it is `running`).
    pub async fn cancel(&self, job_id: &str) -> Result<bool> {
        let job = self
            .db
            .jobs()
            .get(job_id)
            .await?
            .ok_or_else(|| CoreError::Db(format!("no job {job_id}")))?;

        if job.state.is_terminal() {
            return Ok(false);
        }
        if let Some(tx) = self.cancels().get(job_id) {
            let _ = tx.send(true);
            return Ok(true);
        }
        // No live runner: only Queued / Blocked can be cancelled from the outside.
        if matches!(job.state, JobState::Queued | JobState::Blocked) {
            self.db
                .jobs()
                .set_state(
                    job_id,
                    JobState::Cancelled,
                    JobPatch {
                        error_text: Some(CANCEL_REASON.into()),
                        set_finished_at: true,
                        ..Default::default()
                    },
                )
                .await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Queue a job.
    pub async fn submit(&self, new: NewJob) -> Result<Job> {
        self.db.jobs().insert(new).await
    }

    /// Mark crash-interrupted jobs as failed. Call once at startup.
    pub async fn recover(&self) -> Result<u64> {
        let n = self.db.jobs().recover_interrupted().await?;
        if n > 0 {
            tracing::warn!(count = n, "recovered interrupted jobs as failed");
        }
        Ok(n)
    }

    /// Process the next runnable job to a resting state. `None` when idle.
    pub async fn run_next(&self) -> Result<Option<JobOutcome>> {
        let Some(job) = self.db.jobs().next_runnable().await? else {
            return Ok(None);
        };
        Ok(Some(self.drive(job).await))
    }

    async fn drive(&self, job: Job) -> JobOutcome {
        let job_id = job.id.clone();

        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.cancels().insert(job_id.clone(), cancel_tx);
        let result = self.try_drive(job, cancel_rx).await;
        self.cancels().remove(&job_id);

        match result {
            Ok(outcome) => outcome,
            Err(err) => {
                // A cancel that raced the first transition surfaces here as an
                // "invalid transition" error; report the real resting state.
                if let Ok(Some(j)) = self.db.jobs().get(&job_id).await {
                    if j.state == JobState::Cancelled {
                        return JobOutcome::Cancelled { job_id };
                    }
                }
                let error = err.to_string();
                let _ = self
                    .db
                    .jobs()
                    .set_state(
                        &job_id,
                        JobState::Failed,
                        JobPatch {
                            error_text: Some(error.clone()),
                            set_finished_at: true,
                            ..Default::default()
                        },
                    )
                    .await;
                JobOutcome::Failed { job_id, error }
            }
        }
    }

    /// Resolve the job's target — honouring an explicit model, or picking one for
    /// a `chat` job that asked for `Auto`. Computes a fresh VRAM fit estimate
    /// (weights + KV cache at the effective context + overhead) so the scheduler
    /// plans against a realistic number, not the on-disk size.
    async fn resolve_target(&self, job: &Job) -> Result<Target> {
        // Image / video jobs run on ComfyUI; the VRAM math is different (no KV
        // cache), so resolve them on their own path — explicit model or `Auto`.
        if job.job_type == "image" || job.job_type == "video" {
            return self.resolve_comfyui_target(job).await;
        }
        if let (Some(runtime_id), Some(model_id)) = (&job.runtime_id, &job.model_id) {
            // Colibri never charges against the VRAM budget (its constraint is
            // system RAM, checked separately by `capability::colibri`'s own
            // preflight) — the GGUF-shaped weights+KV-cache estimate below
            // would otherwise treat a multi-GB model directory's on-disk size
            // as VRAM weight bytes and wrongly block every Colibri job.
            if runtime_id == COLIBRI {
                let name = self.model_label(model_id).await;
                return Ok(Target {
                    runtime_id: runtime_id.clone(),
                    model_id: model_id.clone(),
                    model_name: name,
                    vram_mb: 0,
                    estimate: None,
                });
            }
            let (name, estimate) = self.vram_estimate(model_id).await;
            let need = estimate.as_ref().map_or(0, |e| e.total_mb);
            return Ok(Target {
                runtime_id: runtime_id.clone(),
                model_id: model_id.clone(),
                model_name: name.unwrap_or_else(|| model_id.clone()),
                vram_mb: job.vram_needed_mb().max(need),
                estimate,
            });
        }
        if job.job_type == "chat" {
            let model = self
                .pick_llm("chat", None)
                .await?
                .ok_or_else(|| CoreError::Runtime {
                    runtime: "llamacpp".into(),
                    message: "no chat model in the library — import a .gguf first".into(),
                })?;
            return self.llm_target(job, model, "auto-selected model").await;
        }
        if job.job_type == "upgrade_check" {
            // Reason with a chat model, or a coding one if that is all there is.
            let model = self
                .pick_llm("chat", Some("coding"))
                .await?
                .ok_or_else(|| CoreError::Runtime {
                    runtime: "llamacpp".into(),
                    message: "no chat or coding model — import a .gguf to run the upgrade check"
                        .into(),
                })?;
            return self.llm_target(job, model, "reasoning with").await;
        }
        if job.job_type == "recommend" {
            // An explicit `reasoner_model_id` (the user picked which local model
            // reasons about the search) wins; otherwise the same chat-or-coding
            // fallback as the upgrade check.
            let explicit = job
                .params
                .get("reasoner_model_id")
                .and_then(serde_json::Value::as_str);
            let model = match explicit {
                Some(id) => self.require_model(id).await?,
                None => self
                    .pick_llm("chat", Some("coding"))
                    .await?
                    .ok_or_else(|| CoreError::Runtime {
                        runtime: "llamacpp".into(),
                        message: "no chat or coding model — import a .gguf to get recommendations"
                            .into(),
                    })?,
            };
            return self.llm_target(job, model, "reasoning with").await;
        }
        Err(CoreError::Runtime {
            runtime: "?".into(),
            message: format!("job {} has no runtime or model to run on", job.id),
        })
    }

    /// Best benchmark-aware pick for `role`, falling back to `fallback_role`.
    async fn pick_llm(&self, role: &str, fallback_role: Option<&str>) -> Result<Option<Model>> {
        let budget = self.scheduler.budget_mb();
        if let Some(m) =
            crate::select::pick_for_role(&self.db, role, budget, self.auto_preference).await?
        {
            return Ok(Some(m));
        }
        match fallback_role {
            Some(r) => {
                crate::select::pick_for_role(&self.db, r, budget, self.auto_preference).await
            }
            None => Ok(None),
        }
    }

    /// Bind `model` to a llama.cpp job, log it, and build the `Target`.
    async fn llm_target(&self, job: &Job, model: Model, verb: &str) -> Result<Target> {
        self.db
            .jobs()
            .assign(&job.id, "llamacpp", &model.id)
            .await?;
        self.db
            .jobs()
            .append_event(
                &job.id,
                EventLevel::Info,
                &format!("{verb} \u{201c}{}\u{201d}", model.name),
            )
            .await?;
        let estimate = compat::estimate(
            &model.vram_dims(),
            compat::effective_ctx(model.ctx_max.and_then(|v| u32::try_from(v).ok())),
        );
        Ok(Target {
            runtime_id: "llamacpp".into(),
            model_id: model.id,
            model_name: model.name,
            vram_mb: job.vram_needed_mb().max(estimate.total_mb),
            estimate: Some(estimate),
        })
    }

    /// Build the `UpgradeTarget` for an `upgrade_check` job from its params.
    async fn upgrade_target(&self, job: &Job) -> Result<upgrade::UpgradeTarget> {
        let model_id = job
            .params
            .get("target_model_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| CoreError::Config("upgrade_check job has no target_model_id".into()))?;
        let model =
            self.db.models().get(model_id).await?.ok_or_else(|| {
                CoreError::Config(format!("model {model_id} is not in the library"))
            })?;

        let family = model
            .family
            .clone()
            .or_else(|| model.arch.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| first_tokens(&model.name, 2));
        let is_llm =
            model.format == "gguf" || model.roles.iter().any(|r| r == "chat" || r == "coding");

        // Repo ids we already have — from the download history's HF URLs.
        let installed_ids = self
            .db
            .downloads()
            .list()
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|d| hf_repo_from_url(&d.url))
            .collect();

        Ok(upgrade::UpgradeTarget {
            label: model.name.clone(),
            family,
            params: model.param_count.and_then(|n| u64::try_from(n).ok()),
            is_llm,
            installed_ids,
        })
    }

    /// Pull the free-text query + [`recommend::MediaKind`] out of a
    /// `recommend` job's params.
    fn recommend_target(&self, job: &Job) -> Result<(String, recommend::MediaKind)> {
        parse_recommend_params(&job.params)
    }

    /// Resolve an `image` / `video` job onto ComfyUI: an explicit model, or
    /// `Auto` (the `base_diffusion` / `base_video`-role model, most-recently-used
    /// first). The VRAM reservation is the model's import estimate (weights + a
    /// headroom for activations / the VAE) — there is no KV cache to size.
    async fn resolve_comfyui_target(&self, job: &Job) -> Result<Target> {
        let (role, missing) = if job.job_type == "video" {
            (
                "base_video",
                "no video model in the library — import Wan 2.2 5B (Video model) first",
            )
        } else {
            (
                "base_diffusion",
                "no image model in the library — import an SDXL .safetensors first",
            )
        };

        let model = match &job.model_id {
            Some(id) => self
                .db
                .models()
                .get(id)
                .await?
                .ok_or_else(|| CoreError::Runtime {
                    runtime: COMFYUI.into(),
                    message: format!("model {id} is not in the library"),
                })?,
            None => {
                let picked = crate::select::pick_for_role(
                    &self.db,
                    role,
                    self.scheduler.budget_mb(),
                    self.auto_preference,
                )
                .await?
                .ok_or_else(|| CoreError::Runtime {
                    runtime: COMFYUI.into(),
                    message: missing.into(),
                })?;
                self.db
                    .jobs()
                    .append_event(
                        &job.id,
                        EventLevel::Info,
                        &format!("auto-selected model \u{201c}{}\u{201d}", picked.name),
                    )
                    .await?;
                picked
            }
        };

        self.db.jobs().assign(&job.id, COMFYUI, &model.id).await?;

        let (width, height, frames) = if job.job_type == "video" {
            let req = VideoRequest::from_params(&job.params)?;
            (req.width, req.height, req.length)
        } else {
            let req = ImageRequest::from_params(&job.params)?;
            (req.width, req.height, 1)
        };

        Ok(Target {
            runtime_id: COMFYUI.into(),
            model_id: model.id.clone(),
            model_name: model.name.clone(),
            vram_mb: job
                .vram_needed_mb()
                .max(media_vram_mb(&model, width, height, frames)),
            estimate: None,
        })
    }

    /// A model's display name, falling back to its id when it isn't in the
    /// library (or the lookup fails).
    async fn model_label(&self, model_id: &str) -> String {
        self.db
            .models()
            .get(model_id)
            .await
            .ok()
            .flatten()
            .map_or_else(|| model_id.to_string(), |m| m.name)
    }

    /// The GPU driver's real free VRAM right now (all processes, not just ours),
    /// from the live telemetry sampler; `None` with no NVIDIA GPU (or the frozen
    /// pre-`with_telemetry` reading in tests) — the scheduler then falls back to
    /// its own budget bookkeeping alone.
    fn live_free_vram_mb(&self) -> Option<u64> {
        match self.telemetry.borrow().gpu {
            GpuStatus::Available(ref gpu) => Some(gpu.vram_free_mb),
            GpuStatus::Unavailable { .. } => None,
        }
    }

    /// `(display name, fit estimate)` for a library model; `(None, None)` when the
    /// job names a model that was never imported (e.g. a synthetic test id).
    async fn vram_estimate(&self, model_id: &str) -> (Option<String>, Option<VramEstimate>) {
        match self.db.models().get(model_id).await.ok().flatten() {
            Some(m) => {
                let ctx = compat::effective_ctx(m.ctx_max.and_then(|v| u32::try_from(v).ok()));
                (
                    Some(m.name.clone()),
                    Some(compat::estimate(&m.vram_dims(), ctx)),
                )
            }
            None => (None, None),
        }
    }

    async fn try_drive(
        &self,
        mut job: Job,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<JobOutcome> {
        let Target {
            runtime_id,
            model_id,
            model_name,
            vram_mb,
            estimate,
        } = self.resolve_target(&job).await?;

        let request = PlanRequest {
            job_id: job.id.clone(),
            runtime_id: runtime_id.clone(),
            model_id: model_id.clone(),
            vram_needed_mb: vram_mb,
            is_agent_session: job.is_agent_session(),
            live_free_vram_mb: self.live_free_vram_mb(),
        };

        // A job already resting in `blocked` is still in the runnable set, so the
        // loop re-enters here on every tick. Re-check the plan silently and only
        // move it (and log) once VRAM has actually freed up — otherwise it would
        // churn `blocked -> scheduled -> blocked` and spam the event log.
        if job.state == JobState::Blocked {
            if let Decision::Blocked { .. } = self.scheduler.plan(&request).await {
                return Ok(JobOutcome::Blocked {
                    reason: job.error_text.clone().unwrap_or_default(),
                    job_id: job.id,
                });
            }
        }

        self.to(&mut job, JobState::Scheduled, JobPatch::default())
            .await?;
        if let Some(o) = self.bail_if_cancelled(&mut job, &mut cancel).await? {
            return Ok(o);
        }

        // How long the engine spent loading the model this run, if it loaded one
        // — the `bench` body reports it as the cold load time.
        let mut load_dur: Option<Duration> = None;
        match self.scheduler.plan(&request).await {
            Decision::Blocked { reason } => {
                let reason = blocked_message(&model_name, estimate.as_ref(), &reason);
                self.db
                    .jobs()
                    .append_event(&job.id, EventLevel::Warn, &reason)
                    .await?;
                self.to(
                    &mut job,
                    JobState::Blocked,
                    JobPatch {
                        error_text: Some(reason.clone()),
                        ..Default::default()
                    },
                )
                .await?;
                return Ok(JobOutcome::Blocked {
                    job_id: job.id,
                    reason,
                });
            }
            Decision::RunNow => {
                self.to(&mut job, JobState::Preparing, JobPatch::default())
                    .await?;
            }
            Decision::LoadThenRun => {
                self.to(&mut job, JobState::Preparing, JobPatch::default())
                    .await?;
                if let Some(o) = self.bail_if_cancelled(&mut job, &mut cancel).await? {
                    return Ok(o);
                }
                let t0 = Instant::now();
                self.load(&runtime_id, &model_id, request.vram_needed_mb)
                    .await?;
                load_dur = Some(t0.elapsed());
            }
            Decision::EvictThenLoad { victim_model } => {
                self.to(&mut job, JobState::Preparing, JobPatch::default())
                    .await?;
                if let Some(o) = self.bail_if_cancelled(&mut job, &mut cancel).await? {
                    return Ok(o);
                }
                let victim = self.model_label(&victim_model).await;
                self.db
                    .jobs()
                    .append_event(
                        &job.id,
                        EventLevel::Info,
                        &format!("made room on the GPU — unloaded \u{201c}{victim}\u{201d}"),
                    )
                    .await?;
                self.evict(&victim_model).await?;
                let t0 = Instant::now();
                self.load(&runtime_id, &model_id, request.vram_needed_mb)
                    .await?;
                load_dur = Some(t0.elapsed());
            }
        }

        if let Some(o) = self.bail_if_cancelled(&mut job, &mut cancel).await? {
            return Ok(o);
        }
        if request.is_agent_session {
            self.scheduler.pin(&model_id);
        }

        self.to(
            &mut job,
            JobState::Running,
            JobPatch {
                set_started_at: true,
                ..Default::default()
            },
        )
        .await?;

        // --- job body ---
        let mut output_path: Option<String> = None;
        if job.job_type == "chat" {
            if runtime_id != LLAMACPP {
                return Err(CoreError::Runtime {
                    runtime: runtime_id.clone(),
                    message: "chat jobs run on llama.cpp".into(),
                });
            }
            let req = chat::ChatRequest::from_params(&job.params)?;
            match chat::run(
                &self.db,
                &self.llama,
                &job.id,
                job.session_id.as_deref(),
                req,
                cancel,
            )
            .await?
            {
                ChatOutcome::Done(done) => {
                    let _ = self.db.models().mark_used(&model_id).await;
                    self.db
                        .jobs()
                        .append_event(
                            &job.id,
                            EventLevel::Info,
                            &format!(
                                "answered — {} tokens, {:.1} tok/s",
                                done.tokens, done.tokens_per_second
                            ),
                        )
                        .await?;
                }
                ChatOutcome::Cancelled { partial } => {
                    self.db
                        .jobs()
                        .append_event(
                            &job.id,
                            EventLevel::Warn,
                            &format!("cancelled after {} chars", partial.chars().count()),
                        )
                        .await?;
                    self.to(
                        &mut job,
                        JobState::Cancelled,
                        JobPatch {
                            error_text: Some(CANCEL_REASON.into()),
                            set_finished_at: true,
                            ..Default::default()
                        },
                    )
                    .await?;
                    return Ok(JobOutcome::Cancelled { job_id: job.id });
                }
            }
        } else if job.job_type == "image" {
            if runtime_id != COMFYUI {
                return Err(CoreError::Runtime {
                    runtime: runtime_id.clone(),
                    message: "image jobs run on ComfyUI".into(),
                });
            }
            let model = self.require_model(&model_id).await?;
            let req = image::ImageRequest::from_params(&job.params)?;
            // Pin the resolved request (concrete seed) back onto the job.
            let mut params = job.params.clone();
            req.apply_to(&mut params);
            self.db.jobs().set_params(&job.id, &params).await?;

            match image::run(
                &self.db,
                &self.comfyui,
                &self.outputs_dir,
                &job.id,
                &model,
                req,
                cancel,
            )
            .await?
            {
                ImageOutcome::Done(done) => {
                    let _ = self.db.models().mark_used(&model_id).await;
                    self.db
                        .jobs()
                        .append_event(
                            &job.id,
                            EventLevel::Info,
                            &format!(
                                "image ready — {}×{}, seed {}",
                                done.width, done.height, done.seed
                            ),
                        )
                        .await?;
                    output_path = Some(done.output_path.to_string_lossy().into_owned());
                }
                ImageOutcome::Cancelled => {
                    self.db
                        .jobs()
                        .append_event(&job.id, EventLevel::Warn, "cancelled while rendering")
                        .await?;
                    self.to(
                        &mut job,
                        JobState::Cancelled,
                        JobPatch {
                            error_text: Some(CANCEL_REASON.into()),
                            set_finished_at: true,
                            ..Default::default()
                        },
                    )
                    .await?;
                    return Ok(JobOutcome::Cancelled { job_id: job.id });
                }
            }
        } else if job.job_type == "video" {
            if runtime_id != COMFYUI {
                return Err(CoreError::Runtime {
                    runtime: runtime_id.clone(),
                    message: "video jobs run on ComfyUI".into(),
                });
            }
            let model = self.require_model(&model_id).await?;
            let req = video::VideoRequest::from_params(&job.params)?;
            let mut params = job.params.clone();
            req.apply_to(&mut params);
            self.db.jobs().set_params(&job.id, &params).await?;

            match video::run(
                &self.db,
                &self.comfyui,
                &self.outputs_dir,
                &job.id,
                &model,
                req,
                cancel,
            )
            .await?
            {
                VideoOutcome::Done(done) => {
                    let _ = self.db.models().mark_used(&model_id).await;
                    self.db
                        .jobs()
                        .append_event(
                            &job.id,
                            EventLevel::Info,
                            &format!(
                                "video ready — {}×{}, {} frames @ {} fps, seed {}",
                                done.width, done.height, done.length, done.fps, done.seed
                            ),
                        )
                        .await?;
                    output_path = Some(done.output_path.to_string_lossy().into_owned());
                }
                VideoOutcome::Cancelled => {
                    self.db
                        .jobs()
                        .append_event(&job.id, EventLevel::Warn, "cancelled while rendering")
                        .await?;
                    self.to(
                        &mut job,
                        JobState::Cancelled,
                        JobPatch {
                            error_text: Some(CANCEL_REASON.into()),
                            set_finished_at: true,
                            ..Default::default()
                        },
                    )
                    .await?;
                    return Ok(JobOutcome::Cancelled { job_id: job.id });
                }
            }
        } else if job.job_type == "bench" {
            if runtime_id != LLAMACPP {
                return Err(CoreError::Runtime {
                    runtime: runtime_id.clone(),
                    message: "benchmarks run on llama.cpp".into(),
                });
            }
            let model = self.require_model(&model_id).await?;
            let req = bench::BenchRequest::from_params(&job.params);
            match bench::run(
                &self.db,
                &self.llama,
                self.telemetry.clone(),
                &job.id,
                &model,
                req,
                load_dur,
                self.scheduler.budget_mb(),
                cancel,
            )
            .await?
            {
                BenchOutcome::Done(report) => {
                    let _ = self.db.models().mark_used(&model_id).await;
                    self.db
                        .jobs()
                        .append_event(
                            &job.id,
                            EventLevel::Info,
                            &format!(
                                "benchmark done — score {}, {:.1} tok/s",
                                report.overall_score,
                                report.gen_tps.unwrap_or(0.0),
                            ),
                        )
                        .await?;
                }
                BenchOutcome::Cancelled => {
                    self.db
                        .jobs()
                        .append_event(&job.id, EventLevel::Warn, "cancelled mid-benchmark")
                        .await?;
                    self.to(
                        &mut job,
                        JobState::Cancelled,
                        JobPatch {
                            error_text: Some(CANCEL_REASON.into()),
                            set_finished_at: true,
                            ..Default::default()
                        },
                    )
                    .await?;
                    return Ok(JobOutcome::Cancelled { job_id: job.id });
                }
            }
        } else if job.job_type == "upgrade_check" {
            if runtime_id != LLAMACPP {
                return Err(CoreError::Runtime {
                    runtime: runtime_id.clone(),
                    message: "the upgrade check reasons with llama.cpp".into(),
                });
            }
            let registry = self.model_index.clone().ok_or_else(|| {
                CoreError::Config("the upgrade check needs the model registry".into())
            })?;
            let target = self.upgrade_target(&job).await?;
            let free_ram_mb = {
                let t = self.telemetry.borrow();
                t.host
                    .ram_total_mb
                    .saturating_sub(t.host.ram_used_mb.min(t.host.ram_total_mb))
            };
            self.db
                .jobs()
                .append_event(
                    &job.id,
                    EventLevel::Info,
                    &format!(
                        "asking Hugging Face for a better \u{201c}{}\u{201d}",
                        target.label
                    ),
                )
                .await?;
            let report = upgrade::run(
                &registry,
                &*self.llama,
                &target,
                self.scheduler.budget_mb(),
                free_ram_mb,
            )
            .await?;
            let json = serde_json::to_string(&report)
                .map_err(|e| CoreError::Db(format!("serialize upgrade report: {e}")))?;
            self.db.jobs().set_result(&job.id, &json).await?;
            let _ = self.db.models().mark_used(&model_id).await;
            self.db
                .jobs()
                .append_event(
                    &job.id,
                    EventLevel::Info,
                    &format!("{} candidate(s) that fit", report.candidates.len()),
                )
                .await?;
        } else if job.job_type == "recommend" {
            if runtime_id != LLAMACPP {
                return Err(CoreError::Runtime {
                    runtime: runtime_id.clone(),
                    message: "model recommendations reason with llama.cpp".into(),
                });
            }
            let registry = self.model_index.clone().ok_or_else(|| {
                CoreError::Config("model recommendations need the model registry".into())
            })?;
            let (query, kind) = self.recommend_target(&job)?;
            let free_ram_mb = {
                let t = self.telemetry.borrow();
                t.host
                    .ram_total_mb
                    .saturating_sub(t.host.ram_used_mb.min(t.host.ram_total_mb))
            };
            self.db
                .jobs()
                .append_event(
                    &job.id,
                    EventLevel::Info,
                    &format!("asking Hugging Face for: \u{201c}{query}\u{201d}"),
                )
                .await?;
            let report = recommend::run(
                &registry,
                &*self.llama,
                &query,
                kind,
                self.scheduler.budget_mb(),
                free_ram_mb,
            )
            .await?;
            let json = serde_json::to_string(&report)
                .map_err(|e| CoreError::Db(format!("serialize recommend report: {e}")))?;
            self.db.jobs().set_result(&job.id, &json).await?;
            let _ = self.db.models().mark_used(&model_id).await;
            self.db
                .jobs()
                .append_event(
                    &job.id,
                    EventLevel::Info,
                    &format!("{} candidate(s) that fit", report.candidates.len()),
                )
                .await?;
        } else if job.job_type == "colibri" {
            if runtime_id != COLIBRI {
                return Err(CoreError::Runtime {
                    runtime: runtime_id.clone(),
                    message: "colibri jobs run on colibri".into(),
                });
            }
            let colibri = self
                .colibri
                .clone()
                .ok_or_else(|| CoreError::Config("colibri is not configured on this app".into()))?;
            let model = self.require_model(&model_id).await?;
            let req = capability::colibri::ColibriChatRequest::from_params(&job.params)?;
            match capability::colibri::run(&self.db, &colibri, &job.id, &model, req, cancel).await?
            {
                ColibriOutcome::Done(done) => {
                    let _ = self.db.models().mark_used(&model_id).await;
                    self.db
                        .jobs()
                        .append_event(
                            &job.id,
                            EventLevel::Info,
                            &format!("answered — {} tokens", done.tokens),
                        )
                        .await?;
                }
                ColibriOutcome::Cancelled { partial } => {
                    self.db
                        .jobs()
                        .append_event(
                            &job.id,
                            EventLevel::Warn,
                            &format!("cancelled after {} chars", partial.chars().count()),
                        )
                        .await?;
                    self.to(
                        &mut job,
                        JobState::Cancelled,
                        JobPatch {
                            error_text: Some(CANCEL_REASON.into()),
                            set_finished_at: true,
                            ..Default::default()
                        },
                    )
                    .await?;
                    return Ok(JobOutcome::Cancelled { job_id: job.id });
                }
            }
        }

        self.to(&mut job, JobState::Post, JobPatch::default())
            .await?;
        self.to(
            &mut job,
            JobState::Completed,
            JobPatch {
                output_path,
                set_finished_at: true,
                ..Default::default()
            },
        )
        .await?;

        Ok(JobOutcome::Completed { job_id: job.id })
    }

    /// Fetch a library model the engine already resolved, erroring if it
    /// vanished between resolution and use.
    async fn require_model(&self, model_id: &str) -> Result<Model> {
        self.db
            .models()
            .get(model_id)
            .await?
            .ok_or_else(|| CoreError::Runtime {
                runtime: COMFYUI.into(),
                message: format!("model {model_id} disappeared from the library"),
            })
    }

    /// If the cancel flag is set, move `job` to `Cancelled` and return the
    /// outcome; otherwise `None` and carry on.
    async fn bail_if_cancelled(
        &self,
        job: &mut Job,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Option<JobOutcome>> {
        if !*cancel.borrow_and_update() {
            return Ok(None);
        }
        self.to(
            job,
            JobState::Cancelled,
            JobPatch {
                error_text: Some(CANCEL_REASON.into()),
                set_finished_at: true,
                ..Default::default()
            },
        )
        .await?;
        Ok(Some(JobOutcome::Cancelled {
            job_id: job.id.clone(),
        }))
    }

    async fn load(&self, runtime_id: &str, model_id: &str, vram_mb: u64) -> Result<()> {
        let rt = self
            .registry
            .get(runtime_id)
            .ok_or_else(|| CoreError::Runtime {
                runtime: runtime_id.to_string(),
                message: "runtime not registered".into(),
            })?;
        rt.load_model(model_id, vram_mb).await
    }

    async fn evict(&self, model_id: &str) -> Result<()> {
        if let Some(rt) = self.registry.runtime_with_model(model_id) {
            self.scheduler.unpin(model_id);
            rt.unload_model(model_id).await?;
        }
        Ok(())
    }

    async fn to(&self, job: &mut Job, next: JobState, patch: JobPatch) -> Result<()> {
        self.db.jobs().set_state(&job.id, next, patch).await?;
        job.state = next;
        Ok(())
    }
}

/// A telemetry receiver frozen at a "no GPU" reading — the engine's default
/// until [`JobEngine::with_telemetry`] wires in the live sampler. `borrow`
/// keeps working after the sender drops.
fn frozen_telemetry() -> watch::Receiver<SystemTelemetry> {
    use crate::telemetry::{GpuStatus, HostStatus};
    watch::channel(SystemTelemetry {
        captured_at_ms: 0,
        gpu: GpuStatus::Unavailable {
            reason: "telemetry not wired to the engine".into(),
        },
        host: HostStatus {
            ram_total_mb: 0,
            ram_used_mb: 0,
            cpu_total_pct: 0,
            cpu_per_core_pct: Vec::new(),
        },
    })
    .1
}

/// First `n` whitespace tokens of `s`, joined — the family fallback for the
/// upgrade check when a model carries no `family` / `arch`.
fn first_tokens(s: &str, n: usize) -> String {
    s.split_whitespace().take(n).collect::<Vec<_>>().join(" ")
}

/// `owner/repo` from a Hugging Face `…/resolve/…` URL, else `None`.
fn hf_repo_from_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://huggingface.co/")
        .or_else(|| url.strip_prefix("http://huggingface.co/"))?;
    let mut parts = rest.split('/');
    let owner = parts.next().filter(|s| !s.is_empty())?;
    let repo = parts.next().filter(|s| !s.is_empty())?;
    (parts.next() == Some("resolve")).then(|| format!("{owner}/{repo}"))
}

/// Pull the free-text query + [`recommend::MediaKind`] out of a `recommend`
/// job's params (`{"query": "...", "kind": "chat"|"coding"|"image"|"video"|"lora"}`,
/// `kind` defaulting to `"chat"`).
fn parse_recommend_params(params: &serde_json::Value) -> Result<(String, recommend::MediaKind)> {
    let query = params
        .get("query")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or_else(|| CoreError::Config("recommend job has no `query`".into()))?
        .to_string();
    let kind = match params.get("kind").and_then(serde_json::Value::as_str) {
        Some("chat") | None => recommend::MediaKind::Chat,
        Some("coding") => recommend::MediaKind::Coding,
        Some("image") => recommend::MediaKind::Image,
        Some("video") => recommend::MediaKind::Video,
        Some("lora") => recommend::MediaKind::Lora,
        Some(other) => {
            return Err(CoreError::Config(format!(
                "unknown recommend kind {other:?}"
            )));
        }
    };
    Ok((query, kind))
}

/// VRAM (MB) to reserve for this specific image/video job. Starts from the
/// model's import estimate (on-disk weight size + a family-shaped headroom —
/// falls back to a family-aware default when an older import left it unset),
/// then rescales the headroom portion to the job's actual pixel count (and,
/// for video, frame count) instead of always charging the full reference-size
/// headroom — see `REFERENCE_IMAGE_PIXELS`/`REFERENCE_VIDEO_PIXEL_FRAMES`.
fn media_vram_mb(model: &Model, width: u32, height: u32, frames: u32) -> u64 {
    let family = model.family.as_deref();
    let fallback = match family {
        Some("wan" | "ltx") => VIDEO_VRAM_FALLBACK_MB,
        _ => IMAGE_VRAM_FALLBACK_MB,
    };
    let stored = model
        .vram_estimate_mb
        .and_then(|mb| u64::try_from(mb).ok())
        .filter(|mb| *mb > 0)
        .unwrap_or(fallback);

    // Split the stored (weights + headroom) figure back into its two parts
    // using today's per-family headroom constant, so the scaling below only
    // ever touches the headroom -- the weights themselves don't shrink just
    // because a smaller image/clip was asked for.
    let base_headroom = u64::try_from(crate::model::media_headroom_mb(family)).unwrap_or(0);
    let weights = stored.saturating_sub(base_headroom);

    let pixel_units = u64::from(width) * u64::from(height) * u64::from(frames.max(1));
    let reference = if frames > 1 {
        REFERENCE_VIDEO_PIXEL_FRAMES
    } else {
        REFERENCE_IMAGE_PIXELS
    };
    let ratio = if reference == 0 {
        1.0
    } else {
        (pixel_units as f64 / reference as f64).max(MIN_HEADROOM_RATIO)
    };
    let scaled_headroom = (base_headroom as f64 * ratio).round() as u64;

    weights + scaled_headroom
}

/// Turn the scheduler's terse `Blocked` reason into a plain-language sentence
/// that names the model and breaks down where the VRAM goes.
fn blocked_message(
    model_name: &str,
    estimate: Option<&VramEstimate>,
    scheduler_reason: &str,
) -> String {
    match estimate {
        Some(est) => format!(
            "not enough VRAM for {model_name}: needs {} — {scheduler_reason}. \
             Free VRAM by closing the resident model, or import a smaller quant / lower the context.",
            est.describe(),
        ),
        None => format!("not enough VRAM for {model_name}: {scheduler_reason}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::NewModel;
    use crate::runtime::{ComfyDirs, FakeRuntimeAdapter, RuntimeAdapter};
    use crate::scheduler::HybridScheduler;

    struct Fixture {
        engine: JobEngine,
        db: Database,
        rt: Arc<FakeRuntimeAdapter>,
        scheduler: Arc<HybridScheduler>,
    }

    fn test_llama(db: &Database) -> Arc<LlamaCppAdapter> {
        Arc::new(LlamaCppAdapter::with_binary(db.clone(), None))
    }

    /// A ComfyUI adapter with no launch command — image jobs never run in these
    /// unit tests, so it only needs to exist for `JobEngine::new`.
    fn test_comfy(db: &Database) -> Arc<crate::runtime::ComfyUiAdapter> {
        let tmp = std::env::temp_dir();
        Arc::new(crate::runtime::ComfyUiAdapter::with_launch(
            db.clone(),
            None,
            ComfyDirs {
                base: tmp.clone(),
                output: tmp.clone(),
                models_store: tmp,
            },
        ))
    }

    fn test_engine(
        db: &Database,
        registry: RuntimeRegistry,
        scheduler: Arc<dyn Scheduler>,
    ) -> JobEngine {
        JobEngine::new(
            db.clone(),
            registry,
            scheduler,
            test_llama(db),
            test_comfy(db),
            std::env::temp_dir(),
        )
    }

    async fn fixture(budget_mb: u64) -> Fixture {
        let db = Database::connect_in_memory().await.unwrap();
        let registry = RuntimeRegistry::new();
        let rt = Arc::new(FakeRuntimeAdapter::healthy("llamacpp"));
        registry.register(rt.clone());
        let scheduler = Arc::new(HybridScheduler::new(registry.clone(), budget_mb));
        let engine = test_engine(&db, registry, scheduler.clone());
        Fixture {
            engine,
            db,
            rt,
            scheduler,
        }
    }

    #[tokio::test]
    async fn happy_path_runs_a_job_to_completion() {
        let fx = fixture(16_384).await;
        let job = fx
            .engine
            .submit(NewJob::new("noop").on("llamacpp", "qwen-14b", 9_000))
            .await
            .unwrap();

        let outcome = fx.engine.run_next().await.unwrap().unwrap();
        assert_eq!(
            outcome,
            JobOutcome::Completed {
                job_id: job.id.clone()
            }
        );

        let stored = fx.db.jobs().get(&job.id).await.unwrap().unwrap();
        assert_eq!(stored.state, JobState::Completed);
        assert!(stored.started_at.is_some() && stored.finished_at.is_some());
        assert_eq!(fx.rt.vram_used_mb(), 9_000);

        // Every transition left an event trail.
        let events = fx.db.jobs().events(&job.id).await.unwrap();
        let messages: Vec<_> = events.iter().map(|e| e.message.as_str()).collect();
        assert!(messages
            .iter()
            .any(|m| m.contains("scheduled -> preparing")));
        assert!(messages.iter().any(|m| m.contains("post -> completed")));
    }

    #[tokio::test]
    async fn run_next_is_none_when_idle() {
        let fx = fixture(16_384).await;
        assert!(fx.engine.run_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn job_without_runtime_fails_cleanly() {
        let fx = fixture(16_384).await;
        let job = fx.engine.submit(NewJob::new("noop")).await.unwrap();

        let outcome = fx.engine.run_next().await.unwrap().unwrap();
        assert!(matches!(outcome, JobOutcome::Failed { .. }));
        let stored = fx.db.jobs().get(&job.id).await.unwrap().unwrap();
        assert_eq!(stored.state, JobState::Failed);
        assert!(stored.error_text.unwrap().contains("no runtime"));
    }

    #[tokio::test]
    async fn simulated_load_failure_fails_the_job() {
        let db = Database::connect_in_memory().await.unwrap();
        let registry = RuntimeRegistry::new();
        let rt = Arc::new(FakeRuntimeAdapter::new(crate::runtime::FakeConfig {
            fail_load_for: vec!["broken".to_string()],
            ..Default::default()
        }));
        registry.register(rt.clone());
        let scheduler = Arc::new(HybridScheduler::new(registry.clone(), 16_384));
        let engine = test_engine(&db, registry, scheduler);

        let job = engine
            .submit(NewJob::new("noop").on("fake", "broken", 100))
            .await
            .unwrap();
        let outcome = engine.run_next().await.unwrap().unwrap();

        assert!(matches!(outcome, JobOutcome::Failed { .. }));
        assert_eq!(
            db.jobs().get(&job.id).await.unwrap().unwrap().state,
            JobState::Failed
        );
    }

    #[tokio::test]
    async fn blocked_job_stays_blocked_and_is_picked_up_again() {
        // Tiny budget: 2000 usable after margins. A 1500 MB job fits; then a
        // pinned session holds it and a second 1500 MB job is blocked.
        let fx = fixture(2_000 + 1024 + 512).await;

        let a = fx
            .engine
            .submit(
                NewJob::new("agent")
                    .on("llamacpp", "agent-model", 1_500)
                    .agent_session(),
            )
            .await
            .unwrap();
        assert!(matches!(
            fx.engine.run_next().await.unwrap().unwrap(),
            JobOutcome::Completed { .. }
        ));
        assert!(fx.scheduler.is_pinned("agent-model"));

        let b = fx
            .engine
            .submit(NewJob::new("noop").on("llamacpp", "other-model", 1_500))
            .await
            .unwrap();
        let outcome = fx.engine.run_next().await.unwrap().unwrap();
        match outcome {
            JobOutcome::Blocked { job_id, reason } => {
                assert_eq!(job_id, b.id);
                assert!(reason.contains("agent-model"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
        assert_eq!(
            fx.db.jobs().get(&b.id).await.unwrap().unwrap().state,
            JobState::Blocked
        );

        // Session ends: unpin, unload, and the blocked job now goes through.
        fx.scheduler.unpin("agent-model");
        fx.rt.unload_model("agent-model").await.unwrap();
        assert!(matches!(
            fx.engine.run_next().await.unwrap().unwrap(),
            JobOutcome::Completed { job_id } if job_id == b.id
        ));
        let _ = a;
    }

    #[tokio::test]
    async fn blocked_reason_explains_the_vram_math() {
        use crate::db::NewModel;

        // 4000 MB usable after margins.
        let fx = fixture(4_000 + 1024 + 512).await;
        let model = fx
            .db
            .models()
            .insert(NewModel {
                name: "Big Model 14B".into(),
                format: "gguf".into(),
                file_path: "E:\\AI\\models\\llm\\big\\big.gguf".into(),
                size_bytes: 6_000 * 1024 * 1024,
                ctx_max: Some(8192),
                n_layers: Some(32),
                n_embd: Some(4096),
                n_heads: Some(32),
                n_kv_heads: Some(32),
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap();

        let job = fx
            .engine
            .submit(NewJob::new("noop").on("llamacpp", &model.id, 0))
            .await
            .unwrap();
        match fx.engine.run_next().await.unwrap().unwrap() {
            JobOutcome::Blocked { job_id, reason } => {
                assert_eq!(job_id, job.id);
                assert!(reason.contains("Big Model 14B"), "{reason}");
                assert!(reason.contains("weights"), "{reason}");
                assert!(reason.contains("KV cache"), "{reason}");
                assert!(reason.to_lowercase().contains("vram"), "{reason}");
            }
            other => panic!("expected Blocked, got {other:?}"),
        }

        let events = fx.db.jobs().events(&job.id).await.unwrap();
        assert!(events.iter().any(|e| e.message.contains("weights")));

        // Re-driving a job that is already `blocked` and still doesn't fit must
        // be silent — no churn back through `scheduled`, no new events.
        let before = fx.db.jobs().events(&job.id).await.unwrap().len();
        assert!(matches!(
            fx.engine.run_next().await.unwrap().unwrap(),
            JobOutcome::Blocked { .. }
        ));
        assert_eq!(
            fx.db.jobs().get(&job.id).await.unwrap().unwrap().state,
            JobState::Blocked
        );
        assert_eq!(fx.db.jobs().events(&job.id).await.unwrap().len(), before);
    }

    #[tokio::test]
    async fn evict_then_load_swaps_models() {
        // 14848 usable. Load a 10 GB model, then a 6 GB job forces its eviction.
        let fx = fixture(16_384).await;

        fx.engine
            .submit(NewJob::new("noop").on("llamacpp", "big", 10_000))
            .await
            .unwrap();
        fx.engine.run_next().await.unwrap();
        assert_eq!(fx.rt.vram_used_mb(), 10_000);

        let small = fx
            .engine
            .submit(NewJob::new("noop").on("llamacpp", "small", 6_000))
            .await
            .unwrap();
        assert!(matches!(
            fx.engine.run_next().await.unwrap().unwrap(),
            JobOutcome::Completed { .. }
        ));

        let loaded: Vec<_> = fx
            .rt
            .loaded_models()
            .into_iter()
            .map(|m| m.model_id)
            .collect();
        assert_eq!(loaded, ["small"]);

        // The eviction is on the job's event trail, not silent.
        let events = fx.db.jobs().events(&small.id).await.unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.message.contains("made room") && e.message.contains("big")),
            "{events:?}"
        );
    }

    #[tokio::test]
    async fn recover_marks_interrupted_jobs_failed() {
        let fx = fixture(16_384).await;
        let job = fx
            .engine
            .submit(NewJob::new("noop").on("llamacpp", "m", 1_000))
            .await
            .unwrap();
        // Simulate a crash mid-run.
        fx.db
            .jobs()
            .set_state(&job.id, JobState::Scheduled, JobPatch::default())
            .await
            .unwrap();
        fx.db
            .jobs()
            .set_state(&job.id, JobState::Preparing, JobPatch::default())
            .await
            .unwrap();
        fx.db
            .jobs()
            .set_state(&job.id, JobState::Running, JobPatch::default())
            .await
            .unwrap();

        assert_eq!(fx.engine.recover().await.unwrap(), 1);
        assert_eq!(
            fx.db.jobs().get(&job.id).await.unwrap().unwrap().state,
            JobState::Failed
        );
    }

    async fn model_with(db: &Database, family: Option<&str>, vram_estimate_mb: i64) -> Model {
        let id = db
            .models()
            .insert(NewModel {
                name: "m".into(),
                family: family.map(str::to_string),
                format: "safetensors".into(),
                file_path: "m.safetensors".into(),
                vram_estimate_mb: Some(vram_estimate_mb),
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id;
        db.models().get(&id).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn media_vram_mb_is_unchanged_at_the_reference_shape() {
        let db = Database::connect_in_memory().await.unwrap();
        let model = model_with(&db, Some("wan"), 15_680).await;
        assert_eq!(media_vram_mb(&model, 832, 480, 81), 15_680);
    }

    #[tokio::test]
    async fn media_vram_mb_scales_the_headroom_down_for_a_small_video_request() {
        let db = Database::connect_in_memory().await.unwrap();
        // Wan headroom is 6144 MB; a 15680 MB stored estimate at the default
        // 832x480x81 shape decomposes to ~9536 MB weights + 6144 MB headroom.
        let model = model_with(&db, Some("wan"), 15_680).await;

        // A tiny test clip needs far less headroom, but the weights -- which
        // must be loaded onto the GPU no matter the resolution -- don't
        // shrink. This is exactly the case that blocked a small test clip the
        // same as a full-size render before this fix.
        let small = media_vram_mb(&model, 256, 256, 9);
        assert!(
            small < 15_680,
            "expected less than the unscaled estimate, got {small}"
        );
        assert!(
            small > 9_536,
            "the weights alone should still be reserved, got {small}"
        );
    }

    #[tokio::test]
    async fn media_vram_mb_never_drops_the_headroom_below_the_floor() {
        let db = Database::connect_in_memory().await.unwrap();
        let model = model_with(&db, Some("sdxl"), 8_000).await;

        // Some fixed sampler/VAE-decode cost remains no matter how small the
        // request -- the headroom never scales below `MIN_HEADROOM_RATIO`.
        let tiny = media_vram_mb(&model, 256, 256, 1);
        let sdxl_headroom = 2_048.0;
        let weights = 8_000.0 - sdxl_headroom;
        let expected = (weights + sdxl_headroom * MIN_HEADROOM_RATIO).round() as u64;
        assert_eq!(tiny, expected);
    }

    #[test]
    fn parse_recommend_params_reads_query_and_kind() {
        let (query, kind) = parse_recommend_params(&serde_json::json!({
            "query": "  realistic uncensored nsfw  ",
            "kind": "image",
        }))
        .unwrap();
        assert_eq!(query, "realistic uncensored nsfw");
        assert_eq!(kind, recommend::MediaKind::Image);
    }

    #[test]
    fn parse_recommend_params_defaults_kind_to_chat() {
        let (_, kind) =
            parse_recommend_params(&serde_json::json!({ "query": "best coder" })).unwrap();
        assert_eq!(kind, recommend::MediaKind::Chat);
    }

    #[test]
    fn parse_recommend_params_rejects_a_blank_or_missing_query() {
        assert!(parse_recommend_params(&serde_json::json!({})).is_err());
        assert!(parse_recommend_params(&serde_json::json!({ "query": "   " })).is_err());
    }

    #[test]
    fn parse_recommend_params_rejects_an_unknown_kind() {
        let err = parse_recommend_params(&serde_json::json!({
            "query": "x",
            "kind": "spreadsheet",
        }))
        .unwrap_err();
        assert!(err.to_string().contains("spreadsheet"));
    }
}
