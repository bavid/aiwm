//! The training runner: everything that happens to a run between "the user
//! pressed Start" and "a LoRA is in the library" — preflight, the detached
//! launch, file-based polling, pause/resume/cancel, startup recovery, and the
//! import of the finished adapter.
//!
//! **Why polling a file instead of a pipe.** The trainer is launched detached
//! ([`crate::launcher::spawn::launch_detached_quiet`]) and deliberately
//! outlives AIWM, so there is no child handle to await and no pipe to read:
//! a run that started yesterday must be just as observable after a restart as
//! one started a second ago. Everything the UI shows therefore comes off
//! disk — `train.log` (progress bars and lifecycle markers) plus the
//! checkpoint files in the work dir — and the only liveness signal is the
//! recorded PID (see [`crate::training::process`]).
//!
//! **Why a vanished process is `interrupted`, never `failed`.** A missing PID
//! only means "this process is not running", which covers a machine that lost
//! power, a user who killed the tree from Task Manager, and a driver crash
//! just as much as a real training failure. Only an explicit `Error running
//! job:` / OOM-abort marker in the log turns a dead run into `failed`; a
//! silent disappearance keeps its checkpoints and stays resumable.
//!
//! **Caller obligation around [`crate::training::process::kill_tree`]**: that
//! function returns `Ok(())` both when it killed the tree *and* when the PID
//! no longer belongs to our image (it warns and no-ops, because a recycled
//! PID must never be killed). Its `Ok` is therefore not evidence that we
//! stopped anything. [`Runner::pause`] and [`Runner::cancel`] consequently
//! check liveness *first* and reconcile a process that had already vanished
//! from the on-disk state exactly like [`Runner::poll_once`] does, instead of
//! reporting a pause that never happened.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::db::{
    now_rfc3339, Database, DatasetMode, NewTrainingRun, Preset, RunState, TrainingRun,
};
use crate::launcher::spawn::{launch_detached_quiet, SpawnSpec};
use crate::model::{import_model, ImportRequest};
use crate::runtime::training::{TrainingAdapter, RUNTIME_ID as TRAINING_RUNTIME_ID};
use crate::runtime::{RuntimeAdapter, RuntimeRegistry};
use crate::scheduler::{HybridScheduler, Scheduler};
use crate::training::config::{
    config_path, merge_hyperparams, render_yaml, training_folder, Hyperparams, RenderInput,
};
use crate::training::process::{is_alive, kill_tree, read_pid_file, write_pid_file};
use crate::training::profile::{find_for_family, find_for_model, preset_values, TrainingProfile};
use crate::training::progress::{parse_marker, parse_progress, scan_work_dir, tail_log, Marker};
use crate::training::{training_err, TRAINING_MODEL_ID};
use crate::Result;

/// How often the poller re-reads every alive run's log, work dir and PID.
pub const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// The image name the detached trainer runs as — checked alongside the PID so
/// a recycled PID is never mistaken for our process (see
/// [`crate::training::process`]).
pub const TRAINER_IMAGE: &str = "python.exe";

/// `<work_dir>/train.log` — ai-toolkit's own `-l` target *and* the file our
/// launcher appends the wrapper's stdout/stderr to.
const LOG_FILE: &str = "train.log";

/// ai-toolkit's headless entry point, relative to its checkout (the run's cwd).
const RUN_PY: &str = "run.py";

/// Free disk wanted on the work dir's volume before a run starts —
/// checkpoints plus samples plus the trainer's own scratch.
const MIN_FREE_DISK_BYTES: u64 = 20 * 1024 * 1024 * 1024;

/// How far back into an existing log [`Runner::recover`] starts reading after
/// an app restart. A multi-hour run's log is megabytes of redrawn `tqdm`
/// bars; the last chunk carries the current step and any recent marker, which
/// is all the poller needs.
const RECOVERY_TAIL_BYTES: u64 = 64 * 1024;

/// What the user picked in the Training tab before pressing Start.
#[derive(Debug, Clone)]
pub struct StartRequest {
    pub name: String,
    pub target_model_id: String,
    pub dataset_id: String,
    pub trigger_word: String,
    pub preset: Preset,
    pub hyperparams: Hyperparams,
    pub sample_prompts: Vec<String>,
}

/// Everything preflight resolved for one run — the profile it maps to and the
/// directories the rendered config points at.
#[derive(Debug, Clone)]
struct Prepared {
    profile: &'static TrainingProfile,
    base_dir: PathBuf,
    dataset_dir: PathBuf,
    data_kind: DatasetMode,
    hyperparams: Hyperparams,
    prompts: Vec<String>,
}

/// Per-run bookkeeping the poller carries between ticks: how far into the log
/// it has read, and the lifecycle markers it has seen so far. Deliberately
/// in-memory — after a restart [`Runner::recover`] rebuilds what it needs from
/// the log tail rather than persisting a parser cursor.
#[derive(Debug, Clone, Default)]
struct PollState {
    offset: u64,
    completed: bool,
    failure: Option<String>,
    checkpoint_step: Option<u64>,
}

/// A trainer command that replaces `python run.py` — set only by tests
/// (Task 8's `aiwm-fake-trainer`), never in production.
#[derive(Debug, Clone)]
struct TrainerCommand {
    program: PathBuf,
    extra_args: Vec<String>,
}

pub struct Runner {
    db: Database,
    adapter: Arc<TrainingAdapter>,
    scheduler: Arc<HybridScheduler>,
    runtimes: RuntimeRegistry,
    /// `<data>/training` — one subdirectory per run id.
    data_dir: PathBuf,
    /// The canonical model store a finished LoRA is imported into.
    store_root: PathBuf,
    polls: Mutex<BTreeMap<String, PollState>>,
    trainer_command: Option<TrainerCommand>,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner")
            .field("data_dir", &self.data_dir)
            .field("store_root", &self.store_root)
            .field("trainer_command", &self.trainer_command)
            .finish()
    }
}

