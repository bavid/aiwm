//! The cleanup apply (Plan 13): delete what the user selected from a
//! [`super::scan`] report, group by group, through the existing gates — or,
//! as a dry run, list exactly what would go. Backs `POST /cleanup/apply`.
//!
//! **Gates.** Nothing here decides on its own whether something may go:
//! - generated media: only a flat regular file in the outputs folder that a
//!   fresh scan, run right here, still offers (the retention rule, the job
//!   rows and the in-place frames re-read — a client-sent path is never
//!   trusted);
//! - discarded frames: `housekeeping::cleanup` (the dataset guard);
//! - unclaimed work folders: the guard's unclaimed rule
//!   (`housekeeping::unclaimed_work_folders`), re-evaluated now;
//! - missing frame rows: only rows whose file is still not found now,
//!   removed through the dataset repo;
//! - finished runs: `check_purge_target` right before the folder or its
//!   disposable files go, the final checkpoint never among them;
//! - caches, logs, backups: regular files strictly inside that root
//!   (canonical), links never followed, never the newest log, never an
//!   active download's staging folder.
//!
//! **Busy first.** A selected dataset that is busy
//! (`housekeeping::busy_reason`), a selected run that has not finished, a
//! selected staging folder whose download is still going: the whole request
//! is refused before anything is deleted. Within a group the work is
//! best-effort per entry; whatever is left alone is reported with a reason.
//!
//! A dry run walks and judges exactly like a real one and deletes nothing;
//! only a real run writes the cleanup log (one row per entry).

mod caches;
mod datasets;
mod media;
mod runs;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::capability::dataset::housekeeping::{
    self, SkippedFile, SKIP_ERROR, SKIP_NOT_A_FILE, SKIP_OUTSIDE,
};
use crate::db::{Database, NewCleanupLogEntry};
use crate::{CoreError, Result};

use super::is_reparse_point;
use super::scan::{self, capped, display, ScanContext, GROUP_KEYS};

/// Skip reasons this module adds to housekeeping's.
/// The id is not (or no longer) something the scan offers.
pub const SKIP_NOT_OFFERED: &str = "not_offered";
/// The file was gone by the time it was to be deleted.
pub const SKIP_MISSING: &str = "missing";
/// A link or junction: never followed, left alone.
pub const SKIP_LINK: &str = "link_not_followed";

/// `POST /cleanup/apply`'s body.
#[derive(Debug, Clone, Deserialize)]
pub struct ApplyRequest {
    pub selections: Vec<Selection>,
    /// `true` (the default when absent) lists exactly what would go and
    /// deletes nothing.
    #[serde(default = "default_true")]
    pub dry_run: bool,
}

fn default_true() -> bool {
    true
}

/// The entries of one group to apply. `entry_ids` are the scan's ids; an
/// id the scan does not (or no longer) offer is skipped with a reason.
#[derive(Debug, Clone, Deserialize)]
pub struct Selection {
    pub group: String,
    pub entry_ids: Vec<String>,
}

/// What an apply did — or, in a dry run, would do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyResult {
    pub dry_run: bool,
    pub deleted_files: u64,
    pub freed_bytes: u64,
    pub removed_rows: u64,
    /// Every skip of every entry, with its reason.
    pub skipped: Vec<SkippedFile>,
    pub entries: Vec<EntryResult>,
}

/// One entry's outcome. In a dry run `files`/`bytes`/`rows` are what would
/// go and `paths` the exact files; after a real run, what went.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryResult {
    pub group: String,
    pub id: String,
    pub label: String,
    pub files: u64,
    pub bytes: u64,
    pub rows: u64,
    pub skipped: Vec<SkippedFile>,
    pub paths: Vec<String>,
}

