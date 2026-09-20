//! The cleanup scan (Plan 13): what the app itself generated and could be
//! removed, grouped by kind with counts and sizes, plus a list of what was
//! deliberately *not* offered and why. Backs `GET /cleanup/scan`. Only
//! reports — deleting is a separate, confirmed step that goes through the
//! existing gates (the dataset housekeeping guard, the training purge
//! check, the output retention rule).
//!
//! **Never listed:** the model store and anything in it, runtimes, voice
//! identities, dataset source folders and files, the app's roots themselves,
//! and anything a non-terminal job, training run or download still needs.
//! Where a user might expect an entry, a [`ProtectedNote`] says why there is
//! none.
//!
//! Database reads are async; every folder walk runs in `spawn_blocking`,
//! never follows a link or junction (the walks are `locations::walk`'s), and
//! treats an unreadable entry as "skipped", never as a failure.

// The selection rules (which files a group offers) are shared with
// `super::apply`, which re-runs them right before deleting.
pub(super) mod caches;
pub(super) mod datasets;
mod media;
pub(super) mod runs;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::capability::dataset::housekeeping::{self, DataRoots, DatasetUsage, UnclaimedFolders};
use crate::capability::dataset::location::same_or_inside;
use crate::db::{Database, Dataset, DatasetFrame, Download, Job, JobFilter, Model, TrainingRun};
use crate::paths::AppPaths;
use crate::{CoreError, Result};

use super::locations::{is_reparse_point, walk};
use super::RetentionPolicy;

/// The whole scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupReport {
    /// RFC 3339, when the scan ran.
    pub scanned_at: String,
    /// Every group, in display order — present even when empty, so the UI
    /// has a stable set of keys.
    pub groups: Vec<CleanupGroup>,
    /// What was not offered, and why.
    pub protected: Vec<ProtectedNote>,
}

/// One kind of removable content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupGroup {
    /// One of [`GROUP_KEYS`].
    pub key: String,
    pub label: String,
    pub entries: Vec<CleanupEntry>,
    pub total_files: u64,
    pub total_bytes: u64,
}

/// One selectable item of a group: a file, a dataset, a run, a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupEntry {
    /// Stable within its group (a file name, a dataset or run id, a folder
    /// name); what a later apply names.
    pub id: String,
    pub label: String,
    /// Files that would be deleted.
    pub files: u64,
    pub bytes: u64,
    /// Database rows that would be removed (frame rows), else 0.
    pub rows: u64,
    /// A few lines for the expanded entry (paths, dates, names) — capped.
    pub detail: Vec<String>,
}

/// Something a user might expect to see offered, and why it is not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedNote {
    pub what: String,
    pub reason: String,
}

/// Group keys in display order.
pub const GROUP_KEYS: [&str; 9] = [
    "media_retention",
    "media_orphans",
    "discarded_frames",
    "unclaimed_dataset_folders",
    "missing_frame_rows",
    "finished_runs",
    "caches",
    "old_logs",
    "db_backups",
];

fn group_label(key: &str) -> &'static str {
    match key {
        "media_retention" => "Generated media beyond the retention rule",
        "media_orphans" => "Generated media without a job",
        "discarded_frames" => "Discarded dataset frames",
        "unclaimed_dataset_folders" => "Dataset work folders without a dataset",
        "missing_frame_rows" => "Frame entries whose files are missing",
        "finished_runs" => "Finished training runs",
        "caches" => "Caches and leftovers",
        "old_logs" => "Logs older than 30 days",
        _ => "Database backups",
    }
}

/// Most lines one entry's `detail` carries; beyond that, "… and N more".
pub(super) const DETAIL_CAP: usize = 20;

/// The folders and rules the scan works with.
#[derive(Debug, Clone)]
pub struct ScanContext {
    pub paths: AppPaths,
    /// `Config::store_path` — the model store, never scanned.
    pub store: PathBuf,
    /// The configured output retention; inactive → `media_retention` is
    /// empty.
    pub policy: RetentionPolicy,
}

impl ScanContext {
    pub(super) fn data_roots(&self) -> DataRoots {
        DataRoots {
            outputs: self.paths.outputs_dir(),
            datasets: self.paths.datasets_dir(),
            models: self.store.clone(),
            training: self.paths.training_dir(),
        }
    }

