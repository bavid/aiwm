//! Groups `caches`, `old_logs` and `db_backups`: regular files strictly
//! inside the folder the id names (canonical), links never followed. The
//! cache and the pending import are emptied, a staging folder goes as a
//! whole, ComfyUI leftovers and old logs are re-selected with the scan's own
//! rules (fresh job states; never the newest log), a backup is one flat file
//! in the exports folder. That no selected staging folder belongs to a
//! download still going was checked before anything was touched
//! ([`super::refuse_if_busy`]).

use std::collections::HashSet;
use std::path::Path;

use time::OffsetDateTime;

use crate::db::{Database, JobFilter};
use crate::Result;

use super::super::scan::caches::{
    comfyui_leftover_files, old_log_candidates, COMFYUI_SCRATCH, OLD_LOG_DAYS,
};
use super::{
    blocking, canonical_dir, empty_folder, entry, not_offered, refused, remove_tree,
    strictly_inside, EntryResult, ScanContext, Tally, SKIP_OUTSIDE,
};

const CACHES: &str = "caches";
const LOGS: &str = "old_logs";
const BACKUPS: &str = "db_backups";
/// `download-staging:<folder name>` — the scan's id for a staging folder.
pub(super) const STAGING_PREFIX: &str = "download-staging:";
const COMFYUI_PREFIX: &str = "comfyui:";

pub(super) async fn caches(
    db: &Database,
    ctx: &ScanContext,
    ids: &[String],
    dry_run: bool,
) -> Result<Vec<EntryResult>> {
    let active_jobs: HashSet<String> = db
        .jobs()
        .list(&JobFilter {
            states: Vec::new(),
            limit: None,
        })
        .await?
        .into_iter()
        .filter(|j| !j.state.is_terminal())
        .map(|j| j.id)
        .collect();
    let ctx = ctx.clone();
    let ids = ids.to_vec();
    blocking(move || {
        let now = OffsetDateTime::now_utc();
        let active: HashSet<&str> = active_jobs.iter().map(String::as_str).collect();
        ids.iter()
            .map(|id| cache_entry(&ctx, id, &active, now, dry_run))
            .collect()
    })
    .await
}

fn cache_entry(
    ctx: &ScanContext,
    id: &str,
    active: &HashSet<&str>,
    now: OffsetDateTime,
    dry_run: bool,
) -> EntryResult {
    if id == "cache" {
        return emptied_root(
            ctx,
            id,
            "Registry and runtime caches",
            &ctx.paths.cache_dir(),
            dry_run,
        );
    }
    if id == "pending-import" {
        return emptied_root(
            ctx,
            id,
            "Pending import staging",
            &ctx.paths.pending_import_dir(),
            dry_run,
        );
    }
    if let Some(name) = id.strip_prefix(STAGING_PREFIX) {
        return staging(ctx, id, name, dry_run);
    }
    if let Some(sub) = id.strip_prefix(COMFYUI_PREFIX) {
        return comfyui(ctx, id, sub, active, now, dry_run);
    }
    not_offered(CACHES, id)
}

/// A root the app owns outright, emptied but kept — unless it is, or holds,
/// another app folder (the scan's rule).
fn emptied_root(
    ctx: &ScanContext,
    id: &str,
    label: &str,
    dir: &Path,
    dry_run: bool,
) -> EntryResult {
    if let Some(why) = ctx.whole_folder_refusal(dir, Some(dir)) {
        return refused(CACHES, id, label, dir, &why);
    }
    let Some(root) = canonical_dir(dir) else {
        return not_offered(CACHES, id);
    };
    let mut tally = Tally::default();
    empty_folder(&mut tally, &root, dry_run);
    entry(CACHES, id, label, tally, 0)
}

