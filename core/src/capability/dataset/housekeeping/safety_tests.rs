//! The deletion guard against every way a dataset's files can overlap with
//! another dataset's, a source folder, or something outside the app's
//! folders. Each test builds the overlap on disk and asserts the other
//! party's files survive.

use std::path::{Path, PathBuf};

use super::tests::{fixture, victim, Fx};
use super::*;
use crate::db::{JobPatch, NewJob};
use crate::orchestrator::JobState;

fn write(path: &Path, bytes: usize) -> PathBuf {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, vec![5u8; bytes]).unwrap();
    path.to_path_buf()
}

async fn frame_rows(fx: &Fx, dataset_id: &str) -> usize {
    fx.db
        .dataset_frames()
        .list_for_dataset(dataset_id)
        .await
        .unwrap()
        .len()
}

// --- C1: a dataset without prep job never walks a folder ---------------------

/// The reported scenario: dataset B was built *in place* from dataset A's
/// extracted frames (B's frames are A's files), then B's prep job was
/// deleted. Deleting B must leave every file of A alone.
#[tokio::test]
async fn deleting_a_dataset_built_in_place_from_another_datasets_frames_keeps_them() {
    let fx = fixture().await;
    let a1 = fx.frame("a1.png", 100, "", false).await;
    fx.frame("a2.png", 100, "", false).await;
    let preview = write(&fx.work_root.join("preview.png"), 30);
    let b = fx.other_dataset(None, &fx.clip_dir).await;
    for name in ["a1.png", "a2.png"] {
        let p = fx.clip_dir.join(name);
        fx.row_in(&b, &p, &p).await;
    }

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &b)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(s.deleted_files, 0, "{s:?}");
    assert_eq!(s.freed_bytes, 0);
    assert!(fx.clip_dir.join("a1.png").exists());
    assert!(fx.clip_dir.join("a2.png").exists());
    assert!(preview.exists(), "A's non-frame files survive too");
    assert!(fx.db.dataset_frames().get(&a1).await.unwrap().is_some());
    assert_eq!(frame_rows(&fx, &fx.dataset.id).await, 2);
}

/// Same, but A's prep job is gone as well — nothing names A's folder any
/// more, only A's frame rows point into it.
#[tokio::test]
async fn deleting_it_keeps_them_even_when_both_prep_jobs_are_gone() {
    let fx = fixture().await;
    fx.frame("a1.png", 100, "", false).await;
    let a_extra = write(&fx.clip_dir.join("a_not_a_frame.png"), 10);
    let b = fx.other_dataset(None, &fx.clip_dir).await;
    let p = fx.clip_dir.join("a1.png");
    fx.row_in(&b, &p, &p).await;
    fx.db.jobs().delete(&fx.job_id).await.unwrap();
    let a = fx.db.datasets().get(&fx.dataset.id).await.unwrap().unwrap();
    assert_eq!(a.prep_job_id, None, "fixture: A's prep job is gone");

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &b)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.deleted_files, 0, "{s:?}");
    assert!(p.exists());
    assert!(a_extra.exists());
    assert!(fx.work_root.exists());
}

/// Without a prep job a dataset still deletes its *own* extracted frames —
/// one by one — and prunes the folders that became empty, but never the
/// top folder `<outputs>/datasets/<X>` itself.
#[tokio::test]
async fn without_a_prep_job_only_its_own_frame_files_go() {
    let fx = fixture().await;
    let x = fx.outputs.join("datasets").join("orphan-job");
    let own = write(&x.join("raw").join("T").join("v").join("f1.png"), 64);
    let stray = write(&x.join("notes.txt"), 7);
    let src = fx.tmp.path().join("src2");
    std::fs::create_dir_all(&src).unwrap();
    let c = fx.other_dataset(None, &src).await;
    fx.row_in(&c, &own, &src.join("v.mp4")).await;

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &c)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((s.deleted_files, s.freed_bytes), (1, 64), "{s:?}");
    assert!(!own.exists());
    assert!(stray.exists(), "no folder is walked without a prep job");
    assert!(!x.join("raw").exists(), "emptied sub-folders are pruned");
    assert!(x.exists(), "the top folder itself stays");
}

