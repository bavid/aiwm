//! Dataset housekeeping: how much disk a dataset uses, deleting it or some of
//! its frames *with their files*, cleaning up discarded frames, and finding
//! near-duplicates across the whole dataset (design spec
//! `2026-09-18-dataset-curation-design.md`).
//!
//! **Path guard.** Every file this module deletes first goes through
//! [`guard::Guard`]: canonicalised, strictly inside one of this dataset's own
//! roots, never a source file or inside any dataset's source folder, and
//! never a file, work folder or export folder of **another** dataset. The
//! module docs of [`guard`] list every rule. Files the guard refuses are
//! reported as skipped with a reason, never deleted.
//!
//! **When deleting is refused.** Deleting a dataset, some of its frames or
//! its discarded frames is refused while a training run of the dataset has
//! not finished, and while the dataset's prep job has not finished (it may
//! still be writing frames). Other datasets need no such check: their files
//! are never touched. An export is not a job, so a concurrently running
//! export cannot be observed here. The checks run before the file work
//! starts; a run or prep job started in the moment between the check and the
//! deletion is not seen — an accepted race, as both are started by the same
//! single user from the same UI. Likewise the guard protects the datasets
//! and frames that exist when the deletion reads its snapshot; a dataset
//! created while a deletion runs is not known to it (a new prep run writes
//! into its own, new work folder, which no deletion ever targets).
//!
//! **Blocking work.** All file-system walking, deleting and image decoding
//! runs on `spawn_blocking` so a large dataset never stalls the async
//! runtime that also serves the API.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::db::{Database, Dataset, DatasetFrame, DatasetMode};
use crate::{CoreError, Result};

use super::{dataset_err, filter};

mod dedup_plan;
mod guard;
use dedup_plan::plan_dedup;
use guard::{skipped, Guard, Snapshot, Verdict};

/// Default Hamming distance for the dataset-wide dedup — the same one the
/// per-source duplicate filter uses.
pub const DEFAULT_DEDUP_THRESHOLD: u32 = filter::DEFAULT_PHASH_MAX_DISTANCE;
/// Upper bound for a caller-chosen dedup threshold. Beyond this, frames that
/// merely share a composition start to count as "the same picture".
pub const MAX_DEDUP_THRESHOLD: u32 = 16;
/// Most frame ids one bulk request may name.
pub const MAX_FRAME_IDS: usize = 10_000;

/// Skip reasons reported in [`SkippedFile::reason`].
pub const SKIP_OUTSIDE: &str = "outside_app_folders";
pub const SKIP_SOURCE: &str = "source_file";
pub const SKIP_IN_USE: &str = "in_use";
pub const SKIP_NOT_A_FILE: &str = "not_a_file";
pub const SKIP_ERROR: &str = "error";
pub const SKIP_OTHER_DATASET: &str = "used_by_other_dataset";

/// A file a deletion left alone, and why (one of the `SKIP_*` constants;
/// `"error: …"` when deleting it failed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

/// `GET /datasets/{id}/usage` — sizes measured by walking the folders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatasetUsage {
    /// The app-owned work folder `<outputs>/datasets/<prep_job_id>`, `None`
    /// when it does not exist (any more) or the prep job is gone.
    pub work_dir: Option<String>,
    /// `true` when deleting the dataset removes the whole work folder (no
    /// other dataset and no source folder points into it).
    pub work_walkable: bool,
    /// What deleting the dataset frees from its work folder: the whole
    /// folder when `work_walkable`, otherwise only this dataset's own frame
    /// files the guard would delete (file by file).
    pub work_bytes: u64,
    pub work_files: u64,
    /// The last export destination, app-owned or not.
    pub export_dir: Option<String>,
    /// Bytes of the export's numbered files; only measured when the export
    /// is app-owned (that is what deleting the dataset would remove).
    pub export_bytes: u64,
    pub export_app_owned: bool,
    /// Excluded + rejected frames, and the bytes a cleanup would free.
    pub discarded_frames: u64,
    pub discarded_bytes: u64,
}

/// `DELETE /datasets/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatasetDeleteSummary {
    /// Frame rows of the dataset (deleted with it when `dataset_deleted`).
    pub frames: u64,
    pub deleted_files: u64,
    pub freed_bytes: u64,
    pub skipped_files: Vec<SkippedFile>,
    /// A user-chosen or shared export folder, left as is.
    pub export_dir_kept: Option<String>,
    /// `false` when a file could not be deleted (see `skipped_files`, reason
    /// `"error: …"`): the dataset row stays so the delete can be retried.
    pub dataset_deleted: bool,
}