/// One staging folder: a real folder strictly inside the staging root,
/// removed as a whole.
fn staging(ctx: &ScanContext, id: &str, name: &str, dry_run: bool) -> EntryResult {
    let label = format!("Download staging {name}");
    let staging_root = ctx.paths.downloads_dir();
    let dir = staging_root.join(name);
    let (Some(root), Some(folder)) = (canonical_dir(&staging_root), real_dir(&dir)) else {
        return not_offered(CACHES, id);
    };
    if !strictly_inside(&folder, &root) {
        return refused(CACHES, id, &label, &dir, SKIP_OUTSIDE);
    }
    if let Some(why) = ctx.whole_folder_refusal(&folder, None) {
        return refused(CACHES, id, &label, &folder, &why);
    }
    let mut tally = Tally::default();
    remove_tree(&mut tally, &folder, dry_run);
    entry(CACHES, id, &label, tally, 0)
}

/// The canonical form of `dir` when it is a real folder — not a link or
/// junction, never followed.
fn real_dir(dir: &Path) -> Option<std::path::PathBuf> {
    let meta = std::fs::symlink_metadata(dir).ok()?;
    if meta.is_symlink() || super::is_reparse_point(&meta) || !meta.is_dir() {
        return None;
    }
    canonical_dir(dir)
}

/// One ComfyUI scratch folder's leftovers, re-selected with the scan's rule
/// and the jobs' states as they are now.
fn comfyui(
    ctx: &ScanContext,
    id: &str,
    sub: &str,
    active: &HashSet<&str>,
    now: OffsetDateTime,
    dry_run: bool,
) -> EntryResult {
    let label = format!("ComfyUI {sub} leftovers");
    if !COMFYUI_SCRATCH.contains(&sub) {
        return not_offered(CACHES, id);
    }
    let base = ctx.paths.comfyui_data_dir();
    if let Some(why) = ctx.inside_store_or_runtimes(&base) {
        return refused(CACHES, id, &label, &base, &why);
    }
    let dir = base.join(sub);
    let Some(root) = canonical_dir(&dir) else {
        return not_offered(CACHES, id);
    };
    let mut tally = Tally::default();
    for f in comfyui_leftover_files(&dir, active, now) {
        tally.remove_file(&root, &f.path, dry_run);
    }
    entry(CACHES, id, &label, tally, 0)
}

/// The one entry `old-logs`: the log files the scan's rule selects right
/// now — older than 30 days, never the newest.
pub(super) async fn old_logs(
    ctx: &ScanContext,
    ids: &[String],
    dry_run: bool,
) -> Result<Vec<EntryResult>> {
    let ctx = ctx.clone();
    let ids = ids.to_vec();
    blocking(move || {
        let now = OffsetDateTime::now_utc();
        let dir = ctx.paths.logs_dir();
        ids.iter()
            .map(|id| {
                if id != "old-logs" {
                    return not_offered(LOGS, id);
                }
                if let Some(why) = ctx.inside_store_or_runtimes(&dir) {
                    return refused(LOGS, id, id, &dir, &why);
                }
                let Some(root) = canonical_dir(&dir) else {
                    return not_offered(LOGS, id);
                };
                let old = old_log_candidates(&dir, now);
                let label = format!("{} log file(s) older than {OLD_LOG_DAYS} days", old.len());
                let mut tally = Tally::default();
                for f in &old {
                    tally.remove_flat_file(&root, &f.path, dry_run);
                }
                entry(LOGS, id, &label, tally, 0)
            })
            .collect()
    })
    .await
}

/// One backup per id: a flat file in the exports folder.
pub(super) async fn backups(
    ctx: &ScanContext,
    ids: &[String],
    dry_run: bool,
) -> Result<Vec<EntryResult>> {
    let ctx = ctx.clone();
    let ids = ids.to_vec();
    blocking(move || {
        let dir = ctx.paths.exports_dir();
        let refusal = ctx.inside_store_or_runtimes(&dir);
        let root = canonical_dir(&dir);
        ids.iter()
            .map(|id| {
                if let Some(why) = &refusal {
                    return refused(BACKUPS, id, id, &dir, why);
                }
                let Some(root) = root.as_deref() else {
                    return not_offered(BACKUPS, id);
                };
                let mut tally = Tally::default();
                tally.remove_flat_file(root, &dir.join(id), dry_run);
                entry(BACKUPS, id, id, tally, 0)
            })
            .collect()
    })
    .await
}
