//! Housekeeping against real files in a temp folder laid out exactly like the
//! app's: `<outputs>/datasets/<prep_job_id>/raw/<tag>/<video stem>/`.

use std::path::{Path, PathBuf};

use super::*;
use crate::db::{
    Database, Dataset, DatasetMode, JobPatch, NewConcept, NewDataset, NewDatasetFrame, NewJob,
    NewTrainingRun, Preset, RunState,
};
use crate::orchestrator::JobState;

pub(super) struct Fx {
    pub(super) tmp: tempfile::TempDir,
    pub(super) db: Database,
    pub(super) outputs: PathBuf,
    /// `<outputs>/datasets/<job>` — the dataset's app-owned work folder.
    pub(super) work_root: PathBuf,
    /// Where the fixture's frames live inside the work folder.
    pub(super) clip_dir: PathBuf,
    pub(super) job_id: String,
    pub(super) dataset: Dataset,
    pub(super) source: PathBuf,
}

/// A `dataset_prep` job that has finished (cancelled is terminal) — a
/// dataset whose prep job still runs cannot be deleted.
pub(super) async fn finished_job(db: &Database) -> String {
    let id = db
        .jobs()
        .insert(NewJob::new("dataset_prep"))
        .await
        .unwrap()
        .id;
    db.jobs()
        .set_state(&id, JobState::Cancelled, JobPatch::default())
        .await
        .unwrap();
    id
}

pub(super) async fn fixture_with_mode(mode: DatasetMode) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let job_id = finished_job(&db).await;
    let outputs = tmp.path().join("outputs");
    let work_root = outputs.join("datasets").join(&job_id);
    let clip_dir = work_root.join("raw").join("Tag").join("clip");
    std::fs::create_dir_all(&clip_dir).unwrap();
    let src_dir = tmp.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let source = src_dir.join("clip.mp4");
    std::fs::write(&source, vec![7u8; 1000]).unwrap();
    let dataset = db
        .datasets()
        .create(NewDataset {
            name: "Demo".into(),
            mode,
            source_root: src_dir.to_string_lossy().into_owned(),
            prep_job_id: Some(job_id.clone()),
        })
        .await
        .unwrap();
    Fx {
        tmp,
        db,
        outputs,
        work_root,
        clip_dir,
        job_id,
        dataset,
        source,
    }
}

pub(super) async fn fixture() -> Fx {
    fixture_with_mode(DatasetMode::Frames).await
}

impl Fx {
    /// A frame row whose file (`bytes` long) sits at `path`.
    pub(super) async fn frame_at(
        &self,
        path: &Path,
        bytes: usize,
        reason: &str,
        excluded: bool,
    ) -> String {
        std::fs::write(path, vec![1u8; bytes]).unwrap();
        self.row(path, &self.source, reason, excluded).await
    }

    /// A frame row in the fixture's clip folder.
    pub(super) async fn frame(
        &self,
        name: &str,
        bytes: usize,
        reason: &str,
        excluded: bool,
    ) -> String {
        self.frame_at(&self.clip_dir.join(name), bytes, reason, excluded)
            .await
    }

    /// A second dataset (another curation set sharing this database and
    /// outputs folder). Returns its id.
    pub(super) async fn other_dataset(
        &self,
        prep_job_id: Option<String>,
        source_root: &Path,
    ) -> String {
        self.db
            .datasets()
            .create(NewDataset {
                name: "Other".into(),
                mode: DatasetMode::Frames,
                source_root: source_root.to_string_lossy().into_owned(),
                prep_job_id,
            })
            .await
            .unwrap()
            .id
    }

    /// A frame row of `dataset_id` (no file is written).
    pub(super) async fn row_in(&self, dataset_id: &str, frame: &Path, source: &Path) -> String {
        self.db
            .dataset_frames()
            .insert(NewDatasetFrame {
                job_id: self.job_id.clone(),
                dataset_id: Some(dataset_id.to_string()),
                tag: "Tag".into(),
                source_path: source.to_string_lossy().into_owned(),
                frame_path: frame.to_string_lossy().into_owned(),
                timestamp_secs: None,
                rejection_reason: String::new(),
                duration_secs: None,
            })
            .await
            .unwrap()
            .id
    }