impl Runner {
    pub fn new(
        db: Database,
        adapter: Arc<TrainingAdapter>,
        scheduler: Arc<HybridScheduler>,
        runtimes: RuntimeRegistry,
        data_dir: PathBuf,
        store_root: PathBuf,
    ) -> Self {
        Self {
            db,
            adapter,
            scheduler,
            runtimes,
            data_dir,
            store_root,
            polls: Mutex::new(BTreeMap::new()),
            trainer_command: None,
        }
    }

    /// Run `program` (with `extra_args` before the config path) instead of the
    /// trainer venv's `python run.py`. Test seam only — Task 8's fake trainer
    /// uses it to exercise the real detached-process lifecycle without a GPU.
    pub fn with_trainer_command(mut self, program: PathBuf, extra_args: Vec<String>) -> Self {
        self.trainer_command = Some(TrainerCommand {
            program,
            extra_args,
        });
        self
    }

    /// `<data>/training/<run_id>` — the run's config, log, PID file and
    /// ai-toolkit output tree. Never deleted automatically, not even on
    /// cancel: the checkpoints in it are the user's.
    pub fn work_dir(&self, run_id: &str) -> PathBuf {
        self.data_dir.join(run_id)
    }

    pub fn log_path(&self, run_id: &str) -> PathBuf {
        self.work_dir(run_id).join(LOG_FILE)
    }

    // ---------------------------------------------------------------- start

    /// Preflight, create the `preparing` row, then launch it.
    pub async fn create_and_start(&self, req: StartRequest) -> Result<TrainingRun> {
        let name = req.name.trim();
        if name.is_empty() {
            return Err(training_err("the training run needs a name"));
        }
        let prep = self
            .preflight(
                Some(req.target_model_id.as_str()),
                Some(req.dataset_id.as_str()),
                req.hyperparams,
                req.sample_prompts.clone(),
            )
            .await?;

        let hyperparams_json = serde_json::to_string(&req.hyperparams)
            .map_err(|e| training_err(format!("cannot store the fine settings: {e}")))?;
        let sample_prompts_json = serde_json::to_string(&prep.prompts)
            .map_err(|e| training_err(format!("cannot store the sample prompts: {e}")))?;

        let run = self
            .db
            .training_runs()
            .create(NewTrainingRun {
                name: name.to_string(),
                profile_family: prep.profile.family.to_string(),
                target_model_id: Some(req.target_model_id.clone()),
                dataset_id: Some(req.dataset_id.clone()),
                data_kind: prep.data_kind,
                trigger_word: req.trigger_word.trim().to_string(),
                preset: req.preset,
                hyperparams_json,
                sample_prompts_json,
                // The id only exists once the row does, so the derived work
                // dir is recorded immediately afterwards.
                work_dir: String::new(),
            })
            .await?;
        let work_dir = self.work_dir(&run.id);
        self.db
            .training_runs()
            .set_work_dir(&run.id, &work_dir.to_string_lossy())
            .await?;

        self.launch_into(&run, &prep, RunState::Preparing, RunState::Running)
            .await?;

        self.db
            .training_runs()
            .get(&run.id)
            .await?
            .ok_or_else(|| training_err("the training run vanished right after starting"))
    }

    /// Launch a row that is still `preparing` (a Start that was interrupted
    /// between the insert and the spawn).
    pub async fn start(&self, run_id: &str) -> Result<()> {
        let run = self.run(run_id).await?;
        if run.state != RunState::Preparing {
            return Err(training_err(format!(
                "this run is {} — only a run that has not started yet can be started",
                run.state.as_str()
            )));
        }
        let prep = self.prepare_from_row(&run).await?;
        self.launch_into(&run, &prep, RunState::Preparing, RunState::Running)
            .await
    }

    // ------------------------------------------------------- pause / resume

    /// Stop the process but keep everything on disk. If the process had
    /// already vanished on its own there was nothing to pause — the run is
    /// reconciled from its log and checkpoints instead (see the module docs on
    /// `kill_tree`'s `Ok`).
    pub async fn pause(&self, run_id: &str) -> Result<()> {
        let run = self.run(run_id).await?;
        if run.state != RunState::Running {
            return Err(training_err(format!(
                "this run is {} — only a running training can be paused",
                run.state.as_str()
            )));
        }
        if !self.stop_process(&run).await? {
            // It died before we got here: whatever the log says it is, it is.
            return self.reconcile_dead(&run).await;
        }
        self.finish(&run, RunState::Paused, None).await
    }

    /// Relaunch a paused/interrupted run with the same config: ai-toolkit sees
    /// its own previous checkpoint under the same job name and resumes from
    /// it. The row stays `resuming` until the poller sees the first progress
    /// line, which is the only proof the trainer really came back.
    pub async fn resume(&self, run_id: &str) -> Result<()> {
        let run = self.run(run_id).await?;
        if !matches!(run.state, RunState::Paused | RunState::Interrupted) {
            return Err(training_err(format!(
                "this run is {} — only a paused or interrupted training can be continued",
                run.state.as_str()
            )));
        }
        let prep = self.prepare_from_row(&run).await?;
        self.db
            .training_runs()
            .set_state_from(&run.id, run.state, RunState::Resuming)
            .await?;
        let resuming = with_state(&run, RunState::Resuming);
        self.launch_into(&resuming, &prep, RunState::Resuming, RunState::Resuming)
            .await
    }

    /// Stop for good. The work dir (checkpoints included) is left alone until
    /// the user deletes the run.
    pub async fn cancel(&self, run_id: &str) -> Result<()> {
        let run = self.run(run_id).await?;
        if run.state.is_terminal() {
            return Err(training_err(format!(
                "this run is already {} — there is nothing to cancel",
                run.state.as_str()
            )));
        }
        if run.state == RunState::Finishing {
            return Err(training_err(
                "this run is importing its result — it will be done in a moment",
            ));
        }
        // Unlike pause, cancel means the same thing whether or not the process
        // was still there, so a `false` here needs no reconciliation.
        self.stop_process(&run).await?;
        self.finish(&run, RunState::Cancelled, None).await
    }