    /// Every folder the app owns, with a label — a folder offered as a whole
    /// must not be or hold any of them.
    pub(super) fn app_roots(&self) -> Vec<(PathBuf, &'static str)> {
        vec![
            (self.store.clone(), "model store"),
            (self.paths.runtimes_dir(), "runtime installs"),
            (self.paths.root().to_path_buf(), "data"),
            (self.paths.outputs_dir(), "outputs"),
            (self.paths.datasets_dir(), "datasets"),
            (self.paths.training_dir(), "training"),
            (self.paths.cache_dir(), "cache"),
            (self.paths.downloads_dir(), "download staging"),
            (self.paths.comfyui_data_dir(), "ComfyUI scratch"),
            (self.paths.voice_identities_dir(), "voice identities"),
            (self.paths.logs_dir(), "logs"),
            (self.paths.exports_dir(), "backups"),
            (self.paths.pending_import_dir(), "pending import"),
        ]
    }

    /// Why `dir` must not be scanned at all: it is, or lies inside, the
    /// model store or the runtime installs.
    pub(super) fn inside_store_or_runtimes(&self, dir: &Path) -> Option<String> {
        if !self.store.as_os_str().is_empty() && same_or_inside(dir, &self.store) {
            return Some(format!(
                "lies inside the model store {}",
                self.store.display()
            ));
        }
        let runtimes = self.paths.runtimes_dir();
        if same_or_inside(dir, &runtimes) {
            return Some(format!(
                "lies inside the runtime installs {}",
                runtimes.display()
            ));
        }
        None
    }

    /// Why `dir` must not be offered as a whole: [`Self::inside_store_or_
    /// runtimes`], or it is / holds one of the app's own folders (the one
    /// named `except`, when `dir` is that folder itself, is not counted).
    pub(super) fn whole_folder_refusal(&self, dir: &Path, except: Option<&Path>) -> Option<String> {
        if let Some(why) = self.inside_store_or_runtimes(dir) {
            return Some(why);
        }
        self.app_roots()
            .into_iter()
            .filter(|(root, _)| except.is_none_or(|e| root != e))
            .find(|(root, _)| same_or_inside(root, dir))
            .map(|(root, label)| format!("is or holds the {label} folder {}", root.display()))
    }
}

/// A dataset with everything the scan needs to know about it.
pub(super) struct DatasetInfo {
    pub(super) dataset: Dataset,
    pub(super) frames: Vec<DatasetFrame>,
    /// `Some(why)` when housekeeping must leave it alone right now.
    pub(super) busy: Option<String>,
    /// Measured only for a dataset that is not busy.
    pub(super) usage: Option<DatasetUsage>,
}

/// Everything read from the database (and measured through housekeeping)
/// before the blocking file work starts.
pub(super) struct Inventory {
    pub(super) datasets: Vec<DatasetInfo>,
    pub(super) unclaimed: UnclaimedFolders,
    pub(super) jobs: Vec<Job>,
    pub(super) runs: Vec<TrainingRun>,
    /// The library rows the finished runs' `result_model_id`s point at.
    pub(super) result_models: HashMap<String, Model>,
    pub(super) downloads: Vec<Download>,
}

impl Inventory {
    /// Ids of jobs that have not finished — their files are never listed.
    pub(super) fn active_job_ids(&self) -> HashSet<&str> {
        self.jobs
            .iter()
            .filter(|j| !j.state.is_terminal())
            .map(|j| j.id.as_str())
            .collect()
    }
}

async fn inventory(db: &Database, ctx: &ScanContext) -> Result<Inventory> {
    let roots = ctx.data_roots();
    let mut datasets = Vec::new();
    for dataset in db.datasets().list().await? {
        let frames = db.dataset_frames().list_for_dataset(&dataset.id).await?;
        let busy = housekeeping::busy_reason(db, &dataset).await?;
        let usage = match busy {
            Some(_) => None,
            None => housekeeping::usage(db, &roots, &dataset.id).await?,
        };
        datasets.push(DatasetInfo {
            dataset,
            frames,
            busy,
            usage,
        });
    }
    let unclaimed = housekeeping::unclaimed_work_folders(db, &roots).await?;
    let jobs = db
        .jobs()
        .list(&JobFilter {
            states: Vec::new(),
            limit: None,
        })
        .await?;
    let runs = db.training_runs().list().await?;
    let mut result_models = HashMap::new();
    for id in runs.iter().filter_map(|r| r.result_model_id.as_deref()) {
        if let Some(model) = db.models().get(id).await? {
            result_models.insert(id.to_string(), model);
        }
    }
    let downloads = db.downloads().list().await?;
    Ok(Inventory {
        datasets,
        unclaimed,
        jobs,
        runs,
        result_models,
        downloads,
    })
}

/// `GET /cleanup/scan`. Database reads first, then every folder walk on one
/// `spawn_blocking` task.
pub async fn report(db: &Database, ctx: &ScanContext) -> Result<CleanupReport> {
    let inv = inventory(db, ctx).await?;
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || build(&ctx, &inv))
        .await
        .map_err(|e| CoreError::Config(format!("cleanup scan did not finish: {e}")))
}

