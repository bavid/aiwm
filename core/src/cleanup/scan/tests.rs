//! The scan against a temp folder laid out like the app's data dir plus an
//! in-memory database — one test per group.

use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, SystemTime};

use super::*;
use crate::db::{
    DatasetMode, DownloadState, JobPatch, NewDataset, NewDatasetFrame, NewDownload, NewJob,
    NewModel, NewTrainingRun, Preset, RunState,
};
use crate::orchestrator::JobState;

pub(super) struct Fx {
    pub(super) tmp: tempfile::TempDir,
    pub(super) db: Database,
    pub(super) ctx: ScanContext,
}

/// `<tmp>/data` as the app's data dir (so `AppPaths` derives every folder),
/// `<tmp>/models` as the store, `<tmp>/src` as a source folder, no retention.
pub(super) async fn fixture() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let paths = AppPaths::rooted(tmp.path().join("data"));
    let store = tmp.path().join("models");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    Fx {
        tmp,
        db,
        ctx: ScanContext {
            paths,
            store,
            policy: RetentionPolicy::default(),
        },
    }
}

/// Write `bytes` zero bytes at `path` (folders created), last modified
/// `age_days` ago.
pub(super) fn touch(path: &Path, bytes: usize, age_days: u64) {
    touch_secs(path, bytes, age_days * 86_400)
}

pub(super) fn touch_secs(path: &Path, bytes: usize, age_secs: u64) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, vec![0u8; bytes]).unwrap();
    let when = SystemTime::now() - StdDuration::from_secs(age_secs);
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(when).unwrap();
}

impl Fx {
    pub(super) fn paths(&self) -> &AppPaths {
        &self.ctx.paths
    }

    pub(super) fn source_dir(&self) -> PathBuf {
        self.tmp.path().join("src")
    }

    pub(super) async fn scan(&self) -> CleanupReport {
        report(&self.db, &self.ctx).await.unwrap()
    }

    /// A job walked to `state`, with `output_path` recorded on the way.
    pub(super) async fn job(&self, state: JobState, output_path: Option<&Path>) -> String {
        let id = self
            .db
            .jobs()
            .insert(NewJob::new("image"))
            .await
            .unwrap()
            .id;
        let chain: &[JobState] = match state {
            JobState::Queued => &[],
            JobState::Cancelled => &[JobState::Cancelled],
            JobState::Running => &[JobState::Scheduled, JobState::Preparing, JobState::Running],
            _ => &[
                JobState::Scheduled,
                JobState::Preparing,
                JobState::Running,
                JobState::Post,
                JobState::Completed,
            ],
        };
        for next in chain {
            let patch = JobPatch {
                output_path: output_path.map(|p| p.to_string_lossy().into_owned()),
                ..JobPatch::default()
            };
            self.db.jobs().set_state(&id, *next, patch).await.unwrap();
        }
        id
    }

    /// A dataset whose prep job is `job_id` (finished unless told
    /// otherwise), source folder `<tmp>/src`.
    pub(super) async fn dataset(&self, name: &str, job_id: &str) -> Dataset {
        self.db
            .datasets()
            .create(NewDataset {
                name: name.into(),
                mode: DatasetMode::Frames,
                source_root: self.source_dir().to_string_lossy().into_owned(),
                prep_job_id: Some(job_id.to_string()),
                work_dir: None,
            })
            .await
            .unwrap()
    }

    /// A frame row of `dataset` at `path` (the file written with `bytes`
    /// when `Some`), rejected with `reason` and/or excluded.
    pub(super) async fn frame(
        &self,
        dataset: &Dataset,
        path: &Path,
        bytes: Option<usize>,
        reason: &str,
        excluded: bool,
    ) -> String {
        if let Some(bytes) = bytes {
            touch(path, bytes, 1);
        }
        let job_id = dataset.prep_job_id.clone().unwrap();
        let frame = self
            .db
            .dataset_frames()
            .insert(NewDatasetFrame {
                job_id,
                dataset_id: Some(dataset.id.clone()),
                tag: "t".into(),
                source_path: self
                    .source_dir()
                    .join("clip.mp4")
                    .to_string_lossy()
                    .into_owned(),
                frame_path: path.to_string_lossy().into_owned(),
                timestamp_secs: None,
                rejection_reason: reason.into(),
                duration_secs: None,
            })
            .await
            .unwrap();
        if excluded {
            self.db
                .dataset_frames()
                .set_excluded(&frame.id, true)
                .await
                .unwrap();
        }
        frame.id
    }

