//! The apply against the scan's fixture tree and database: a dry run
//! deletes nothing, a real run deletes exactly the dry run's list, every
//! gate is re-checked at apply time (each test here fails when its gate
//! call is removed — checked by mutation while writing them), skips carry
//! reasons, and a real run writes the log.

use std::collections::BTreeMap;
use std::path::Path;

use super::super::scan::tests::{entry_ids, fixture, group_of, touch, touch_secs, Fx};
use super::super::scan::walk_files;
use super::*;
use crate::capability::dataset::housekeeping::SKIP_OUTSIDE;
use crate::cleanup::CleanupReport;
use crate::db::{DownloadState, NewDataset, RunState};
use crate::orchestrator::JobState;

pub(super) fn sel(group: &str, ids: &[&str]) -> Selection {
    Selection {
        group: group.into(),
        entry_ids: ids.iter().map(|s| s.to_string()).collect(),
    }
}

/// Every entry of every group of `r`.
pub(super) fn everything(r: &CleanupReport) -> Vec<Selection> {
    r.groups
        .iter()
        .map(|g| Selection {
            group: g.key.clone(),
            entry_ids: g.entries.iter().map(|e| e.id.clone()).collect(),
        })
        .collect()
}

pub(super) async fn run(fx: &Fx, dry_run: bool, selections: Vec<Selection>) -> Result<ApplyResult> {
    apply(
        &fx.db,
        &fx.ctx,
        ApplyRequest {
            selections,
            dry_run,
        },
    )
    .await
}

/// Every regular file below `dir` (links never followed), canonical
/// display path → bytes.
pub(super) fn tree(dir: &Path) -> BTreeMap<String, u64> {
    walk_files(dir)
        .into_iter()
        .map(|f| {
            let canonical = std::fs::canonicalize(&f.path).unwrap_or(f.path);
            (display(&canonical), f.bytes)
        })
        .collect()
}

pub(super) fn by_id<'a>(r: &'a ApplyResult, group: &str, id: &str) -> &'a EntryResult {
    r.entries
        .iter()
        .find(|e| e.group == group && e.id == id)
        .unwrap_or_else(|| panic!("no entry {group}/{id} in {:?}", r.entries))
}

pub(super) fn reasons(e: &EntryResult) -> Vec<&str> {
    e.skipped.iter().map(|s| s.reason.as_str()).collect()
}

/// The media gate is the scan, re-run at apply time: a file a job now owns
/// again, or one that vanished, is not offered any more — nothing else is
/// touched, and the other entry still goes.
#[tokio::test]
async fn a_media_file_the_scan_no_longer_offers_is_skipped() {
    let fx = fixture().await;
    let outputs = fx.paths().outputs_dir();
    touch(&outputs.join("claimed.png"), 100, 1);
    touch(&outputs.join("vanished.png"), 100, 1);
    touch(&outputs.join("stray.mp4"), 500, 1);
    let scan = fx.scan().await;
    let mut offered = entry_ids(group_of(&scan, "media_orphans"));
    offered.sort_unstable();
    assert_eq!(offered, ["claimed.png", "stray.mp4", "vanished.png"]);
    // Between scan and apply: a running job claims one, the other is gone.
    fx.job(JobState::Running, Some(&outputs.join("claimed.png")))
        .await;
    std::fs::remove_file(outputs.join("vanished.png")).unwrap();

    let r = run(
        &fx,
        false,
        vec![sel(
            "media_orphans",
            &["claimed.png", "vanished.png", "stray.mp4"],
        )],
    )
    .await
    .unwrap();

    assert!(
        outputs.join("claimed.png").is_file(),
        "a running job's file stays"
    );
    assert!(!outputs.join("stray.mp4").exists());
    assert_eq!(
        reasons(by_id(&r, "media_orphans", "claimed.png")),
        [SKIP_NOT_OFFERED]
    );
    assert_eq!(
        reasons(by_id(&r, "media_orphans", "vanished.png")),
        [SKIP_NOT_OFFERED]
    );
    assert_eq!(by_id(&r, "media_orphans", "stray.mp4").files, 1);
    assert_eq!((r.deleted_files, r.freed_bytes), (1, 500));
}

