//! Dataset housekeeping: how much disk a dataset uses, deleting it or some of
//! its frames *with their files*, cleaning up discarded frames, and finding
//! near-duplicates across the whole dataset (design spec
//! `2026-09-18-dataset-curation-design.md`).
//!
//! **Path guard.** Every file this module deletes first goes through one
//! [`Guard`]: the path is canonicalised and must lie strictly inside one of
//! the dataset's app-owned roots — its work folder
//! `<outputs>/datasets/<prep_job_id>/`, or its export folder when that lies
//! inside the outputs folder — and must not be the source file of any of the
//! dataset's frames. A `frame_path` pointing anywhere else (a crafted row,
//! an image referenced in place) is reported as skipped, never deleted.
//! Canonicalising first means `..` segments and symlinks cannot walk out of a
//! root.
//!
//! **Training runs.** Deleting a dataset, some of its frames or its
//! discarded frames is refused while a training run of that dataset is not
//! finished (see [`TrainingRunRepo::list_active_for_dataset`]).
//!
//! **Blocking work.** All file-system walking, deleting and image decoding
//! runs on `spawn_blocking` so a large dataset never stalls the async
//! runtime that also serves the API.
//!
//! [`TrainingRunRepo::list_active_for_dataset`]: crate::db::TrainingRunRepo::list_active_for_dataset

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::db::{Database, Dataset, DatasetFrame, DatasetMode};
use crate::{CoreError, Result};

use super::{dataset_err, filter};

mod dedup_plan;
use dedup_plan::plan_dedup;

/// Default Hamming distance for the dataset-wide dedup — the same one the
/// per-source duplicate filter uses.
pub const DEFAULT_DEDUP_THRESHOLD: u32 = filter::DEFAULT_PHASH_MAX_DISTANCE;
/// Upper bound for a caller-chosen dedup threshold. Beyond this, frames that
/// merely share a composition start to count as "the same picture".
pub const MAX_DEDUP_THRESHOLD: u32 = 16;

/// Skip reasons reported in [`SkippedFile::reason`].
pub const SKIP_OUTSIDE: &str = "outside_app_folders";
pub const SKIP_SOURCE: &str = "source_file";
pub const SKIP_IN_USE: &str = "in_use";
pub const SKIP_NOT_A_FILE: &str = "not_a_file";
pub const SKIP_ERROR: &str = "error";

/// A file a deletion left alone, and why (one of the `SKIP_*` constants).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

/// `GET /datasets/{id}/usage` — sizes measured by walking the folders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatasetUsage {
    /// The app-owned work folder, `None` when it does not exist (any more).
    pub work_dir: Option<String>,
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
    /// Frame rows that went with the dataset.
    pub frames: u64,
    pub deleted_files: u64,
    pub freed_bytes: u64,
    pub skipped_files: Vec<SkippedFile>,
    /// A user-chosen export folder outside the outputs folder, left as is.
    pub export_dir_kept: Option<String>,
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

// --- public operations ------------------------------------------------------

/// How much disk `dataset_id` uses. `None` for an unknown dataset.
pub async fn usage(
    db: &Database,
    outputs_dir: &Path,
    dataset_id: &str,
) -> Result<Option<DatasetUsage>> {
    let Some(dataset) = db.datasets().get(dataset_id).await? else {
        return Ok(None);
    };
    let frames = db.dataset_frames().list_for_dataset(dataset_id).await?;
    let outputs_dir = outputs_dir.to_path_buf();
    blocking(move || {
        let guard = Guard::build(&outputs_dir, &dataset, &frames);
        let (work_files, work_bytes) = guard.work_dir.as_deref().map(dir_totals).unwrap_or((0, 0));
        let export_bytes = guard
            .export_dir
            .as_deref()
            .map(|d| export_files(d).iter().map(|(_, len)| len).sum())
            .unwrap_or(0);
        let (discarded, remaining): (Vec<&DatasetFrame>, Vec<&DatasetFrame>) =
            frames.iter().partition(|f| is_discarded(f));
        let in_use: HashSet<&str> = remaining.iter().map(|f| f.frame_path.as_str()).collect();
        let mut seen = HashSet::new();
        let discarded_bytes = discarded
            .iter()
            .filter(|f| !in_use.contains(f.frame_path.as_str()))
            .filter_map(|f| match guard.check(Path::new(&f.frame_path)) {
                Verdict::Delete { path, bytes } => seen.insert(path).then_some(bytes),
                Verdict::Missing | Verdict::Skip(_) => None,
            })
            .sum();
        DatasetUsage {
            work_dir: guard.work_dir.as_deref().map(display),
            work_bytes,
            work_files,
            export_dir: dataset.export_dir.clone(),
            export_bytes,
            export_app_owned: guard.export_dir.is_some(),
            discarded_frames: discarded.len() as u64,
            discarded_bytes,
        }
    })
    .await
    .map(Some)
}