/// Even with a prep job, a work folder another dataset points into is not
/// walked: only this dataset's own frames go, file by file, and nothing
/// under the other dataset's source folder.
#[tokio::test]
async fn a_work_folder_another_dataset_points_into_is_not_walked() {
    let fx = fixture().await;
    let b_frame = fx.frame("b1.png", 100, "", false).await;
    let other_dir = fx.work_root.join("raw").join("Tag").join("second");
    let mine = write(&other_dir.join("b2.png"), 40);
    let mine_id = fx.row(&mine, &fx.source, "", false).await;
    let preview = write(&fx.work_root.join("preview.png"), 30);
    // C was built in place from B's clip folder.
    let c = fx.other_dataset(None, &fx.clip_dir).await;
    let shared = fx.clip_dir.join("b1.png");
    fx.row_in(&c, &shared, &shared).await;

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(shared.exists(), "C's frame survives");
    assert!(!mine.exists(), "B's own frame elsewhere in its folder goes");
    assert!(preview.exists(), "no walk: B's non-frame files stay");
    assert_eq!(s.deleted_files, 1, "{s:?}");
    let _ = (b_frame, mine_id);
}

/// Without a prep job, a dataset's own extracted frame is still kept when it
/// lies in a `<outputs>/datasets/<X>` folder another dataset's frames also
/// live in — even after that dataset's prep job is gone too, so no work
/// folder names X any more. (Only the "claimed X" rule protects this file:
/// it is neither a source nor another dataset's frame.)
#[tokio::test]
async fn without_a_prep_job_a_folder_another_dataset_uses_is_left_alone() {
    let fx = fixture().await;
    fx.frame("a1.png", 100, "", false).await;
    let src = fx.tmp.path().join("src-c");
    std::fs::create_dir_all(&src).unwrap();
    let c = fx.other_dataset(None, &src).await;
    let c_own = write(&fx.clip_dir.join("c_own.png"), 25);
    fx.row_in(&c, &c_own, &src.join("v.mp4")).await;
    fx.db.jobs().delete(&fx.job_id).await.unwrap();

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &c)
        .await
        .unwrap()
        .unwrap();
    assert!(c_own.exists(), "{s:?}");
    assert_eq!(s.deleted_files, 0);
    assert_eq!(s.skipped_files[0].reason, SKIP_OUTSIDE);
}

/// Another dataset exported *into this dataset's work folder*: a frame row
/// of this dataset pointing at that export's file must not delete it.
/// (Only the "other datasets' folders" rule protects this file.)
#[tokio::test]
async fn another_datasets_export_inside_this_work_folder_is_left_alone() {
    let fx = fixture().await;
    let export_b = fx.work_root.join("export-b");
    let exported = write(&export_b.join("0001.png"), 40);
    let b = fx.other_dataset(None, &fx.tmp.path().join("src")).await;
    fx.db
        .datasets()
        .set_export_dir(&b, &export_b.to_string_lossy())
        .await
        .unwrap();
    let id = fx.row(&exported, &fx.source, "", false).await;

    let s = delete_frames(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id, &[id])
        .await
        .unwrap()
        .unwrap();
    assert!(exported.exists());
    assert_eq!(s.skipped_files[0].reason, SKIP_OTHER_DATASET);
    delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap();
    assert!(exported.exists(), "the walk does not reach it either");
}

// --- I1: never touch a file another dataset or a source folder holds --------

