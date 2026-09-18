//! Housekeeping against real files in a temp folder laid out exactly like the
//! app's: `<outputs>/datasets/<prep_job_id>/raw/<tag>/<video stem>/`.

use std::path::{Path, PathBuf};

use image::{GrayImage, Luma};

use super::*;
use crate::db::{
    Database, Dataset, DatasetMode, NewConcept, NewDataset, NewDatasetFrame, NewJob,
    NewTrainingRun, Preset, RunState,
};

struct Fx {
    tmp: tempfile::TempDir,
    db: Database,
    outputs: PathBuf,
    /// `<outputs>/datasets/<job>` — the dataset's app-owned work folder.
    work_root: PathBuf,
    /// Where the fixture's frames live inside the work folder.
    clip_dir: PathBuf,
    job_id: String,
    dataset: Dataset,
    source: PathBuf,
}

async fn fixture_with_mode(mode: DatasetMode) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let job_id = db
        .jobs()
        .insert(NewJob::new("dataset_prep"))
        .await
        .unwrap()
        .id;
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

async fn fixture() -> Fx {
    fixture_with_mode(DatasetMode::Frames).await
}

impl Fx {
    /// A frame row whose file (`bytes` long) sits at `path`.
    async fn frame_at(&self, path: &Path, bytes: usize, reason: &str, excluded: bool) -> String {
        std::fs::write(path, vec![1u8; bytes]).unwrap();
        self.row(path, &self.source, reason, excluded).await
    }

    /// A frame row in the fixture's clip folder.
    async fn frame(&self, name: &str, bytes: usize, reason: &str, excluded: bool) -> String {
        self.frame_at(&self.clip_dir.join(name), bytes, reason, excluded)
            .await
    }

    async fn row(&self, frame: &Path, source: &Path, reason: &str, excluded: bool) -> String {
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

    async fn frame_ids(&self) -> Vec<String> {
        self.db
            .dataset_frames()
            .list_for_dataset(&self.dataset.id)
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.id)
            .collect()
    }

