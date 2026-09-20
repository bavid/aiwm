//! Groups `discarded_frames`, `unclaimed_dataset_folders` and
//! `missing_frame_rows` — all per dataset, all skipping a busy dataset (a
//! prep job still writing frames, a training run reading them), which is
//! listed under *protected* instead.
//!
//! Discarded frames come from `housekeeping::usage` (the guard's own
//! measurement of what a cleanup would delete); unclaimed folders from
//! `housekeeping::unclaimed_work_folders` (the guard's "unclaimed under the
//! datasets root" rule); missing rows are counted here, file by file.

use std::path::Path;

use super::{
    display, file_name, group, note, walk_totals, CleanupEntry, CleanupGroup, Inventory,
    ProtectedNote, ScanContext,
};

pub(super) fn groups(
    ctx: &ScanContext,
    inv: &Inventory,
) -> (Vec<CleanupGroup>, Vec<ProtectedNote>) {
    let mut notes: Vec<ProtectedNote> = inv
        .datasets
        .iter()
        .filter_map(|d| {
            d.busy
                .as_ref()
                .map(|why| note(format!("Dataset \"{}\"", d.dataset.name), why))
        })
        .collect();
    let (unclaimed, unclaimed_notes) = unclaimed(ctx, inv);
    notes.extend(unclaimed_notes);
    (
        vec![
            group("discarded_frames", discarded(inv)),
            group("unclaimed_dataset_folders", unclaimed),
            group("missing_frame_rows", missing_rows(inv)),
        ],
        notes,
    )
}

/// Per dataset that is not busy: its excluded and rejected frames, as the
/// housekeeping guard would delete them.
fn discarded(inv: &Inventory) -> Vec<CleanupEntry> {
    inv.datasets
        .iter()
        .filter(|d| d.busy.is_none())
        .filter_map(|d| d.usage.as_ref().map(|u| (d, u)))
        .filter(|(_, u)| u.discarded_frames > 0)
        .map(|(d, u)| CleanupEntry {
            id: d.dataset.id.clone(),
            label: d.dataset.name.clone(),
            files: u.discarded_files,
            bytes: u.discarded_bytes,
            rows: u.discarded_frames,
            detail: u
                .work_dir
                .iter()
                .map(|w| format!("work folder {w}"))
                .collect(),
        })
        .collect()
}

/// The datasets root's direct children no dataset claims — unless one of
/// them is, or holds, an app folder (a training root configured inside the
/// datasets root, say): that is a root, never an orphaned work folder.
fn unclaimed(ctx: &ScanContext, inv: &Inventory) -> (Vec<CleanupEntry>, Vec<ProtectedNote>) {
    let mut entries = Vec::new();
    let mut notes = Vec::new();
    for folder in &inv.unclaimed.folders {
        if let Some(why) = ctx.whole_folder_refusal(folder, None) {
            notes.push(note(format!("Folder {}", display(folder)), why));
            continue;
        }
        let (files, bytes) = walk_totals(folder);
        let name = file_name(folder);
        entries.push(CleanupEntry {
            id: name.clone(),
            label: name,
            files,
            bytes,
            rows: 0,
            detail: vec![display(folder)],
        });
    }
    for link in &inv.unclaimed.links {
        notes.push(note(
            format!("Folder {}", display(link)),
            "it is a link or junction, never followed",
        ));
    }
    (entries, notes)
}

/// Per dataset that is not busy: rows whose file is gone. Only a row with
/// a non-empty absolute path that the disk reports as not found counts —
/// an unreadable or oddly stored path is not "missing".
fn missing_rows(inv: &Inventory) -> Vec<CleanupEntry> {
    inv.datasets
        .iter()
        .filter(|d| d.busy.is_none())
        .filter_map(|d| {
            let rows = d
                .frames
                .iter()
                .filter(|f| is_missing(&f.frame_path))
                .count() as u64;
            (rows > 0).then(|| CleanupEntry {
                id: d.dataset.id.clone(),
                label: d.dataset.name.clone(),
                files: 0,
                bytes: 0,
                rows,
                detail: vec![format!("{rows} of {} frame entries", d.frames.len())],
            })
        })
        .collect()
}

fn is_missing(frame_path: &str) -> bool {
    let path = Path::new(frame_path);
    if frame_path.is_empty() || !path.is_absolute() {
        return false;
    }
    matches!(
        std::fs::symlink_metadata(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound
    )
}
