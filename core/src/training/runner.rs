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

mod preflight;
mod settle;
#[cfg(test)]
mod tests;

use preflight::Prepared;
use settle::{with_state, PollState};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;

use crate::db::{Database, NewTrainingRun, Preset, RunState, TrainingRun};
use crate::launcher::spawn::{launch_detached_quiet, SpawnSpec};
use crate::runtime::training::TrainingAdapter;
use crate::runtime::{RuntimeAdapter, RuntimeRegistry};
use crate::scheduler::{HybridScheduler, Scheduler};
use crate::training::config::{
    config_path, merge_hyperparams, render_yaml, Hyperparams, RenderInput,
};
use crate::training::process::{is_alive, kill_tree, read_pid_file, write_pid_file};
use crate::training::profile::{find_for_family, preset_values};
use crate::training::{training_err, training_refusal, TRAINING_MODEL_ID};
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

/// `(free, total)` bytes on the volume a path lives on — the shape of
/// [`crate::cleanup::volume_free`], behind a function pointer so the
/// preflight disk check can be driven from a test.
pub type FreeSpaceProbe = fn(&Path) -> Option<(u64, u64)>;

/// [`crate::training::bases::verify_base_dir`], behind a function pointer so
/// the preflight base-weight check can be driven from a test. The real one
/// compares a staged snapshot against sizes and SHA-256 sums pinned from a
/// 16 GB download; no test can arrange weights that satisfy it.
pub type BaseVerifier = fn(&Path, &crate::training::bases::TrainingBase, bool) -> Result<()>;

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
    verify_base: BaseVerifier,
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
            verify_base: crate::training::bases::verify_base_dir,
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

    /// Replace the base-weight verifier the preflight check uses. Test seam
    /// only — see [`BaseVerifier`].
    #[cfg(test)]
    #[must_use]
    pub fn with_base_verifier(mut self, verify: BaseVerifier) -> Self {
        self.verify_base = verify;
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
            return Err(training_refusal("the training run needs a name"));
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
            return Err(training_refusal(format!(
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
            return Err(training_refusal(format!(
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
            return Err(training_refusal(format!(
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
            return Err(training_refusal(format!(
                "this run is already {} — there is nothing to cancel",
                run.state.as_str()
            )));
        }
        if run.state == RunState::Finishing {
            return Err(training_refusal(
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
        // Serialised against `start`/`resume`, which hold this from preflight
        // until the relaunched trainer's PID is on disk. Without it a poll
        // that lands inside that window sees a `resuming` row whose only PID
        // file still names the *previous*, dead process and settles the run
        // as `interrupted` — killing a relaunch that was seconds from
        // printing its first step. A poll pass is a log tail and a
        // `tasklist`, so the Start button never waits long for it.
        let _not_while_starting = self.start_lock.lock().await;
        for run in self.db.training_runs().list_recoverable().await? {
            if let Err(e) = self.poll_run(&run).await {
                tracing::warn!(run = %run.id, error = %e, "polling a training run failed");
            }
        }
        Ok(())
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
            // A request naming a run that is not there is a refusal, not a
            // fault: the row was deleted, or the id was never real.
            .ok_or_else(|| training_refusal(format!("there is no training run {run_id}")))
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
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
}

/// Poll every [`POLL_INTERVAL`] forever. Logged, never panicking — one bad
/// poll must not take the loop down with it.
///
/// Recovery is **not** done here. It has to finish before anything can create
/// a run, or a run started in the first instants of a session is indis-
/// tinguishable from one left behind by the previous session and gets settled
/// as `interrupted` on the spot; so [`crate::App::load`] awaits
/// [`Runner::recover`] itself and only then spawns this loop.
pub fn spawn_poller(runner: Arc<Runner>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(e) = runner.poll_once().await {
                tracing::warn!(error = %e, "the training poller hit an error");
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
}