/// `POST /datasets/{id}/frames/delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FramesDeleteSummary {
    /// Frame rows deleted.
    pub deleted: u64,
    pub deleted_files: u64,
    pub freed_bytes: u64,
    pub skipped_files: Vec<SkippedFile>,
}

/// `POST /datasets/{id}/cleanup`. In a dry run `frames`/`bytes` are what a
/// real run would delete and free; after a real run, what it did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CleanupSummary {
    pub dry_run: bool,
    pub frames: u64,
    pub bytes: u64,
    pub deleted_files: u64,
    pub skipped_files: Vec<SkippedFile>,
}

/// `POST /datasets/{id}/dedup`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DedupSummary {
    /// The threshold actually used (after defaulting and clamping).
    pub threshold: u32,
    /// Kept frames looked at (not excluded, not rejected).
    pub scanned: u64,
    /// Groups of near-duplicates found (each keeps its sharpest frame).
    pub groups: u64,
    /// Frames newly marked `duplicate_global`.
    pub marked: u64,
    /// Frames whose image could not be read; left untouched.
    pub unreadable: u64,
}

/// The caller's threshold, defaulted and clamped to `0..=16`.
pub fn dedup_threshold(requested: Option<u32>) -> u32 {
    requested
        .unwrap_or(DEFAULT_DEDUP_THRESHOLD)
        .min(MAX_DEDUP_THRESHOLD)
}

/// Refuse a request naming more than [`MAX_FRAME_IDS`] frames.
pub fn check_frame_ids(ids: &[String]) -> Result<()> {
    if ids.len() > MAX_FRAME_IDS {
        return Err(CoreError::Config(format!(
            "at most {MAX_FRAME_IDS} frames per request, got {}",
            ids.len()
        )));
    }
    Ok(())
}

// --- public operations ------------------------------------------------------

/// How much disk `dataset_id` uses. `None` for an unknown dataset.
pub async fn usage(
    db: &Database,
    outputs_dir: &Path,
    dataset_id: &str,
) -> Result<Option<DatasetUsage>> {
    let Some(snap) = snapshot(db, dataset_id).await? else {
        return Ok(None);
    };
    let outputs_dir = outputs_dir.to_path_buf();
    blocking(move || {
        let (discarded, staying): (Vec<&DatasetFrame>, Vec<&DatasetFrame>) =
            snap.frames.iter().partition(|f| is_discarded(f));
        let discarded_frames = discarded.len() as u64;
        let guard = Guard::build(&outputs_dir, &snap);
        let walkable = guard.walkable;
        let work_dir = guard.work_dir.as_deref().map(display);
        // What deleting the dataset frees from its work folder: the walk
        // when the folder may go as a whole, otherwise its own frame files
        // one by one (nothing stays when the whole dataset is deleted).
        let (work_files, work_bytes) = match guard.work_dir.as_deref().filter(|_| walkable) {
            Some(work) => dir_totals(work),
            None => deletable_totals(&guard, snap.frames.iter()),
        };
        let export_bytes = guard
            .export_dir
            .as_deref()
            .map(|d| export_files(d).iter().map(|(_, len)| len).sum())
            .unwrap_or(0);
        let export_app_owned = guard.export_dir.is_some();
        // A cleanup keeps every non-discarded frame; without discarded frames
        // there is nothing to check.
        let discarded_bytes = if discarded.is_empty() {
            0
        } else {
            let staying: Vec<&str> = staying.iter().map(|f| f.frame_path.as_str()).collect();
            deletable_totals(&guard.keeping(&staying), discarded.into_iter()).1
        };
        DatasetUsage {
            work_dir,
            work_walkable: walkable,
            work_bytes,
            work_files,
            export_dir: snap.dataset.export_dir.clone(),
            export_bytes,
            export_app_owned,
            discarded_frames,
            discarded_bytes,
        }
    })
    .await
    .map(Some)
}

