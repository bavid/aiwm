//! Plan 10: datasets whose work folder was recorded on the row (`work_dir`,
//! possibly a folder the user chose outside the datasets root). The guard
//! treats it as the dataset's app-owned work folder — unless it overlaps a
//! source folder, another dataset's work folder, the model store or an app
//! root, or is a drive root. Every refusal test here fails when its check is
//! removed from `guard.rs`.

use std::path::{Path, PathBuf};

use super::tests::{finished_job, fixture, Fx};
use super::*;
use crate::db::{DatasetMode, NewDataset};

fn write(path: &Path, bytes: usize) -> PathBuf {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, vec![6u8; bytes]).unwrap();
    path.to_path_buf()
}

/// A dataset with a finished prep job whose recorded work folder is
/// `work_dir` (not created here). Returns its id.
async fn recorded_dataset(fx: &Fx, source_root: &Path, work_dir: &Path) -> String {
    let job = finished_job(&fx.db).await;
    fx.db
        .datasets()
        .create(NewDataset {
            name: "Custom".into(),
            mode: DatasetMode::Frames,
            source_root: source_root.to_string_lossy().into_owned(),
            prep_job_id: Some(job),
            work_dir: Some(work_dir.to_string_lossy().into_owned()),
        })
        .await
        .unwrap()
        .id
}

/// A source folder with one video, outside every app folder.
fn source(fx: &Fx, name: &str) -> (PathBuf, PathBuf) {
    let dir = fx.tmp.path().join(name);
    let video = write(&dir.join("clip.mp4"), 1000);
    (dir, video)
}

async fn usage_of(fx: &Fx, id: &str) -> DatasetUsage {
    usage(&fx.db, &fx.roots(), id).await.unwrap().unwrap()
}

// --- the happy path -------------------------------------------------------------

#[tokio::test]
async fn a_dataset_in_a_chosen_folder_is_measured_and_deleted_with_its_folder() {
    let fx = fixture().await;
    let (src, video) = source(&fx, "src-custom");
    let data_dir = fx.tmp.path().join("other-drive").join("frames");
    let work = data_dir.join("job-custom");
    let id = recorded_dataset(&fx, &src, &work).await;
    let f1 = write(
        &work.join("raw").join("Tag").join("clip").join("f1.png"),
        100,
    );
    let f2 = write(
        &work.join("raw").join("Tag").join("clip").join("f2.png"),
        50,
    );
    let preview = write(&work.join("preview.png"), 30);
    fx.row_in(&id, &f1, &video).await;
    fx.row_in(&id, &f2, &video).await;

    let u = usage_of(&fx, &id).await;
    assert!(u.work_walkable, "{u:?}");
    assert!(
        u.work_dir.as_deref().unwrap().ends_with("job-custom"),
        "{u:?}"
    );
    assert_eq!((u.work_files, u.work_bytes), (3, 180));

    let s = delete_dataset_with_files(&fx.db, &fx.roots(), &id)
        .await
        .unwrap()
        .unwrap();
    assert!(s.dataset_deleted, "{s:?}");
    assert_eq!((s.deleted_files, s.freed_bytes), (3, 180), "{s:?}");
    assert!(!preview.exists());
    assert!(!work.exists(), "the recorded work folder itself goes");
    assert!(data_dir.exists(), "the chosen parent folder stays");
    assert!(video.exists(), "the source is untouched");
    assert!(
        fx.work_root.exists(),
        "the fixture dataset's folder is untouched"
    );
}