#[tokio::test]
async fn an_export_another_dataset_uses_in_place_survives_deleting_its_owner() {
    let fx = fixture().await;
    let export = fx.outputs.join("exports").join("a");
    let e1 = write(&export.join("0001.png"), 40);
    write(&export.join("0001.txt"), 5);
    fx.db
        .datasets()
        .set_export_dir(&fx.dataset.id, &export.to_string_lossy())
        .await
        .unwrap();
    let b = fx.other_dataset(None, &export).await;
    let b_frame = fx.row_in(&b, &e1, &e1).await;

    delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(e1.exists(), "B's frame (A's exported file) survives");
    assert!(
        export.join("0001.txt").exists(),
        "B's source folder is untouched"
    );
    assert!(fx
        .db
        .dataset_frames()
        .get(&b_frame)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn an_export_folder_shared_with_another_dataset_survives() {
    let fx = fixture().await;
    let export = fx.outputs.join("exports").join("shared");
    let e1 = write(&export.join("0001.png"), 40);
    let b = fx.other_dataset(None, &fx.tmp.path().join("src")).await;
    for id in [fx.dataset.id.as_str(), b.as_str()] {
        fx.db
            .datasets()
            .set_export_dir(id, &export.to_string_lossy())
            .await
            .unwrap();
    }
    let u = usage(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!u.export_app_owned);

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(e1.exists());
    assert_eq!(
        s.export_dir_kept.as_deref(),
        Some(export.to_string_lossy().as_ref())
    );
}

/// A user's source folder that is also (by accident) an export folder
/// never loses a file, even when no frame points at it yet.
#[tokio::test]
async fn an_export_inside_another_datasets_source_folder_survives() {
    let fx = fixture().await;
    let export = fx.outputs.join("exports").join("a");
    let e1 = write(&export.join("0001.png"), 40);
    fx.db
        .datasets()
        .set_export_dir(&fx.dataset.id, &export.to_string_lossy())
        .await
        .unwrap();
    fx.other_dataset(None, &export).await;

    delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(e1.exists());
}

/// A frame of another dataset inside this dataset's work folder is not
/// deleted by `delete_frames`/cleanup either.
#[tokio::test]
async fn deleting_frames_keeps_a_file_another_dataset_uses() {
    let fx = fixture().await;
    let mine = fx.frame("m.png", 50, "", true).await;
    let shared = fx.clip_dir.join("m.png");
    let b = fx.other_dataset(None, &fx.tmp.path().join("src")).await;
    fx.row_in(&b, &shared, &fx.source).await;

    let s = cleanup(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id, false)
        .await
        .unwrap()
        .unwrap();
    assert!(shared.exists());
    assert_eq!(s.deleted_files, 0);
    assert!(fx.db.dataset_frames().get(&mine).await.unwrap().is_none());
    assert_eq!(s.skipped_files[0].reason, SKIP_OTHER_DATASET);
}

// --- I2: a prep job that has not finished blocks every deletion --------------

#[tokio::test]
async fn a_running_prep_job_blocks_every_deletion() {
    let fx = fixture().await;
    let job = fx
        .db
        .jobs()
        .insert(NewJob::new("dataset_prep"))
        .await
        .unwrap()
        .id;
    let d = fx
        .db
        .datasets()
        .create(crate::db::NewDataset {
            name: "Busy".into(),
            mode: crate::db::DatasetMode::Frames,
            source_root: fx.tmp.path().join("src").to_string_lossy().into_owned(),
            prep_job_id: Some(job.clone()),
        })
        .await
        .unwrap()
        .id;
    let f = write(&fx.outputs.join("datasets").join(&job).join("f.png"), 10);
    let row = fx.row_in(&d, &f, &fx.source).await;

    let refused = |e: CoreError| {
        assert!(matches!(e, CoreError::Config(_)), "{e}");
        assert!(e.to_string().contains("still being prepared"), "{e}");
    };
    refused(
        delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &d)
            .await
            .unwrap_err(),
    );
    refused(
        delete_frames(
            &fx.db,
            &fx.outputs,
            &fx.datasets,
            &d,
            std::slice::from_ref(&row),
        )
        .await
        .unwrap_err(),
    );
    refused(
        cleanup(&fx.db, &fx.outputs, &fx.datasets, &d, false)
            .await
            .unwrap_err(),
    );
    assert!(f.exists());
    // A preview and the usage figures stay available.
    assert!(cleanup(&fx.db, &fx.outputs, &fx.datasets, &d, true)
        .await
        .is_ok());

    fx.db
        .jobs()
        .set_state(&job, JobState::Cancelled, JobPatch::default())
        .await
        .unwrap();
    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &d)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.freed_bytes, 10);
}

