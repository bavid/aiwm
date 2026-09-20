//! What the scan must never list, whatever the disk holds — one test per
//! "never" rule, each with the folder or row a user might expect to see.
//! Every one of these fails when its rule in the scan is disabled (checked
//! by mutation while writing them).

use super::tests::{
    assert_never_listed, entry_ids, fixture, group_of, has_note, touch, touch_secs,
};
use super::*;
use crate::db::{DownloadState, RunState};
use crate::orchestrator::JobState;

/// An outputs folder configured inside the model store: its files are
/// model-adjacent, so no media group lists them.
#[tokio::test]
async fn model_store_contents_are_never_listed() {
    let mut fx = fixture().await;
    let inside = fx.ctx.store.join("outputs");
    fx.ctx.paths = fx
        .ctx
        .paths
        .clone()
        .with_outputs_override(Some(inside.clone()));
    touch(&inside.join("stray.png"), 100, 400);
    touch(
        &fx.ctx
            .store
            .join("image")
            .join("loras")
            .join("x.safetensors"),
        100,
        400,
    );

    let r = fx.scan().await;

    assert!(group_of(&r, "media_orphans").entries.is_empty());
    assert!(group_of(&r, "media_retention").entries.is_empty());
    assert_never_listed(&r, "stray.png");
    assert_never_listed(&r, "x.safetensors");
    assert!(has_note(&r, "outputs", "model store"));
}

/// A finished run whose recorded folder lies inside the model store is
/// refused by the purge check (never our folder to delete) and protected.
#[tokio::test]
async fn a_run_folder_inside_the_model_store_is_never_listed() {
    let fx = fixture().await;
    let run = fx
        .run(
            "stray-run",
            RunState::Cancelled,
            &fx.ctx.store.join("placeholder"),
        )
        .await;
    let dir = fx.ctx.store.join(&run.id);
    touch(&dir.join("train.log"), 100, 1);
    touch(&dir.join("optimizer.pt"), 100, 1);
    fx.db
        .training_runs()
        .set_work_dir(&run.id, &dir.to_string_lossy())
        .await
        .unwrap();

    let r = fx.scan().await;

    assert!(group_of(&r, "finished_runs").entries.is_empty());
    assert!(has_note(&r, "stray-run", "model store"));
}

/// A cache folder configured inside the runtime installs is not cleaned.
#[tokio::test]
async fn runtimes_are_never_listed() {
    let mut fx = fixture().await;
    let runtimes = fx.ctx.paths.runtimes_dir();
    fx.ctx.paths = fx
        .ctx
        .paths
        .clone()
        .with_cache_override(Some(runtimes.join("cache")));
    touch(&runtimes.join("cache").join("blob"), 100, 1);
    touch(&runtimes.join("comfyui").join("main.py"), 100, 1);

    let r = fx.scan().await;

    assert!(
        !entry_ids(group_of(&r, "caches")).contains(&"cache"),
        "{:?}",
        entry_ids(group_of(&r, "caches"))
    );
    assert_never_listed(&r, "main.py");
    assert!(has_note(&r, "Cache folder", "runtime installs"));
}

/// A dataset whose source folder is directly under the datasets root: the
/// folder is claimed by the source, never an "unclaimed" work folder.
#[tokio::test]
async fn a_source_folder_under_the_datasets_root_is_never_listed() {
    let fx = fixture().await;
    let root = fx.ctx.paths.datasets_dir();
    let source = root.join("my-videos");
    touch(&source.join("clip.mp4"), 5_000, 1);
    let job = fx.job(JobState::Cancelled, None).await;
    fx.db
        .datasets()
        .create(crate::db::NewDataset {
            name: "From videos".into(),
            mode: crate::db::DatasetMode::Frames,
            source_root: source.to_string_lossy().into_owned(),
            prep_job_id: Some(job),
            work_dir: None,
        })
        .await
        .unwrap();
    touch(&root.join("truly-orphaned").join("a.png"), 10, 1);

    let r = fx.scan().await;

    assert_eq!(
        entry_ids(group_of(&r, "unclaimed_dataset_folders")),
        ["truly-orphaned"]
    );
    assert_never_listed(&r, "my-videos");
}

/// An outputs folder that is a dataset's source folder holds the user's
/// media: no flat file in it is an orphan.
#[tokio::test]
async fn source_media_in_the_outputs_folder_is_never_listed() {
    let fx = fixture().await;
    let outputs = fx.ctx.paths.outputs_dir();
    touch(&outputs.join("holiday.mp4"), 5_000, 400);
    let job = fx.job(JobState::Cancelled, None).await;
    fx.db
        .datasets()
        .create(crate::db::NewDataset {
            name: "Holiday".into(),
            mode: crate::db::DatasetMode::Frames,
            source_root: outputs.to_string_lossy().into_owned(),
            prep_job_id: Some(job),
            work_dir: None,
        })
        .await
        .unwrap();

    let r = fx.scan().await;

    assert!(group_of(&r, "media_orphans").entries.is_empty());
    assert_never_listed(&r, "holiday.mp4");
    assert!(has_note(&r, "outputs", "source folder"));
}