/// `POST /cleanup/apply`.
pub async fn apply(db: &Database, ctx: &ScanContext, req: ApplyRequest) -> Result<ApplyResult> {
    let groups = merge(&req.selections)?;
    refuse_if_busy(db, &groups).await?;
    let dry_run = req.dry_run;
    let fresh = if groups.iter().any(|(g, _)| g.starts_with("media_")) {
        Some(scan::report(db, ctx).await?)
    } else {
        None
    };
    let mut entries = Vec::new();
    for (group, ids) in &groups {
        let done = match group.as_str() {
            "media_retention" | "media_orphans" => {
                let offered = fresh
                    .as_ref()
                    .and_then(|r| r.groups.iter().find(|g| &g.key == group))
                    .map(|g| g.entries.iter().map(|e| e.id.clone()).collect())
                    .unwrap_or_default();
                media::apply(ctx, group, offered, ids, dry_run).await?
            }
            "discarded_frames" => datasets::discarded(db, ctx, ids, dry_run).await?,
            "unclaimed_dataset_folders" => datasets::unclaimed(db, ctx, ids, dry_run).await?,
            "missing_frame_rows" => datasets::missing_rows(db, ids, dry_run).await?,
            "finished_runs" => runs::apply(db, ctx, ids, dry_run).await?,
            "caches" => caches::caches(db, ctx, ids, dry_run).await?,
            "old_logs" => caches::old_logs(ctx, ids, dry_run).await?,
            _ => caches::backups(ctx, ids, dry_run).await?,
        };
        entries.extend(done);
    }
    if !dry_run {
        log(db, &entries).await?;
    }
    let result = summarise(dry_run, entries);
    tracing::info!(
        dry_run,
        entries = result.entries.len(),
        files = result.deleted_files,
        bytes = result.freed_bytes,
        rows = result.removed_rows,
        skipped = result.skipped.len(),
        "cleanup apply"
    );
    Ok(result)
}

/// The selections by group, in the scan's group order, ids deduplicated in
/// request order. An unknown group is a client error.
fn merge(selections: &[Selection]) -> Result<Vec<(String, Vec<String>)>> {
    let mut by_group: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for s in selections {
        if !GROUP_KEYS.contains(&s.group.as_str()) {
            return Err(CoreError::Config(format!(
                "unknown cleanup group \"{}\"",
                s.group
            )));
        }
        let ids = by_group.entry(s.group.clone()).or_default();
        for id in &s.entry_ids {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
    }
    // The scan's order is what the page shows and what the log records.
    Ok(GROUP_KEYS
        .iter()
        .filter_map(|k| by_group.remove(*k).map(|ids| (k.to_string(), ids)))
        .collect())
}

/// Refuse the whole request while any selected dataset is busy (the one
/// busy rule, [`housekeeping::busy_reason`]), any selected run has not
/// finished, or any selected staging folder's download is still going.
async fn refuse_if_busy(db: &Database, groups: &[(String, Vec<String>)]) -> Result<()> {
    let ids = |group: &str| {
        groups
            .iter()
            .find(|(g, _)| g == group)
            .map(|(_, ids)| ids.as_slice())
            .unwrap_or(&[])
    };
    for id in ids("discarded_frames")
        .iter()
        .chain(ids("missing_frame_rows"))
    {
        if let Some(dataset) = db.datasets().get(id).await? {
            if let Some(why) = housekeeping::busy_reason(db, &dataset).await? {
                return Err(CoreError::Config(why));
            }
        }
    }
    for id in ids("finished_runs") {
        if let Some(run) = db.training_runs().get(id).await? {
            if !run.state.is_terminal() {
                return Err(CoreError::Config(format!(
                    "training run \"{}\" is {} \u{2014} wait for it to finish or cancel it first",
                    run.name,
                    run.state.as_str()
                )));
            }
        }
    }
    for name in ids("caches")
        .iter()
        .filter_map(|id| id.strip_prefix(caches::STAGING_PREFIX))
    {
        if let Some(d) = db.downloads().get(name).await? {
            if !d.state.is_terminal() {
                return Err(CoreError::Config(format!(
                    "the download of {} is {} \u{2014} wait for it to finish or cancel it first",
                    d.filename,
                    d.state.as_str()
                )));
            }
        }
    }
    Ok(())
}

/// One cleanup log row per entry.
async fn log(db: &Database, entries: &[EntryResult]) -> Result<()> {
    for e in entries {
        db.cleanup_log()
            .insert(NewCleanupLogEntry {
                group_key: e.group.clone(),
                entry_id: e.id.clone(),
                entry_label: e.label.clone(),
                deleted_files: e.files,
                freed_bytes: e.bytes,
                removed_rows: e.rows,
                skipped_count: e.skipped.len() as u64,
                detail: serde_json::json!({
                    "skipped": e.skipped,
                    "paths": capped(e.paths.iter().cloned()),
                }),
            })
            .await?;
    }
    Ok(())
}

fn summarise(dry_run: bool, entries: Vec<EntryResult>) -> ApplyResult {
    ApplyResult {
        dry_run,
        deleted_files: entries.iter().map(|e| e.files).sum(),
        freed_bytes: entries.iter().map(|e| e.bytes).sum(),
        removed_rows: entries.iter().map(|e| e.rows).sum(),
        skipped: entries.iter().flat_map(|e| e.skipped.clone()).collect(),
        entries,
    }
}

// --- shared by the per-group modules ------------------------------------------

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CoreError::Config(format!("cleanup apply did not finish: {e}")))
}

