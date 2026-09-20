//! Groups `caches`, `old_logs` and `db_backups`.
//!
//! `caches`: everything under the cache folder; download staging folders
//! no unfinished download owns; ComfyUI `input`/`temp`/`output` leftovers
//! older than an hour that no unfinished job staged (`<job_id>.<ext>`); the
//! pending-import staging. `old_logs`: log files older than 30 days, never
//! the newest one. `db_backups`: every export, one entry each, with its
//! date. A folder offered as a whole (cache, a staging folder, the pending
//! import) is refused when it is, or holds, another app folder.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use time::{Duration, OffsetDateTime};

use super::{
    capped, date_of, display, file_name, group, note, walk_files, walk_totals, CleanupEntry,
    CleanupGroup, FileInfo, Inventory, ProtectedNote, ScanContext,
};

/// Log files older than this are offered.
pub(in crate::cleanup) const OLD_LOG_DAYS: i64 = 30;
/// A ComfyUI scratch file younger than this may still be in use.
pub(super) const COMFYUI_LEFTOVER_MIN_AGE: Duration = Duration::hours(1);
pub(in crate::cleanup) const COMFYUI_SCRATCH: [&str; 3] = ["input", "temp", "output"];

pub(super) fn groups(
    ctx: &ScanContext,
    inv: &Inventory,
) -> (Vec<CleanupGroup>, Vec<ProtectedNote>) {
    let now = OffsetDateTime::now_utc();
    let mut entries = Vec::new();
    let mut notes = Vec::new();

    let cache = ctx.paths.cache_dir();
    match ctx.whole_folder_refusal(&cache, Some(&cache)) {
        Some(why) => notes.push(note(format!("Cache folder {}", display(&cache)), why)),
        None => entries.extend(folder_entry("cache", "Registry and runtime caches", &cache)),
    }

    let (staging, staging_notes) = staging_dirs(ctx, inv);
    entries.extend(staging);
    notes.extend(staging_notes);

    entries.extend(comfyui_leftovers(ctx, inv, now));

    let pending = ctx.paths.pending_import_dir();
    match ctx.whole_folder_refusal(&pending, Some(&pending)) {
        Some(why) => notes.push(note(format!("Pending import {}", display(&pending)), why)),
        None => entries.extend(folder_entry(
            "pending-import",
            "Pending import staging",
            &pending,
        )),
    }

    let (logs, log_notes) = old_logs(ctx, now);
    notes.extend(log_notes);
    let (backups, backup_notes) = backups(ctx);
    notes.extend(backup_notes);
    (
        vec![
            group("caches", entries),
            group("old_logs", logs),
            group("db_backups", backups),
        ],
        notes,
    )
}

/// One entry for a whole folder, when it holds anything.
fn folder_entry(id: &str, label: &str, dir: &Path) -> Option<CleanupEntry> {
    let (files, bytes) = walk_totals(dir);
    (files > 0).then(|| CleanupEntry {
        id: id.to_string(),
        label: label.to_string(),
        files,
        bytes,
        rows: 0,
        detail: vec![display(dir)],
    })
}

/// Direct child folders of the download staging root: one entry each,
/// unless a download that has not finished owns it (its id is the folder
/// name).
fn staging_dirs(ctx: &ScanContext, inv: &Inventory) -> (Vec<CleanupEntry>, Vec<ProtectedNote>) {
    let active: HashMap<&str, &crate::db::Download> = inv
        .downloads
        .iter()
        .filter(|d| !d.state.is_terminal())
        .map(|d| (d.id.as_str(), d))
        .collect();
    let root = ctx.paths.downloads_dir();
    let mut entries = Vec::new();
    let mut notes = Vec::new();
    let Ok(dirs) = std::fs::read_dir(&root) else {
        return (entries, notes);
    };
    let mut children: Vec<_> = dirs
        .flatten()
        .filter(|e| {
            e.metadata()
                .is_ok_and(|m| m.is_dir() && !m.is_symlink() && !super::is_reparse_point(&m))
        })
        .map(|e| e.path())
        .collect();
    children.sort();
    for dir in children {
        let name = file_name(&dir);
        if let Some(d) = active.get(name.as_str()) {
            notes.push(note(
                format!("Download staging of {}", d.filename),
                format!("the download is {}", d.state.as_str()),
            ));
            continue;
        }
        if let Some(why) = ctx.whole_folder_refusal(&dir, None) {
            notes.push(note(format!("Download staging {}", display(&dir)), why));
            continue;
        }
        entries.extend(folder_entry(
            &format!("download-staging:{name}"),
            &format!("Download staging {name}"),
            &dir,
        ));
    }
    (entries, notes)
}