// --- I3: path tricks ---------------------------------------------------------

#[tokio::test]
async fn dot_dot_segments_cannot_leave_the_work_folder() {
    let fx = fixture().await;
    let victim = victim(&fx);
    // <tmp>/outputs/datasets/<job>/raw/Tag/clip/../../../../../../elsewhere/victim.png
    let mut crafted = fx.clip_dir.clone();
    for _ in 0..6 {
        crafted.push("..");
    }
    crafted.push("elsewhere");
    crafted.push("victim.png");
    assert!(
        crafted.exists(),
        "fixture: the crafted path resolves to the victim"
    );
    let id = fx.row(&crafted, &fx.source, "", false).await;

    let s = delete_frames(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id, &[id])
        .await
        .unwrap()
        .unwrap();
    assert!(victim.exists());
    assert_eq!(s.skipped_files[0].reason, SKIP_OUTSIDE);
    delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap();
    assert!(victim.exists());
}

#[cfg(windows)]
#[tokio::test]
async fn a_junction_inside_the_work_folder_is_never_followed() {
    let fx = fixture().await;
    let victim = victim(&fx);
    let link = fx.work_root.join("raw").join("linked");
    junction::create(victim.parent().unwrap(), &link).unwrap();
    let through = link.join("victim.png");
    assert!(through.exists(), "fixture: the junction resolves");
    let id = fx.row(&through, &fx.source, "", false).await;

    let s = delete_frames(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id, &[id])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.skipped_files[0].reason, SKIP_OUTSIDE);
    delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap();
    assert!(
        victim.exists(),
        "neither a frame row nor the walk crosses a junction"
    );
    assert!(victim.parent().unwrap().exists());
}

/// Opt-in: creating a file symlink needs Developer Mode or admin rights.
/// Run with `--ignored` on a machine that has them; without them it fails
/// loudly instead of passing silently.
#[cfg(windows)]
#[tokio::test]
#[ignore = "needs symlink privilege (Developer Mode)"]
async fn a_file_symlink_inside_the_work_folder_never_deletes_its_target() {
    let fx = fixture().await;
    let victim = victim(&fx);
    let link = fx.clip_dir.join("link.png");
    if let Err(e) = std::os::windows::fs::symlink_file(&victim, &link) {
        panic!("cannot create a file symlink here (needs Developer Mode or admin): {e}");
    }
    let id = fx.row(&link, &fx.source, "", false).await;
    delete_frames(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id, &[id])
        .await
        .unwrap();
    delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap();
    assert!(victim.exists());
}

#[tokio::test]
async fn an_export_in_another_jobs_work_folder_is_not_app_owned() {
    let fx = fixture().await;
    let export = fx.outputs.join("datasets").join("other-job");
    let e1 = write(&export.join("0001.png"), 40);
    fx.db
        .datasets()
        .set_export_dir(&fx.dataset.id, &export.to_string_lossy())
        .await
        .unwrap();
    let u = usage(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!u.export_app_owned);
    delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap();
    assert!(e1.exists());
}

#[tokio::test]
async fn an_export_at_the_datasets_root_is_not_app_owned() {
    let fx = fixture().await;
    let export = fx.outputs.join("datasets");
    let e1 = write(&export.join("0001.png"), 40);
    fx.db
        .datasets()
        .set_export_dir(&fx.dataset.id, &export.to_string_lossy())
        .await
        .unwrap();
    let u = usage(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!u.export_app_owned);
    delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap();
    assert!(e1.exists());
}