/// An id the scan does not offer: nothing happens, one skip says so.
fn not_offered(group: &str, id: &str) -> EntryResult {
    refused(group, id, id, Path::new(id), SKIP_NOT_OFFERED)
}

/// An entry left alone as a whole, with the reason.
fn refused(group: &str, id: &str, label: &str, path: &Path, why: &str) -> EntryResult {
    EntryResult {
        group: group.to_string(),
        id: id.to_string(),
        label: label.to_string(),
        skipped: vec![SkippedFile {
            path: display(path),
            reason: why.to_string(),
        }],
        ..EntryResult::default()
    }
}

fn entry(group: &str, id: &str, label: &str, tally: Tally, rows: u64) -> EntryResult {
    EntryResult {
        group: group.to_string(),
        id: id.to_string(),
        label: label.to_string(),
        files: tally.files,
        bytes: tally.bytes,
        rows,
        skipped: tally.skipped,
        paths: tally.paths,
    }
}

/// Running totals of one entry's file work.
#[derive(Debug, Default)]
struct Tally {
    files: u64,
    bytes: u64,
    skipped: Vec<SkippedFile>,
    paths: Vec<String>,
}

impl Tally {
    fn skip(&mut self, path: &Path, reason: &str) {
        self.skipped.push(SkippedFile {
            path: display(path),
            reason: reason.to_string(),
        });
    }

    /// Delete `path` — or, in a dry run, only record it — when it is a
    /// regular file strictly inside the canonical `root`, never a link.
    fn remove_file(&mut self, root: &Path, path: &Path, dry_run: bool) {
        self.remove_checked(root, path, dry_run, false);
    }

    /// [`Self::remove_file`], and the file must sit directly in `root`.
    fn remove_flat_file(&mut self, root: &Path, path: &Path, dry_run: bool) {
        self.remove_checked(root, path, dry_run, true);
    }

    fn remove_checked(&mut self, root: &Path, path: &Path, dry_run: bool, flat: bool) {
        let (canonical, bytes) = match deletable_file(root, path) {
            Ok(found) => found,
            // A flat id names a file directly; one that names nothing was
            // never offered.
            Err(reason) if flat && reason == SKIP_MISSING => {
                return self.skip(path, SKIP_NOT_OFFERED)
            }
            Err(reason) => return self.skip(path, &reason),
        };
        if flat && canonical.parent() != Some(root) {
            return self.skip(path, SKIP_NOT_OFFERED);
        }
        if !dry_run {
            if let Err(e) = std::fs::remove_file(&canonical) {
                let reason = if e.kind() == std::io::ErrorKind::NotFound {
                    SKIP_MISSING.to_string()
                } else {
                    tracing::warn!(path = %canonical.display(), error = %e, "cleanup could not delete a file");
                    format!("{SKIP_ERROR}: {e}")
                };
                return self.skip(path, &reason);
            }
        }
        self.files += 1;
        self.bytes += bytes;
        self.paths.push(display(&canonical));
    }
}