/// Files under ComfyUI's `input`, `temp` and `output` older than an hour
/// whose stem is not an unfinished job's id — one entry per folder.
fn comfyui_leftovers(ctx: &ScanContext, inv: &Inventory, now: OffsetDateTime) -> Vec<CleanupEntry> {
    let active = inv.active_job_ids();
    let base = ctx.paths.comfyui_data_dir();
    if ctx.inside_store_or_runtimes(&base).is_some() {
        return Vec::new();
    }
    COMFYUI_SCRATCH
        .iter()
        .filter_map(|sub| {
            let leftovers = comfyui_leftover_files(&base.join(sub), &active, now);
            (!leftovers.is_empty()).then(|| CleanupEntry {
                id: format!("comfyui:{sub}"),
                label: format!("ComfyUI {sub} leftovers"),
                files: leftovers.len() as u64,
                bytes: leftovers.iter().map(|f| f.bytes).sum(),
                rows: 0,
                detail: capped(leftovers.iter().map(|f| file_name(&f.path))),
            })
        })
        .collect()
}

/// The files under one ComfyUI scratch folder that are leftovers: older
/// than [`COMFYUI_LEFTOVER_MIN_AGE`] and not staged by an unfinished job
/// (`active` holds those jobs' ids; a staged file is `<job_id>.<ext>`).
/// Shared with the apply, which re-computes it with fresh job states right
/// before deleting.
pub(in crate::cleanup) fn comfyui_leftover_files(
    dir: &Path,
    active: &HashSet<&str>,
    now: OffsetDateTime,
) -> Vec<FileInfo> {
    walk_files(dir)
        .into_iter()
        .filter(|f| {
            f.modified
                .is_some_and(|m| now - m >= COMFYUI_LEFTOVER_MIN_AGE)
        })
        .filter(|f| {
            let stem = f
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            !active.contains(stem.as_str())
        })
        .collect()
}

/// Regular files directly in `dir` (never through a link), by name.
pub(in crate::cleanup) fn flat_files(dir: &Path) -> Vec<FileInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<FileInfo> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            (meta.is_file() && !meta.is_symlink() && !super::is_reparse_point(&meta)).then(|| {
                FileInfo {
                    path: e.path(),
                    bytes: meta.len(),
                    modified: meta.modified().ok().map(OffsetDateTime::from),
                }
            })
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// Log files older than [`OLD_LOG_DAYS`], never the newest one (today's,
/// or the only one there is).
fn old_logs(ctx: &ScanContext, now: OffsetDateTime) -> (Vec<CleanupEntry>, Vec<ProtectedNote>) {
    let dir = ctx.paths.logs_dir();
    if let Some(why) = ctx.inside_store_or_runtimes(&dir) {
        return (
            Vec::new(),
            vec![note(format!("Logs {}", display(&dir)), why)],
        );
    }
    let old = old_log_candidates(&dir, now);
    if old.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let entry = CleanupEntry {
        id: "old-logs".into(),
        label: format!("{} log file(s) older than {OLD_LOG_DAYS} days", old.len()),
        files: old.len() as u64,
        bytes: old.iter().map(|f| f.bytes).sum(),
        rows: 0,
        detail: capped(old.iter().map(|f| file_name(&f.path))),
    };
    (vec![entry], Vec::new())
}

/// The log files older than [`OLD_LOG_DAYS`] — never the newest one
/// (today's, or the only one there is). Shared with the apply, which
/// re-computes it right before deleting: the newest log is never in it.
pub(in crate::cleanup) fn old_log_candidates(dir: &Path, now: OffsetDateTime) -> Vec<FileInfo> {
    let files = flat_files(dir);
    let newest = files
        .iter()
        .max_by_key(|f| (f.modified, f.path.clone()))
        .map(|f| f.path.clone());
    let cutoff = now - Duration::days(OLD_LOG_DAYS);
    files
        .into_iter()
        .filter(|f| Some(&f.path) != newest.as_ref())
        .filter(|f| f.modified.is_some_and(|m| m < cutoff))
        .collect()
}

/// Every file in the exports folder, one entry each, dated.
fn backups(ctx: &ScanContext) -> (Vec<CleanupEntry>, Vec<ProtectedNote>) {
    let dir = ctx.paths.exports_dir();
    if let Some(why) = ctx.inside_store_or_runtimes(&dir) {
        return (
            Vec::new(),
            vec![note(format!("Backups {}", display(&dir)), why)],
        );
    }
    let entries = flat_files(&dir)
        .into_iter()
        .map(|f| {
            let name = file_name(&f.path);
            CleanupEntry {
                id: name.clone(),
                label: format!("{name} ({})", date_of(f.modified)),
                files: 1,
                bytes: f.bytes,
                rows: 0,
                detail: vec![display(&f.path)],
            }
        })
        .collect();
    (entries, Vec::new())
}
