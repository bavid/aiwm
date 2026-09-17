//! The whole training lifecycle against a real detached process, no GPU:
//! start → progress → "the app was closed" → recover → crash → interrupted →
//! resume from the last checkpoint → completed → the LoRA is in the library,
//! plus an OOM abort.
//!
//! The stand-in trainer is `aiwm-fake-trainer` (`core/src/bin/`), which speaks
//! the same log dialect as `ai-toolkit` (tqdm bars, `Saved checkpoint to …`,
//! the resume markers, the OOM block, the completion block) and writes the
//! same checkpoint/sample layout. It is wired in through
//! [`Runner::with_trainer_command`], which also overrides the image name the
//! PID checks compare against (`aiwm-fake-trainer.exe` instead of
//! `python.exe`).
//!
//! **Why the runner is built here instead of taking `App::load`'s.** `App`
//! starts a 3 s background poller over the same rows. That poller races a
//! test that drives `poll_once` by hand — most sharply in `resume`, where the
//! window between "the row is `resuming`" and "the new PID file is written"
//! makes a concurrent poll settle the run as `interrupted` while the relaunch
//! is still in flight. Everything else is wired exactly like `App::load`
//! wires it (`AppPaths::rooted` + the real on-disk database + the same
//! adapter/registry/scheduler objects), so only the poller is missing.

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use aiwm_core::db::{Database, DatasetMode, NewDataset, NewModel, Preset, RunState, TrainingRun};
use aiwm_core::runtime::training::{marker_file, TrainingAdapter, PINNED_COMMIT};
use aiwm_core::training::config::{training_folder, Hyperparams};
use aiwm_core::training::process::{is_alive, kill_tree};
use aiwm_core::training::profile::find_for_family;
use aiwm_core::training::runner::{Runner, StartRequest};
use aiwm_core::training::TRAINING_MODEL_ID;
use aiwm_core::{AppPaths, HybridScheduler, RuntimeRegistry, Scheduler};

/// The family every assertion here is about — the only profile with a
/// complete base-weight contract at this point in the plan.
const FAMILY: &str = "flux2-klein-4b";

/// A VRAM budget that comfortably fits the profile's reservation, so
/// `free_mb()` before/after is exactly the reservation.
const BUDGET_MB: u64 = 24_576;

/// Steps for the lifecycle run. The Fast preset saves every 200 steps and
/// `save_every` is deliberately not user-overridable, so a run shorter than
/// 200 steps would never write the mid-run checkpoint the resume half of this
/// test resumes *from*. 250 gives exactly one (at 200) plus the final one.
const LIFECYCLE_STEPS: u32 = 250;

/// Steps for the OOM run — the validator's floor is 50, and the abort happens
/// long before the end anyway.
const OOM_STEPS: u32 = 60;

/// Slow enough that the checkpoint-then-kill window is seconds wide, fast
/// enough that the whole lifecycle is well under a minute.
const MS_PER_STEP: &str = "100";

/// How long any single "wait until" loop may take before the test fails.
const WAIT_CAP: Duration = Duration::from_secs(60);

/// How often those loops re-poll.
const TICK: Duration = Duration::from_millis(150);

fn fixture_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-trainer"))
}

/// The image name `tasklist`/`taskkill` see for the fixture — derived from the
/// binary itself rather than hard-coded, so a renamed fixture cannot silently
/// make every liveness check report "dead".
fn fixture_image() -> String {
    fixture_bin()
        .file_name()
        .and_then(|n| n.to_str())
        .expect("the fixture binary has a file name")
        .to_string()
}

/// Kills every trainer this test started, however the test ends — a failed
/// assertion unwinds through this, so no fake trainer is left running.
struct PidGuard {
    pids: Arc<Mutex<Vec<u32>>>,
}

