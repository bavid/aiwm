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

use tokio::sync::Mutex as AsyncMutex;

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
use crate::training::progress::{
    parse_marker, parse_progress, scan_work_dir, split_updates, tail_log, Marker,
};
use crate::training::{training_err, TRAINING_MODEL_ID};
use crate::{CoreError, Result};

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
const MIN_FREE_DISK_BYTES: u64 = 20 * BYTES_PER_GB;

/// One gigabyte, the unit both halves of the disk message are phrased in.
const BYTES_PER_GB: u64 = 1024 * 1024 * 1024;

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

/// `(free, total)` bytes on the volume a path lives on — the shape of
/// [`crate::cleanup::volume_free`], behind a function pointer so the
/// preflight disk check can be driven from a test.
pub type FreeSpaceProbe = fn(&Path) -> Option<(u64, u64)>;

/// What one pass over a run's on-disk state found: the updated cursor and
/// markers, the run's state after any progress-driven promotion, and the
/// newest checkpoint it has written.
#[derive(Debug, Clone)]
struct Observation {
    poll: PollState,
    state: RunState,
    checkpoint: Option<PathBuf>,
}

/// A LoRA that reached the library. `warning` is set when the file itself
/// landed but the metadata on top of it could not be finalised — the run is
/// still linked to it, because an imported-but-unfindable LoRA is worse than
/// one with a stale family.
#[derive(Debug, Clone)]
struct ImportedLora {
    model_id: String,
    warning: Option<String>,
}