    /// A training run in `state` whose folder is `work_dir` (the folder is
    /// not created here).
    pub(super) async fn run(&self, name: &str, state: RunState, work_dir: &Path) -> TrainingRun {
        let run = self
            .db
            .training_runs()
            .create(NewTrainingRun {
                name: name.into(),
                profile_family: "flux2-klein-4b".into(),
                target_model_id: None,
                dataset_id: None,
                data_kind: DatasetMode::Frames,
                trigger_word: "t".into(),
                preset: Preset::Fast,
                hyperparams_json: "{}".into(),
                sample_prompts_json: "[]".into(),
                work_dir: work_dir.to_string_lossy().into_owned(),
                init_lora_model_id: None,
                image_count: None,
            })
            .await
            .unwrap();
        let chain: &[RunState] = match state {
            RunState::Preparing => &[],
            RunState::Running => &[RunState::Running],
            RunState::Paused => &[RunState::Running, RunState::Paused],
            RunState::Cancelled => &[RunState::Cancelled],
            RunState::Failed => &[RunState::Failed],
            _ => &[RunState::Running, RunState::Finishing, RunState::Completed],
        };
        for next in chain {
            self.db
                .training_runs()
                .set_state(&run.id, *next)
                .await
                .unwrap();
        }
        self.db.training_runs().get(&run.id).await.unwrap().unwrap()
    }

    /// A completed run's folder as ai-toolkit leaves it: two numbered
    /// checkpoints, the final unsuffixed one, optimizer state, two samples,
    /// the log and the config. Returns the folder.
    pub(super) fn run_folder(&self, run_id: &str, name: &str) -> PathBuf {
        let dir = self.paths().training_dir().join(run_id);
        let out = dir.join("output").join(name);
        touch(&out.join(format!("{name}_000000200.safetensors")), 200, 3);
        touch(&out.join(format!("{name}_000000400.safetensors")), 400, 2);
        touch(&out.join(format!("{name}.safetensors")), 800, 1);
        touch(&out.join("optimizer.pt"), 50, 1);
        touch(
            &out.join("samples").join("1789641631350__000000200_0.jpg"),
            10,
            3,
        );
        touch(
            &out.join("samples").join("1789642005095__000000400_0.jpg"),
            10,
            2,
        );
        touch(&dir.join("train.log"), 30, 1);
        touch(&dir.join("config.yaml"), 5, 4);
        dir
    }

    /// A library model row whose file is `path` (written when `bytes` is
    /// `Some`).
    pub(super) async fn model(&self, name: &str, path: &Path, bytes: Option<usize>) -> Model {
        if let Some(bytes) = bytes {
            touch(path, bytes, 1);
        }
        self.db
            .models()
            .insert(NewModel {
                name: name.into(),
                format: "safetensors".into(),
                file_path: path.to_string_lossy().into_owned(),
                size_bytes: bytes.unwrap_or(0) as i64,
                source: "manual".into(),
                ..NewModel::default()
            })
            .await
            .unwrap()
    }

    /// A download row in `state`, staged under the app's downloads folder
    /// with one partial file. Returns its id (= its staging folder name).
    pub(super) async fn download(&self, state: DownloadState) -> String {
        let staging = self.paths().downloads_dir();
        let d = self
            .db
            .downloads()
            .create(
                NewDownload {
                    url: "https://example.test/w.safetensors".into(),
                    filename: "w.safetensors".into(),
                    ..NewDownload::default()
                },
                &staging,
            )
            .await
            .unwrap();
        self.db
            .downloads()
            .set_state(&d.id, state, None)
            .await
            .unwrap();
        touch(&staging.join(&d.id).join("w.safetensors.part"), 77, 1);
        d.id
    }
}

pub(super) fn group_of<'a>(r: &'a CleanupReport, key: &str) -> &'a CleanupGroup {
    r.groups
        .iter()
        .find(|g| g.key == key)
        .unwrap_or_else(|| panic!("no group {key}"))
}

pub(super) fn entry_ids(g: &CleanupGroup) -> Vec<&str> {
    g.entries.iter().map(|e| e.id.as_str()).collect()
}

pub(super) fn has_note(r: &CleanupReport, what: &str, reason: &str) -> bool {
    r.protected
        .iter()
        .any(|n| n.what.contains(what) && n.reason.contains(reason))
}