/// An in-place frame (its row points at a flat file in outputs) is a
/// dataset's file, not an orphan.
#[tokio::test]
async fn a_frame_file_in_the_outputs_folder_is_never_an_orphan() {
    let fx = fixture().await;
    let outputs = fx.ctx.paths.outputs_dir();
    let job = fx.job(JobState::Cancelled, None).await;
    let ds = fx.dataset("Demo", &job).await;
    fx.frame(&ds, &outputs.join("frame.png"), Some(100), "", false)
        .await;
    touch(&outputs.join("stray.png"), 100, 1);

    let r = fx.scan().await;

    assert_eq!(entry_ids(group_of(&r, "media_orphans")), ["stray.png"]);
}

/// A dataset whose prep job is still running: nothing of it is listed —
/// neither its discarded frames nor its missing rows (the job may still be
/// writing them).
#[tokio::test]
async fn a_busy_dataset_is_never_listed() {
    let fx = fixture().await;
    let job = fx.job(JobState::Running, None).await;
    let ds = fx.dataset("Busy", &job).await;
    let work = fx.ctx.paths.datasets_dir().join(&job).join("raw");
    fx.frame(&ds, &work.join("blurry.png"), Some(200), "blurry", false)
        .await;
    fx.frame(&ds, &work.join("pending.png"), None, "", false)
        .await;

    let r = fx.scan().await;

    assert!(group_of(&r, "discarded_frames").entries.is_empty());
    assert!(group_of(&r, "missing_frame_rows").entries.is_empty());
    assert!(has_note(&r, "Busy", "still being prepared"));
}

/// A dataset a training run is using is busy too.
#[tokio::test]
async fn a_dataset_in_training_is_never_listed() {
    let fx = fixture().await;
    let job = fx.job(JobState::Cancelled, None).await;
    let ds = fx.dataset("Training", &job).await;
    let work = fx.ctx.paths.datasets_dir().join(&job).join("raw");
    fx.frame(&ds, &work.join("blurry.png"), Some(200), "blurry", false)
        .await;
    let run = fx
        .db
        .training_runs()
        .create(crate::db::NewTrainingRun {
            name: "r".into(),
            profile_family: "flux2-klein-4b".into(),
            target_model_id: None,
            dataset_id: Some(ds.id.clone()),
            data_kind: crate::db::DatasetMode::Frames,
            trigger_word: "t".into(),
            preset: crate::db::Preset::Fast,
            hyperparams_json: "{}".into(),
            sample_prompts_json: "[]".into(),
            work_dir: String::new(),
            init_lora_model_id: None,
            image_count: None,
        })
        .await
        .unwrap();
    fx.db
        .training_runs()
        .set_state(&run.id, RunState::Running)
        .await
        .unwrap();

    let r = fx.scan().await;

    assert!(group_of(&r, "discarded_frames").entries.is_empty());
    assert!(has_note(&r, "Training", "in use by training run"));
}

/// A running (or paused) run's folder is never offered, whatever it holds.
#[tokio::test]
async fn a_running_training_run_is_never_listed() {
    let fx = fixture().await;
    for (name, state) in [("running", RunState::Running), ("paused", RunState::Paused)] {
        let run = fx
            .run(
                name,
                state,
                &fx.ctx.paths.training_dir().join("placeholder"),
            )
            .await;
        let dir = fx.run_folder(&run.id, name);
        fx.db
            .training_runs()
            .set_work_dir(&run.id, &dir.to_string_lossy())
            .await
            .unwrap();
    }

    let r = fx.scan().await;

    assert!(group_of(&r, "finished_runs").entries.is_empty());
    assert!(has_note(&r, "running", "running"));
    assert!(has_note(&r, "paused", "paused"));
}

/// A finished run whose "library" LoRA row points at a file *inside* the
/// run folder: the folder is not offered as a whole (that would delete the
/// only copy).
#[tokio::test]
async fn a_result_lora_inside_the_run_folder_keeps_the_final_checkpoint() {
    let fx = fixture().await;
    let run = fx
        .run(
            "lora-v1",
            RunState::Completed,
            &fx.ctx.paths.training_dir().join("placeholder"),
        )
        .await;
    let dir = fx.run_folder(&run.id, "lora-v1");
    fx.db
        .training_runs()
        .set_work_dir(&run.id, &dir.to_string_lossy())
        .await
        .unwrap();
    let inside = dir
        .join("output")
        .join("lora-v1")
        .join("lora-v1.safetensors");
    let model = fx.model("lora-v1", &inside, None).await;
    fx.db
        .training_runs()
        .set_result(&run.id, &model.id)
        .await
        .unwrap();

    let r = fx.scan().await;

    let g = group_of(&r, "finished_runs");
    assert_eq!(g.entries.len(), 1);
    assert_eq!(g.entries[0].files, 6, "not the whole folder");
    assert!(has_note(&r, "lora-v1.safetensors", "only copy"));
}