/// Delete the dataset: its own files (see [`guard`]), then its rows (frames
/// and concepts cascade). When any file fails to delete, the rows stay and
/// `dataset_deleted` is `false`, so the delete can be retried. `None` for an
/// unknown dataset.
pub async fn delete_dataset_with_files(
    db: &Database,
    outputs_dir: &Path,
    dataset_id: &str,
) -> Result<Option<DatasetDeleteSummary>> {
    let Some(snap) = snapshot(db, dataset_id).await? else {
        return Ok(None);
    };
    refuse_if_busy(db, &snap.dataset).await?;
    let dataset = snap.dataset.clone();
    let frame_count = snap.frames.len() as u64;
    let outputs = outputs_dir.to_path_buf();
    let (tally, export_kept) = blocking(move || {
        let guard = Guard::build(&outputs, &snap);
        let mut tally = Tally::default();
        if let Some(work) = guard.work_dir.as_deref().filter(|_| guard.walkable) {
            for file in walk_files(work) {
                tally.remove(&guard, &file);
            }
        }
        if let Some(export) = guard.export_dir.as_deref() {
            for (file, _) in export_files(export) {
                tally.remove(&guard, &file);
            }
        }
        for f in &snap.frames {
            tally.remove(&guard, Path::new(&f.frame_path));
        }
        match guard.work_dir.as_deref().filter(|_| guard.walkable) {
            Some(work) => prune_empty_dirs(work),
            None => guard.prune_after(&tally.deleted),
        }
        guard.remove_export_dir_if_empty();
        let export_kept = match (&snap.dataset.export_dir, &guard.export_dir) {
            (Some(dir), None) => Some(dir.clone()),
            _ => None,
        };
        (tally, export_kept)
    })
    .await?;
    let dataset_deleted = tally.failed == 0;
    if dataset_deleted {
        db.datasets().delete(&dataset.id).await?;
    }
    tracing::info!(
        dataset = %dataset.id,
        frames = frame_count,
        files = tally.deleted.len(),
        bytes = tally.freed_bytes,
        skipped = tally.skipped.len(),
        failed = tally.failed,
        dataset_deleted,
        "deleted dataset with its files"
    );
    Ok(Some(DatasetDeleteSummary {
        frames: frame_count,
        deleted_files: tally.deleted.len() as u64,
        freed_bytes: tally.freed_bytes,
        skipped_files: tally.skipped,
        export_dir_kept: export_kept,
        dataset_deleted,
    }))
}

/// Delete some frames of a dataset: their files (through the guard) and
/// their rows. Ids of another dataset are ignored. A row whose file could
/// not be deleted because of an I/O error stays, so the curator can retry;
/// a row whose file is not the app's to delete goes, the file stays.
pub async fn delete_frames(
    db: &Database,
    outputs_dir: &Path,
    dataset_id: &str,
    frame_ids: &[String],
) -> Result<Option<FramesDeleteSummary>> {
    check_frame_ids(frame_ids)?;
    let Some(snap) = snapshot(db, dataset_id).await? else {
        return Ok(None);
    };
    refuse_if_busy(db, &snap.dataset).await?;
    let wanted: HashSet<&str> = frame_ids.iter().map(String::as_str).collect();
    let targets: HashSet<String> = snap
        .frames
        .iter()
        .filter(|f| wanted.contains(f.id.as_str()))
        .map(|f| f.id.clone())
        .collect();
    let s = remove_frames(db, outputs_dir, snap, targets).await?;
    tracing::info!(
        dataset = %dataset_id,
        frames = s.deleted,
        files = s.deleted_files,
        bytes = s.freed_bytes,
        skipped = s.skipped_files.len(),
        "deleted dataset frames with their files"
    );
    Ok(Some(s))
}

/// Delete every discarded frame (excluded or rejected) with its file; a dry
/// run only measures. Kept frames are never touched.
pub async fn cleanup(
    db: &Database,
    outputs_dir: &Path,
    dataset_id: &str,
    dry_run: bool,
) -> Result<Option<CleanupSummary>> {
    if dry_run {
        let usage = usage(db, outputs_dir, dataset_id).await?;
        return Ok(usage.map(|u| CleanupSummary {
            dry_run: true,
            frames: u.discarded_frames,
            bytes: u.discarded_bytes,
            deleted_files: 0,
            skipped_files: Vec::new(),
        }));
    }
    let Some(snap) = snapshot(db, dataset_id).await? else {
        return Ok(None);
    };
    refuse_if_busy(db, &snap.dataset).await?;
    let targets: HashSet<String> = snap
        .frames
        .iter()
        .filter(|f| is_discarded(f))
        .map(|f| f.id.clone())
        .collect();
    let s = remove_frames(db, outputs_dir, snap, targets).await?;
    tracing::info!(
        dataset = %dataset_id,
        frames = s.deleted,
        files = s.deleted_files,
        bytes = s.freed_bytes,
        skipped = s.skipped_files.len(),
        "cleaned up discarded dataset frames"
    );
    Ok(Some(CleanupSummary {
        dry_run: false,
        frames: s.deleted,
        bytes: s.freed_bytes,
        deleted_files: s.deleted_files,
        skipped_files: s.skipped_files,
    }))
}