    pub(super) async fn row(
        &self,
        frame: &Path,
        source: &Path,
        reason: &str,
        excluded: bool,
    ) -> String {
        let id = self
            .db
            .dataset_frames()
            .insert(NewDatasetFrame {
                job_id: self.job_id.clone(),
                dataset_id: Some(self.dataset.id.clone()),
                tag: "Tag".into(),
                source_path: source.to_string_lossy().into_owned(),
                frame_path: frame.to_string_lossy().into_owned(),
                timestamp_secs: Some(0.0),
                rejection_reason: reason.into(),
                duration_secs: None,
            })
            .await
            .unwrap()
            .id;
        if excluded {
            self.db
                .dataset_frames()
                .set_excluded(&id, true)
                .await
                .unwrap();
        }
        id
    }

    pub(super) async fn frame_ids(&self) -> Vec<String> {
        self.db
            .dataset_frames()
            .list_for_dataset(&self.dataset.id)
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.id)
            .collect()
    }

    pub(super) async fn start_run(&self) -> String {
        self.db
            .training_runs()
            .create(NewTrainingRun {
                name: "Style v1".into(),
                profile_family: "flux2_klein_4b".into(),
                target_model_id: None,
                dataset_id: Some(self.dataset.id.clone()),
                data_kind: DatasetMode::Frames,
                trigger_word: "demo_xy".into(),
                preset: Preset::Fast,
                hyperparams_json: "{}".into(),
                sample_prompts_json: "[]".into(),
                work_dir: "unused".into(),
            })
            .await
            .unwrap()
            .id
    }
}

/// A file somewhere outside the app's folders, as a crafted `frame_path`.
pub(super) fn victim(fx: &Fx) -> PathBuf {
    let dir = fx.tmp.path().join("elsewhere");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("victim.png");
    std::fs::write(&path, vec![9u8; 4321]).unwrap();
    path
}

// --- usage ------------------------------------------------------------------

#[tokio::test]
async fn usage_walks_the_work_folder_and_sizes_discarded_frames() {
    let fx = fixture().await;
    fx.frame("kept.png", 100, "", false).await;
    fx.frame("excluded.png", 200, "", true).await;
    fx.frame("blurry.png", 300, "blur", false).await;
    // A non-frame file in the work folder (a preview, a log) counts too.
    std::fs::write(fx.work_root.join("note.txt"), vec![0u8; 50]).unwrap();

    let u = usage(&fx.db, &fx.outputs, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(u.work_bytes, 650);
    assert_eq!(u.work_files, 4);
    assert!(u.work_dir.as_deref().unwrap().ends_with(&fx.job_id));
    assert_eq!(u.discarded_frames, 2);
    assert_eq!(u.discarded_bytes, 500);
    assert_eq!(u.export_dir, None);
    assert!(!u.export_app_owned);

    assert!(usage(&fx.db, &fx.outputs, "no-such-dataset")
        .await
        .unwrap()
        .is_none());
}

// --- delete frames ------------------------------------------------------------

#[tokio::test]
async fn delete_frames_removes_files_rows_and_concept_links_and_counts_freed_bytes() {
    let fx = fixture().await;
    let a = fx.frame("a.png", 100, "", false).await;
    let b = fx.frame("b.png", 250, "blur", false).await;
    let keep = fx
        .frame_at(&fx.work_root.join("keep.png"), 70, "", false)
        .await;
    let concept = fx
        .db
        .concepts()
        .create(NewConcept {
            dataset_id: fx.dataset.id.clone(),
            name: "Kenji".into(),
            token: "kenji_xy".into(),
            description: String::new(),
        })
        .await
        .unwrap();
    fx.db
        .concepts()
        .assign(&concept.id, &[a.clone(), keep.clone()])
        .await
        .unwrap();

    let s = delete_frames(&fx.db, &fx.outputs, &fx.dataset.id, &[a.clone(), b.clone()])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.deleted, 2);
    assert_eq!(s.deleted_files, 2);
    assert_eq!(s.freed_bytes, 350);
    assert!(s.skipped_files.is_empty(), "{:?}", s.skipped_files);

    assert!(!fx.clip_dir.join("a.png").exists());
    assert!(!fx.clip_dir.join("b.png").exists());
    assert_eq!(fx.frame_ids().await, vec![keep.clone()]);
    let map = fx
        .db
        .concepts()
        .map_for_dataset(&fx.dataset.id)
        .await
        .unwrap();
    assert!(!map.contains_key(&a), "the concept link went with the row");
    assert!(map.contains_key(&keep));
    // The emptied clip folders are pruned; the work folder itself stays.
    assert!(!fx.work_root.join("raw").exists());
    assert!(fx.work_root.join("keep.png").exists());
}