    // ----------------------------------------------------------------- poll

    /// One pass over every `running`/`resuming` run. Never fails because one
    /// run misbehaved — a per-run error is logged and the others still run.
    pub async fn poll_once(&self) -> Result<()> {
        for run in self.db.training_runs().list_alive().await? {
            if let Err(e) = self.poll_run(&run).await {
                tracing::warn!(run = %run.id, error = %e, "polling a training run failed");
            }
        }
        Ok(())
    }

    async fn poll_run(&self, run: &TrainingRun) -> Result<()> {
        let work_dir = self.work_dir(&run.id);
        let mut poll = self.poll_state(&run.id);
        let mut state = run.state;

        let (chunk, offset) = tail_log(&self.log_path(&run.id), poll.offset).await?;
        poll.offset = offset;

        let mut latest = None;
        // Split on CR/LF exactly like [`split_updates`], but keep each
        // raw line for [`parse_marker`]: some markers are matched with their
        // leading indentation (` - 1 completed job`), which trimming would
        // eat. `parse_progress` is indifferent either way.
        for raw in chunk.split(['\r', '\n']) {
            let update = raw.trim();
            if update.is_empty() {
                continue;
            }
            if let Some(progress) = parse_progress(update) {
                latest = Some(progress);
                continue;
            }
            match parse_marker(raw) {
                Some(Marker::Resuming(path)) => {
                    tracing::info!(run = %run.id, %path, "the trainer is resuming from a checkpoint");
                }
                Some(Marker::FoundStep(step)) => {
                    tracing::info!(run = %run.id, step, "the trainer picked up its previous step");
                }
                Some(Marker::Oom { attempt }) => {
                    tracing::warn!(
                        run = %run.id,
                        attempt,
                        "the trainer ran out of video memory and skipped a batch"
                    );
                }
                Some(Marker::OomAbort) => {
                    poll.failure = Some(
                        "ran out of video memory three times in a row — try a smaller \
                         resolution or rank"
                            .to_string(),
                    );
                }
                Some(Marker::Error(msg)) => poll.failure = Some(msg),
                Some(Marker::Completed) => poll.completed = true,
                Some(Marker::Stopped) => {
                    tracing::info!(run = %run.id, "the trainer reported a clean stop");
                }
                Some(Marker::SavedCheckpoint(path)) => {
                    tracing::info!(run = %run.id, %path, "the trainer saved a checkpoint");
                }
                None => {}
            }
        }

        if let Some(progress) = &latest {
            self.db
                .training_runs()
                .set_progress(
                    &run.id,
                    i64::try_from(progress.step).unwrap_or(i64::MAX),
                    i64::try_from(progress.total).unwrap_or(i64::MAX),
                    progress.loss.or(run.last_loss),
                )
                .await?;
            // A relaunched run is only really back once it prints a step.
            if state == RunState::Resuming {
                self.db
                    .training_runs()
                    .set_state_from(&run.id, RunState::Resuming, RunState::Running)
                    .await?;
                state = RunState::Running;
            }
        }

        let scanned = scan_work_dir(&training_folder(&work_dir).join(&run.name), &run.name)?;
        let checkpoint = scanned.latest_checkpoint;
        if let Some((step, _)) = &checkpoint {
            if poll.checkpoint_step != Some(*step) {
                poll.checkpoint_step = Some(*step);
                self.db
                    .training_runs()
                    .set_checkpoint_at(&run.id, &now_rfc3339())
                    .await?;
            }
        }
        self.set_poll_state(&run.id, poll.clone());

        if self.process_alive(run, &work_dir).await {
            return Ok(());
        }

        self.settle(
            &with_state(run, state),
            &poll,
            checkpoint.map(|(_, path)| path),
        )
        .await
    }

    // ------------------------------------------------------------- recovery