/// Nothing in any entry (id, label, detail) mentions `needle`.
pub(super) fn assert_never_listed(r: &CleanupReport, needle: &str) {
    for g in &r.groups {
        for e in &g.entries {
            let text = format!("{} {} {}", e.id, e.label, e.detail.join(" "));
            assert!(
                !text.to_lowercase().contains(&needle.to_lowercase()),
                "{needle:?} listed under {}: {text}",
                g.key
            );
        }
    }
}

#[tokio::test]
async fn report_has_every_group_in_order_even_when_nothing_exists() {
    let fx = fixture().await;

    let r = fx.scan().await;

    let keys: Vec<&str> = r.groups.iter().map(|g| g.key.as_str()).collect();
    assert_eq!(keys, GROUP_KEYS);
    for g in &r.groups {
        assert!(g.entries.is_empty(), "{} must be empty", g.key);
        assert_eq!((g.total_files, g.total_bytes), (0, 0), "{}", g.key);
        assert!(!g.label.is_empty());
    }
    assert!(OffsetDateTime::parse(&r.scanned_at, &Rfc3339).is_ok());
    assert!(has_note(&r, "Model store", "never cleaned up"));
    assert!(has_note(&r, "Runtime installs", "never cleaned up"));
    assert!(has_note(&r, "Voice identities", "user assets"));
}

#[tokio::test]
async fn media_retention_lists_files_with_job_rows_beyond_the_policy() {
    let mut fx = fixture().await;
    fx.ctx.policy = RetentionPolicy {
        max_age_days: 30,
        max_total_mb: 0,
    };
    let outputs = fx.paths().outputs_dir();
    let old = outputs.join("old.png");
    let fresh = outputs.join("fresh.png");
    touch(&old, 1_000, 40);
    touch(&fresh, 1_000, 2);
    fx.job(JobState::Completed, Some(&old)).await;
    fx.job(JobState::Completed, Some(&fresh)).await;

    let r = fx.scan().await;

    let g = group_of(&r, "media_retention");
    assert_eq!(entry_ids(g), ["old.png"]);
    assert_eq!(g.entries[0].files, 1);
    assert_eq!(g.entries[0].bytes, 1_000);
    assert_eq!((g.total_files, g.total_bytes), (1, 1_000));
    assert!(group_of(&r, "media_orphans").entries.is_empty());
}

#[tokio::test]
async fn media_retention_is_empty_without_an_active_policy() {
    let fx = fixture().await;
    let old = fx.paths().outputs_dir().join("old.png");
    touch(&old, 1_000, 400);
    fx.job(JobState::Completed, Some(&old)).await;

    let r = fx.scan().await;

    assert!(group_of(&r, "media_retention").entries.is_empty());
}

#[tokio::test]
async fn media_orphans_lists_flat_files_without_a_job_row_only() {
    let fx = fixture().await;
    let outputs = fx.paths().outputs_dir();
    let with_row = outputs.join("job.png");
    touch(&with_row, 10, 1);
    fx.job(JobState::Completed, Some(&with_row)).await;
    touch(&outputs.join("stray.mp4"), 500, 1);
    // Subfolders are never flat files: dataset work folders and exports.
    touch(&outputs.join("datasets").join("x").join("f.png"), 100, 1);
    touch(&outputs.join("export-1").join("0001.png"), 100, 1);

    let r = fx.scan().await;

    let g = group_of(&r, "media_orphans");
    assert_eq!(entry_ids(g), ["stray.mp4"]);
    assert_eq!(g.entries[0].bytes, 500);
    assert_eq!(g.total_bytes, 500);
    assert_never_listed(&r, "f.png");
    assert_never_listed(&r, "0001.png");
}

#[tokio::test]
async fn discarded_frames_are_counted_per_dataset() {
    let fx = fixture().await;
    let job = fx.job(JobState::Cancelled, None).await;
    let ds = fx.dataset("Demo", &job).await;
    let work = fx.paths().datasets_dir().join(&job).join("raw");
    fx.frame(&ds, &work.join("kept.png"), Some(100), "", false)
        .await;
    fx.frame(&ds, &work.join("blurry.png"), Some(200), "blurry", false)
        .await;
    fx.frame(&ds, &work.join("excluded.png"), Some(300), "", true)
        .await;

    let r = fx.scan().await;

    let g = group_of(&r, "discarded_frames");
    assert_eq!(entry_ids(g), [ds.id.as_str()]);
    let e = &g.entries[0];
    assert_eq!(e.label, "Demo");
    assert_eq!(e.rows, 2);
    assert_eq!(e.files, 2);
    assert_eq!(e.bytes, 500);
    assert_never_listed(&r, "kept.png");
}