#[tokio::test]
async fn delete_frames_never_deletes_a_file_outside_the_app_folders() {
    let fx = fixture().await;
    let victim = victim(&fx);
    let evil = fx.row(&victim, &fx.source, "", false).await;

    let s = delete_frames(&fx.db, &fx.outputs, &fx.dataset.id, &[evil])
        .await
        .unwrap()
        .unwrap();

    assert!(
        victim.exists(),
        "a crafted frame_path must never be deleted"
    );
    assert_eq!(std::fs::metadata(&victim).unwrap().len(), 4321);
    assert_eq!(s.deleted_files, 0);
    assert_eq!(s.freed_bytes, 0);
    assert_eq!(s.skipped_files.len(), 1);
    assert_eq!(s.skipped_files[0].path, victim.to_string_lossy());
    assert_eq!(s.skipped_files[0].reason, SKIP_OUTSIDE);
}

#[tokio::test]
async fn delete_frames_never_deletes_a_source_file_even_inside_the_work_folder() {
    let fx = fixture().await;
    // An image referenced in place: frame_path == source_path.
    let in_place = fx.clip_dir.join("in_place.png");
    std::fs::write(&in_place, vec![3u8; 64]).unwrap();
    let id = fx.row(&in_place, &in_place, "", false).await;
    // The fixture's video source itself, crafted as a frame_path.
    let src_row = fx.row(&fx.source, &fx.source, "", false).await;

    let s = delete_frames(&fx.db, &fx.outputs, &fx.dataset.id, &[id, src_row])
        .await
        .unwrap()
        .unwrap();
    assert!(in_place.exists());
    assert!(fx.source.exists());
    assert_eq!(s.deleted_files, 0);
    assert_eq!(s.skipped_files.len(), 2);
    assert_eq!(s.skipped_files[0].reason, SKIP_SOURCE);
}

#[tokio::test]
async fn delete_frames_keeps_a_file_another_remaining_frame_still_uses() {
    let fx = fixture().await;
    let shared = fx.clip_dir.join("shared.png");
    let a = fx.frame_at(&shared, 80, "", false).await;
    let _b = fx.row(&shared, &fx.source, "", false).await;

    let s = delete_frames(&fx.db, &fx.outputs, &fx.dataset.id, &[a])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.deleted, 1);
    assert!(shared.exists());
    assert_eq!(s.skipped_files[0].reason, SKIP_IN_USE);
}

#[tokio::test]
async fn delete_frames_of_an_unknown_dataset_is_none() {
    let fx = fixture().await;
    assert!(delete_frames(&fx.db, &fx.outputs, "nope", &[])
        .await
        .unwrap()
        .is_none());
}

// --- active-training refusal ------------------------------------------------

#[tokio::test]
async fn an_active_training_run_blocks_every_deletion_until_it_finishes() {
    let fx = fixture().await;
    let a = fx.frame("a.png", 100, "", true).await;
    let run = fx.start_run().await;

    let expect_refusal = |r: Result<Option<()>>| {
        let err = r.unwrap_err();
        assert!(matches!(err, CoreError::Config(_)), "{err}");
        assert!(err.to_string().contains("Style v1"), "{err}");
    };
    expect_refusal(
        delete_frames(
            &fx.db,
            &fx.outputs,
            &fx.dataset.id,
            std::slice::from_ref(&a),
        )
        .await
        .map(|o| o.map(|_| ())),
    );
    expect_refusal(
        cleanup(&fx.db, &fx.outputs, &fx.dataset.id, false)
            .await
            .map(|o| o.map(|_| ())),
    );
    expect_refusal(
        delete_dataset_with_files(&fx.db, &fx.outputs, &fx.dataset.id)
            .await
            .map(|o| o.map(|_| ())),
    );
    assert!(fx.clip_dir.join("a.png").exists());
    assert_eq!(fx.frame_ids().await.len(), 1);

    // A preview deletes nothing, so it is allowed.
    let preview = cleanup(&fx.db, &fx.outputs, &fx.dataset.id, true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(preview.frames, 1);

    // Once the run is finished the dataset is free again.
    fx.db
        .training_runs()
        .set_state(&run, RunState::Cancelled)
        .await
        .unwrap();
    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.freed_bytes, 100);
}

// --- cleanup ----------------------------------------------------------------