/// Delete the dataset: its work folder, its app-owned export's numbered
/// files, then its rows (frames and concepts cascade). Source files and
/// anything outside the app-owned roots stay. `None` for an unknown dataset.
pub async fn delete_dataset_with_files(
    db: &Database,
    outputs_dir: &Path,
    dataset_id: &str,
) -> Result<Option<DatasetDeleteSummary>> {
    let Some(dataset) = db.datasets().get(dataset_id).await? else {
        return Ok(None);
    };
    refuse_if_training(db, &dataset).await?;
    let frames = db.dataset_frames().list_for_dataset(dataset_id).await?;
    let frame_count = frames.len() as u64;
    let outputs = outputs_dir.to_path_buf();
    let ds = dataset.clone();
    let (tally, export_kept) = blocking(move || {
        let guard = Guard::build(&outputs, &ds, &frames);
        let mut tally = Tally::default();
        let none = HashSet::new();
        if let Some(work) = guard.work_dir.as_deref() {
            for file in walk_files(work) {
                tally.remove(&guard, &file, &none);
            }
        }
        if let Some(export) = guard.export_dir.as_deref() {
            for (file, _) in export_files(export) {
                tally.remove(&guard, &file, &none);
            }
            // Only removed when nothing else was in it.
            let _ = std::fs::remove_dir(export);
        }
        // Frames outside the work folder: deleted if app-owned, else reported.
        for f in &frames {
            if !f.frame_path.is_empty() {
                tally.remove(&guard, Path::new(&f.frame_path), &none);
            }
        }
        if let Some(work) = guard.work_dir.as_deref() {
            prune_empty_dirs(work, true);
        }
        let export_kept = match (&ds.export_dir, &guard.export_dir) {
            (Some(dir), None) => Some(dir.clone()),
            _ => None,
        };
        (tally, export_kept)
    })
    .await?;
    db.datasets().delete(&dataset.id).await?;
    tracing::info!(
        dataset = %dataset.id,
        frames = frame_count,
        files = tally.deleted_files,
        bytes = tally.freed_bytes,
        skipped = tally.skipped.len(),
        "deleted dataset with its files"
    );
    Ok(Some(DatasetDeleteSummary {
        frames: frame_count,
        deleted_files: tally.deleted_files,
        freed_bytes: tally.freed_bytes,
        skipped_files: tally.skipped,
        export_dir_kept: export_kept,
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
    let Some(dataset) = db.datasets().get(dataset_id).await? else {
        return Ok(None);
    };
    refuse_if_training(db, &dataset).await?;
    let wanted: HashSet<&str> = frame_ids.iter().map(String::as_str).collect();
    let frames = db.dataset_frames().list_for_dataset(dataset_id).await?;
    let target_ids: Vec<String> = frames
        .iter()
        .filter(|f| wanted.contains(f.id.as_str()))
        .map(|f| f.id.clone())
        .collect();
    let s = remove_frames(db, outputs_dir, &dataset, frames, target_ids).await?;
    tracing::info!(
        dataset = %dataset.id,
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
    let Some(dataset) = db.datasets().get(dataset_id).await? else {
        return Ok(None);
    };
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
    refuse_if_training(db, &dataset).await?;
    let frames = db.dataset_frames().list_for_dataset(dataset_id).await?;
    let target_ids: Vec<String> = frames
        .iter()
        .filter(|f| is_discarded(f))
        .map(|f| f.id.clone())
        .collect();
    let s = remove_frames(db, outputs_dir, &dataset, frames, target_ids).await?;
    tracing::info!(
        dataset = %dataset.id,
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

/// Find near-duplicates across every kept frame of the dataset — all
/// sources, not just neighbours — and mark all but the sharpest of each
/// group `duplicate_global`. Only marks: the files stay until a cleanup, and
/// moving a frame back to "Keep" clears the mark.
///
/// Greedy and deterministic: frames are visited sharpest first (ties: the
/// earlier frame), each joins the first already-kept frame within
/// `threshold` or becomes a keeper itself. Unlike single-linkage clustering
/// this cannot chain two different pictures together through a series of
/// small steps, and the keeper of every group is by construction its
/// sharpest member.
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

async fn refuse_if_training(db: &Database, dataset: &Dataset) -> Result<()> {
    let runs = db
        .training_runs()
        .list_active_for_dataset(&dataset.id)
        .await?;
    match runs.first() {
        Some(run) => Err(CoreError::Config(format!(
            "dataset \"{}\" is in use by training run \"{}\" ({}) \u{2014} wait for it to \
             finish or cancel it first",
            dataset.name,
            run.name,
            run.state.as_str()
        ))),
        None => Ok(()),
    }
}

/// Files of `frames` whose id is in `target_ids`, then their rows.
async fn remove_frames(
    db: &Database,
    outputs_dir: &Path,
    dataset: &Dataset,
    frames: Vec<DatasetFrame>,
    target_ids: Vec<String>,
) -> Result<FramesDeleteSummary> {
    let outputs = outputs_dir.to_path_buf();
    let ds = dataset.clone();
    let (tally, row_ids) = blocking(move || {
        let guard = Guard::build(&outputs, &ds, &frames);
        let targets: HashSet<&str> = target_ids.iter().map(String::as_str).collect();
        // A file another remaining frame still shows is not deleted.
        let in_use: HashSet<PathBuf> = frames
            .iter()
            .filter(|f| !targets.contains(f.id.as_str()) && !f.frame_path.is_empty())
            .map(|f| PathBuf::from(&f.frame_path))
            .collect();
        let mut tally = Tally::default();
        let mut row_ids = Vec::with_capacity(target_ids.len());
        for f in frames.iter().filter(|f| targets.contains(f.id.as_str())) {
            let removable =
                f.frame_path.is_empty() || tally.remove(&guard, Path::new(&f.frame_path), &in_use);
            if removable {
                row_ids.push(f.id.clone());
            }
        }
        if let Some(work) = guard.work_dir.as_deref() {
            prune_empty_dirs(work, false);
        }
        (tally, row_ids)
    })
    .await?;
    let deleted = db
        .dataset_frames()
        .delete_many(&dataset.id, &row_ids)
        .await?;
    Ok(FramesDeleteSummary {
        deleted,
        deleted_files: tally.deleted_files,
        freed_bytes: tally.freed_bytes,
        skipped_files: tally.skipped,
    })
}

/// The one place that decides whether a file may be deleted.
#[derive(Debug, Clone)]
struct Guard {
    /// Canonical `<outputs>/datasets/<prep_job_id>`, if it exists.
    work_dir: Option<PathBuf>,
    /// Canonical export folder, only when it lies inside the outputs folder.
    export_dir: Option<PathBuf>,
    /// Canonical source files of the dataset's frames — never deleted.
    sources: HashSet<PathBuf>,
}

enum Verdict {
    Delete { path: PathBuf, bytes: u64 },
    Missing,
    Skip(SkippedFile),
}

impl Guard {
    fn build(outputs_dir: &Path, dataset: &Dataset, frames: &[DatasetFrame]) -> Self {
        let outputs = std::fs::canonicalize(outputs_dir).ok();
        let datasets_root = outputs
            .as_ref()
            .and_then(|o| std::fs::canonicalize(o.join("datasets")).ok());
        let work_dir = datasets_root
            .as_deref()
            .and_then(|root| work_dir_of(root, dataset, frames));
        let export_dir = match (&outputs, &dataset.export_dir) {
            (Some(outputs), Some(export)) => {
                app_owned_export(outputs, datasets_root.as_deref(), Path::new(export))
            }
            _ => None,
        };
        let source_strings: HashSet<&str> = frames.iter().map(|f| f.source_path.as_str()).collect();
        let sources = source_strings
            .into_iter()
            .filter(|s| !s.is_empty())
            .filter_map(|s| std::fs::canonicalize(s).ok())
            .collect();
        Self {
            work_dir,
            export_dir,
            sources,
        }
    }

    fn check(&self, raw: &Path) -> Verdict {
        if raw.as_os_str().is_empty() {
            return Verdict::Missing;
        }
        // A relative path would be resolved against the process's working
        // directory, which says nothing about where the frame really is.
        if !raw.is_absolute() {
            return Verdict::Skip(skipped(raw, SKIP_OUTSIDE));
        }
        let canonical = match std::fs::canonicalize(raw) {
            Ok(p) => p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Verdict::Missing,
            Err(e) => return Verdict::Skip(skipped(raw, &format!("{SKIP_ERROR}: {e}"))),
        };
        if self.sources.contains(&canonical) {
            return Verdict::Skip(skipped(raw, SKIP_SOURCE));
        }
        let inside = [&self.work_dir, &self.export_dir]
            .into_iter()
            .flatten()
            .any(|root| canonical.starts_with(root) && canonical != *root);
        if !inside {
            return Verdict::Skip(skipped(raw, SKIP_OUTSIDE));
        }
        match std::fs::metadata(&canonical) {
            Ok(m) if m.is_file() => Verdict::Delete {
                path: canonical,
                bytes: m.len(),
            },
            Ok(_) => Verdict::Skip(skipped(raw, SKIP_NOT_A_FILE)),
            Err(e) => Verdict::Skip(skipped(raw, &format!("{SKIP_ERROR}: {e}"))),
        }
    }
}

fn skipped(path: &Path, reason: &str) -> SkippedFile {
    SkippedFile {
        path: path.to_string_lossy().into_owned(),
        reason: reason.to_string(),
    }
}

/// The dataset's work folder under the canonical `datasets_root`: named after
/// the prep job, or — once that job is deleted and `prep_job_id` is `NULL` —
/// the one folder under `datasets_root` all its app-extracted frames share.
fn work_dir_of(
    datasets_root: &Path,
    dataset: &Dataset,
    frames: &[DatasetFrame],
) -> Option<PathBuf> {
    let name = match dataset.prep_job_id.as_deref() {
        Some(id) => single_component(id)?.to_owned(),
        None => inferred_work_name(datasets_root, frames)?,
    };
    let dir = std::fs::canonicalize(datasets_root.join(name)).ok()?;
    (dir.is_dir() && dir.starts_with(datasets_root) && dir != datasets_root).then_some(dir)
}

/// `name` when it is exactly one ordinary path component (no separator, no
/// `..`, no drive) — a job id always is.
fn single_component(name: &str) -> Option<&str> {
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(_)), None) => Some(name),
        _ => None,
    }
}

fn inferred_work_name(datasets_root: &Path, frames: &[DatasetFrame]) -> Option<String> {
    let names: HashSet<String> = frames
        .iter()
        .filter(|f| Path::new(&f.frame_path).is_absolute())
        .filter_map(|f| std::fs::canonicalize(&f.frame_path).ok())
        .filter_map(|p| {
            let rest = p.strip_prefix(datasets_root).ok()?;
            match rest.components().next()? {
                Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
                _ => None,
            }
        })
        .collect();
    (names.len() == 1)
        .then(|| names.into_iter().next())
        .flatten()
}

/// The export folder when the app may delete its numbered files: strictly
/// inside the outputs folder and not the `datasets` root (or above it).
fn app_owned_export(
    outputs: &Path,
    datasets_root: Option<&Path>,
    export: &Path,
) -> Option<PathBuf> {
    if !export.is_absolute() {
        return None;
    }
    let export = std::fs::canonicalize(export).ok()?;
    let inside = export.starts_with(outputs) && export != outputs;
    let covers_work = datasets_root.is_some_and(|root| root.starts_with(&export));
    (inside && !covers_work && export.is_dir()).then_some(export)
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

/// Every file (and file symlink) below `dir`, without following links.
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

/// Remove directories below `root` that are empty (deepest first);
/// `include_root` also removes `root` itself when it ends up empty.
/// `remove_dir` refuses a non-empty directory, so nothing with content goes.
fn prune_empty_dirs(root: &Path, include_root: bool) {
    let dirs: Vec<walkdir::DirEntry> = walkdir::WalkDir::new(root)
        .follow_links(false)
        .contents_first(true)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_dir())
        .collect();
    for d in dirs {
        if d.depth() == 0 && !include_root {
            continue;
        }
        let _ = std::fs::remove_dir(d.path());
    }
}

/// Running totals of one deletion pass.
#[derive(Debug, Default)]
struct Tally {
    deleted_files: u64,
    freed_bytes: u64,
    skipped: Vec<SkippedFile>,
    /// Canonical paths already handled, so a file two rows share (or the
    /// walk and a row both reach) is counted once.
    seen: HashSet<PathBuf>,
}

impl Tally {
    /// Delete `raw` if the guard allows it and no remaining frame (`in_use`,
    /// raw paths) still shows it. `false` only when deleting failed with an
    /// I/O error — everything else (deleted, missing, skipped) is `true`.
    fn remove(&mut self, guard: &Guard, raw: &Path, in_use: &HashSet<PathBuf>) -> bool {
        if in_use.contains(raw) {
            self.skipped.push(skipped(raw, SKIP_IN_USE));
            return true;
        }
        match guard.check(raw) {
            Verdict::Missing => true,
            Verdict::Skip(s) => {
                if !self.skipped.contains(&s) {
                    self.skipped.push(s);
                }
                true
            }
            Verdict::Delete { path, bytes } => {
                if !self.seen.insert(path.clone()) {
                    return true;
                }
                match std::fs::remove_file(&path) {
                    Ok(()) => {
                        self.deleted_files += 1;
                        self.freed_bytes += bytes;
                        true
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "could not delete dataset file");
                        self.skipped
                            .push(skipped(raw, &format!("{SKIP_ERROR}: {e}")));
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
mod tests;