    /// At app start: every `running`/`resuming` row is checked against its PID
    /// file. A live process keeps its GPU reservation (and the poller picks it
    /// up from there); anything else is `interrupted` and stays resumable.
    pub async fn recover(&self) -> Result<()> {
        for run in self.db.training_runs().list_alive().await? {
            let work_dir = self.work_dir(&run.id);
            if self.process_alive(&run, &work_dir).await {
                let reserve = find_for_family(&run.profile_family)
                    .map(|p| p.vram.reserve_mb)
                    .unwrap_or_default();
                self.adapter.load_model(TRAINING_MODEL_ID, reserve).await?;
                self.adapter.mark_alive(&run.id, reserve);
                self.scheduler.pin(TRAINING_MODEL_ID);
                let log = self.log_path(&run.id);
                let len = tokio::fs::metadata(&log)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0);
                self.set_poll_state(
                    &run.id,
                    PollState {
                        offset: len.saturating_sub(RECOVERY_TAIL_BYTES),
                        ..PollState::default()
                    },
                );
                tracing::info!(run = %run.id, "reattached to a training run that survived the restart");
                continue;
            }
            tracing::warn!(run = %run.id, "a training run did not survive the restart");
            self.finish(&run, RunState::Interrupted, None).await?;
        }
        Ok(())
    }

    // -------------------------------------------------------------- helpers

    async fn run(&self, run_id: &str) -> Result<TrainingRun> {
        self.db
            .training_runs()
            .get(run_id)
            .await?
            .ok_or_else(|| training_err(format!("there is no training run {run_id}")))
    }

    /// Everything that must be true before a trainer process may be spawned.
    /// Every message is meant to be readable straight out of the UI.
    async fn preflight(
        &self,
        target_model_id: Option<&str>,
        dataset_id: Option<&str>,
        hyperparams: Hyperparams,
        prompts: Vec<String>,
    ) -> Result<Prepared> {
        if !self.adapter.is_installed() {
            return Err(training_err(
                "the trainer is not installed yet — set it up in Settings first",
            ));
        }
        if self.adapter.env_broken() {
            return Err(training_err(
                "the trainer environment is broken — set it up again in Settings",
            ));
        }

        if let Some(other) = self.adapter.alive_run() {
            return Err(training_err(format!(
                "only one training can run at a time — run {other} is still going"
            )));
        }
        if let Some(other) = self.db.training_runs().list_alive().await?.first() {
            return Err(training_err(format!(
                "only one training can run at a time — \"{}\" is still going",
                other.name
            )));
        }

        let target_model_id = target_model_id
            .ok_or_else(|| training_err("this run has no target model to train on"))?;
        let model = self
            .db
            .models()
            .get(target_model_id)
            .await?
            .ok_or_else(|| training_err("the model you picked is no longer in the library"))?;
        let profile = find_for_model(model.family.as_deref(), &model.name, model.param_count)
            .ok_or_else(|| {
                training_err(format!("\"{}\" cannot be trained in AIWM yet", model.name))
            })?;

        let dataset_id =
            dataset_id.ok_or_else(|| training_err("this run has no dataset to learn from"))?;
        let dataset = self
            .db
            .datasets()
            .get(dataset_id)
            .await?
            .ok_or_else(|| training_err("the dataset you picked no longer exists"))?;
        if !profile.data_kind.accepts(dataset.mode) {
            return Err(training_err(format!(
                "\"{}\" learns from {} — the dataset \"{}\" holds {}",
                profile.label,
                describe_kind(profile.data_kind),
                dataset.name,
                dataset.mode.as_str()
            )));
        }
        let export = dataset
            .export_dir
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .ok_or_else(|| {
                training_err(format!(
                    "the dataset \"{}\" has not been exported yet — prepare it first",
                    dataset.name
                ))
            })?;
        let dataset_dir = PathBuf::from(export);
        if !dataset_dir.is_dir() {
            return Err(training_err(format!(
                "the export folder of \"{}\" is gone ({}) — export the dataset again",
                dataset.name,
                dataset_dir.display()
            )));
        }
        if media_count(&dataset_dir) == 0 {
            return Err(training_err(format!(
                "the export folder of \"{}\" contains no images or clips — export it again",
                dataset.name
            )));
        }

        let base_dir = self.base_weights_dir(profile).await?;

        let prompts: Vec<String> = prompts
            .into_iter()
            .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|p| !p.is_empty())
            .collect();
        if prompts.is_empty() {
            return Err(training_err(
                "at least one sample prompt is required so you can see what the run learns",
            ));
        }
        hyperparams.validate()?;

        self.check_disk();

        self.release_gpu().await;
        let free = self.scheduler.free_mb();
        if free < profile.vram.reserve_mb {
            return Err(training_err(format!(
                "not enough free video memory for \"{}\": it needs {} MB and only {} MB is \
                 free{}",
                profile.label,
                profile.vram.reserve_mb,
                free,
                self.pinned_note()
            )));
        }

        Ok(Prepared {
            profile,
            base_dir,
            dataset_dir,
            data_kind: dataset.mode,
            hyperparams,
            prompts,
        })
    }

    /// Re-derive [`Prepared`] for an existing row (start/resume of a run the
    /// user created earlier, possibly in a previous session).
    async fn prepare_from_row(&self, run: &TrainingRun) -> Result<Prepared> {
        let hyperparams: Hyperparams = serde_json::from_str(&run.hyperparams_json)
            .map_err(|e| training_err(format!("this run's fine settings are unreadable: {e}")))?;
        let prompts: Vec<String> = serde_json::from_str(&run.sample_prompts_json)
            .map_err(|e| training_err(format!("this run's sample prompts are unreadable: {e}")))?;
        self.preflight(
            run.target_model_id.as_deref(),
            run.dataset_id.as_deref(),
            hyperparams,
            prompts,
        )
        .await
    }

    /// The library's directory model for `profile.base.role` whose folder
    /// actually holds every file the trainer needs.
    async fn base_weights_dir(&self, profile: &TrainingProfile) -> Result<PathBuf> {
        let candidates = self.db.models().for_role(profile.base.role).await?;
        if candidates.is_empty() {
            return Err(training_err(format!(
                "the base weights for \"{}\" are not in the library yet — download {} first",
                profile.label, profile.base.repo
            )));
        }
        candidates
            .iter()
            .map(|m| PathBuf::from(&m.file_path))
            .find(|dir| {
                profile
                    .base
                    .required_files
                    .iter()
                    .all(|f| dir.join(f).is_file())
            })
            .ok_or_else(|| {
                training_err(format!(
                    "the base weights for \"{}\" are incomplete — download {} again",
                    profile.label, profile.base.repo
                ))
            })
    }

    /// Free every runtime's non-pinned models so the trainer gets the GPU.
    /// Best effort: a runtime that refuses is logged, never fatal — the VRAM
    /// check right after this is what actually decides.
    async fn release_gpu(&self) {
        for rt in self.runtimes.all() {
            if rt.id() == TRAINING_RUNTIME_ID {
                continue;
            }
            for loaded in rt.loaded_models() {
                if self.scheduler.is_pinned(&loaded.model_id) {
                    continue;
                }
                if let Err(e) = rt.unload_model(&loaded.model_id).await {
                    tracing::warn!(
                        runtime = rt.id(),
                        model = %loaded.model_id,
                        error = %e,
                        "could not free a model before starting a training run"
                    );
                }
            }
        }
    }

    /// The trailing half of the "not enough VRAM" message: which pinned
    /// (agent-owned) models are still holding the card.
    fn pinned_note(&self) -> String {
        let held: Vec<String> = self
            .runtimes
            .all()
            .iter()
            .flat_map(|rt| rt.loaded_models())
            .filter(|m| self.scheduler.is_pinned(&m.model_id))
            .map(|m| m.model_id.clone())
            .collect();
        if held.is_empty() {
            String::new()
        } else {
            format!(
                " — {} is still loaded for an open session; close it first",
                held.join(", ")
            )
        }
    }

    /// Warn (never block) when the work dir's volume is nearly full. Uses the
    /// same `sysinfo`-based helper the storage report uses, so this needs no
    /// extra dependency and no `unsafe` Win32 call.
    fn check_disk(&self) {
        match crate::cleanup::volume_free(&self.data_dir) {
            Some((free, _total)) if free < MIN_FREE_DISK_BYTES => tracing::warn!(
                free_gb = free / (1024 * 1024 * 1024),
                "starting a training run with little free disk space"
            ),
            Some(_) => {}
            None => tracing::debug!("could not determine free disk space for the training folder"),
        }
    }

    /// Render the config, spawn the detached trainer, take the reservation.
    async fn launch(&self, run: &TrainingRun, prep: &Prepared) -> Result<()> {
        let work_dir = self.work_dir(&run.id);
        tokio::fs::create_dir_all(&work_dir).await.map_err(|e| {
            training_err(format!(
                "cannot create the training folder {}: {e}",
                work_dir.display()
            ))
        })?;

        let merged = merge_hyperparams(preset_values(prep.profile, run.preset), &prep.hyperparams);
        let yaml = render_yaml(&RenderInput {
            profile: prep.profile,
            run_name: &run.name,
            trigger_word: &run.trigger_word,
            preset: merged,
            base_dir: &prep.base_dir,
            dataset_dir: &prep.dataset_dir,
            work_dir: &work_dir,
            prompts: &prep.prompts,
            data_kind: prep.data_kind,
            overrides: Some(prep.hyperparams),
        })?;
        let config = config_path(&work_dir);
        tokio::fs::write(&config, yaml)
            .await
            .map_err(|e| training_err(format!("cannot write the training config: {e}")))?;

        let log = self.log_path(&run.id);
        // Everything already in the log belongs to an earlier attempt: a stale
        // `Error running job:` from the run we are resuming past must not fail
        // the relaunch the moment it starts.
        let offset = tokio::fs::metadata(&log)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let (program, mut args, cwd) = match &self.trainer_command {
            // A stand-in trainer has no ai-toolkit checkout to start in, so it
            // runs in the work dir it is about to write into.
            Some(cmd) => (
                cmd.program.clone(),
                cmd.extra_args.clone(),
                work_dir.clone(),
            ),
            None => (
                self.adapter.python_bin(),
                vec![RUN_PY.to_string()],
                self.adapter.source_dir(),
            ),
        };
        args.push(config.to_string_lossy().into_owned());
        args.push("-l".to_string());
        args.push(log.to_string_lossy().into_owned());

        let pid = launch_detached_quiet(
            &SpawnSpec {
                title: run.name.clone(),
                cwd,
                program,
                args,
                env: Vec::new(),
            },
            &log,
        )
        .await?;

        write_pid_file(&work_dir, pid, TRAINER_IMAGE).await?;
        self.db
            .training_runs()
            .set_pid(&run.id, Some(i64::from(pid)))
            .await?;

        let reserve = prep.profile.vram.reserve_mb;
        self.adapter.load_model(TRAINING_MODEL_ID, reserve).await?;
        self.adapter.mark_alive(&run.id, reserve);
        self.scheduler.pin(TRAINING_MODEL_ID);

        self.db
            .training_runs()
            .set_progress(&run.id, run.step, i64::from(merged.steps), run.last_loss)
            .await?;
        self.set_poll_state(
            &run.id,
            PollState {
                offset,
                ..PollState::default()
            },
        );
        Ok(())
    }

    /// [`Self::launch`] plus the state move around it: `then` on success, and
    /// `failed` (with the reason recorded) when the spawn itself fell over.
    async fn launch_into(
        &self,
        run: &TrainingRun,
        prep: &Prepared,
        from: RunState,
        then: RunState,
    ) -> Result<()> {
        match self.launch(run, prep).await {
            Ok(()) => {
                if then != from {
                    self.db
                        .training_runs()
                        .set_state_from(&run.id, from, then)
                        .await?;
                }
                Ok(())
            }
            Err(e) => {
                if let Err(inner) = self
                    .finish(run, RunState::Failed, Some(e.to_string()))
                    .await
                {
                    tracing::warn!(run = %run.id, error = %inner, "could not record a failed launch");
                }
                Err(e)
            }
        }
    }

    /// Whether the run's process is still there. Prefers the PID file (it
    /// carries the image name the process was launched as) and falls back to
    /// the recorded PID.
    async fn process_alive(&self, run: &TrainingRun, work_dir: &Path) -> bool {
        let from_file = match read_pid_file(work_dir).await {
            Ok(found) => found,
            Err(e) => {
                tracing::warn!(run = %run.id, error = %e, "unreadable trainer.pid");
                None
            }
        };
        let Some((pid, image)) = from_file.or_else(|| {
            run.pid
                .and_then(|p| u32::try_from(p).ok())
                .map(|p| (p, TRAINER_IMAGE.to_string()))
        }) else {
            return false;
        };
        match is_alive(pid, &image).await {
            Ok(alive) => alive,
            Err(e) => {
                tracing::warn!(run = %run.id, error = %e, "could not check the trainer process");
                false
            }
        }
    }

    /// Kill the run's process tree. `Ok(true)` when there was something to
    /// kill, `Ok(false)` when the process had already gone — see the module
    /// docs on why that distinction cannot come from `kill_tree` itself.
    async fn stop_process(&self, run: &TrainingRun) -> Result<bool> {
        let work_dir = self.work_dir(&run.id);
        if !self.process_alive(run, &work_dir).await {
            return Ok(false);
        }
        let from_file = read_pid_file(&work_dir).await?;
        let Some((pid, image)) = from_file.or_else(|| {
            run.pid
                .and_then(|p| u32::try_from(p).ok())
                .map(|p| (p, TRAINER_IMAGE.to_string()))
        }) else {
            return Ok(false);
        };
        kill_tree(pid, &image).await?;
        tracing::info!(run = %run.id, pid, "stopped a training process tree");
        Ok(true)
    }

    /// What a dead process means, read off the log and the work dir.
    async fn reconcile_dead(&self, run: &TrainingRun) -> Result<()> {
        let poll = self.poll_state(&run.id);
        let work_dir = self.work_dir(&run.id);
        let scanned = scan_work_dir(&training_folder(&work_dir).join(&run.name), &run.name)?;
        self.settle(run, &poll, scanned.latest_checkpoint.map(|(_, p)| p))
            .await
    }

    /// The single place a dead run's outcome is decided: completion (with a
    /// checkpoint to import) wins, then an explicit failure marker, then
    /// `interrupted` — never `failed` on a silent disappearance.
    async fn settle(
        &self,
        run: &TrainingRun,
        poll: &PollState,
        checkpoint: Option<PathBuf>,
    ) -> Result<()> {
        if poll.completed {
            if let Some(path) = checkpoint {
                self.transition(&run.id, run.state, RunState::Finishing)
                    .await?;
                let finishing = with_state(run, RunState::Finishing);
                return match self.import_result(run, &path).await {
                    Ok(model_id) => {
                        self.db
                            .training_runs()
                            .set_result(&run.id, &model_id)
                            .await?;
                        self.finish(&finishing, RunState::Completed, None).await
                    }
                    Err(e) => {
                        self.finish(
                            &finishing,
                            RunState::Failed,
                            Some(format!("the finished LoRA could not be imported: {e}")),
                        )
                        .await
                    }
                };
            }
            tracing::warn!(
                run = %run.id,
                "the trainer reported completion but left no checkpoint behind"
            );
        }
        match &poll.failure {
            Some(reason) => {
                self.finish(run, RunState::Failed, Some(reason.clone()))
                    .await
            }
            None => self.finish(run, RunState::Interrupted, None).await,
        }
    }

    /// Land a run in `next`, record `error` if there is one, and hand the GPU
    /// back.
    async fn finish(&self, run: &TrainingRun, next: RunState, error: Option<String>) -> Result<()> {
        if let Some(text) = &error {
            self.db.training_runs().set_error(&run.id, text).await?;
        }
        self.transition(&run.id, run.state, next).await?;
        self.db.training_runs().set_pid(&run.id, None).await?;
        self.release(&run.id).await;
        Ok(())
    }

    /// A state move that also knows the one detour the state machine needs:
    /// `resuming` has edges to `running`/`failed`/`cancelled` only, so a
    /// relaunched run that produced no progress line before dying reaches
    /// `interrupted` (or `finishing`) through `running` — it did run, it just
    /// never said so.
    async fn transition(&self, run_id: &str, from: RunState, next: RunState) -> Result<()> {
        if from == next {
            return Ok(());
        }
        let runs = self.db.training_runs();
        if from.can_transition_to(next) {
            return runs.set_state_from(run_id, from, next).await;
        }
        if from == RunState::Resuming && RunState::Running.can_transition_to(next) {
            runs.set_state_from(run_id, RunState::Resuming, RunState::Running)
                .await?;
            return runs.set_state_from(run_id, RunState::Running, next).await;
        }
        from.ensure_transition(next)
    }

    /// Hand the GPU back and forget the run's poll cursor.
    async fn release(&self, run_id: &str) {
        if self.adapter.alive_run().as_deref() == Some(run_id) {
            if let Err(e) = self.adapter.unload_model(TRAINING_MODEL_ID).await {
                tracing::warn!(run = %run_id, error = %e, "could not release the training reservation");
            }
            self.adapter.clear_alive();
        }
        self.scheduler.unpin(TRAINING_MODEL_ID);
        self.polls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(run_id);
    }

    /// Put the finished adapter in the library as a LoRA the Image tab can
    /// actually pick: `import_model` does the hashing, the store placement and
    /// the ComfyUI link, then the two fields it cannot know — the inference
    /// family and the `training:<run_id>` provenance — are set on the row.
    async fn import_result(&self, run: &TrainingRun, checkpoint: &Path) -> Result<String> {
        let outcome = import_model(
            &self.db,
            &self.store_root,
            ImportRequest {
                source_path: checkpoint.to_path_buf(),
                roles: Vec::new(),
                // The work dir stays intact: its checkpoints are the user's,
                // and a later resume needs them back.
                keep_original: true,
                model_type: Some("lora".to_string()),
            },
        )
        .await?;
        self.db
            .models()
            .set_family_and_source(
                &outcome.model.id,
                Some(library_family(&run.profile_family)),
                &format!("training:{}", run.id),
            )
            .await?;
        self.db
            .models()
            .rename(&outcome.model.id, &run.name)
            .await?;
        tracing::info!(run = %run.id, model = %outcome.model.id, "imported a trained LoRA");
        Ok(outcome.model.id)
    }

    fn poll_state(&self, run_id: &str) -> PollState {
        self.polls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .cloned()
            .unwrap_or_default()
    }

    fn set_poll_state(&self, run_id: &str, state: PollState) {
        self.polls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run_id.to_string(), state);
    }
}