/// An emptied export folder is removed only when it is a dedicated folder
/// (two levels below outputs) — never a first-level folder of outputs.
#[tokio::test]
async fn a_first_level_export_folder_is_emptied_but_kept() {
    let fx = fixture().await;
    let export = fx.outputs.join("my-export");
    write(&export.join("0001.png"), 40);
    fx.db
        .datasets()
        .set_export_dir(&fx.dataset.id, &export.to_string_lossy())
        .await
        .unwrap();
    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.freed_bytes, 40);
    assert!(!export.join("0001.png").exists());
    assert!(export.exists());
}

/// Open `path` without any sharing, so deleting it fails with a sharing
/// violation while the handle lives. (A read-only attribute is not enough:
/// current Rust's `remove_file` deletes read-only files on Windows.)
#[cfg(windows)]
pub(super) fn lock(path: &Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

#[cfg(windows)]
#[tokio::test]
async fn a_file_that_cannot_be_deleted_keeps_its_row() {
    let fx = fixture().await;
    let id = fx.frame("locked.png", 20, "", false).await;
    let locked = fx.clip_dir.join("locked.png");
    let handle = lock(&locked);

    let s = delete_frames(
        &fx.db,
        &fx.outputs,
        &fx.datasets,
        &fx.dataset.id,
        std::slice::from_ref(&id),
    )
    .await
    .unwrap()
    .unwrap();
    drop(handle);
    assert!(locked.exists());
    assert_eq!(s.deleted, 0, "the row stays for a retry");
    assert!(s.skipped_files[0].reason.starts_with(SKIP_ERROR));
    assert!(fx.db.dataset_frames().get(&id).await.unwrap().is_some());
}

/// Two rows naming the same file differently (`..`) still count as one file
/// in use: deleting one row keeps the file.
#[tokio::test]
async fn in_use_compares_canonical_paths() {
    let fx = fixture().await;
    let a = fx.frame("same.png", 30, "", false).await;
    let other_spelling = fx.clip_dir.join("..").join("clip").join("same.png");
    fx.row(&other_spelling, &fx.source, "", false).await;

    let s = delete_frames(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id, &[a])
        .await
        .unwrap()
        .unwrap();
    assert!(fx.clip_dir.join("same.png").exists());
    assert_eq!(s.deleted_files, 0);
    assert_eq!(s.skipped_files.len(), 1, "{:?}", s.skipped_files);
    assert_eq!(s.skipped_files[0].reason, SKIP_IN_USE);
}

// --- stage 2: behaviour that needs the new API ---------------------------------

/// A file that could not be deleted keeps the whole dataset row, so the
/// delete can be retried; the summary says so.
#[cfg(windows)]
#[tokio::test]
async fn a_failed_file_delete_keeps_the_dataset_row() {
    let fx = fixture().await;
    fx.frame("ok.png", 10, "", false).await;
    fx.frame("locked.png", 20, "", false).await;
    let handle = lock(&fx.clip_dir.join("locked.png"));

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    drop(handle);
    assert!(!s.dataset_deleted, "{s:?}");
    assert_eq!(s.freed_bytes, 10);
    assert!(fx
        .db
        .datasets()
        .get(&fx.dataset.id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(frame_rows(&fx, &fx.dataset.id).await, 2);

    let again = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(again.dataset_deleted);
    assert!(fx
        .db
        .datasets()
        .get(&fx.dataset.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn more_than_ten_thousand_frame_ids_are_refused() {
    let fx = fixture().await;
    let ids: Vec<String> = (0..=MAX_FRAME_IDS).map(|i| i.to_string()).collect();
    let err = delete_frames(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id, &ids)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::Config(_)), "{err}");
    assert!(check_frame_ids(&ids[..MAX_FRAME_IDS]).is_ok());
}

#[test]
fn the_guard_refuses_a_relative_path_whatever_the_working_directory() {
    // `check` only returns a verdict; nothing is deleted here.
    let cwd = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
    let guard = guard::Guard::with_work_dir_for_test(cwd);
    // Would resolve inside the "work folder" against the working directory.
    match guard.check(Path::new("Cargo.toml")) {
        guard::Verdict::Skip(s) => assert_eq!(s.reason, SKIP_OUTSIDE),
        guard::Verdict::Delete { .. } | guard::Verdict::Missing => {
            panic!("a relative path must be skipped")
        }
    }
}

/// A source file is recognised by its resolved path (a frame row spelling it
/// differently) and, outside every root, reported as a source by its stored
/// path rather than merely "outside".
#[tokio::test]
async fn own_sources_are_recognised_resolved_and_as_stored() {
    let fx = fixture().await;
    let src_in_work = write(&fx.clip_dir.join("src.png"), 12);
    let respelled = fx.clip_dir.join("..").join("clip").join("src.png");
    let a = fx.row(&respelled, &src_in_work, "", false).await;
    let elsewhere = write(&fx.tmp.path().join("pictures").join("img.png"), 5);
    let b = fx.row(&elsewhere, &elsewhere, "", false).await;

    let s = delete_frames(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id, &[a, b])
        .await
        .unwrap()
        .unwrap();
    assert!(src_in_work.exists());
    assert!(elsewhere.exists());
    let reasons: Vec<&str> = s.skipped_files.iter().map(|f| f.reason.as_str()).collect();
    assert_eq!(reasons, vec![SKIP_SOURCE, SKIP_SOURCE], "{s:?}");
}

/// Pins the canonical-folder decision: another dataset's frame whose folder
/// is spelled differently (`…\clip\..\clip\f.png`) but lies inside this work
/// folder is recognised — kept as another dataset's file, and the work
/// folder is not walked.
#[tokio::test]
async fn a_foreign_frame_in_a_respelled_folder_inside_the_work_folder_is_kept() {
    let fx = fixture().await;
    let shared = write(&fx.clip_dir.join("f.png"), 9);
    let mine = fx.row(&shared, &fx.source, "", false).await;
    let preview = write(&fx.work_root.join("preview.png"), 30);
    let respelled = fx.clip_dir.join("..").join("clip").join("f.png");
    let b = fx.other_dataset(None, &fx.tmp.path().join("src")).await;
    fx.row_in(&b, &respelled, &fx.source).await;

    let s = delete_frames(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id, &[mine])
        .await
        .unwrap()
        .unwrap();
    assert!(shared.exists());
    assert_eq!(s.skipped_files[0].reason, SKIP_OTHER_DATASET, "{s:?}");

    delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap();
    assert!(shared.exists());
    assert!(preview.exists(), "the work folder was not walked");
}

/// A folder that exists but cannot be resolved (permission denied, a broken
/// reparse point, …) is not the same as a gone folder: its files may still
/// be there, so it must stop the walk and protect everything under it.
/// Constructing such a folder portably needs ACL changes, so the error
/// classification is tested directly.
#[test]
fn only_a_missing_folder_counts_as_gone() {
    use std::io::{Error, ErrorKind};
    let p = PathBuf::from(r"C:\x");
    assert!(matches!(
        guard::classify_folder(Ok(p.clone())),
        guard::FolderState::Resolved(q) if q == p
    ));
    assert!(matches!(
        guard::classify_folder(Err(Error::from(ErrorKind::NotFound))),
        guard::FolderState::Gone
    ));
    for kind in [
        ErrorKind::PermissionDenied,
        ErrorKind::InvalidInput,
        ErrorKind::Other,
    ] {
        assert!(
            matches!(
                guard::classify_folder(Err(Error::from(kind))),
                guard::FolderState::Unresolvable
            ),
            "{kind:?}"
        );
    }
}

/// An unresolvable folder of another dataset stops the walk and its files
/// (matched by the stored folder) are never deleted.
#[test]
fn an_unresolvable_foreign_folder_stops_the_walk_and_protects_its_files() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("locked");
    let file = write(&folder.join("f.png"), 3);
    let index = guard::index_folders(
        vec![(folder.clone(), vec![file.to_string_lossy().into_owned()])],
        &[tmp.path()],
        |_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
    );
    assert!(!index.all_located, "the walk must stop");
    assert_eq!(index.unresolved, vec![folder]);
    assert!(index.folders.is_empty());
}

/// A relative path in another dataset's rows cannot be located, so it may
/// point anywhere — including into this work folder. The folder is then not
/// walked: only this dataset's own frames go.
#[tokio::test]
async fn a_relative_foreign_path_stops_the_walk() {
    let fx = fixture().await;
    fx.frame("own.png", 10, "", false).await;
    let preview = write(&fx.work_root.join("preview.png"), 30);
    let b = fx.other_dataset(None, &fx.tmp.path().join("src")).await;
    fx.row_in(&b, Path::new("preview.png"), Path::new("clip.mp4"))
        .await;

    let s = delete_dataset_with_files(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.freed_bytes, 10, "{s:?}");
    assert!(preview.exists(), "no walk while a foreign path is relative");
}

/// `work_walkable` and `work_bytes`: a walkable work folder reports its whole
/// size; otherwise only this dataset's own frame files deleting would free.
#[tokio::test]
async fn usage_reports_what_deleting_would_free_from_the_work_folder() {
    let fx = fixture().await;
    fx.frame("own.png", 100, "", false).await;
    write(&fx.work_root.join("preview.png"), 30);
    let u = usage(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(u.work_walkable);
    assert_eq!((u.work_files, u.work_bytes), (2, 130));

    // Another dataset built in place from this work folder.
    let b = fx.other_dataset(None, &fx.work_root).await;
    let shared = write(&fx.clip_dir.join("shared.png"), 7);
    fx.row_in(&b, &shared, &shared).await;
    let u = usage(&fx.db, &fx.outputs, &fx.datasets, &fx.dataset.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!u.work_walkable);
    assert_eq!(
        (u.work_files, u.work_bytes),
        (0, 0),
        "B's source folder covers this frame, so deleting frees nothing"
    );
}

/// Without a prep job there is no work folder to walk; `work_bytes` counts
/// the own frame files deleting would free.
#[tokio::test]
async fn usage_without_a_prep_job_counts_own_deletable_frames() {
    let fx = fixture().await;
    let x = fx.outputs.join("datasets").join("orphan-job");
    let own = write(&x.join("raw").join("f1.png"), 64);
    let src = fx.tmp.path().join("src2");
    std::fs::create_dir_all(&src).unwrap();
    let c = fx.other_dataset(None, &src).await;
    fx.row_in(&c, &own, &src.join("v.mp4")).await;

    let u = usage(&fx.db, &fx.outputs, &fx.datasets, &c)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(u.work_dir, None);
    assert!(!u.work_walkable);
    assert_eq!((u.work_files, u.work_bytes), (1, 64));
}

#[test]
fn the_guard_never_deletes_a_root_itself() {
    let tmp = tempfile::tempdir().unwrap();
    let file = write(&tmp.path().join("root-file"), 3);
    let canonical = std::fs::canonicalize(&file).unwrap();
    let guard = guard::Guard::with_work_dir_for_test(canonical);
    match guard.check(&file) {
        guard::Verdict::Skip(s) => assert_eq!(s.reason, SKIP_OUTSIDE),
        guard::Verdict::Delete { .. } | guard::Verdict::Missing => {
            panic!("a root is never itself a deletable file")
        }
    }
}