#[tokio::test]
async fn unclaimed_dataset_folders_lists_only_folders_no_dataset_claims() {
    let fx = fixture().await;
    let job = fx.job(JobState::Cancelled, None).await;
    let ds = fx.dataset("Demo", &job).await;
    let root = fx.paths().datasets_dir();
    fx.frame(
        &ds,
        &root.join(&job).join("raw").join("a.png"),
        Some(10),
        "",
        false,
    )
    .await;
    touch(&root.join("gone-dataset").join("raw").join("b.png"), 400, 1);
    touch(
        &root.join("gone-dataset").join("previews").join("b.jpg"),
        40,
        1,
    );
    touch(&root.join("loose.txt"), 5, 1);

    let r = fx.scan().await;

    let g = group_of(&r, "unclaimed_dataset_folders");
    assert_eq!(entry_ids(g), ["gone-dataset"]);
    assert_eq!(g.entries[0].files, 2);
    assert_eq!(g.entries[0].bytes, 440);
    assert_never_listed(&r, "loose.txt");
    assert_never_listed(&r, &job);
}

#[tokio::test]
async fn missing_frame_rows_counts_rows_whose_file_is_gone() {
    let fx = fixture().await;
    let job = fx.job(JobState::Cancelled, None).await;
    let ds = fx.dataset("Demo", &job).await;
    let work = fx.paths().datasets_dir().join(&job).join("raw");
    fx.frame(&ds, &work.join("a.png"), Some(10), "", false)
        .await;
    fx.frame(&ds, &work.join("b.png"), Some(10), "", false)
        .await;
    fx.frame(&ds, &work.join("never-written.png"), None, "", false)
        .await;

    let r = fx.scan().await;

    let g = group_of(&r, "missing_frame_rows");
    assert_eq!(entry_ids(g), [ds.id.as_str()]);
    assert_eq!(g.entries[0].rows, 1);
    assert_eq!(g.entries[0].bytes, 0);
    assert_eq!(g.entries[0].files, 0);
}

#[tokio::test]
async fn finished_runs_offer_the_whole_folder_when_the_lora_is_in_the_library() {
    let fx = fixture().await;
    let run = fx
        .run(
            "lora-v1",
            RunState::Completed,
            &fx.paths().training_dir().join("placeholder"),
        )
        .await;
    let dir = fx.run_folder(&run.id, "lora-v1");
    fx.db
        .training_runs()
        .set_work_dir(&run.id, &dir.to_string_lossy())
        .await
        .unwrap();
    let lora = fx
        .ctx
        .store
        .join("image")
        .join("loras")
        .join("lora-v1.safetensors");
    let model = fx.model("lora-v1", &lora, Some(800)).await;
    fx.db
        .training_runs()
        .set_result(&run.id, &model.id)
        .await
        .unwrap();

    let r = fx.scan().await;

    let g = group_of(&r, "finished_runs");
    assert_eq!(entry_ids(g), [run.id.as_str()]);
    let e = &g.entries[0];
    assert!(e.label.contains("lora-v1"), "{}", e.label);
    // Every file in the folder: 3 checkpoints, optimizer, 2 samples, log, config.
    assert_eq!(e.files, 8);
    assert_eq!(e.bytes, 200 + 400 + 800 + 50 + 10 + 10 + 30 + 5);
    assert!(
        !r.protected.iter().any(|n| n.what.contains("lora-v1")),
        "nothing of a run whose LoRA is in the library is protected: {:?}",
        r.protected
    );
}

#[tokio::test]
async fn finished_runs_keep_the_final_checkpoint_when_the_lora_is_not_in_the_library() {
    let fx = fixture().await;
    let run = fx
        .run(
            "lora-v1",
            RunState::Failed,
            &fx.paths().training_dir().join("placeholder"),
        )
        .await;
    let dir = fx.run_folder(&run.id, "lora-v1");
    fx.db
        .training_runs()
        .set_work_dir(&run.id, &dir.to_string_lossy())
        .await
        .unwrap();

    let r = fx.scan().await;

    let g = group_of(&r, "finished_runs");
    assert_eq!(entry_ids(g), [run.id.as_str()]);
    let e = &g.entries[0];
    // Two numbered checkpoints, optimizer, two samples, the log — not the
    // final checkpoint, not the config.
    assert_eq!(e.files, 6);
    assert_eq!(e.bytes, 200 + 400 + 50 + 10 + 10 + 30);
    assert!(
        e.detail.iter().any(|d| d.contains("lora-v1_000000200")),
        "{:?}",
        e.detail
    );
    assert!(
        !e.detail.iter().any(|d| d == "lora-v1.safetensors"),
        "{:?}",
        e.detail
    );
    assert!(has_note(&r, "lora-v1.safetensors", "only copy"));
}

