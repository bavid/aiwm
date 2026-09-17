//! The runner's unit tests: an in-memory database, a trainer install that
//! is only files on disk, and PIDs that are dead by construction.

use super::preflight::*;
use super::settle::*;
use super::*;
use crate::db::{DatasetMode, NewDataset, NewModel};
use crate::runtime::training::install;
use crate::training::config::training_folder;

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
    // Base-weight verification is off for the shared fixture: its stand-in
    // files are seven bytes each, and no test can stage 16 GB of real
    // weights. `preflight_refuses_base_weights_that_do_not_match_the_pinned_manifest`
    // is the one test that keeps the real verifier.
    let runner = Runner::new(
        db.clone(),
        adapter.clone(),
        scheduler.clone(),
        runtimes.clone(),
        root.join("training"),
        root.join("store"),
    )
    .with_base_verifier(|_, _, _| Ok(()));
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
async fn preflight_refuses_base_weights_that_do_not_match_the_pinned_manifest() {
    // `install_base_weights` writes a seven-byte stand-in for each required
    // file: complete enough for `find_staged_base`, nothing like the real
    // 7.7 GB transformer. With the real verifier in place -- the one every
    // production `Runner` gets -- that is exactly the corrupt/truncated
    // download the manifest exists to catch, and it has to be caught here
    // rather than an hour into the run.
    let (fx, target, ds) = ready_fixture().await;
    let runner = Runner::new(
        fx.db.clone(),
        fx.adapter.clone(),
        fx.scheduler.clone(),
        fx.runtimes.clone(),
        fx.root.join("training"),
        fx.root.join("store"),
    );

    let err = runner
        .create_and_start(start_request(&target, &ds))
        .await
        .expect_err("base weights that do not match their manifest must not start a run");

    let msg = err.to_string();
    assert!(
        msg.contains("flux-2-klein-base-4b.safetensors"),
        "the refusal must name the offending file: {msg}"
    );
    assert!(
        msg.contains("black-forest-labs/FLUX.2-klein-base-4B"),
        "the refusal must say what to download again: {msg}"
    );
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
    .with_base_verifier(|_, _, _| Ok(()))
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
    .with_base_verifier(|_, _, _| Ok(()))
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
async fn poll_leaves_a_relaunch_in_flight_alone() {
    let fx = fixture().await;
    let run = running_run(&fx, "").await;
    // Exactly the window `resume` opens: the row is `resuming`, the PID
    // of the attempt that ended is cleared, and the new trainer has not
    // written its PID file yet — so the *stale* file beside it still
    // names the dead process from last time.
    for next in [RunState::Interrupted, RunState::Resuming] {
        fx.db
            .training_runs()
            .set_state(&run.id, next)
            .await
            .expect("into the relaunch window");
    }
    fx.db
        .training_runs()
        .set_pid(&run.id, None)
        .await
        .expect("clear the old pid");

    fx.runner.poll_once().await.expect("poll");

    assert_eq!(
        reload(&fx, &run.id).await.state,
        RunState::Resuming,
        "a run whose relaunch is still in flight must not be settled as dead"
    );
}

#[tokio::test]
async fn poll_leaves_a_run_it_has_no_evidence_about_alone() {
    let fx = fixture().await;
    let run = running_run(&fx, "").await;
    // No recorded PID and no PID file: nothing on disk says this run died,
    // only that we cannot tell. (`running_run` writes both, so both go.)
    std::fs::remove_file(crate::training::process::pid_file(
        &fx.runner.work_dir(&run.id),
    ))
    .expect("drop the pid file");
    fx.db
        .training_runs()
        .set_pid(&run.id, None)
        .await
        .expect("drop the pid");

    fx.runner.poll_once().await.expect("poll");

    assert_eq!(
        reload(&fx, &run.id).await.state,
        RunState::Running,
        "\"we cannot tell\" must not be reported as \"it died\""
    );
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