    async fn start_run(&self) -> String {
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
fn victim(fx: &Fx) -> PathBuf {
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

#[test]
fn the_guard_refuses_a_relative_path_whatever_the_working_directory() {
    let guard = Guard {
        // `check` only returns a verdict; nothing is deleted here.
        work_dir: Some(std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap()),
        export_dir: None,
        sources: HashSet::new(),
    };
    // Would resolve inside the "work folder" if it were canonicalised
    // against the process working directory.
    match guard.check(Path::new("Cargo.toml")) {
        Verdict::Skip(s) => assert_eq!(s.reason, SKIP_OUTSIDE),
        Verdict::Delete { .. } | Verdict::Missing => panic!("a relative path must be skipped"),
    }
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

// --- global dedup -------------------------------------------------------------

/// 64x64 with 16px blocks: low-frequency structure a perceptual hash sees.
fn blocks() -> GrayImage {
    GrayImage::from_fn(64, 64, |x, y| {
        Luma([if (x / 16 + y / 16) % 2 == 0 { 230 } else { 20 }])
    })
}

fn stripes() -> GrayImage {
    GrayImage::from_fn(64, 64, |x, _| {
        Luma([if (x / 8) % 2 == 0 { 240 } else { 10 }])
    })
}

fn gradient() -> GrayImage {
    GrayImage::from_fn(64, 64, |x, y| Luma([((x + y) * 2).min(255) as u8]))
}

fn save(dir: &Path, name: &str, img: &GrayImage) -> PathBuf {
    let path = dir.join(name);
    img.save(&path).unwrap();
    path
}

#[tokio::test]
async fn dedup_groups_across_sources_and_keeps_the_sharpest_frame() {
    let fx = fixture().await;
    let dir_a = fx.clip_dir.clone();
    let dir_b = fx.work_root.join("raw").join("Tag").join("other");
    std::fs::create_dir_all(&dir_b).unwrap();
    let source_b = fx.tmp.path().join("src").join("other.mp4");
    let blurred = imageproc::filter::gaussian_blur_f32(&blocks(), 1.5);

    // The blurred copies land first, so insertion order cannot pick the keeper.
    let a_blur = fx
        .row(&save(&dir_a, "a_blur.png", &blurred), &fx.source, "", false)
        .await;
    let a_sharp = fx
        .row(
            &save(&dir_a, "a_sharp.png", &blocks()),
            &fx.source,
            "",
            false,
        )
        .await;
    let a_other = fx
        .row(
            &save(&dir_a, "a_other.png", &stripes()),
            &fx.source,
            "",
            false,
        )
        .await;
    let b_blur = fx
        .row(&save(&dir_b, "b_blur.png", &blurred), &source_b, "", false)
        .await;
    let b_other = fx
        .row(
            &save(&dir_b, "b_other.png", &gradient()),
            &source_b,
            "",
            false,
        )
        .await;
    // Already discarded frames are not looked at.
    let excluded = fx
        .row(
            &save(&dir_b, "b_excluded.png", &blocks()),
            &source_b,
            "",
            true,
        )
        .await;

    // Fixture sanity: the blurred copy really is a near-duplicate, the other
    // scenes really are different pictures.
    let (h_sharp, s_sharp) = filter::phash_and_sharpness(&dir_a.join("a_sharp.png")).unwrap();
    let (h_blur, s_blur) = filter::phash_and_sharpness(&dir_a.join("a_blur.png")).unwrap();
    let (h_stripes, _) = filter::phash_and_sharpness(&dir_a.join("a_other.png")).unwrap();
    assert!(h_sharp.dist(&h_blur) <= DEFAULT_DEDUP_THRESHOLD);
    assert!(s_sharp > s_blur);
    assert!(h_sharp.dist(&h_stripes) > MAX_DEDUP_THRESHOLD);

    let s = dedup(&fx.db, &fx.dataset.id, None).await.unwrap().unwrap();
    assert_eq!(s.threshold, DEFAULT_DEDUP_THRESHOLD);
    assert_eq!(s.scanned, 5);
    assert_eq!(s.groups, 1);
    assert_eq!(s.marked, 2);
    assert_eq!(s.unreadable, 0);

    let reason = |id: String| {
        let db = &fx.db;
        async move {
            db.dataset_frames()
                .get(&id)
                .await
                .unwrap()
                .unwrap()
                .rejection_reason
        }
    };
    assert_eq!(reason(a_sharp).await, "");
    assert_eq!(reason(a_other).await, "");
    assert_eq!(reason(b_other).await, "");
    assert_eq!(reason(excluded).await, "");
    assert_eq!(reason(a_blur).await, "duplicate_global");
    assert_eq!(reason(b_blur).await, "duplicate_global");

    // Deterministic and idempotent: a second run finds nothing new.
    let again = dedup(&fx.db, &fx.dataset.id, None).await.unwrap().unwrap();
    assert_eq!((again.scanned, again.marked), (3, 0));
}

#[tokio::test]
async fn dedup_breaks_a_sharpness_tie_toward_the_earlier_frame() {
    let fx = fixture().await;
    let first = fx
        .row(
            &save(&fx.clip_dir, "1.png", &blocks()),
            &fx.source,
            "",
            false,
        )
        .await;
    let second = fx
        .row(
            &save(&fx.clip_dir, "2.png", &blocks()),
            &fx.source,
            "",
            false,
        )
        .await;
    let s = dedup(&fx.db, &fx.dataset.id, Some(0))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((s.groups, s.marked), (1, 1));
    let repo = fx.db.dataset_frames();
    assert_eq!(
        repo.get(&first).await.unwrap().unwrap().rejection_reason,
        ""
    );
    assert_eq!(
        repo.get(&second).await.unwrap().unwrap().rejection_reason,
        "duplicate_global"
    );
}

#[tokio::test]
async fn dedup_counts_unreadable_frames_instead_of_failing() {
    let fx = fixture().await;
    fx.frame("broken.png", 10, "", false).await;
    fx.row(
        &save(&fx.clip_dir, "ok.png", &blocks()),
        &fx.source,
        "",
        false,
    )
    .await;
    let s = dedup(&fx.db, &fx.dataset.id, None).await.unwrap().unwrap();
    assert_eq!((s.scanned, s.unreadable, s.marked), (2, 1, 0));
}

#[test]
fn dedup_threshold_defaults_to_the_filter_constant_and_clamps_to_16() {
    assert_eq!(dedup_threshold(None), filter::DEFAULT_PHASH_MAX_DISTANCE);
    assert_eq!(dedup_threshold(Some(0)), 0);
    assert_eq!(dedup_threshold(Some(16)), 16);
    assert_eq!(dedup_threshold(Some(64)), 16);
}

#[tokio::test]
async fn dedup_reports_the_clamped_threshold() {
    let fx = fixture().await;
    let s = dedup(&fx.db, &fx.dataset.id, Some(99))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.threshold, MAX_DEDUP_THRESHOLD);
}

#[tokio::test]
async fn dedup_refuses_a_clips_dataset_and_ignores_unknown_ids() {
    let fx = fixture_with_mode(DatasetMode::Clips).await;
    let err = dedup(&fx.db, &fx.dataset.id, None).await.unwrap_err();
    assert!(matches!(err, CoreError::Config(_)), "{err}");
    assert!(dedup(&fx.db, "nope", None).await.unwrap().is_none());
}

/// Timing probe, run on demand:
/// `cargo test -p aiwm-core --lib dedup_timing -- --ignored --nocapture`.
/// 1000 distinct 64x64 frames, then 50 1280x720 frames to extrapolate to
/// real extracted stills.
#[tokio::test]
#[ignore]
async fn dedup_timing() {
    let fx = fixture().await;
    for i in 0..1000u32 {
        let img = GrayImage::from_fn(64, 64, |x, y| {
            let v = (x.wrapping_mul(i + 3) ^ y.wrapping_mul(i * 7 + 1)) % 256;
            Luma([v as u8])
        });
        fx.row(
            &save(&fx.clip_dir, &format!("{i:04}.png"), &img),
            &fx.source,
            "",
            false,
        )
        .await;
    }
    let t = std::time::Instant::now();
    let s = dedup(&fx.db, &fx.dataset.id, None).await.unwrap().unwrap();
    println!(
        "dedup 1000 x 64x64: {:?} (scanned {}, marked {})",
        t.elapsed(),
        s.scanned,
        s.marked
    );

    let big = fixture().await;
    for i in 0..50u32 {
        let img = GrayImage::from_fn(1280, 720, |x, y| {
            Luma([((x / (i + 8)) ^ (y / (i + 5))).wrapping_mul(37) as u8])
        });
        big.row(
            &save(&big.clip_dir, &format!("{i:04}.png"), &img),
            &big.source,
            "",
            false,
        )
        .await;
    }
    let t = std::time::Instant::now();
    let s = dedup(&big.db, &big.dataset.id, None)
        .await
        .unwrap()
        .unwrap();
    println!(
        "dedup 50 x 1280x720: {:?} (scanned {}, marked {})",
        t.elapsed(),
        s.scanned,
        s.marked
    );
}