#[tokio::test]
async fn cleanup_and_delete_frames_work_in_a_chosen_folder() {
    let fx = fixture().await;
    let (src, video) = source(&fx, "src-custom");
    let work = fx.tmp.path().join("chosen").join("job-c");
    let id = recorded_dataset(&fx, &src, &work).await;
    let clip = work.join("raw").join("Tag").join("clip");
    let keep = write(&clip.join("keep.png"), 10);
    let drop = write(&clip.join("drop.png"), 20);
    let gone = write(&clip.join("gone.png"), 40);
    fx.row_in(&id, &keep, &video).await;
    let drop_id = fx.row_in(&id, &drop, &video).await;
    let gone_id = fx.row_in(&id, &gone, &video).await;
    fx.db
        .dataset_frames()
        .set_excluded(&drop_id, true)
        .await
        .unwrap();

    let c = cleanup(&fx.db, &fx.roots(), &id, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((c.deleted_files, c.bytes), (1, 20), "{c:?}");
    assert!(!drop.exists());

    let d = delete_frames(&fx.db, &fx.roots(), &id, &[gone_id])
        .await
        .unwrap()
        .unwrap();
    assert_eq!((d.deleted_files, d.freed_bytes), (1, 40), "{d:?}");
    assert!(!gone.exists());
    assert!(keep.exists());
}

/// The row names the folder, so it stays the dataset's own even once the
/// prep job is gone — and a recorded folder is never "loose".
#[tokio::test]
async fn a_recorded_folder_is_still_owned_after_the_prep_job_is_deleted() {
    let fx = fixture().await;
    let (src, video) = source(&fx, "src-custom");
    let work = fx.tmp.path().join("chosen").join("job-d");
    let id = recorded_dataset(&fx, &src, &work).await;
    let f = write(&work.join("f.png"), 10);
    fx.row_in(&id, &f, &video).await;
    // Loose mode stays limited to rows without a recorded folder: an
    // unclaimed `<datasets root>/<X>` is not this dataset's to delete from.
    let stray = write(&fx.datasets.join("orphan-x").join("f.png"), 4);
    fx.row_in(&id, &stray, &video).await;
    let job = fx
        .db
        .datasets()
        .get(&id)
        .await
        .unwrap()
        .unwrap()
        .prep_job_id
        .unwrap();
    fx.db.jobs().delete(&job).await.unwrap();

    let s = delete_dataset_with_files(&fx.db, &fx.roots(), &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.deleted_files, 1, "{s:?}");
    assert!(!work.exists());
    assert!(stray.exists(), "{s:?}");
    assert_eq!(s.skipped_files[0].reason, SKIP_OUTSIDE);
}

/// Another dataset's recorded folder that cannot be located (here: stored
/// relative) may lie anywhere — including inside this work folder — so the
/// work folder is not walked.
#[tokio::test]
async fn an_unlocatable_recorded_folder_of_another_dataset_stops_the_walk() {
    let fx = fixture().await;
    fx.frame("a.png", 10, "", false).await;
    let preview = write(&fx.work_root.join("preview.png"), 30);
    let (src_b, _) = source(&fx, "src-b");
    recorded_dataset(&fx, &src_b, Path::new("relative\\job-b")).await;

    let u = usage_of(&fx, &fx.dataset.id).await;
    assert!(!u.work_walkable, "{u:?}");
    let s = delete_dataset_with_files(&fx.db, &fx.roots(), &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(preview.exists(), "{s:?}");
    assert_eq!(s.freed_bytes, 10, "only A's own frame goes");
}

// --- neighbours ------------------------------------------------------------------

/// Dataset B's chosen folder lies inside the fixture dataset A's work folder
/// and holds only non-frame files: only the "other datasets' work folders"
/// rule keeps A's walk out of it.
#[tokio::test]
async fn deleting_a_dataset_never_touches_a_neighbours_chosen_folder() {
    let fx = fixture().await;
    fx.frame("a.png", 10, "", false).await;
    let (src_b, _) = source(&fx, "src-b");
    let work_b = fx.work_root.join("nested").join("job-b");
    let b_file = write(&work_b.join("previews").join("p.png"), 7);
    recorded_dataset(&fx, &src_b, &work_b).await;

    let u = usage_of(&fx, &fx.dataset.id).await;
    assert!(!u.work_walkable, "B's folder inside A's stops the walk");
    let s = delete_dataset_with_files(&fx.db, &fx.roots(), &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(b_file.exists(), "{s:?}");
    assert!(
        !fx.clip_dir.join("a.png").exists(),
        "A's own frame still goes"
    );
}

/// A frame row of A pointing into B's chosen folder is B's file.
#[tokio::test]
async fn a_frame_row_pointing_into_a_neighbours_chosen_folder_is_kept() {
    let fx = fixture().await;
    let (src_b, _) = source(&fx, "src-b");
    let work_b = fx.tmp.path().join("chosen").join("job-b");
    let b_file = write(&work_b.join("f.png"), 7);
    recorded_dataset(&fx, &src_b, &work_b).await;
    let id = fx.row(&b_file, &fx.source, "", false).await;

    let s = delete_frames(&fx.db, &fx.roots(), &fx.dataset.id, &[id])
        .await
        .unwrap()
        .unwrap();
    assert!(b_file.exists());
    assert_eq!(s.skipped_files[0].reason, SKIP_OTHER_DATASET, "{s:?}");
}

/// A dataset without prep job ("loose") may only delete inside an unclaimed
/// `<datasets root>/<X>`; a chosen folder of another dataset there claims X.
#[tokio::test]
async fn a_loose_dataset_leaves_a_chosen_folder_under_the_datasets_root_alone() {
    let fx = fixture().await;
    let (src_b, _) = source(&fx, "src-b");
    let work_b = fx.datasets.join("custom-x").join("job-b");
    write(&work_b.join("previews").join("p.png"), 3);
    recorded_dataset(&fx, &src_b, &work_b).await;
    let stray = write(&fx.datasets.join("custom-x").join("stray.png"), 5);
    let (src_c, video_c) = source(&fx, "src-c");
    let c = fx.other_dataset(None, &src_c).await;
    fx.row_in(&c, &stray, &video_c).await;

    let s = delete_dataset_with_files(&fx.db, &fx.roots(), &c)
        .await
        .unwrap()
        .unwrap();
    assert!(stray.exists(), "{s:?}");
    assert_eq!(s.skipped_files[0].reason, SKIP_OUTSIDE);
}

// --- refusals: source folders ---------------------------------------------------

/// A recorded folder equal to the dataset's own source folder is refused:
/// no work folder is reported, and nothing under the source goes.
#[tokio::test]
async fn a_recorded_folder_equal_to_a_source_folder_is_refused() {
    let fx = fixture().await;
    let (src, video) = source(&fx, "src-eq");
    let id = recorded_dataset(&fx, &src, &src).await;
    let f = write(&src.join("f.png"), 9);
    fx.row_in(&id, &f, &video).await;

    let u = usage_of(&fx, &id).await;
    assert_eq!(u.work_dir, None, "{u:?}");
    let s = delete_dataset_with_files(&fx.db, &fx.roots(), &id)
        .await
        .unwrap()
        .unwrap();
    assert!(f.exists() && video.exists(), "{s:?}");
    assert_eq!(s.deleted_files, 0);
}

#[tokio::test]
async fn a_recorded_folder_inside_another_datasets_source_folder_is_refused() {
    let fx = fixture().await;
    // The fixture dataset's source folder hosts C's recorded folder.
    let (src_c, video_c) = source(&fx, "src-c");
    let work = PathBuf::from(&fx.dataset.source_root)
        .join("frames")
        .join("job-c");
    let id = recorded_dataset(&fx, &src_c, &work).await;
    let f = write(&work.join("f.png"), 9);
    fx.row_in(&id, &f, &video_c).await;

    assert_eq!(usage_of(&fx, &id).await.work_dir, None);
    delete_dataset_with_files(&fx.db, &fx.roots(), &id)
        .await
        .unwrap()
        .unwrap();
    assert!(f.exists());
}

/// A recorded folder *holding* a source folder: the frame file next to the
/// source (not under it) is only protected by the refusal itself.
#[tokio::test]
async fn a_recorded_folder_containing_a_source_folder_is_refused() {
    let fx = fixture().await;
    let outer = fx.tmp.path().join("mixed");
    let src = outer.join("src");
    let video = write(&src.join("clip.mp4"), 1000);
    let id = recorded_dataset(&fx, &src, &outer).await;
    let f = write(&outer.join("raw").join("f.png"), 9);
    fx.row_in(&id, &f, &video).await;

    assert_eq!(usage_of(&fx, &id).await.work_dir, None);
    let s = delete_dataset_with_files(&fx.db, &fx.roots(), &id)
        .await
        .unwrap()
        .unwrap();
    assert!(f.exists(), "{s:?}");
    assert!(video.exists());
    assert_eq!(s.skipped_files[0].reason, SKIP_OUTSIDE);
}

// --- refusals: other datasets' work folders -------------------------------------

/// Only the overlap refusal protects `f.png`: it is this dataset's own frame
/// row, not under B's folder.
#[tokio::test]
async fn a_recorded_folder_containing_another_datasets_folder_is_refused() {
    let fx = fixture().await;
    let (src_a, video_a) = source(&fx, "src-a");
    let (src_b, _) = source(&fx, "src-b");
    let work_a = fx.tmp.path().join("chosen").join("job-a");
    let work_b = work_a.join("inner").join("job-b");
    std::fs::create_dir_all(&work_b).unwrap();
    let a = recorded_dataset(&fx, &src_a, &work_a).await;
    recorded_dataset(&fx, &src_b, &work_b).await;
    let f = write(&work_a.join("raw").join("f.png"), 9);
    fx.row_in(&a, &f, &video_a).await;

    assert_eq!(usage_of(&fx, &a).await.work_dir, None);
    let s = delete_dataset_with_files(&fx.db, &fx.roots(), &a)
        .await
        .unwrap()
        .unwrap();
    assert!(f.exists(), "{s:?}");
    assert_eq!(s.skipped_files[0].reason, SKIP_OUTSIDE);
}

#[tokio::test]
async fn a_recorded_folder_inside_another_datasets_derived_work_folder_is_refused() {
    let fx = fixture().await;
    let (src_a, video_a) = source(&fx, "src-a");
    let work_a = fx.work_root.join("sub").join("job-a");
    let a = recorded_dataset(&fx, &src_a, &work_a).await;
    let f = write(&work_a.join("f.png"), 9);
    fx.row_in(&a, &f, &video_a).await;

    assert_eq!(usage_of(&fx, &a).await.work_dir, None);
    delete_dataset_with_files(&fx.db, &fx.roots(), &a)
        .await
        .unwrap()
        .unwrap();
    assert!(f.exists());
}

#[tokio::test]
async fn two_datasets_recording_the_same_folder_both_refuse_it() {
    let fx = fixture().await;
    let (src_a, video_a) = source(&fx, "src-a");
    let (src_b, _) = source(&fx, "src-b");
    let work = fx.tmp.path().join("chosen").join("shared");
    let a = recorded_dataset(&fx, &src_a, &work).await;
    recorded_dataset(&fx, &src_b, &work).await;
    let f = write(&work.join("f.png"), 9);
    fx.row_in(&a, &f, &video_a).await;
    let other = write(&work.join("b-preview.png"), 3);

    assert_eq!(usage_of(&fx, &a).await.work_dir, None);
    delete_dataset_with_files(&fx.db, &fx.roots(), &a)
        .await
        .unwrap()
        .unwrap();
    assert!(f.exists());
    assert!(other.exists());
}

// --- refusals: model store, app roots, drive roots -------------------------------

#[tokio::test]
async fn a_recorded_folder_inside_the_model_store_is_refused() {
    let fx = fixture().await;
    let (src, video) = source(&fx, "src-m");
    let work = fx.models.join("frames").join("job-m");
    let id = recorded_dataset(&fx, &src, &work).await;
    let f = write(&work.join("f.png"), 9);
    fx.row_in(&id, &f, &video).await;

    assert_eq!(usage_of(&fx, &id).await.work_dir, None);
    let s = delete_dataset_with_files(&fx.db, &fx.roots(), &id)
        .await
        .unwrap()
        .unwrap();
    assert!(f.exists(), "{s:?}");
}

/// The store folder need not exist yet (it is created lazily): a recorded
/// folder that would hold it is still refused.
#[tokio::test]
async fn a_recorded_folder_holding_a_store_that_does_not_exist_yet_is_refused() {
    let fx = fixture().await;
    let (src, video) = source(&fx, "src-m");
    let work = fx.tmp.path().join("chosen").join("job-s");
    let id = recorded_dataset(&fx, &src, &work).await;
    let f = write(&work.join("f.png"), 9);
    fx.row_in(&id, &f, &video).await;
    let roots = DataRoots {
        models: work.join("models"),
        ..fx.roots()
    };
    assert!(!roots.models.exists(), "fixture: the store is missing");

    let u = usage(&fx.db, &roots, &id).await.unwrap().unwrap();
    assert_eq!(u.work_dir, None, "{u:?}");
    let s = delete_dataset_with_files(&fx.db, &roots, &id)
        .await
        .unwrap()
        .unwrap();
    assert!(f.exists(), "{s:?}");
}

#[tokio::test]
async fn a_recorded_drive_root_is_refused() {
    let fx = fixture().await;
    let (src, video) = source(&fx, "src-r");
    let drive = fx.tmp.path().ancestors().last().unwrap().to_path_buf();
    let id = recorded_dataset(&fx, &src, &drive).await;
    let f = write(&fx.tmp.path().join("loose").join("f.png"), 9);
    fx.row_in(&id, &f, &video).await;

    let u = usage_of(&fx, &id).await;
    assert_eq!(u.work_dir, None, "{u:?}");
    assert!(!u.work_walkable);
    let s = delete_dataset_with_files(&fx.db, &fx.roots(), &id)
        .await
        .unwrap()
        .unwrap();
    assert!(f.exists(), "{s:?}");
    assert_eq!(s.skipped_files[0].reason, SKIP_OUTSIDE);
}

/// The static rules, one at a time on synthetic paths — each would pass a
/// folder the other rules do not catch.
#[cfg(windows)]
#[test]
fn unfit_work_folders_on_synthetic_paths() {
    let datasets = Path::new(r"D:\app\outputs\datasets");
    let outputs = Path::new(r"D:\app\outputs");
    let models = Path::new(r"F:\models");
    let unfit =
        |w: &str| guard::unfit_work_folder(Path::new(w), &[datasets, outputs], Some(models));

    assert!(unfit(r"E:\"), "a drive root");
    assert!(unfit(r"E:\frames"), "one component below the root");
    assert!(!unfit(r"E:\frames\job"), "two components are enough");
    assert!(unfit(r"F:\models\frames\job"), "inside the store");
    assert!(unfit(r"F:\models"), "the store itself");
    assert!(!unfit(r"F:\models-and\x"), "a name prefix is not a parent");
    assert!(unfit(r"D:\app"), "holds the outputs and datasets roots");
    assert!(
        unfit(r"D:\app\outputs\datasets"),
        "the datasets root itself"
    );
    assert!(
        !unfit(r"D:\app\outputs\datasets\job"),
        "a folder inside the datasets root"
    );
    assert!(
        !unfit(r"D:\app\outputs\frames\job"),
        "a folder inside outputs"
    );
}

/// A store folder that holds the work folder's parent: the recorded folder
/// contains the store when the store sits inside it.
#[cfg(windows)]
#[test]
fn a_work_folder_holding_the_model_store_is_unfit() {
    let unfit = guard::unfit_work_folder(
        Path::new(r"F:\ai\stuff"),
        &[],
        Some(Path::new(r"F:\ai\stuff\models")),
    );
    assert!(unfit);
}