#[tokio::test]
async fn caches_list_cache_stale_staging_comfyui_leftovers_and_pending_import() {
    let fx = fixture().await;
    let p = fx.paths();
    touch(&p.cache_dir().join("registry").join("index.json"), 1_000, 1);
    // A staging folder nothing tracks, and one an active download owns.
    touch(&p.downloads_dir().join("stale-id").join("w.part"), 2_000, 9);
    let active = fx.download(DownloadState::Running).await;
    // ComfyUI: a leftover older than an hour, a fresh one, and a staged
    // input of a job that has not finished.
    let comfy = p.comfyui_data_dir();
    touch_secs(&comfy.join("input").join("old.png"), 300, 2 * 3_600);
    touch_secs(&comfy.join("input").join("fresh.png"), 300, 60);
    let running = fx.job(JobState::Running, None).await;
    touch_secs(
        &comfy.join("input").join(format!("{running}.png")),
        300,
        2 * 3_600,
    );
    touch_secs(&comfy.join("temp").join("preview.png"), 400, 2 * 3_600);
    touch(&p.pending_import_dir().join("aiwm.db"), 5_000, 1);

    let r = fx.scan().await;

    let g = group_of(&r, "caches");
    let ids = entry_ids(g);
    assert_eq!(
        ids,
        [
            "cache",
            "download-staging:stale-id",
            "comfyui:input",
            "comfyui:temp",
            "pending-import"
        ],
        "{ids:?}"
    );
    let by = |id: &str| g.entries.iter().find(|e| e.id == id).unwrap();
    assert_eq!((by("cache").files, by("cache").bytes), (1, 1_000));
    assert_eq!(by("download-staging:stale-id").bytes, 2_000);
    assert_eq!(
        (by("comfyui:input").files, by("comfyui:input").bytes),
        (1, 300)
    );
    assert_eq!(by("comfyui:temp").files, 1);
    assert_eq!(by("pending-import").bytes, 5_000);
    assert_never_listed(&r, &active);
    assert_never_listed(&r, "fresh.png");
    assert_never_listed(&r, &running);
    assert!(has_note(&r, "w.safetensors", "running"));
}

#[tokio::test]
async fn old_logs_list_files_older_than_30_days_but_never_the_newest() {
    let fx = fixture().await;
    let logs = fx.paths().logs_dir();
    touch(&logs.join("aiwm.log.2026-01-01"), 100, 60);
    touch(&logs.join("aiwm.log.2026-02-01"), 100, 45);
    touch(&logs.join("aiwm.log.2026-03-01"), 100, 10);
    touch(&logs.join("aiwm.log"), 100, 0);

    let r = fx.scan().await;

    let g = group_of(&r, "old_logs");
    assert_eq!(entry_ids(g), ["old-logs"]);
    assert_eq!(g.entries[0].files, 2);
    assert_eq!(g.entries[0].bytes, 200);
    assert_eq!(
        g.entries[0].detail,
        ["aiwm.log.2026-01-01", "aiwm.log.2026-02-01"]
    );
}

#[tokio::test]
async fn old_logs_never_offer_the_only_log_even_when_it_is_old() {
    let fx = fixture().await;
    touch(&fx.paths().logs_dir().join("aiwm.log"), 100, 90);

    let r = fx.scan().await;

    assert!(group_of(&r, "old_logs").entries.is_empty());
}

#[tokio::test]
async fn db_backups_list_each_export_with_its_date() {
    let fx = fixture().await;
    let exports = fx.paths().exports_dir();
    touch(&exports.join("aiwm-backup-1.zip"), 1_000, 20);
    touch(&exports.join("aiwm-backup-2.zip"), 2_000, 1);

    let r = fx.scan().await;

    let g = group_of(&r, "db_backups");
    assert_eq!(entry_ids(g), ["aiwm-backup-1.zip", "aiwm-backup-2.zip"]);
    assert_eq!(g.total_bytes, 3_000);
    let today = date_of(Some(OffsetDateTime::now_utc() - time::Duration::days(1)));
    assert!(
        g.entries[1].label.contains(&today),
        "{} should carry the date {today}",
        g.entries[1].label
    );
}