/// One dedup at a time: its decoding already uses up to eight threads, and
/// two concurrent requests would double that.
static DEDUP_SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// Find near-duplicates across every kept frame of the dataset — all
/// sources, not just neighbours — and mark all but the sharpest of each
/// group `duplicate_global`. Only marks: the files stay until a cleanup, and
/// moving a frame back to "Keep" clears the mark. See
/// [`dedup_plan::plan_dedup`] for the grouping.
pub async fn dedup(
    db: &Database,
    dataset_id: &str,
    threshold: Option<u32>,
) -> Result<Option<DedupSummary>> {
    let Some(dataset) = db.datasets().get(dataset_id).await? else {
        return Ok(None);
    };
    if dataset.mode == DatasetMode::Clips {
        return Err(CoreError::Config(
            "duplicate search works on still frames; this dataset holds clips".into(),
        ));
    }
    let threshold = dedup_threshold(threshold);
    let items: Vec<(String, PathBuf)> = db
        .dataset_frames()
        .list_for_dataset(dataset_id)
        .await?
        .into_iter()
        .filter(|f| !is_discarded(f) && !f.frame_path.is_empty())
        .map(|f| (f.id, PathBuf::from(f.frame_path)))
        .collect();
    let scanned = items.len() as u64;
    let _slot = DEDUP_SLOT
        .acquire()
        .await
        .map_err(|e| dataset_err(format!("duplicate search unavailable: {e}")))?;
    let started = std::time::Instant::now();
    let plan = blocking(move || plan_dedup(&items, threshold)).await?;
    let marked = db
        .dataset_frames()
        .set_rejection_reason_many(
            &dataset.id,
            &plan.duplicates,
            filter::RejectionReason::DuplicateGlobal.as_str(),
        )
        .await?;
    tracing::info!(
        dataset = %dataset.id,
        scanned,
        groups = plan.groups,
        marked,
        unreadable = plan.unreadable,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "global duplicate search"
    );
    Ok(Some(DedupSummary {
        threshold,
        scanned,
        groups: plan.groups,
        marked,
        unreadable: plan.unreadable,
    }))
}

// --- internals ----------------------------------------------------------------

fn is_discarded(f: &DatasetFrame) -> bool {
    f.excluded || !f.rejection_reason.is_empty()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| dataset_err(format!("housekeeping task failed: {e}")))
}

/// This dataset, its frames, and what every other dataset references.
async fn snapshot(db: &Database, dataset_id: &str) -> Result<Option<Snapshot>> {
    let Some(dataset) = db.datasets().get(dataset_id).await? else {
        return Ok(None);
    };
    let frames = db.dataset_frames().list_for_dataset(dataset_id).await?;
    let others = db
        .datasets()
        .list()
        .await?
        .into_iter()
        .filter(|d| d.id != dataset.id)
        .collect();
    let foreign_frames = db
        .dataset_frames()
        .list_paths_outside_dataset(dataset_id)
        .await?;
    Ok(Some(Snapshot {
        dataset,
        frames,
        others,
        foreign_frames,
    }))
}

/// Refuse while a training run of the dataset or its prep job is not
/// finished. See the module docs for the accepted race.
async fn refuse_if_busy(db: &Database, dataset: &Dataset) -> Result<()> {
    let runs = db
        .training_runs()
        .list_active_for_dataset(&dataset.id)
        .await?;
    if let Some(run) = runs.first() {
        return Err(CoreError::Config(format!(
            "dataset \"{}\" is in use by training run \"{}\" ({}) \u{2014} wait for it to \
             finish or cancel it first",
            dataset.name,
            run.name,
            run.state.as_str()
        )));
    }
    if let Some(job_id) = dataset.prep_job_id.as_deref() {
        if let Some(job) = db.jobs().get(job_id).await? {
            if !job.state.is_terminal() {
                return Err(CoreError::Config(format!(
                    "dataset \"{}\" is still being prepared (job {job_id} is {}) \u{2014} wait \
                     for it to finish or cancel it first",
                    dataset.name,
                    job.state.as_str()
                )));
            }
        }
    }
    Ok(())
}