/// The blocking half: every group from the inventory and the disk.
fn build(ctx: &ScanContext, inv: &Inventory) -> CleanupReport {
    let mut protected = fixed_protected(ctx);
    let (media_groups, media_notes) = media::groups(ctx, inv);
    protected.extend(media_notes);
    let (dataset_groups, dataset_notes) = datasets::groups(ctx, inv);
    protected.extend(dataset_notes);
    let (runs_group, run_notes) = runs::group(ctx, inv);
    protected.extend(run_notes);
    let (cache_groups, cache_notes) = caches::groups(ctx, inv);
    protected.extend(cache_notes);

    let groups = media_groups
        .into_iter()
        .chain(dataset_groups)
        .chain(std::iter::once(runs_group))
        .chain(cache_groups)
        .collect();
    CleanupReport {
        scanned_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default(),
        groups,
        protected,
    }
}

/// What is never offered, whatever the disk holds.
fn fixed_protected(ctx: &ScanContext) -> Vec<ProtectedNote> {
    vec![
        ProtectedNote {
            what: format!("Model store {}", ctx.store.display()),
            reason: "models are never cleaned up here — delete a model from the library".into(),
        },
        ProtectedNote {
            what: format!("Runtime installs {}", ctx.paths.runtimes_dir().display()),
            reason: "installed runtimes are managed from Settings, never cleaned up here".into(),
        },
        ProtectedNote {
            what: format!(
                "Voice identities {}",
                ctx.paths.voice_identities_dir().display()
            ),
            reason: "your saved voices' reference clips are user assets".into(),
        },
    ]
}

/// A group with its totals summed from its entries.
pub(super) fn group(key: &str, entries: Vec<CleanupEntry>) -> CleanupGroup {
    let total_files = entries.iter().map(|e| e.files).sum();
    let total_bytes = entries.iter().map(|e| e.bytes).sum();
    CleanupGroup {
        key: key.to_string(),
        label: group_label(key).to_string(),
        entries,
        total_files,
        total_bytes,
    }
}

pub(super) fn note(what: impl Into<String>, reason: impl Into<String>) -> ProtectedNote {
    ProtectedNote {
        what: what.into(),
        reason: reason.into(),
    }
}

/// One regular file a walk found.
#[derive(Debug, Clone)]
pub(super) struct FileInfo {
    pub(super) path: PathBuf,
    pub(super) bytes: u64,
    pub(super) modified: Option<OffsetDateTime>,
}

/// Every regular file below `dir`, never through a link or junction; an
/// unreadable entry is left out. A missing `dir` is empty.
pub(super) fn walk_files(dir: &Path) -> Vec<FileInfo> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return out;
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_symlink() || is_reparse_point(&meta) {
                continue;
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                out.push(FileInfo {
                    path: entry.path(),
                    bytes: meta.len(),
                    modified: meta.modified().ok().map(OffsetDateTime::from),
                });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// `(files, bytes)` below `dir` — `locations::walk`'s totals.
pub(super) fn walk_totals(dir: &Path) -> (u64, u64) {
    let totals = walk(dir);
    (totals.files, totals.bytes)
}

/// `names`, capped at [`DETAIL_CAP`] lines plus "… and N more".
pub(super) fn capped(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let all: Vec<String> = names.into_iter().collect();
    if all.len() <= DETAIL_CAP {
        return all;
    }
    let more = all.len() - DETAIL_CAP;
    all.into_iter()
        .take(DETAIL_CAP)
        .chain(std::iter::once(format!("\u{2026} and {more} more")))
        .collect()
}

/// A path for people: canonical paths on Windows carry a `\\?\` prefix.
pub(super) fn display(p: &Path) -> String {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => rest.to_string(),
        _ => s.into_owned(),
    }
}

/// The file name of `p`, as a string.
pub(super) fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A file name in the form names are compared in (case-folded on Windows).
pub(super) fn name_key(name: &str) -> String {
    if cfg!(windows) {
        name.to_lowercase()
    } else {
        name.to_string()
    }
}

/// `YYYY-MM-DD` of a file's modification time, or `"?"`.
pub(super) fn date_of(modified: Option<OffsetDateTime>) -> String {
    modified
        .and_then(|m| {
            m.format(&time::macros::format_description!("[year]-[month]-[day]"))
                .ok()
        })
        .unwrap_or_else(|| "?".to_string())
}

#[cfg(test)]
mod never_tests;
#[cfg(test)]
pub(crate) mod tests;