/// A trainer command that replaces `python run.py` — set only by tests
/// (Task 8's `aiwm-fake-trainer`), never in production. `image` is the name
/// the stand-in's process shows up as in `tasklist`/`taskkill`, which is what
/// every PID check compares against (see [`crate::training::process`]); a
/// fixture is not `python.exe`, so the seam has to carry it.
#[derive(Debug, Clone)]
struct TrainerCommand {
    program: PathBuf,
    extra_args: Vec<String>,
    image: String,
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
    free_space: FreeSpaceProbe,
    /// Held across preflight + create + launch. Preflight refuses a second
    /// concurrent run by reading the database, which is only a decision about
    /// the state it saw; this is what stops two Start clicks from both
    /// passing that check before either has written a row.
    start_lock: AsyncMutex<()>,
    /// PIDs the launch rollback has killed. Test-only: the rollback is a
    /// best-effort side effect with nothing observable left behind once it
    /// has run, so this is the only way to assert it happened at all.
    #[cfg(test)]
    rolled_back: Mutex<Vec<u32>>,
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
            free_space: crate::cleanup::volume_free,
            start_lock: AsyncMutex::new(()),
            #[cfg(test)]
            rolled_back: Mutex::new(Vec::new()),
        }
    }

    /// Replace the free-disk probe the preflight check uses. Test seam only —
    /// the real one reports the machine's actual volumes, which no test can
    /// arrange.
    #[must_use]
    pub fn with_free_space_probe(mut self, probe: FreeSpaceProbe) -> Self {
        self.free_space = probe;
        self
    }

    /// Run `program` (with `extra_args` before the config path) instead of the
    /// trainer venv's `python run.py`, and expect its process to run as
    /// `image` rather than [`TRAINER_IMAGE`]. Test seam only — Task 8's fake
    /// trainer uses it to exercise the real detached-process lifecycle
    /// without a GPU.
    #[must_use]
    pub fn with_trainer_command(
        mut self,
        program: PathBuf,
        extra_args: Vec<String>,
        image: impl Into<String>,
    ) -> Self {
        self.trainer_command = Some(TrainerCommand {
            program,
            extra_args,
            image: image.into(),
        });
        self
    }

    /// The image name this runner's trainer runs as: [`TRAINER_IMAGE`] in
    /// production, the stand-in's own executable when the test seam is set.
    fn trainer_image(&self) -> &str {
        self.trainer_command
            .as_ref()
            .map_or(TRAINER_IMAGE, |cmd| cmd.image.as_str())
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
        let _one_at_a_time = self.start_lock.lock().await;
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
        let _one_at_a_time = self.start_lock.lock().await;
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
        self.finish(&run, RunState::Paused, None).await.map(|_| ())
    }

    /// Relaunch a paused/interrupted run with the same config: ai-toolkit sees
    /// its own previous checkpoint under the same job name and resumes from
    /// it. The row stays `resuming` until the poller sees the first progress
    /// line, which is the only proof the trainer really came back.
    pub async fn resume(&self, run_id: &str) -> Result<()> {
        let _one_at_a_time = self.start_lock.lock().await;
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
        if !self.stop_process(&run).await? {
            // Nothing was killed because nothing was running. The run may
            // have *finished* rather than merely stopped, and rewriting a
            // completed run as `cancelled` would throw away its LoRA — so
            // read the same evidence the poller reads first.
            self.reconcile_dead(&run).await?;
            let settled = self.run(run_id).await?;
            if settled.state.is_terminal() {
                tracing::info!(
                    run = %run_id,
                    state = settled.state.as_str(),
                    "cancel found a run that had already finished on its own"
                );
                return Ok(());
            }
            return self
                .finish(&settled, RunState::Cancelled, None)
                .await
                .map(|_| ());
        }
        self.finish(&run, RunState::Cancelled, None)
            .await
            .map(|_| ())
    }

    // ----------------------------------------------------------------- poll

    /// One pass over every `running`/`resuming` run. Never fails because one
    /// run misbehaved — a per-run error is logged and the others still run.
    pub async fn poll_once(&self) -> Result<()> {
        for run in self.db.training_runs().list_recoverable().await? {
            if let Err(e) = self.poll_run(&run).await {
                tracing::warn!(run = %run.id, error = %e, "polling a training run failed");
            }
        }
        Ok(())
    }

    async fn poll_run(&self, run: &TrainingRun) -> Result<()> {
        // Liveness **before** the reads in `observe`, not after them. A
        // trainer writes its last words — the final checkpoint, the
        // completion block, an `Error running job:` — in the instant before
        // it exits, so a check made *after* the read can find the process
        // already gone while the chunk we read still ends a step or two short
        // of them. That is exactly how a finished run gets settled as
        // `interrupted` (reproduced by `tests/training_run.rs`: the log's
        // completion block landed between the read and the check). Checked
        // first, a `false` here means "everything this run will ever write is
        // already on disk", so the reads that follow are complete by
        // construction; a `true` costs nothing but one more poll interval
        // before the run settles.
        //
        // A `finishing` row is exempt: it has no process left, only an import
        // that did not get to run.
        let alive = run.state != RunState::Finishing
            && self.process_alive(run, &self.work_dir(&run.id)).await;

        let seen = self.observe(run).await?;
        if alive {
            return Ok(());
        }
        self.settle(&with_state(run, seen.state), &seen.poll, seen.checkpoint)
            .await
    }

    /// Read everything new off a run's disk state: the log tail (progress
    /// bars and lifecycle markers) and the work dir (the newest checkpoint).
    /// Shared by the poller, startup recovery and the pause/cancel paths, so
    /// all three judge a dead process from exactly the same evidence.
    async fn observe(&self, run: &TrainingRun) -> Result<Observation> {
        let work_dir = self.work_dir(&run.id);
        let mut poll = self.poll_state(&run.id);
        let mut state = run.state;

        let (chunk, offset) = tail_log(&self.log_path(&run.id), poll.offset).await?;
        poll.offset = offset;

        let mut latest = None;
        for update in split_updates(&chunk) {
            if let Some(progress) = parse_progress(update) {
                latest = Some(progress);
                continue;
            }
            match parse_marker(update) {
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
            if state == RunState::Resuming
                && self
                    .transition(&run.id, RunState::Resuming, RunState::Running)
                    .await?
            {
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

        Ok(Observation {
            poll,
            state,
            checkpoint: checkpoint.map(|(_, path)| path),
        })
    }

    // ------------------------------------------------------------- recovery

    /// At app start: every `running`/`resuming`/`finishing` row is checked
    /// against its PID file. A live process keeps its GPU reservation (and the
    /// poller picks it up from there); everything else is judged from the
    /// same on-disk evidence the poller uses, so a run that finished — or
    /// failed — while AIWM was closed lands where it belongs instead of being
    /// blanket-marked `interrupted`.
    pub async fn recover(&self) -> Result<()> {
        for run in self.db.training_runs().list_recoverable().await? {
            if let Err(e) = self.recover_run(&run).await {
                tracing::warn!(run = %run.id, error = %e, "recovering a training run failed");
            }
        }
        Ok(())
    }

    async fn recover_run(&self, run: &TrainingRun) -> Result<()> {
        // A `finishing` row never has a process — only an unfinished import.
        if run.state != RunState::Finishing
            && self.process_alive(run, &self.work_dir(&run.id)).await
        {
            let reserve = find_for_family(&run.profile_family)
                .map(|p| p.vram.reserve_mb)
                .unwrap_or_default();
            self.adapter.load_model(TRAINING_MODEL_ID, reserve).await?;
            self.adapter.mark_alive(&run.id, reserve);
            self.scheduler.pin(TRAINING_MODEL_ID);
            self.seed_cursor_at_the_tail(&run.id).await;
            tracing::info!(run = %run.id, "reattached to a training run that survived the restart");
            return Ok(());
        }

        tracing::warn!(run = %run.id, "a training run did not survive the restart");
        self.seed_cursor_at_the_tail(&run.id).await;
        self.reconcile_dead(run).await
    }

    /// Start reading this run's log near its end. A multi-hour run's log is
    /// megabytes of redrawn `tqdm` bars, but the markers that decide its fate
    /// are all in the last chunk.
    async fn seed_cursor_at_the_tail(&self, run_id: &str) {
        let len = tokio::fs::metadata(self.log_path(run_id))
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        self.set_poll_state(
            run_id,
            PollState {
                offset: len.saturating_sub(RECOVERY_TAIL_BYTES),
                ..PollState::default()
            },
        );
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
        if let Some(other) = self.db.training_runs().list_recoverable().await?.first() {
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

        self.check_disk()?;

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
    fn check_disk(&self) -> Result<()> {
        let Some((free, _total)) = (self.free_space)(&self.data_dir) else {
            // A volume we cannot measure is not a volume we may refuse: the
            // probe misses network and mounted-folder paths, and a run the
            // user could have completed is worse than a disk-full failure
            // they can read straight off the trainer's log.
            tracing::debug!("could not determine free disk space for the training folder");
            return Ok(());
        };
        if free >= MIN_FREE_DISK_BYTES {
            return Ok(());
        }
        Err(training_err(format!(
            "only {} GB free on {}, at least {} GB needed for the checkpoints and \
             preview images this run writes",
            free / BYTES_PER_GB,
            volume_label(&self.data_dir),
            MIN_FREE_DISK_BYTES / BYTES_PER_GB
        )))
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

        // From here on a real process is running that nothing else knows
        // about: if any of the bookkeeping below fails, the run is not
        // started but the trainer is, and it would grind away on the GPU for
        // hours with no row, no PID file and no way for the user to stop it.
        if let Err(e) = self
            .adopt(run, prep, pid, &work_dir, offset, merged.steps)
            .await
        {
            self.roll_back_spawn(&run.id, pid).await;
            return Err(e);
        }
        Ok(())
    }

    /// Everything between "the trainer is running" and "the run owns it":
    /// the PID file, the recorded PID, the GPU reservation and the poller's
    /// starting cursor. Split out of [`Self::launch`] so the rollback has one
    /// fallible unit to guard.
    async fn adopt(
        &self,
        run: &TrainingRun,
        prep: &Prepared,
        pid: u32,
        work_dir: &Path,
        offset: u64,
        steps: u32,
    ) -> Result<()> {
        write_pid_file(work_dir, pid, self.trainer_image()).await?;
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
            .set_progress(&run.id, run.step, i64::from(steps), run.last_loss)
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

    /// Undo a spawn we could not adopt: kill the process we just started and
    /// give back anything [`Self::adopt`] managed to take before it failed.
    async fn roll_back_spawn(&self, run_id: &str, pid: u32) {
        #[cfg(test)]
        self.rolled_back
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(pid);

        if let Err(e) = kill_tree(pid, self.trainer_image()).await {
            tracing::warn!(pid, error = %e, "could not stop a trainer we failed to adopt");
        }
        if let Err(e) = self.db.training_runs().set_pid(run_id, None).await {
            tracing::warn!(run = %run_id, error = %e, "could not clear a rolled-back PID");
        }
        self.release(run_id).await;
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
                .map(|p| (p, self.trainer_image().to_string()))
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
                .map(|p| (p, self.trainer_image().to_string()))
        }) else {
            return Ok(false);
        };
        kill_tree(pid, &image).await?;
        tracing::info!(run = %run.id, pid, "stopped a training process tree");
        Ok(true)
    }

    /// What a dead process means, read off the log and the work dir.
    async fn reconcile_dead(&self, run: &TrainingRun) -> Result<()> {
        let seen = self.observe(run).await?;
        self.settle(&with_state(run, seen.state), &seen.poll, seen.checkpoint)
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
        // A `finishing` row has already been judged complete — the verdict is
        // in the database, not in this tick's markers (which, after a
        // restart, no longer include the completion block). All that is left
        // is the import, which the app may have died in the middle of.
        if run.state == RunState::Finishing {
            return self.complete(run, checkpoint).await;
        }

        if poll.completed {
            if let Some(path) = checkpoint {
                if !self
                    .transition(&run.id, run.state, RunState::Finishing)
                    .await?
                {
                    // Someone else claimed this run between our read and now.
                    return Ok(());
                }
                return self
                    .complete(&with_state(run, RunState::Finishing), Some(path))
                    .await;
            }
            tracing::warn!(
                run = %run.id,
                "the trainer reported completion but left no checkpoint behind"
            );
        }
        match &poll.failure {
            Some(reason) => self
                .finish(run, RunState::Failed, Some(reason.clone()))
                .await
                .map(|_| ()),
            None => self
                .finish(run, RunState::Interrupted, None)
                .await
                .map(|_| ()),
        }
    }

    /// Turn a `finishing` run into a `completed` one by importing its
    /// checkpoint. Safe to re-enter: `import_model` deduplicates by SHA-256,
    /// so a retry after a crash mid-import finds the same library row rather
    /// than making a second copy.
    async fn complete(&self, run: &TrainingRun, checkpoint: Option<PathBuf>) -> Result<()> {
        let Some(path) = checkpoint else {
            return self
                .finish(
                    run,
                    RunState::Failed,
                    Some("the run reported success but left no checkpoint behind".to_string()),
                )
                .await
                .map(|_| ());
        };
        match self.import_result(run, &path).await {
            Ok(imported) => {
                self.db
                    .training_runs()
                    .set_result(&run.id, &imported.model_id)
                    .await?;
                self.finish(run, RunState::Completed, imported.warning)
                    .await
                    .map(|_| ())
            }
            Err(e) => self
                .finish(
                    run,
                    RunState::Failed,
                    Some(format!("the finished LoRA could not be imported: {e}")),
                )
                .await
                .map(|_| ()),
        }
    }

    /// Land a run in `next`, record `error` if there is one, and hand the GPU
    /// back.
    async fn finish(
        &self,
        run: &TrainingRun,
        next: RunState,
        error: Option<String>,
    ) -> Result<bool> {
        if !self.transition(&run.id, run.state, next).await? {
            // Another writer settled this run first. Its own `finish` already
            // released the GPU; do not stamp our verdict over theirs.
            self.release(&run.id).await;
            return Ok(false);
        }
        if let Some(text) = &error {
            self.db.training_runs().set_error(&run.id, text).await?;
        }
        self.db.training_runs().set_pid(&run.id, None).await?;
        self.release(&run.id).await;
        Ok(true)
    }

    /// A state move that also knows the one detour the state machine needs:
    /// `resuming` has edges to `running`/`failed`/`cancelled` only, so a
    /// relaunched run that produced no progress line before dying reaches
    /// `interrupted` (or `finishing`) through `running` — it did run, it just
    /// never said so.
    async fn transition(&self, run_id: &str, from: RunState, next: RunState) -> Result<bool> {
        if from == next {
            return Ok(true);
        }
        let runs = self.db.training_runs();
        let moved = if from.can_transition_to(next) {
            runs.set_state_from(run_id, from, next).await
        } else if from == RunState::Resuming && RunState::Running.can_transition_to(next) {
            match runs
                .set_state_from(run_id, RunState::Resuming, RunState::Running)
                .await
            {
                Ok(()) => runs.set_state_from(run_id, RunState::Running, next).await,
                Err(e) => Err(e),
            }
        } else {
            return from.ensure_transition(next).map(|()| true);
        };

        match moved {
            Ok(()) => Ok(true),
            Err(e) if is_lost_race(&e) => {
                tracing::info!(
                    run = %run_id,
                    from = from.as_str(),
                    to = next.as_str(),
                    "a training-run state change was overtaken by another writer"
                );
                Ok(false)
            }
            Err(e) => Err(e),
        }
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
    async fn import_result(&self, run: &TrainingRun, checkpoint: &Path) -> Result<ImportedLora> {
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
        let model_id = outcome.model.id;

        // The file is in the store and the library row exists; only the
        // cosmetics are outstanding. Losing the *run* over those would leave
        // a LoRA nothing points at, so a second failure becomes a warning on
        // a completed run rather than an error that discards it.
        let warning = match self.finalise_metadata(&model_id, run).await {
            Ok(()) => None,
            Err(first) => {
                tracing::warn!(run = %run.id, error = %first, "retrying the LoRA's library entry");
                match self.finalise_metadata(&model_id, run).await {
                    Ok(()) => None,
                    Err(second) => Some(format!(
                        "the LoRA was imported but its library entry could not be \
                         finalised ({second}) — rename it and set its family by hand"
                    )),
                }
            }
        };

        tracing::info!(run = %run.id, model = %model_id, "imported a trained LoRA");
        Ok(ImportedLora { model_id, warning })
    }

    /// The two fields `import_model` cannot infer, plus the run's own name.
    async fn finalise_metadata(&self, model_id: &str, run: &TrainingRun) -> Result<()> {
        self.db
            .models()
            .set_family_and_source(
                model_id,
                Some(library_family(&run.profile_family)),
                &format!("training:{}", run.id),
            )
            .await?;
        self.db.models().rename(model_id, &run.name).await?;
        Ok(())
    }

    fn poll_state(&self, run_id: &str) -> PollState {
        self.polls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .cloned()
            .unwrap_or_default()
    }

    /// PIDs [`Self::launch`]'s rollback has killed, oldest first.
    #[cfg(test)]
    fn rolled_back(&self) -> Vec<u32> {
        self.rolled_back
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
/// The compare-and-swap in `TrainingRunRepo::set_state_from` reports a lost
/// race as exactly this error. Losing it is ordinary — the poller settling a
/// process that died while the user was clicking Cancel — not a failure, so
/// the callers that can lose it treat this as "no change" instead of
/// surfacing a scary message the user can do nothing about.
fn is_lost_race(e: &CoreError) -> bool {
    matches!(e, CoreError::Config(msg) if msg.contains("state changed concurrently"))
}

/// The volume a path lives on, for the disk message: `E:` on Windows, the
/// whole path anywhere it has no drive prefix.
fn volume_label(path: &Path) -> String {
    match path.components().next() {
        Some(std::path::Component::Prefix(prefix)) => {
            prefix.as_os_str().to_string_lossy().into_owned()
        }
        _ => path.display().to_string(),
    }
}

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

    /// The only profile with a complete contract at this point in the plan.
    const FAMILY: &str = "flux2-klein-4b";

    struct Fx {
        _tmp: tempfile::TempDir,
        db: Database,
        adapter: Arc<TrainingAdapter>,
        scheduler: Arc<HybridScheduler>,
        runner: Runner,
        runtimes: RuntimeRegistry,
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
            runtimes.clone(),
            root.join("training"),
            root.join("store"),
        );
        Fx {
            _tmp: tmp,
            db,
            adapter,
            scheduler,
            runner,
            runtimes,
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

    /// The complete base-weight directory the 4B klein profile insists on,
    /// registered under its role so preflight can find it.
    async fn install_base_weights(fx: &Fx) -> PathBuf {
        let profile = find_for_family(FAMILY).expect("the 4B klein profile exists");
        let dir = fx.root.join("base");
        for rel in profile.base.required_files {
            let file = dir.join(rel);
            std::fs::create_dir_all(file.parent().expect("a required file has a parent"))
                .expect("create the base weight dir");
            std::fs::write(&file, b"weights").expect("write a base weight");
        }
        fx.db
            .models()
            .insert(NewModel {
                publisher: None,
                name: "FLUX.2 klein 4B base".into(),
                family: None,
                format: "safetensors".into(),
                quant: None,
                arch: None,
                param_count: None,
                file_path: dir.to_string_lossy().into_owned(),
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
                roles: vec![profile.base.role.to_string()],
            })
            .await
            .expect("register the base weights");
        dir
    }

    /// Everything preflight asks for, so a test can reach the check it is
    /// actually about.
    async fn ready_fixture() -> (Fx, String, String) {
        let fx = fixture().await;
        fake_install(&fx.runtimes_dir);
        install_base_weights(&fx).await;
        let target = target_model(&fx.db).await;
        let ds = dataset(&fx.db, Some(&fx.root.join("export"))).await;
        (fx, target, ds)
    }

    /// A `Prepared` built by hand, so [`Runner::launch`] can be driven without
    /// going through preflight.
    fn prepared(fx: &Fx, base_dir: PathBuf) -> Prepared {
        Prepared {
            profile: find_for_family(FAMILY).expect("the 4B klein profile exists"),
            base_dir,
            dataset_dir: fx.root.join("export"),
            data_kind: DatasetMode::Frames,
            hyperparams: Hyperparams::default(),
            prompts: vec!["tgr_xy a cat".to_string()],
        }
    }

    /// The log a finished ai-toolkit run leaves behind.
    const COMPLETION_LOG: &str = "Saved checkpoint to out\n\nResult:\n - 1 completed job\n";

    /// Put the checkpoint a completed run is expected to have written on disk.
    fn write_checkpoint(fx: &Fx, run: &TrainingRun) -> PathBuf {
        let out = training_folder(&fx.runner.work_dir(&run.id)).join(&run.name);
        std::fs::create_dir_all(&out).expect("create the output dir");
        let path = out.join(format!("{}_000000100.safetensors", run.name));
        std::fs::write(&path, b"not really a lora").expect("write the checkpoint");
        path
    }

    async fn reload(fx: &Fx, run_id: &str) -> TrainingRun {
        fx.db
            .training_runs()
            .get(run_id)
            .await
            .expect("re-read")
            .expect("the run exists")
    }

    fn with_name(run: &TrainingRun, name: &str) -> TrainingRun {
        TrainingRun {
            name: name.to_string(),
            ..run.clone()
        }
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
    async fn cancel_of_a_finished_run_completes_it_instead() {
        let fx = fixture().await;
        let run = running_run(&fx, COMPLETION_LOG).await;
        write_checkpoint(&fx, &run);

        fx.runner.cancel(&run.id).await.expect("cancel");

        let after = reload(&fx, &run.id).await;
        assert_eq!(
            after.state,
            RunState::Completed,
            "a run that had already finished must not be rewritten as cancelled"
        );
        assert!(after.result_model_id.is_some(), "the LoRA must be imported");
        assert_released(&fx);
    }

    #[tokio::test]
    async fn preflight_blocks_when_the_disk_is_nearly_full() {
        let (fx, target, ds) = ready_fixture().await;
        let runner = Runner::new(
            fx.db.clone(),
            fx.adapter.clone(),
            fx.scheduler.clone(),
            fx.runtimes.clone(),
            fx.root.join("training"),
            fx.root.join("store"),
        )
        .with_free_space_probe(|_| Some((5 * 1024 * 1024 * 1024, 200 * 1024 * 1024 * 1024)));

        let err = runner
            .create_and_start(start_request(&target, &ds))
            .await
            .expect_err("a nearly full disk must stop the run before it starts");

        let msg = err.to_string();
        assert!(msg.contains("20 GB"), "unexpected error: {msg}");
        assert!(msg.contains("5 GB"), "the free figure must be named: {msg}");
    }

    #[tokio::test]
    async fn preflight_passes_when_the_volume_is_unknown() {
        let (fx, target, ds) = ready_fixture().await;
        let runner = Runner::new(
            fx.db.clone(),
            fx.adapter.clone(),
            fx.scheduler.clone(),
            fx.runtimes.clone(),
            fx.root.join("training"),
            fx.root.join("store"),
        )
        .with_free_space_probe(|_| None);

        // The fake install's `python` is not a real interpreter, so the run
        // gets as far as the spawn and dies there — which is proof the disk
        // check let it through rather than refusing it.
        let err = runner
            .create_and_start(start_request(&target, &ds))
            .await
            .expect_err("the fake interpreter cannot actually launch");
        assert!(
            !err.to_string().contains("20 GB"),
            "an unknown volume must not block: {err}"
        );
    }

    #[tokio::test]
    async fn recover_completes_a_run_that_finished_while_the_app_was_down() {
        let fx = fixture().await;
        let run = running_run(&fx, COMPLETION_LOG).await;
        write_checkpoint(&fx, &run);

        fx.runner.recover().await.expect("recover");

        let after = reload(&fx, &run.id).await;
        assert_eq!(after.state, RunState::Completed);
        assert!(after.result_model_id.is_some());
        assert_released(&fx);
    }

    #[tokio::test]
    async fn recover_finishes_a_finishing_row_that_has_a_checkpoint() {
        let fx = fixture().await;
        let run = running_run(&fx, COMPLETION_LOG).await;
        write_checkpoint(&fx, &run);
        fx.db
            .training_runs()
            .set_state(&run.id, RunState::Finishing)
            .await
            .expect("to finishing");

        fx.runner.recover().await.expect("recover");

        let after = reload(&fx, &run.id).await;
        assert_eq!(
            after.state,
            RunState::Completed,
            "an import that the app died in the middle of must be picked up again"
        );
        assert!(after.result_model_id.is_some());
        assert_released(&fx);
    }

    #[tokio::test]
    async fn recover_fails_a_run_whose_log_ends_in_an_error() {
        let fx = fixture().await;
        let run = running_run(&fx, "Error running job: boom\n").await;

        fx.runner.recover().await.expect("recover");

        let after = reload(&fx, &run.id).await;
        assert_eq!(after.state, RunState::Failed);
        assert_eq!(after.error_text.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn a_failed_adoption_kills_the_process_it_just_spawned() {
        let fx = fixture().await;
        let base = install_base_weights(&fx).await;
        let runner = Runner::new(
            fx.db.clone(),
            fx.adapter.clone(),
            fx.scheduler.clone(),
            fx.runtimes.clone(),
            fx.root.join("training"),
            fx.root.join("store"),
        )
        .with_trainer_command(
            PathBuf::from("ping"),
            vec!["-n".into(), "2".into(), "127.0.0.1".into()],
            "PING.EXE",
        );
        let run = fx
            .db
            .training_runs()
            .create(NewTrainingRun {
                name: "testlora".into(),
                profile_family: FAMILY.into(),
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
        let work_dir = runner.work_dir(&run.id);
        std::fs::create_dir_all(&work_dir).expect("create the work dir");
        // A directory where the PID file belongs: the first step after the
        // spawn now fails, with a real process already running.
        std::fs::create_dir_all(work_dir.join("trainer.pid")).expect("block the pid file");

        let err = runner
            .launch(&run, &prepared(&fx, base))
            .await
            .expect_err("adoption must fail");

        assert!(
            err.to_string().contains("trainer.pid"),
            "unexpected error: {err}"
        );
        assert_eq!(
            runner.rolled_back().len(),
            1,
            "the process we spawned must be killed again, not orphaned"
        );
        assert_eq!(
            runner.adapter.alive_run(),
            None,
            "a rolled-back launch must not leave a reservation behind"
        );
    }

    #[tokio::test]
    async fn a_lost_state_race_is_reported_as_no_change_not_an_error() {
        let fx = fixture().await;
        let run = running_run(&fx, "").await;
        // The poller got there first.
        fx.db
            .training_runs()
            .set_state(&run.id, RunState::Interrupted)
            .await
            .expect("to interrupted");

        // ... and the user's Cancel still believes the run is `running`.
        let landed = fx
            .runner
            .finish(
                &with_state(&run, RunState::Running),
                RunState::Cancelled,
                None,
            )
            .await
            .expect("losing the race is not an error");

        assert!(!landed, "the move must report that it did not land");
        assert_eq!(reload(&fx, &run.id).await.state, RunState::Interrupted);
    }

    #[tokio::test]
    async fn metadata_that_cannot_be_finalised_still_links_the_lora() {
        let fx = fixture().await;
        let run = running_run(&fx, COMPLETION_LOG).await;
        let checkpoint = write_checkpoint(&fx, &run);

        // `rename` refuses a blank name, so this is a finalisation that fails
        // for real rather than one faked with a test-only switch.
        let imported = fx
            .runner
            .import_result(&with_name(&run, "   "), &checkpoint)
            .await
            .expect("a metadata problem must not lose the imported file");

        assert!(
            imported.warning.is_some(),
            "the failure must be reported, not swallowed"
        );
        let model = fx
            .db
            .models()
            .get(&imported.model_id)
            .await
            .expect("read the model")
            .expect("the LoRA is in the library even so");
        assert!(model.file_path.contains("store"), "it is in the store");
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