/// A busy dataset refuses the whole request — before the orphan named
/// alongside it is deleted — in a dry run too, and for its missing rows too.
#[tokio::test]
async fn a_busy_dataset_refuses_the_whole_request_before_anything_goes() {
    let fx = fixture().await;
    let outputs = fx.paths().outputs_dir();
    touch(&outputs.join("stray.mp4"), 500, 1);
    let job = fx.job(JobState::Running, None).await;
    let ds = fx.dataset("Busy", &job).await;
    let work = fx.paths().datasets_dir().join(&job).join("raw");
    fx.frame(&ds, &work.join("blurry.png"), Some(200), "blurry", false)
        .await;
    fx.frame(&ds, &work.join("pending.png"), None, "", false)
        .await;

    for (dry_run, group) in [
        (false, "discarded_frames"),
        (true, "discarded_frames"),
        (false, "missing_frame_rows"),
    ] {
        let err = run(
            &fx,
            dry_run,
            vec![
                sel("media_orphans", &["stray.mp4"]),
                sel(group, &[ds.id.as_str()]),
            ],
        )
        .await
        .expect_err("refused");
        assert!(err.to_string().contains("still being prepared"), "{err}");
    }
    assert!(outputs.join("stray.mp4").is_file(), "nothing was deleted");
    assert!(work.join("blurry.png").is_file());
    assert_eq!(
        fx.db
            .dataset_frames()
            .list_for_dataset(&ds.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn an_unfinished_run_refuses_the_whole_request() {
    let fx = fixture().await;
    let logs = fx.paths().logs_dir();
    touch(&logs.join("aiwm.log.2026-01-01"), 100, 60);
    touch(&logs.join("aiwm.log"), 100, 0);
    let run_row = fx
        .run(
            "going",
            RunState::Running,
            &fx.paths().training_dir().join("placeholder"),
        )
        .await;
    let dir = fx.run_folder(&run_row.id, "going");
    fx.db
        .training_runs()
        .set_work_dir(&run_row.id, &dir.to_string_lossy())
        .await
        .unwrap();

    let err = run(
        &fx,
        false,
        vec![
            sel("old_logs", &["old-logs"]),
            sel("finished_runs", &[run_row.id.as_str()]),
        ],
    )
    .await
    .expect_err("refused");

    assert!(err.to_string().contains("is running"), "{err}");
    assert!(logs.join("aiwm.log.2026-01-01").is_file());
    assert!(dir.join("train.log").is_file());
}

#[tokio::test]
async fn an_active_download_refuses_the_whole_request() {
    let fx = fixture().await;
    let p = fx.paths();
    touch(&p.cache_dir().join("index.json"), 1_000, 1);
    let active = fx.download(DownloadState::Running).await;

    let err = run(
        &fx,
        false,
        vec![sel(
            "caches",
            &["cache", &format!("download-staging:{active}")],
        )],
    )
    .await
    .expect_err("refused");

    assert!(err.to_string().contains("is running"), "{err}");
    assert!(p.cache_dir().join("index.json").is_file());
    assert!(p
        .downloads_dir()
        .join(&active)
        .join("w.safetensors.part")
        .is_file());
}

/// The guard's unclaimed rule decides at apply time: a folder a dataset
/// claims (its recorded work folder) is not deleted however it is named.
#[tokio::test]
async fn a_folder_a_dataset_claims_is_never_deleted_as_unclaimed() {
    let fx = fixture().await;
    let root = fx.paths().datasets_dir();
    let claimed = root.join("claimed");
    touch(&claimed.join("raw").join("a.png"), 100, 1);
    touch(&root.join("gone-dataset").join("raw").join("b.png"), 400, 1);
    let job = fx.job(JobState::Cancelled, None).await;
    let ds = fx
        .db
        .datasets()
        .create(NewDataset {
            name: "Owner".into(),
            mode: crate::db::DatasetMode::Frames,
            source_root: fx.source_dir().to_string_lossy().into_owned(),
            prep_job_id: Some(job),
            work_dir: Some(claimed.to_string_lossy().into_owned()),
        })
        .await
        .unwrap();
    fx.frame(&ds, &claimed.join("raw").join("a.png"), None, "", false)
        .await;

    let r = run(
        &fx,
        false,
        vec![sel(
            "unclaimed_dataset_folders",
            &["claimed", "gone-dataset"],
        )],
    )
    .await
    .unwrap();

    assert!(claimed.join("raw").join("a.png").is_file());
    assert_eq!(
        reasons(by_id(&r, "unclaimed_dataset_folders", "claimed")),
        [SKIP_NOT_OFFERED]
    );
    assert!(!root.join("gone-dataset").exists());
    assert_eq!(
        by_id(&r, "unclaimed_dataset_folders", "gone-dataset").files,
        1
    );
}

/// `check_purge_target` runs right before a run folder goes: a folder
/// holding another run's folder is refused with its reason.
#[tokio::test]
async fn a_run_folder_the_purge_check_refuses_is_skipped() {
    let fx = fixture().await;
    let training = fx.paths().training_dir();
    let outer = fx.run("outer", RunState::Cancelled, &training).await;
    let outer_dir = training.join(&outer.id);
    fx.db
        .training_runs()
        .set_work_dir(&outer.id, &outer_dir.to_string_lossy())
        .await
        .unwrap();
    touch(&outer_dir.join("train.log"), 30, 1);
    let inner = fx.run("inner", RunState::Cancelled, &training).await;
    let inner_dir = outer_dir.join(&inner.id);
    fx.db
        .training_runs()
        .set_work_dir(&inner.id, &inner_dir.to_string_lossy())
        .await
        .unwrap();
    touch(&inner_dir.join("train.log"), 30, 1);

    let r = run(&fx, false, vec![sel("finished_runs", &[outer.id.as_str()])])
        .await
        .unwrap();

    let e = by_id(&r, "finished_runs", &outer.id);
    assert_eq!(e.files, 0);
    assert!(
        e.skipped[0].reason.contains("overlaps the folder"),
        "{:?}",
        e.skipped
    );
    assert!(outer_dir.join("train.log").is_file());
    assert!(inner_dir.join("train.log").is_file());
}

#[tokio::test]
async fn a_partial_run_cleanup_keeps_the_final_checkpoint_and_the_config() {
    let fx = fixture().await;
    let run_row = fx
        .run(
            "lora-v2",
            RunState::Failed,
            &fx.paths().training_dir().join("placeholder"),
        )
        .await;
    let dir = fx.run_folder(&run_row.id, "lora-v2");
    fx.db
        .training_runs()
        .set_work_dir(&run_row.id, &dir.to_string_lossy())
        .await
        .unwrap();

    let r = run(
        &fx,
        false,
        vec![sel("finished_runs", &[run_row.id.as_str()])],
    )
    .await
    .unwrap();

    let e = by_id(&r, "finished_runs", &run_row.id);
    assert_eq!((e.files, e.bytes), (6, 200 + 400 + 50 + 10 + 10 + 30));
    let out = dir.join("output").join("lora-v2");
    assert!(
        out.join("lora-v2.safetensors").is_file(),
        "the final checkpoint"
    );
    assert!(dir.join("config.yaml").is_file());
    assert!(!out.join("lora-v2_000000200.safetensors").exists());
    assert!(!out.join("optimizer.pt").exists());
    assert!(!dir.join("train.log").exists());
    assert!(dir.is_dir(), "the folder itself stays");
}

#[tokio::test]
async fn only_rows_whose_file_is_still_missing_are_removed() {
    let fx = fixture().await;
    let job = fx.job(JobState::Cancelled, None).await;
    let ds = fx.dataset("Demo", &job).await;
    let work = fx.paths().datasets_dir().join(&job).join("raw");
    fx.frame(&ds, &work.join("a.png"), Some(10), "", false)
        .await;
    fx.frame(&ds, &work.join("gone.png"), None, "", false).await;
    fx.frame(&ds, &work.join("came-back.png"), None, "", false)
        .await;
    // Written between scan and apply: its row must stay.
    touch(&work.join("came-back.png"), 10, 1);

    let r = run(
        &fx,
        false,
        vec![sel("missing_frame_rows", &[ds.id.as_str()])],
    )
    .await
    .unwrap();

    let e = by_id(&r, "missing_frame_rows", &ds.id);
    assert_eq!((e.rows, e.files, e.bytes), (1, 0, 0));
    assert_eq!(e.label, "Demo");
    let mut left: Vec<String> = fx
        .db
        .dataset_frames()
        .list_for_dataset(&ds.id)
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.frame_path)
        .collect();
    left.sort();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left[0].ends_with("a.png") && left[1].ends_with("came-back.png"));
    assert!(work.join("a.png").is_file());
}

#[tokio::test]
async fn old_logs_apply_never_deletes_the_newest_log_even_when_it_is_old() {
    let fx = fixture().await;
    let logs = fx.paths().logs_dir();
    touch(&logs.join("aiwm.log.2026-01-01"), 100, 60);
    touch(&logs.join("aiwm.log.2026-02-01"), 100, 45);
    touch(&logs.join("aiwm.log"), 100, 40);

    let r = run(&fx, false, vec![sel("old_logs", &["old-logs"])])
        .await
        .unwrap();

    let e = by_id(&r, "old_logs", "old-logs");
    assert_eq!((e.files, e.bytes), (2, 200));
    assert!(e.label.starts_with("2 log file(s)"), "{}", e.label);
    assert!(logs.join("aiwm.log").is_file(), "the newest log stays");
    assert!(!logs.join("aiwm.log.2026-01-01").exists());
    assert!(!logs.join("aiwm.log.2026-02-01").exists());
}

#[tokio::test]
async fn comfyui_leftovers_of_an_unfinished_job_are_kept() {
    let fx = fixture().await;
    let input = fx.paths().comfyui_data_dir().join("input");
    touch_secs(&input.join("old.png"), 300, 2 * 3_600);
    touch_secs(&input.join("fresh.png"), 300, 60);
    let running = fx.job(JobState::Running, None).await;
    touch_secs(&input.join(format!("{running}.png")), 300, 2 * 3_600);

    let r = run(&fx, false, vec![sel("caches", &["comfyui:input"])])
        .await
        .unwrap();

    let e = by_id(&r, "caches", "comfyui:input");
    assert_eq!((e.files, e.bytes), (1, 300));
    assert!(!input.join("old.png").exists());
    assert!(input.join("fresh.png").is_file());
    assert!(input.join(format!("{running}.png")).is_file());
}

/// A junction inside the cache pointing at the model store: reported as a
/// link, never followed, the store untouched, the cache root kept.
#[cfg(windows)]
#[tokio::test]
async fn a_link_inside_the_cache_is_skipped_and_never_followed() {
    let fx = fixture().await;
    let cache = fx.paths().cache_dir();
    touch(&cache.join("registry").join("index.json"), 1_000, 1);
    let weights = fx.ctx.store.join("image").join("big.safetensors");
    touch(&weights, 10_000, 1);
    let link = cache.join("models");
    if !crate::training::location::tests::make_junction(&link, &fx.ctx.store) {
        eprintln!("skipped: cannot create a junction here");
        return;
    }

    let r = run(&fx, false, vec![sel("caches", &["cache"])])
        .await
        .unwrap();

    let e = by_id(&r, "caches", "cache");
    assert_eq!((e.files, e.bytes), (1, 1_000));
    assert!(weights.is_file(), "the store is untouched");
    assert!(
        !cache.join("registry").exists(),
        "emptied folders are pruned"
    );
    assert!(cache.is_dir(), "the root stays");
    assert_eq!(reasons(e), [SKIP_LINK]);
    assert!(e.skipped[0].path.ends_with("models"), "{:?}", e.skipped);
}

/// An id that resolves outside its root — a traversal, a folder elsewhere —
/// is refused by the strictly-inside check, whatever it names.
#[tokio::test]
async fn an_id_resolving_outside_its_root_is_skipped() {
    let fx = fixture().await;
    let clip = fx.source_dir().join("clip.mp4");
    touch(&clip, 5_000, 1);
    std::fs::create_dir_all(fx.paths().exports_dir()).unwrap();
    std::fs::create_dir_all(fx.paths().downloads_dir()).unwrap();
    // `<data>/exports/../../src/clip.mp4` and `<data>/.downloads/../../src`.
    let traversal = format!("..{s}..{s}src{s}clip.mp4", s = std::path::MAIN_SEPARATOR);
    let folder = format!(
        "download-staging:..{s}..{s}src",
        s = std::path::MAIN_SEPARATOR
    );

    let r = run(
        &fx,
        false,
        vec![
            sel("db_backups", &[traversal.as_str()]),
            sel("caches", &[folder.as_str()]),
        ],
    )
    .await
    .unwrap();

    assert!(clip.is_file(), "the source file is untouched");
    assert_eq!(reasons(by_id(&r, "db_backups", &traversal)), [SKIP_OUTSIDE]);
    assert_eq!(reasons(by_id(&r, "caches", &folder)), [SKIP_OUTSIDE]);
    assert_eq!(r.deleted_files, 0);
}

#[tokio::test]
async fn a_real_apply_writes_one_log_row_per_entry_and_a_dry_run_none() {
    let fx = fixture().await;
    let exports = fx.paths().exports_dir();
    touch(&exports.join("aiwm-backup-1.zip"), 1_000, 20);
    touch(&fx.paths().outputs_dir().join("stray.mp4"), 500, 1);
    let selections = || {
        vec![
            sel("db_backups", &["aiwm-backup-1.zip", "nope.zip"]),
            sel("media_orphans", &["stray.mp4"]),
        ]
    };

    let backup_path = display(&std::fs::canonicalize(exports.join("aiwm-backup-1.zip")).unwrap());
    run(&fx, true, selections()).await.unwrap();
    assert!(fx.db.cleanup_log().list(20).await.unwrap().is_empty());

    let r = run(&fx, false, selections()).await.unwrap();
    assert_eq!((r.deleted_files, r.freed_bytes), (2, 1_500));
    let log = fx.db.cleanup_log().list(20).await.unwrap();
    assert_eq!(log.len(), 3);
    // Newest first — the groups ran in the scan's order (media before
    // backups), the ids in request order.
    let keys: Vec<(&str, &str)> = log
        .iter()
        .map(|l| (l.group_key.as_str(), l.entry_id.as_str()))
        .collect();
    assert_eq!(
        keys,
        [
            ("db_backups", "nope.zip"),
            ("db_backups", "aiwm-backup-1.zip"),
            ("media_orphans", "stray.mp4"),
        ]
    );
    let backup = &log[1];
    assert_eq!((backup.deleted_files, backup.freed_bytes), (1, 1_000));
    assert_eq!(backup.detail["paths"][0].as_str().unwrap(), backup_path);
    let unknown = &log[0];
    assert_eq!((unknown.deleted_files, unknown.skipped_count), (0, 1));
    assert_eq!(unknown.detail["skipped"][0]["reason"], SKIP_NOT_OFFERED);
}