/// Recover any runs left behind by the previous session, then poll every
/// [`POLL_INTERVAL`] forever. Logged, never panicking — one bad poll must not
/// take the loop down with it.
pub fn spawn_poller(runner: Arc<Runner>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = runner.recover().await {
            tracing::warn!(error = %e, "training-run recovery failed");
        }
        loop {
            if let Err(e) = runner.poll_once().await {
                tracing::warn!(error = %e, "the training poller hit an error");
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
}

/// The `models.family` a trained LoRA must carry for the Image tab's filter to
/// offer it next to its base model. The training profiles split FLUX.2 [klein]
/// by size (`flux2-klein-4b`/`-9b`) because the two train differently; the
/// library does not — both infer as `flux2`.
fn library_family(profile_family: &str) -> &str {
    if profile_family.starts_with("flux2-klein") || profile_family.starts_with("flux2_klein") {
        "flux2"
    } else {
        profile_family
    }
}

fn describe_kind(kind: crate::training::profile::DataKind) -> &'static str {
    use crate::training::profile::DataKind;
    match kind {
        DataKind::Frames => "images",
        DataKind::Clips => "video clips",
        DataKind::Both => "images or video clips",
    }
}

/// Exported media files (`NNNN.<ext>`) in a dataset's export folder — the
/// captions next to them are `.txt` and do not count.
fn media_count(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            let numbered = path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()));
            let media = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| !x.eq_ignore_ascii_case("txt"));
            numbered && media && path.is_file()
        })
        .count()
}