/// `(canonical path, size)` of `path` when it may go: a regular file — not
/// a link or junction, which is never followed — strictly inside the
/// canonical `root`. Else the skip reason.
fn deletable_file(root: &Path, path: &Path) -> std::result::Result<(PathBuf, u64), String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(SKIP_MISSING.into()),
        Err(e) => return Err(format!("{SKIP_ERROR}: {e}")),
    };
    if meta.is_symlink() || is_reparse_point(&meta) {
        return Err(SKIP_LINK.into());
    }
    if !meta.is_file() {
        return Err(SKIP_NOT_A_FILE.into());
    }
    let canonical = std::fs::canonicalize(path).map_err(|e| format!("{SKIP_ERROR}: {e}"))?;
    if !strictly_inside(&canonical, root) {
        return Err(SKIP_OUTSIDE.into());
    }
    Ok((canonical, meta.len()))
}

fn strictly_inside(inner: &Path, outer: &Path) -> bool {
    inner.starts_with(outer) && inner != outer
}

/// The canonical form of an existing folder, or `None`.
fn canonical_dir(dir: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(dir).ok().filter(|p| p.is_dir())
}

/// Everything below `dir`, one level of metadata each, links and junctions
/// never followed.
#[derive(Debug, Default)]
struct Walk {
    files: Vec<PathBuf>,
    links: Vec<PathBuf>,
    /// Folders below `dir` (not `dir` itself), deepest first.
    dirs: Vec<PathBuf>,
}

fn walk(dir: &Path) -> Walk {
    let mut out = Walk::default();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else {
                continue;
            };
            let path = e.path();
            if meta.is_symlink() || is_reparse_point(&meta) {
                out.links.push(path);
            } else if meta.is_dir() {
                out.dirs.push(path.clone());
                stack.push(path);
            } else if meta.is_file() {
                out.files.push(path);
            }
        }
    }
    out.files.sort();
    out.links.sort();
    out.dirs
        .sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    out
}

/// Delete every regular file below the canonical `dir` and then, in a real
/// run, the folder itself with `remove_dir_all` — which removes a leftover
/// link or junction without walking into it. For a folder that goes as a
/// whole (an unclaimed work folder, a staging folder, a finished run).
fn remove_tree(tally: &mut Tally, dir: &Path, dry_run: bool) {
    let found = walk(dir);
    for file in &found.files {
        tally.remove_file(dir, file, dry_run);
    }
    if dry_run {
        return;
    }
    if let Err(e) = std::fs::remove_dir_all(dir) {
        tracing::warn!(dir = %dir.display(), error = %e, "cleanup could not remove a folder");
        tally.skip(dir, &format!("{SKIP_ERROR}: {e}"));
    }
}

/// Delete every regular file below the canonical `root` and prune the
/// folders that became empty, deepest first — the root itself stays, and a
/// link or junction inside is reported and left alone, never followed. For
/// a root that is emptied, not removed (the cache, the pending import).
fn empty_folder(tally: &mut Tally, root: &Path, dry_run: bool) {
    let found = walk(root);
    for file in &found.files {
        tally.remove_file(root, file, dry_run);
    }
    for link in &found.links {
        tally.skip(link, SKIP_LINK);
    }
    if dry_run {
        return;
    }
    for dir in &found.dirs {
        // `remove_dir` refuses a non-empty folder: whatever stayed keeps its
        // parents.
        let _ = std::fs::remove_dir(dir);
    }
}

#[cfg(test)]
mod full_tests;
#[cfg(test)]
mod tests;