/// The training root configured under the datasets root: it is a root,
/// never an "unclaimed" work folder. Same for a cache folder that holds the
/// store.
#[tokio::test]
async fn the_roots_themselves_are_never_listed() {
    let mut fx = fixture().await;
    let datasets = fx.ctx.paths.datasets_dir();
    fx.ctx.paths = fx
        .ctx
        .paths
        .clone()
        .with_training_override(Some(datasets.join("training")))
        .with_cache_override(Some(fx.tmp.path().to_path_buf()));
    touch(
        &datasets.join("training").join("run-1").join("train.log"),
        100,
        1,
    );
    touch(&datasets.join("orphan").join("a.png"), 100, 1);

    let r = fx.scan().await;

    assert_eq!(
        entry_ids(group_of(&r, "unclaimed_dataset_folders")),
        ["orphan"]
    );
    assert!(has_note(&r, "training", "training folder"));
    assert!(
        !entry_ids(group_of(&r, "caches")).contains(&"cache"),
        "a cache folder holding the store is not offered"
    );
    assert!(has_note(&r, "Cache folder", "model store"));
}

/// A staging folder of a download that has not finished stays.
#[tokio::test]
async fn an_active_download_staging_folder_is_never_listed() {
    let fx = fixture().await;
    let queued = fx.download(DownloadState::Queued).await;
    let paused = fx.download(DownloadState::Paused).await;
    let failed = fx.download(DownloadState::Failed).await;

    let r = fx.scan().await;

    assert_eq!(
        entry_ids(group_of(&r, "caches")),
        [format!("download-staging:{failed}").as_str()]
    );
    assert_never_listed(&r, &queued);
    assert_never_listed(&r, &paused);
    assert!(has_note(&r, "w.safetensors", "queued"));
}

/// A job that has not finished: its output file is not retention material
/// and its staged ComfyUI input is not a leftover, however old.
#[tokio::test]
async fn a_non_terminal_jobs_files_are_never_listed() {
    let mut fx = fixture().await;
    fx.ctx.policy = RetentionPolicy {
        max_age_days: 1,
        max_total_mb: 0,
    };
    let out = fx.ctx.paths.outputs_dir().join("in-progress.mp4");
    touch(&out, 1_000, 40);
    let job = fx.job(JobState::Running, Some(&out)).await;
    touch_secs(
        &fx.ctx
            .paths
            .comfyui_data_dir()
            .join("input")
            .join(format!("{job}.png")),
        100,
        5 * 3_600,
    );

    let r = fx.scan().await;

    assert!(group_of(&r, "media_retention").entries.is_empty());
    assert!(group_of(&r, "media_orphans").entries.is_empty());
    assert!(group_of(&r, "caches").entries.is_empty());
    assert_never_listed(&r, "in-progress.mp4");
    assert_never_listed(&r, &job);
}

/// A junction directly under the datasets root pointing into the model
/// store: never followed, never listed as a folder.
#[cfg(windows)]
#[tokio::test]
async fn a_junction_under_the_datasets_root_is_never_followed() {
    let fx = fixture().await;
    touch(
        &fx.ctx.store.join("image").join("big.safetensors"),
        10_000,
        1,
    );
    let root = fx.ctx.paths.datasets_dir();
    std::fs::create_dir_all(&root).unwrap();
    let link = root.join("looks-orphaned");
    if !crate::training::location::tests::make_junction(&link, &fx.ctx.store) {
        eprintln!("skipped: cannot create a junction here");
        return;
    }
    // An unclaimed folder that holds a junction into the store: the
    // folder is listed, the store's bytes are not.
    let orphan = root.join("orphan");
    touch(&orphan.join("a.png"), 100, 1);
    assert!(crate::training::location::tests::make_junction(
        &orphan.join("models"),
        &fx.ctx.store
    ));

    let r = fx.scan().await;

    let g = group_of(&r, "unclaimed_dataset_folders");
    assert_eq!(entry_ids(g), ["orphan"]);
    assert_eq!((g.entries[0].files, g.entries[0].bytes), (1, 100));
    assert_never_listed(&r, "big.safetensors");
    assert!(has_note(&r, "looks-orphaned", "junction"));
}