/// A copy of `run` as the caller now knows it to be — the poller learns a
/// run's real state mid-tick and the helpers below take it by value.
fn with_state(run: &TrainingRun, state: RunState) -> TrainingRun {
    TrainingRun {
        state,
        ..run.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{NewDataset, NewModel};
    use crate::runtime::training::install;

    /// A PID that cannot plausibly be live on Windows.
    const DEAD_PID: u32 = 4_000_000;

    struct Fx {
        _tmp: tempfile::TempDir,
        db: Database,
        adapter: Arc<TrainingAdapter>,
        scheduler: Arc<HybridScheduler>,
        runner: Runner,
        runtimes_dir: PathBuf,
        root: PathBuf,
    }

    /// A complete-looking trainer install so preflight gets past its
    /// "is it installed?" guard without a real download.
    fn fake_install(runtimes_dir: &Path) {
        let py = install::venv_python(runtimes_dir);
        std::fs::create_dir_all(py.parent().expect("venv python has a parent"))
            .expect("create the venv dir");
        std::fs::write(&py, b"py").expect("write the venv python");
        std::fs::create_dir_all(install::source_dir(runtimes_dir)).expect("create the source dir");
        std::fs::write(install::run_py(runtimes_dir), b"# ai-toolkit").expect("write run.py");
        std::fs::write(install::marker_file(runtimes_dir), install::PINNED_COMMIT)
            .expect("write the marker");
    }

    async fn fixture() -> Fx {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let db = Database::connect_in_memory().await.expect("in-memory db");
        let runtimes_dir = root.join("runtimes");
        let adapter = Arc::new(TrainingAdapter::discover(&runtimes_dir));
        let runtimes = RuntimeRegistry::new();
        runtimes.register(adapter.clone());
        let scheduler = Arc::new(HybridScheduler::new(runtimes.clone(), 24_576));
        let runner = Runner::new(
            db.clone(),
            adapter.clone(),
            scheduler.clone(),
            runtimes,
            root.join("training"),
            root.join("store"),
        );
        Fx {
            _tmp: tmp,
            db,
            adapter,
            scheduler,
            runner,
            runtimes_dir,
            root,
        }
    }

    /// A library model the profile registry resolves to the 4B klein profile.
    async fn target_model(db: &Database) -> String {
        db.models()
            .insert(NewModel {
                publisher: None,
                name: "FLUX.2 klein 4b".into(),
                family: Some("flux2".into()),
                format: "safetensors".into(),
                quant: None,
                arch: None,
                param_count: Some(4_000_000_000),
                file_path: "E:\\store\\flux2.safetensors".into(),
                sha256: None,
                size_bytes: 1,
                ctx_max: None,
                vram_estimate_mb: None,
                ram_estimate_mb: None,
                source: "manual".into(),
                source_revision: None,
                n_layers: None,
                n_embd: None,
                n_heads: None,
                n_kv_heads: None,
                roles: Vec::new(),
            })
            .await
            .expect("insert the target model")
            .id
    }

    async fn dataset(db: &Database, export_dir: Option<&Path>) -> String {
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "cats".into(),
                mode: DatasetMode::Frames,
                source_root: "E:\\pics".into(),
                prep_job_id: None,
            })
            .await
            .expect("create the dataset");
        if let Some(dir) = export_dir {
            std::fs::create_dir_all(dir).expect("create the export dir");
            std::fs::write(dir.join("0001.png"), b"png").expect("write a frame");
            std::fs::write(dir.join("0001.txt"), b"a cat").expect("write a caption");
            db.datasets()
                .set_export_dir(&ds.id, &dir.to_string_lossy())
                .await
                .expect("record the export dir");
        }
        ds.id
    }

    fn start_request(target: &str, dataset_id: &str) -> StartRequest {
        StartRequest {
            name: "testlora".into(),
            target_model_id: target.into(),
            dataset_id: dataset_id.into(),
            trigger_word: "tgr_xy".into(),
            preset: Preset::Fast,
            hyperparams: Hyperparams::default(),
            sample_prompts: vec!["tgr_xy a cat".into()],
        }
    }

    /// Insert a row already in `running`, with a work dir, a dead PID file and
    /// the given log contents — the shape the poller meets after a crash.
    async fn running_run(fx: &Fx, log: &str) -> TrainingRun {
        let run = fx
            .db
            .training_runs()
            .create(NewTrainingRun {
                name: "testlora".into(),
                profile_family: "flux2-klein-4b".into(),
                target_model_id: None,
                dataset_id: None,
                data_kind: DatasetMode::Frames,
                trigger_word: "tgr_xy".into(),
                preset: Preset::Fast,
                hyperparams_json: "{}".into(),
                sample_prompts_json: "[\"tgr_xy a cat\"]".into(),
                work_dir: String::new(),
            })
            .await
            .expect("create the run");
        let work_dir = fx.runner.work_dir(&run.id);
        std::fs::create_dir_all(&work_dir).expect("create the work dir");
        std::fs::write(fx.runner.log_path(&run.id), log).expect("write the log");
        write_pid_file(&work_dir, DEAD_PID, TRAINER_IMAGE)
            .await
            .expect("write the pid file");
        fx.db
            .training_runs()
            .set_pid(&run.id, Some(i64::from(DEAD_PID)))
            .await
            .expect("record the pid");
        fx.db
            .training_runs()
            .set_state(&run.id, RunState::Running)
            .await
            .expect("to running");
        // The reservation a live run holds, so its release can be asserted.
        fx.adapter
            .load_model(TRAINING_MODEL_ID, 12_288)
            .await
            .expect("reserve");
        fx.adapter.mark_alive(&run.id, 12_288);
        fx.scheduler.pin(TRAINING_MODEL_ID);
        reload(fx, &run.id).await
    }

    async fn reload(fx: &Fx, run_id: &str) -> TrainingRun {
        fx.db
            .training_runs()
            .get(run_id)
            .await
            .expect("re-read")
            .expect("the run exists")
    }

    fn assert_released(fx: &Fx) {
        assert_eq!(fx.adapter.alive_run(), None, "the reservation must be gone");
        assert!(
            fx.adapter.loaded_models().is_empty(),
            "the synthetic model must be unloaded"
        );
        assert!(
            !fx.scheduler.is_pinned(TRAINING_MODEL_ID),
            "the reservation must be unpinned"
        );
    }

    #[test]
    fn the_library_family_collapses_both_klein_sizes() {
        assert_eq!(library_family("flux2-klein-4b"), "flux2");
        assert_eq!(library_family("flux2-klein-9b"), "flux2");
        assert_eq!(library_family("sdxl"), "sdxl");
        assert_eq!(library_family("wan"), "wan");
    }

    #[tokio::test]
    async fn preflight_rejects_when_the_trainer_is_not_installed() {
        let fx = fixture().await;
        let target = target_model(&fx.db).await;
        let ds = dataset(&fx.db, Some(&fx.root.join("export"))).await;

        let err = fx
            .runner
            .create_and_start(start_request(&target, &ds))
            .await
            .expect_err("no trainer, no run");

        assert!(
            err.to_string().contains("not installed"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn preflight_rejects_a_dataset_without_an_export() {
        let fx = fixture().await;
        fake_install(&fx.runtimes_dir);
        let target = target_model(&fx.db).await;
        let ds = dataset(&fx.db, None).await;

        let err = fx
            .runner
            .create_and_start(start_request(&target, &ds))
            .await
            .expect_err("an unexported dataset cannot train");

        assert!(
            err.to_string().contains("not been exported"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn preflight_rejects_a_second_concurrent_run() {
        let fx = fixture().await;
        fake_install(&fx.runtimes_dir);
        let _alive = running_run(&fx, "").await;
        let target = target_model(&fx.db).await;
        let ds = dataset(&fx.db, Some(&fx.root.join("export"))).await;

        let err = fx
            .runner
            .create_and_start(start_request(&target, &ds))
            .await
            .expect_err("only one run at a time");

        assert!(
            err.to_string().contains("one training"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn poll_marks_a_vanished_process_interrupted_not_failed() {
        let fx = fixture().await;
        let run = running_run(
            &fx,
            "testlora: 10%|#| 60/600 [00:30<04:30, 2.0it/s, loss: 0.21]\n",
        )
        .await;

        fx.runner.poll_once().await.expect("poll");

        let after = reload(&fx, &run.id).await;
        assert_eq!(after.state, RunState::Interrupted);
        assert_eq!(after.step, 60);
        assert_eq!(after.total_steps, 600);
        assert_eq!(after.error_text, None, "a silent death is not a failure");
        assert_released(&fx);
    }

    #[tokio::test]
    async fn poll_turns_an_error_marker_into_failed_with_the_message() {
        let fx = fixture().await;
        let run = running_run(&fx, "Error running job: boom\n").await;

        fx.runner.poll_once().await.expect("poll");

        let after = reload(&fx, &run.id).await;
        assert_eq!(after.state, RunState::Failed);
        assert_eq!(after.error_text.as_deref(), Some("boom"));
        assert_released(&fx);
    }

    #[tokio::test]
    async fn poll_completes_and_imports_when_the_completion_marker_and_a_checkpoint_exist() {
        let fx = fixture().await;
        let run = running_run(&fx, "Result:\n - 1 completed job\n").await;
        let out = training_folder(&fx.runner.work_dir(&run.id)).join(&run.name);
        std::fs::create_dir_all(&out).expect("create the output dir");
        let checkpoint = out.join(format!("{}_000000100.safetensors", run.name));
        std::fs::write(&checkpoint, b"not really a lora").expect("write the checkpoint");

        fx.runner.poll_once().await.expect("poll");

        let after = reload(&fx, &run.id).await;
        assert_eq!(after.state, RunState::Completed);
        assert!(
            after.last_checkpoint_at.is_some(),
            "the checkpoint timestamp must be recorded"
        );
        let model_id = after.result_model_id.expect("a result model");
        let model = fx
            .db
            .models()
            .get(&model_id)
            .await
            .expect("read the model")
            .expect("the model exists");
        assert_eq!(model.source, format!("training:{}", run.id));
        assert_eq!(model.family.as_deref(), Some("flux2"));
        assert_eq!(model.name, run.name);
        assert!(
            checkpoint.is_file(),
            "the work dir must survive the import untouched"
        );
        assert_released(&fx);
    }

    #[tokio::test]
    async fn cancel_from_interrupted_releases_the_reservation() {
        let fx = fixture().await;
        let run = running_run(&fx, "").await;
        fx.db
            .training_runs()
            .set_state(&run.id, RunState::Interrupted)
            .await
            .expect("to interrupted");

        fx.runner.cancel(&run.id).await.expect("cancel");

        let after = reload(&fx, &run.id).await;
        assert_eq!(after.state, RunState::Cancelled);
        assert_eq!(after.pid, None);
        assert_released(&fx);
    }

    #[tokio::test]
    async fn pause_of_an_already_vanished_process_reports_interrupted() {
        let fx = fixture().await;
        let run = running_run(&fx, "").await;

        fx.runner.pause(&run.id).await.expect("pause");

        let after = reload(&fx, &run.id).await;
        assert_eq!(
            after.state,
            RunState::Interrupted,
            "a process that died on its own was never paused"
        );
        assert_released(&fx);
    }

    #[tokio::test]
    async fn recover_marks_dead_runs_interrupted() {
        let fx = fixture().await;
        let run = running_run(&fx, "").await;

        fx.runner.recover().await.expect("recover");

        let after = reload(&fx, &run.id).await;
        assert_eq!(after.state, RunState::Interrupted);
        assert_released(&fx);
    }
}
