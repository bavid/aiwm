//! Where a dataset's work folder goes (Plan 10): comparing a chosen folder
//! against the source and the model store, and the free-space preflight
//! before a prep run writes anything.
//!
//! Folder relations are decided on two forms of each path and a match in
//! either counts: the lexically normalised path (`.`/`..` folded) and the
//! resolved one (the nearest existing ancestor canonicalised, links and
//! junctions followed, the rest appended). On Windows both are compared
//! case-insensitively, as the file system does.

use std::path::{Component, Path, PathBuf};

use crate::cleanup::{volume_label, FreeSpaceProbe};
use crate::{CoreError, Result};

/// A prep run refuses to start when the drive its work folder lives on has
/// less than this free: extracted frames of even a short clip set run into
/// gigabytes.
pub const MIN_PREP_FREE_BYTES: u64 = 5 * BYTES_PER_GB;
const BYTES_PER_GB: u64 = 1024 * 1024 * 1024;

/// `.` dropped and `..` folded without touching the disk (never above the
/// root), then case-folded on Windows.
fn lexical(path: &Path) -> PathBuf {
    fold_case(lexical_plain(path))
}

#[cfg(windows)]
fn fold_case(path: PathBuf) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

#[cfg(not(windows))]
fn fold_case(path: PathBuf) -> PathBuf {
    path
}

/// The nearest ancestor of `path` (itself included) that exists.
pub(crate) fn nearest_existing(path: &Path) -> Option<&Path> {
    path.ancestors().find(|a| a.exists())
}

/// The nearest existing ancestor canonicalised with the rest appended, then
/// lexically normalised — `None` when nothing of the path exists.
fn resolved(path: &Path) -> Option<PathBuf> {
    let path = lexical_plain(path);
    let base = nearest_existing(&path)?;
    let rest = path.strip_prefix(base).ok()?;
    let canonical = std::fs::canonicalize(base).ok()?;
    Some(fold_case(canonical.join(rest)))
}

/// [`lexical`] without the case folding (the disk is asked with it).
fn lexical_plain(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `inner` is `outer` or lies inside it, by either comparison form.
pub(crate) fn same_or_inside(inner: &Path, outer: &Path) -> bool {
    if lexical(inner).starts_with(lexical(outer)) {
        return true;
    }
    matches!(
        (resolved(inner), resolved(outer)),
        (Some(i), Some(o)) if i.starts_with(&o)
    )
}

/// `path` is a drive or volume root (nothing below its root).
pub(crate) fn is_volume_root(path: &Path) -> bool {
    !lexical_plain(path)
        .components()
        .any(|c| matches!(c, Component::Normal(_)))
}

/// The folder a work folder may be stored in, checked against the prep's
/// source folder: absolute, not a whole drive, not the source or inside it,
/// and not holding the source.
pub(crate) fn check_data_dir(data_dir: &Path, source_root: &Path) -> Result<()> {
    if !data_dir.is_absolute() {
        return Err(CoreError::Config(format!(
            "the folder to store frames in must be an absolute path, got {:?}",
            data_dir.display().to_string()
        )));
    }
    if is_volume_root(data_dir) {
        return Err(CoreError::Config(format!(
            "choose a folder to store frames in, not the whole drive {}",
            data_dir.display()
        )));
    }
    if same_or_inside(data_dir, source_root) {
        return Err(CoreError::Config(format!(
            "frames cannot be stored inside the source folder {} \u{2014} choose a folder \
             outside it",
            source_root.display()
        )));
    }
    if same_or_inside(source_root, data_dir) {
        return Err(CoreError::Config(format!(
            "the source folder {} lies inside {} \u{2014} choose a folder to store frames in \
             that does not hold your source media",
            source_root.display(),
            data_dir.display()
        )));
    }
    Ok(())
}

/// `data_dir` must not be the model store or lie inside it.
pub(crate) fn check_outside_store(data_dir: &Path, store: &Path) -> Result<()> {
    if store.as_os_str().is_empty() || !same_or_inside(data_dir, store) {
        return Ok(());
    }
    Err(CoreError::Config(format!(
        "frames cannot be stored inside the model store {} \u{2014} choose another folder",
        store.display()
    )))
}

/// Refuse a prep run whose work root's drive has less than
/// [`MIN_PREP_FREE_BYTES`] free (measured on the nearest existing ancestor),
/// then create the work root. A volume the probe cannot measure is not
/// refused — same reasoning as the training preflight.
pub fn prepare_work_root(work_root: &Path, probe: FreeSpaceProbe) -> Result<()> {
    let measured = nearest_existing(work_root).and_then(probe);
    if let Some((free, _total)) = measured {
        if free < MIN_PREP_FREE_BYTES {
            return Err(CoreError::Config(format!(
                "only {:.1} GB free on {}, at least {} GB needed to store the extracted frames \
                 \u{2014} choose a folder on another drive or free up space",
                free as f64 / BYTES_PER_GB as f64,
                volume_label(work_root),
                MIN_PREP_FREE_BYTES / BYTES_PER_GB
            )));
        }
    } else {
        tracing::debug!(
            path = %work_root.display(),
            "could not determine free disk space for the dataset work folder"
        );
    }
    std::fs::create_dir_all(work_root).map_err(|e| {
        CoreError::Config(format!(
            "cannot create the folder to store frames in ({}): {e}",
            work_root.display()
        ))
    })
}