#[tokio::test]
async fn cleanup_dry_run_only_measures_and_the_real_run_deletes_discarded_frames() {
    let fx = fixture().await;
    let kept = fx.frame("kept.png", 100, "", false).await;
    fx.frame("excluded.png", 200, "", true).await;
    fx.frame("dup.png", 300, "duplicate_global", false).await;

    let preview = cleanup(&fx.db, &fx.outputs, &fx.dataset.id, true)
        .await
        .unwrap()
        .unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.frames, 2);
    assert_eq!(preview.bytes, 500);
    assert_eq!(preview.deleted_files, 0);
    assert!(fx.clip_dir.join("excluded.png").exists());
    assert!(fx.clip_dir.join("dup.png").exists());
    assert_eq!(fx.frame_ids().await.len(), 3);

    let done = cleanup(&fx.db, &fx.outputs, &fx.dataset.id, false)
        .await
        .unwrap()
        .unwrap();
    assert!(!done.dry_run);
    assert_eq!(done.frames, 2);
    assert_eq!(done.bytes, 500);
    assert_eq!(done.deleted_files, 2);
    assert!(!fx.clip_dir.join("excluded.png").exists());
    assert!(!fx.clip_dir.join("dup.png").exists());
    assert!(fx.clip_dir.join("kept.png").exists());
    assert_eq!(fx.frame_ids().await, vec![kept]);
}

// --- delete dataset -----------------------------------------------------------

#[tokio::test]
async fn delete_dataset_removes_the_work_folder_and_an_app_owned_export() {
    let fx = fixture().await;
    fx.frame("a.png", 100, "", false).await;
    fx.frame("b.png", 200, "blur", false).await;
    std::fs::write(fx.work_root.join("preview.png"), vec![0u8; 30]).unwrap();
    let victim = victim(&fx);
    fx.row(&victim, &fx.source, "", false).await;

    let export = fx.outputs.join("exports").join("demo");
    std::fs::create_dir_all(&export).unwrap();
    std::fs::write(export.join("0001.png"), vec![0u8; 40]).unwrap();
    std::fs::write(export.join("0001.txt"), vec![0u8; 5]).unwrap();
    fx.db
        .datasets()
        .set_export_dir(&fx.dataset.id, &export.to_string_lossy())
        .await
        .unwrap();

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.frames, 3);
    assert_eq!(s.freed_bytes, 100 + 200 + 30 + 40 + 5);
    assert_eq!(s.deleted_files, 5);
    assert_eq!(s.export_dir_kept, None);
    assert_eq!(s.skipped_files.len(), 1, "{:?}", s.skipped_files);
    assert_eq!(s.skipped_files[0].reason, SKIP_OUTSIDE);

    assert!(!fx.work_root.exists(), "the work folder is gone");
    assert!(!export.exists(), "the app-owned export is gone");
    assert!(fx.outputs.join("datasets").exists());
    assert!(victim.exists());
    assert!(fx.source.exists());
    assert!(fx
        .db
        .datasets()
        .get(&fx.dataset.id)
        .await
        .unwrap()
        .is_none());
    assert!(fx.frame_ids().await.is_empty());
}

#[tokio::test]
async fn delete_dataset_leaves_a_user_chosen_export_folder_alone() {
    let fx = fixture().await;
    fx.frame("a.png", 100, "", false).await;
    let export = fx.tmp.path().join("my-exports");
    std::fs::create_dir_all(&export).unwrap();
    std::fs::write(export.join("0001.png"), vec![0u8; 40]).unwrap();
    fx.db
        .datasets()
        .set_export_dir(&fx.dataset.id, &export.to_string_lossy())
        .await
        .unwrap();

    let u = usage(&fx.db, &fx.outputs, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!u.export_app_owned);
    assert_eq!(u.export_bytes, 0);

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.freed_bytes, 100);
    assert_eq!(
        s.export_dir_kept.as_deref(),
        Some(export.to_string_lossy().as_ref())
    );
    assert!(export.join("0001.png").exists());
}

#[tokio::test]
async fn an_export_in_the_outputs_folder_only_loses_its_numbered_files() {
    let fx = fixture().await;
    let export = fx.outputs.join("mixed");
    std::fs::create_dir_all(&export).unwrap();
    std::fs::write(export.join("0001.png"), vec![0u8; 40]).unwrap();
    std::fs::write(export.join("keep-me.png"), vec![0u8; 10]).unwrap();
    fx.db
        .datasets()
        .set_export_dir(&fx.dataset.id, &export.to_string_lossy())
        .await
        .unwrap();

    let u = usage(&fx.db, &fx.outputs, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(u.export_app_owned);
    assert_eq!(u.export_bytes, 40);

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.freed_bytes, 40);
    assert!(!export.join("0001.png").exists());
    assert!(export.join("keep-me.png").exists());
}

#[tokio::test]
async fn delete_dataset_of_an_unknown_id_is_none() {
    let fx = fixture().await;
    assert!(delete_dataset_with_files(&fx.db, &fx.outputs, "nope")
        .await
        .unwrap()
        .is_none());
}