/// Files of the frames in `targets`, then their rows.
async fn remove_frames(
    db: &Database,
    outputs_dir: &Path,
    snap: Snapshot,
    targets: HashSet<String>,
) -> Result<FramesDeleteSummary> {
    let dataset_id = snap.dataset.id.clone();
    let outputs = outputs_dir.to_path_buf();
    let (tally, row_ids) = blocking(move || {
        let staying: Vec<&str> = snap
            .frames
            .iter()
            .filter(|f| !targets.contains(&f.id))
            .map(|f| f.frame_path.as_str())
            .collect();
        let guard = Guard::build(&outputs, &snap).keeping(&staying);
        let mut tally = Tally::default();
        let mut row_ids = Vec::with_capacity(targets.len());
        for f in snap.frames.iter().filter(|f| targets.contains(&f.id)) {
            if tally.remove(&guard, Path::new(&f.frame_path)) {
                row_ids.push(f.id.clone());
            }
        }
        guard.prune_after(&tally.deleted);
        (tally, row_ids)
    })
    .await?;
    let deleted = db
        .dataset_frames()
        .delete_many(&dataset_id, &row_ids)
        .await?;
    Ok(FramesDeleteSummary {
        deleted,
        deleted_files: tally.deleted.len() as u64,
        freed_bytes: tally.freed_bytes,
        skipped_files: tally.skipped,
    })
}

/// (files, bytes) of the frames' files `guard` would delete, each file once.
fn deletable_totals<'a>(
    guard: &Guard,
    frames: impl Iterator<Item = &'a DatasetFrame>,
) -> (u64, u64) {
    let mut seen = HashSet::new();
    frames
        .filter_map(|f| match guard.check(Path::new(&f.frame_path)) {
            Verdict::Delete { path, bytes } => seen.insert(path).then_some(bytes),
            Verdict::Missing | Verdict::Skip(_) => None,
        })
        .fold((0, 0), |(n, total), bytes| (n + 1, total + bytes))
}

/// The files an export wrote: `NNNN.<ext>` directly in the folder, with their
/// sizes. Anything else in the folder is not the export's to delete.
fn export_files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| is_export_name(&e.file_name().to_string_lossy()))
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.len())))
        .collect();
    files.sort();
    files
}

fn is_export_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    stem.len() >= 4
        && stem.bytes().all(|b| b.is_ascii_digit())
        && !ext.is_empty()
        && ext.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Every file (and link) below `dir`, without following links.
fn walk_files(dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| !e.file_type().is_dir())
        .map(walkdir::DirEntry::into_path)
        .collect()
}

/// (file count, total bytes) below `dir`.
fn dir_totals(dir: &Path) -> (u64, u64) {
    walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| !e.file_type().is_dir())
        .fold((0, 0), |(n, bytes), e| {
            (n + 1, bytes + e.metadata().map(|m| m.len()).unwrap_or(0))
        })
}

/// Remove every empty directory below and including a walkable work folder
/// (deepest first). `remove_dir` refuses a non-empty directory, and links are
/// not directories to walkdir, so nothing with content and nothing behind a
/// link goes. Only called for a work folder the guard found walkable.
fn prune_empty_dirs(root: &Path) {
    let dirs: Vec<walkdir::DirEntry> = walkdir::WalkDir::new(root)
        .follow_links(false)
        .contents_first(true)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_dir())
        .collect();
    for d in dirs {
        let _ = std::fs::remove_dir(d.path());
    }
}

/// Running totals of one deletion pass.
#[derive(Debug, Default)]
struct Tally {
    /// Canonical paths actually deleted, in order.
    deleted: Vec<PathBuf>,
    /// The same paths, for the "already handled" check.
    seen: HashSet<PathBuf>,
    freed_bytes: u64,
    skipped: Vec<SkippedFile>,
    /// Deletions that failed with an I/O error.
    failed: u64,
}

impl Tally {
    fn skip(&mut self, s: SkippedFile) {
        if !self.skipped.contains(&s) {
            self.skipped.push(s);
        }
    }

    /// Delete `raw` if the guard allows it. `false` only when deleting failed
    /// with an I/O error — deleted, missing and skipped are all `true`.
    fn remove(&mut self, guard: &Guard, raw: &Path) -> bool {
        match guard.check(raw) {
            Verdict::Missing => true,
            Verdict::Skip(s) => {
                self.skip(s);
                true
            }
            Verdict::Delete { path, bytes } => {
                if self.seen.contains(&path) {
                    return true;
                }
                match std::fs::remove_file(&path) {
                    Ok(()) => {
                        self.seen.insert(path.clone());
                        self.deleted.push(path);
                        self.freed_bytes += bytes;
                        true
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "could not delete dataset file"
                        );
                        self.failed += 1;
                        self.skip(skipped(raw, &format!("{SKIP_ERROR}: {e}")));
                        false
                    }
                }
            }
        }
    }
}

/// A path for people: canonical paths on Windows carry a `\\?\` prefix.
fn display(p: &Path) -> String {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => rest.to_string(),
        _ => s.into_owned(),
    }
}

#[cfg(test)]
mod dedup_tests;
#[cfg(test)]
mod perf_probe;
#[cfg(test)]
mod safety_tests;
#[cfg(test)]
mod tests;
