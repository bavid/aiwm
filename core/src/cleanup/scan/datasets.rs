//! Groups `discarded_frames`, `unclaimed_dataset_folders` and
//! `missing_frame_rows` — all per dataset, all skipping a busy dataset (a
//! prep job still writing frames, a training run reading them), which is
//! listed under *protected* instead.
//!
//! Discarded frames come from `housekeeping::usage` (the guard's own
//! measurement of what a cleanup would delete); unclaimed folders from
//! `housekeeping::unclaimed_work_folders` (the guard's "unclaimed under the
//! datasets root" rule); missing rows are counted here, file by file — and
//! a file on a drive that is not connected right now is *not* missing (see
//! [`file_state`]): it comes back with the drive, and its row's curation
//! would be lost for nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::capability::dataset::location::nearest_existing;

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
    let (missing, missing_notes) = missing_rows(inv);
    notes.extend(missing_notes);
    (
        vec![
            group("discarded_frames", discarded(inv)),
            group("unclaimed_dataset_folders", unclaimed),
            group("missing_frame_rows", missing),
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
/// a non-empty absolute path that the disk reports as not found *while its
/// drive is connected* counts — an unreadable or oddly stored path is not
/// "missing", and a row on an absent drive is protected (one note per
/// dataset and drive) rather than offered.
fn missing_rows(inv: &Inventory) -> (Vec<CleanupEntry>, Vec<ProtectedNote>) {
    let mut entries = Vec::new();
    let mut notes = Vec::new();
    for d in inv.datasets.iter().filter(|d| d.busy.is_none()) {
        let mut missing = 0u64;
        let mut disconnected: BTreeMap<PathBuf, u64> = BTreeMap::new();
        for f in &d.frames {
            match file_state(&f.frame_path) {
                FileState::Missing => missing += 1,
                FileState::DriveAbsent(root) => *disconnected.entry(root).or_default() += 1,
                FileState::Other => {}
            }
        }
        for (root, rows) in disconnected {
            notes.push(note(
                format!("{rows} frame rows of dataset \"{}\"", d.dataset.name),
                format!(
                    "their files are on a drive that is not connected ({}) \u{2014} reconnect \
                     it or leave them",
                    root.display()
                ),
            ));
        }
        if missing > 0 {
            entries.push(CleanupEntry {
                id: d.dataset.id.clone(),
                label: d.dataset.name.clone(),
                files: 0,
                bytes: 0,
                rows: missing,
                detail: vec![format!("{missing} of {} frame entries", d.frames.len())],
            });
        }
    }
    (entries, notes)
}

/// What the disk says about a frame row's file, as far as "missing" goes.
enum FileState {
    /// The drive is there, the file is not.
    Missing,
    /// Nothing of the path exists, not even its volume root (`E:\`,
    /// `\\server\share\`): the drive is not connected. An unplugged
    /// external drive answers `NotFound` for every path on it, exactly like
    /// a deleted file would — telling them apart is the whole point.
    DriveAbsent(PathBuf),
    /// Present, unreadable, or a path that cannot be judged.
    Other,
}

fn file_state(frame_path: &str) -> FileState {
    let path = Path::new(frame_path);
    if frame_path.is_empty() || !path.is_absolute() {
        return FileState::Other;
    }
    let not_found = matches!(
        std::fs::symlink_metadata(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound
    );
    if !not_found {
        return FileState::Other;
    }
    // `nearest_existing` climbs up to the volume root itself; nothing found
    // means the root is absent — the drive is not connected.
    match nearest_existing(path) {
        Some(_) => FileState::Missing,
        None => FileState::DriveAbsent(volume_root(path)),
    }
}

/// `E:\` for `E:\x\y`, `\\server\share\` for a UNC path — the last
/// ancestor, which is the root.
fn volume_root(path: &Path) -> PathBuf {
    path.ancestors()
        .last()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.to_path_buf())
}