impl PidGuard {
    fn new() -> Self {
        Self {
            pids: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn record(&self, pid: u32) {
        self.pids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(pid);
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let pids = std::mem::take(&mut *self.pids.lock().unwrap_or_else(PoisonError::into_inner));
        for pid in pids {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
        }
    }
}

/// Everything `App::load` would have wired, minus the background poller.
struct Harness {
    _tmp: tempfile::TempDir,
    paths: AppPaths,
    db: Database,
    adapter: Arc<TrainingAdapter>,
    scheduler: Arc<HybridScheduler>,
    runtimes: RuntimeRegistry,
    store: PathBuf,
    started: Instant,
    guard: PidGuard,
}

impl Harness {
    /// A runner pointed at the fake trainer. Built fresh wherever the real app
    /// would have built one (startup, restart).
    fn runner(&self, extra: &[&str]) -> Runner {
        let mut args = vec!["--ms-per-step".to_string(), MS_PER_STEP.to_string()];
        args.extend(extra.iter().map(|a| (*a).to_string()));
        Runner::new(
            self.db.clone(),
            self.adapter.clone(),
            self.scheduler.clone(),
            self.runtimes.clone(),
            self.paths.root().join("training"),
            self.store.clone(),
        )
        .with_trainer_command(fixture_bin(), args, fixture_image())
        // The staged base weights here are byte-sized stand-ins; the real
        // verifier compares them against sizes and hashes pinned from an
        // actual 7.7 GB download, which no test can reproduce. Preflight's
        // *completeness* check still runs for real.
        .with_base_verifier(|_, _, _| Ok(()))
    }

    fn note(&self, what: &str) {
        println!("[t+{:>6.2}s] {what}", self.started.elapsed().as_secs_f64());
    }

    async fn run_row(&self, id: &str) -> TrainingRun {
        self.db
            .training_runs()
            .get(id)
            .await
            .expect("read the run")
            .expect("the run exists")
    }
}

/// A complete-looking trainer install: preflight only asks whether the entry
/// point, the interpreter and the commit marker are on disk.
fn stage_install(adapter: &TrainingAdapter, runtimes_dir: &Path) {
    let python = adapter.python_bin();
    std::fs::create_dir_all(python.parent().expect("the venv python has a parent"))
        .expect("create the venv dir");
    std::fs::write(&python, b"py").expect("write the venv python");
    std::fs::create_dir_all(adapter.source_dir()).expect("create the source dir");
    std::fs::write(adapter.source_dir().join("run.py"), b"# ai-toolkit").expect("write run.py");
    std::fs::write(marker_file(runtimes_dir), PINNED_COMMIT).expect("write the install marker");
}

async fn harness() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = AppPaths::rooted(tmp.path().join("aiwm"));
    paths.ensure().expect("create the app directories");
    let db = Database::connect(&paths.db_file())
        .await
        .expect("open the database");

    let runtimes_dir = paths.runtimes_dir();
    let adapter = Arc::new(TrainingAdapter::discover(&runtimes_dir));
    stage_install(&adapter, &runtimes_dir);
    assert!(
        adapter.is_installed(),
        "the staged install must satisfy the preflight gate"
    );
    assert!(
        !adapter.env_broken(),
        "a freshly discovered adapter has not failed a probe"
    );

    let runtimes = RuntimeRegistry::new();
    runtimes.register(adapter.clone());
    let scheduler = Arc::new(HybridScheduler::new(runtimes.clone(), BUDGET_MB));

    Harness {
        _tmp: tmp,
        store: paths.root().join("store"),
        paths,
        db,
        adapter,
        scheduler,
        runtimes,
        started: Instant::now(),
        guard: PidGuard::new(),
    }
}

/// The library row the run trains on, plus the base-weight directory model its
/// profile needs (every `required_files` entry present, contents irrelevant).
async fn seed_library(h: &Harness) -> String {
    let profile = find_for_family(FAMILY).expect("the 4B klein profile exists");
    let base_dir = h.paths.root().join("bases").join(FAMILY);
    for rel in profile.base.required_files {
        let file = base_dir.join(rel);
        std::fs::create_dir_all(file.parent().expect("a required file has a parent"))
            .expect("create the base subdir");
        std::fs::write(&file, b"weights").expect("write a base file");
    }
    h.db.models()
        .insert(new_model(
            "FLUX.2 klein base 4B",
            &base_dir.to_string_lossy(),
            vec![profile.base.role.to_string()],
        ))
        .await
        .expect("insert the base weights");

    h.db.models()
        .insert(new_model(
            "flux2-klein-4b-test",
            &h.paths
                .root()
                .join("store")
                .join("flux2.safetensors")
                .to_string_lossy(),
            Vec::new(),
        ))
        .await
        .expect("insert the target model")
        .id
}

fn new_model(name: &str, file_path: &str, roles: Vec<String>) -> NewModel {
    NewModel {
        publisher: None,
        name: name.into(),
        family: Some("flux2".into()),
        format: "safetensors".into(),
        quant: None,
        arch: None,
        param_count: Some(4_000_000_000),
        file_path: file_path.into(),
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
        roles,
    }
}

/// An exported dataset: one frame plus its caption, which is what the
/// preflight's media count looks for.
async fn seed_dataset(h: &Harness) -> String {
    let ds =
        h.db.datasets()
            .create(NewDataset {
                name: "cats".into(),
                mode: DatasetMode::Frames,
                source_root: h.paths.root().join("pics").to_string_lossy().into_owned(),
                prep_job_id: None,
            })
            .await
            .expect("create the dataset");
    let export = h.paths.root().join("export");
    std::fs::create_dir_all(&export).expect("create the export dir");
    std::fs::write(export.join("0001.png"), b"png").expect("write a frame");
    std::fs::write(export.join("0001.txt"), b"a cat").expect("write a caption");
    h.db.datasets()
        .set_export_dir(&ds.id, &export.to_string_lossy())
        .await
        .expect("record the export dir");
    ds.id
}

fn start_request(name: &str, target: &str, dataset: &str, steps: u32) -> StartRequest {
    StartRequest {
        name: name.into(),
        target_model_id: target.into(),
        dataset_id: dataset.into(),
        trigger_word: "tgr_xy".into(),
        preset: Preset::Fast,
        hyperparams: Hyperparams {
            steps: Some(steps),
            ..Hyperparams::default()
        },
        sample_prompts: vec!["tgr_xy a test".into()],
    }
}

fn pid_of(run: &TrainingRun) -> u32 {
    u32::try_from(run.pid.expect("a launched run has a pid")).expect("a plausible pid")
}

fn read_log(runner: &Runner, id: &str) -> String {
    std::fs::read_to_string(runner.log_path(id)).unwrap_or_default()
}

/// The run's ai-toolkit output directory: `<work_dir>/output/<name>`.
fn out_dir(runner: &Runner, run: &TrainingRun) -> PathBuf {
    training_folder(&runner.work_dir(&run.id)).join(&run.name)
}

/// The `_<step:09>` suffix of a checkpoint file name.
fn checkpoint_step(path: &Path) -> Option<u64> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|stem| stem.rsplit_once('_'))
        .and_then(|(_, digits)| digits.parse().ok())
}

fn checkpoints(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "safetensors"))
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn a_training_run_survives_a_restart_a_crash_and_a_resume_and_lands_in_the_library() {
    let h = harness().await;
    let target = seed_library(&h).await;
    let dataset = seed_dataset(&h).await;
    let image = fixture_image();

    // ---------------------------------------------------------------- start
    let free_before = h.scheduler.free_mb();
    let runner = h.runner(&[]);
    let run = runner
        .create_and_start(start_request(
            "lifecycle",
            &target,
            &dataset,
            LIFECYCLE_STEPS,
        ))
        .await
        .expect("start the run");
    let id = run.id.clone();
    let pid = pid_of(&run);
    h.guard.record(pid);
    h.note(&format!("started run {id} as pid {pid}"));

    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.total_steps, i64::from(LIFECYCLE_STEPS));

    // ------------------------------------------- the GPU is held by the run
    assert_eq!(
        h.scheduler.free_mb(),
        free_before - find_for_family(FAMILY).expect("profile").vram.reserve_mb,
        "a live run must charge its reservation against the budget"
    );
    assert!(
        h.scheduler.is_pinned(TRAINING_MODEL_ID),
        "the reservation must be pinned so nothing can evict it"
    );
    assert_eq!(h.adapter.alive_run().as_deref(), Some(id.as_str()));

    // -------------------------------------------------------------- progress
    let deadline = Instant::now() + WAIT_CAP;
    let stepped = loop {
        runner.poll_once().await.expect("poll");
        let row = h.run_row(&id).await;
        if row.step > 0 {
            break row;
        }
        assert!(
            Instant::now() < deadline,
            "the trainer never reported a step; log:\n{}",
            read_log(&runner, &id)
        );
        tokio::time::sleep(TICK).await;
    };
    h.note(&format!(
        "first progress: step {}/{}",
        stepped.step, stepped.total_steps
    ));
    assert!(
        !read_log(&runner, &id).is_empty(),
        "train.log must be growing"
    );

    // ------------------------------------------ "the app was closed" + restart
    drop(runner);
    assert!(
        is_alive(pid, &image).await.expect("liveness check"),
        "the detached trainer must outlive the runner that started it"
    );
    let runner = h.runner(&[]);
    runner.recover().await.expect("recover");
    let after_recover = h.run_row(&id).await;
    h.note(&format!(
        "after recover: state {}, step {}",
        after_recover.state.as_str(),
        after_recover.step
    ));
    assert_eq!(
        after_recover.state,
        RunState::Running,
        "a live process must be re-attached, not marked interrupted"
    );
    assert_eq!(
        h.adapter.alive_run().as_deref(),
        Some(id.as_str()),
        "recovery must re-pin the reservation"
    );
    assert!(h.scheduler.is_pinned(TRAINING_MODEL_ID));

    // ----------------------------------------- wait for the first checkpoint
    let out = out_dir(&runner, &after_recover);
    let deadline = Instant::now() + WAIT_CAP;
    loop {
        runner.poll_once().await.expect("poll");
        if !checkpoints(&out).is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no checkpoint appeared in {}; log:\n{}",
            out.display(),
            read_log(&runner, &id)
        );
        tokio::time::sleep(TICK).await;
    }
    let first_checkpoints = checkpoints(&out);
    h.note(&format!(
        "checkpoint on disk: {}",
        first_checkpoints
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    // The step the resumed trainer must pick up from — anything lower would
    // mean it started over rather than continuing the run.
    let resume_from = first_checkpoints
        .iter()
        .filter_map(|p| checkpoint_step(p))
        .max()
        .expect("a checkpoint carries its step");

    // ------------------------------------------------------------- the crash
    kill_tree(pid, &image).await.expect("kill the trainer");
    let deadline = Instant::now() + WAIT_CAP;
    while is_alive(pid, &image).await.expect("liveness check") {
        assert!(Instant::now() < deadline, "the trainer would not die");
        tokio::time::sleep(TICK).await;
    }
    runner.poll_once().await.expect("poll");
    let crashed = h.run_row(&id).await;
    h.note(&format!(
        "after the kill: state {} at step {}",
        crashed.state.as_str(),
        crashed.step
    ));
    assert_eq!(
        crashed.state,
        RunState::Interrupted,
        "a process that vanished is interrupted, never failed"
    );
    assert_eq!(crashed.error_text, None);
    // Read *after* the crash poll: the row carries the last step the dying
    // trainer managed to print, which is what the resumed one has to overtake
    // before "it is training again" means anything.
    let step_before_crash = crashed.step;
    assert_eq!(h.adapter.alive_run(), None, "the GPU must be handed back");
    assert!(!h.scheduler.is_pinned(TRAINING_MODEL_ID));
    assert_eq!(h.scheduler.free_mb(), free_before);

    // ------------------------------------------------------------- the resume
    runner.resume(&id).await.expect("resume");
    let resumed = h.run_row(&id).await;
    let resumed_pid = pid_of(&resumed);
    h.guard.record(resumed_pid);
    h.note(&format!("resumed as pid {resumed_pid}"));

    let deadline = Instant::now() + WAIT_CAP;
    let found_step = format!("Found step {resume_from} in metadata, starting from there");
    loop {
        runner.poll_once().await.expect("poll");
        let row = h.run_row(&id).await;
        let log = read_log(&runner, &id);
        // The markers prove it picked the checkpoint up; overtaking the step
        // the crash left behind proves it really trained on from there (the
        // row still carries that older, higher step until the first new bar).
        if log.contains("#### IMPORTANT RESUMING FROM")
            && log.contains(&found_step)
            && row.step > step_before_crash
        {
            h.note(&format!(
                "resumed from checkpoint step {resume_from} and passed the pre-crash step {}: now {}",
                step_before_crash, row.step
            ));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the trainer never resumed past step {step_before_crash}; log tail:\n{}",
            log.chars().rev().take(600).collect::<String>()
        );
        tokio::time::sleep(TICK).await;
    }

    // --------------------------------------------------------- the completion
    let deadline = Instant::now() + WAIT_CAP;
    let done = loop {
        runner.poll_once().await.expect("poll");
        let row = h.run_row(&id).await;
        if row.state.is_terminal() {
            break row;
        }
        assert!(
            Instant::now() < deadline,
            "the run never finished (state {}, step {})",
            row.state.as_str(),
            row.step
        );
        tokio::time::sleep(TICK).await;
    };
    h.note(&format!(
        "finished: state {} at step {}",
        done.state.as_str(),
        done.step
    ));
    assert_eq!(
        done.state,
        RunState::Completed,
        "error: {:?}",
        done.error_text
    );
    assert!(done.last_checkpoint_at.is_some());
    assert_eq!(done.pid, None);

    let model_id = done.result_model_id.expect("a result model");
    let model =
        h.db.models()
            .get(&model_id)
            .await
            .expect("read the imported model")
            .expect("the imported model exists");
    h.note(&format!("imported {} as {}", model.name, model.id));
    assert_eq!(model.source, format!("training:{id}"));
    assert_eq!(model.family.as_deref(), Some("flux2"));
    assert_eq!(model.name, "lifecycle");
    assert!(
        !checkpoints(&out).is_empty(),
        "the work dir keeps its checkpoints after the import"
    );
    assert_eq!(h.adapter.alive_run(), None);
    assert!(!h.scheduler.is_pinned(TRAINING_MODEL_ID));
    assert_eq!(h.scheduler.free_mb(), free_before);

    // ---------------------------------------------------------------- the OOM
    let oom_runner = h.runner(&["--oom-at", "20"]);
    let oom = oom_runner
        .create_and_start(start_request("oomrun", &target, &dataset, OOM_STEPS))
        .await
        .expect("start the OOM run");
    h.guard.record(pid_of(&oom));
    h.note(&format!("started the OOM run as pid {}", pid_of(&oom)));

    let deadline = Instant::now() + WAIT_CAP;
    let failed = loop {
        oom_runner.poll_once().await.expect("poll");
        let row = h.run_row(&oom.id).await;
        if row.state.is_terminal() {
            break row;
        }
        assert!(
            Instant::now() < deadline,
            "the OOM run never finished (state {})",
            row.state.as_str()
        );
        tokio::time::sleep(TICK).await;
    };
    h.note(&format!(
        "OOM run: state {} — {:?}",
        failed.state.as_str(),
        failed.error_text
    ));
    assert_eq!(failed.state, RunState::Failed);
    let error = failed.error_text.unwrap_or_default();
    assert!(
        error.contains("video memory"),
        "the OOM abort must be reported as an out-of-video-memory failure, got: {error}"
    );
    assert_eq!(h.adapter.alive_run(), None);
    assert!(!h.scheduler.is_pinned(TRAINING_MODEL_ID));
}
