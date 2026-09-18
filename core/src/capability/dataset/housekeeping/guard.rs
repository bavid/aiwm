//! The one place that decides whether a file may be deleted, and which
//! folders may be walked or pruned.
//!
//! A file is deleted only when **all** of these hold (see [`Guard::check`]):
//! - the stored path is absolute (a relative one would resolve against the
//!   process's working directory);
//! - its canonical path (`..` and links resolved) is not a source file of
//!   this dataset and not under any dataset's `source_root` — this one's
//!   included, so a source folder that doubles as an export never loses a
//!   file;
//! - it is not a frame or source file of **any other dataset** (or of a
//!   frame without dataset), and not under another dataset's work folder,
//!   export folder or source folder — deleting never touches another
//!   dataset's files, so another dataset's training run cannot be affected
//!   either;
//! - no frame of this dataset that stays still shows it;
//! - it lies strictly inside one of this dataset's roots: its work folder
//!   `<outputs>/datasets/<prep_job_id>`, its export folder when that is a
//!   dedicated folder inside the outputs folder, or — once the prep job is
//!   gone — some `<outputs>/datasets/<X>` that nothing else claims.
//!
//! A folder is walked (every file in it considered) only for a dataset that
//! still has its prep job, and only when nothing of another dataset and no
//! source folder lies inside it; otherwise deletion goes file by file over
//! the dataset's own frame rows.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::db::{Dataset, DatasetFrame};

use super::{
    SkippedFile, SKIP_ERROR, SKIP_IN_USE, SKIP_NOT_A_FILE, SKIP_OTHER_DATASET, SKIP_OUTSIDE,
    SKIP_SOURCE,
};

/// Everything the guard needs, read from the database before the blocking
/// file work starts.
pub(super) struct Snapshot {
    pub(super) dataset: Dataset,
    pub(super) frames: Vec<DatasetFrame>,
    /// Every other dataset.
    pub(super) others: Vec<Dataset>,
    /// `(frame_path, source_path)` of every frame not in this dataset.
    pub(super) foreign_frames: Vec<(String, String)>,
}

pub(super) enum Verdict {
    Delete { path: PathBuf, bytes: u64 },
    Missing,
    Skip(SkippedFile),
}

#[derive(Debug, Default)]
pub(super) struct Guard {
    outputs: Option<PathBuf>,
    datasets_root: Option<PathBuf>,
    /// Canonical `<outputs>/datasets/<prep_job_id>` when it exists.
    pub(super) work_dir: Option<PathBuf>,
    /// The work folder may be walked and removed as a whole.
    pub(super) walkable: bool,
    /// No prep job: frames may be deleted inside an unclaimed
    /// `<datasets_root>/<X>`.
    loose: bool,
    /// First components under `datasets_root` another dataset claims.
    claimed: HashSet<OsString>,
    /// Canonical export folder, only when it is this dataset's own.
    pub(super) export_dir: Option<PathBuf>,
    own_sources: HashSet<PathBuf>,
    source_dirs: Vec<PathBuf>,
    foreign_files: HashSet<PathBuf>,
    foreign_dirs: Vec<PathBuf>,
    in_use: HashSet<PathBuf>,
}

fn canonical(raw: &str) -> Option<PathBuf> {
    let p = Path::new(raw);
    if raw.is_empty() || !p.is_absolute() {
        return None;
    }
    std::fs::canonicalize(p).ok()
}

fn canonical_all<'a>(raws: impl Iterator<Item = &'a str>) -> HashSet<PathBuf> {
    raws.collect::<HashSet<&str>>()
        .into_iter()
        .filter_map(canonical)
        .collect()
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

/// `inner` lies strictly inside `outer`.
fn strictly_inside(inner: &Path, outer: &Path) -> bool {
    inner.starts_with(outer) && inner != outer
}

/// Either path contains the other (or they are equal).
fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

impl Guard {
    /// `staying`: frame paths of this dataset that are not being deleted.
    pub(super) fn build(outputs_dir: &Path, snap: &Snapshot, staying: &[&str]) -> Self {
        let outputs = std::fs::canonicalize(outputs_dir).ok();
        let datasets_root = outputs
            .as_ref()
            .and_then(|o| std::fs::canonicalize(o.join("datasets")).ok());
        let work_of = |d: &Dataset| -> Option<PathBuf> {
            let root = datasets_root.as_deref()?;
            let id = single_component(d.prep_job_id.as_deref()?)?;
            let dir = std::fs::canonicalize(root.join(id)).ok()?;
            (dir.is_dir() && strictly_inside(&dir, root)).then_some(dir)
        };

        let own_sources = canonical_all(snap.frames.iter().map(|f| f.source_path.as_str()));
        let source_dirs: Vec<PathBuf> = canonical_all(
            std::iter::once(snap.dataset.source_root.as_str())
                .chain(snap.others.iter().map(|d| d.source_root.as_str())),
        )
        .into_iter()
        .collect();
        let foreign_files = canonical_all(
            snap.foreign_frames
                .iter()
                .flat_map(|(frame, source)| [frame.as_str(), source.as_str()]),
        );
        let other_exports: Vec<PathBuf> =
            canonical_all(snap.others.iter().filter_map(|d| d.export_dir.as_deref()))
                .into_iter()
                .collect();
        let foreign_dirs: Vec<PathBuf> = other_exports
            .iter()
            .cloned()
            .chain(snap.others.iter().filter_map(work_of))
            .collect();
        let in_use = canonical_all(staying.iter().copied());

        let work_dir = work_of(&snap.dataset);
        let walkable = work_dir.as_deref().is_some_and(|w| {
            !foreign_files.iter().any(|f| f.starts_with(w))
                && !own_sources.iter().any(|f| f.starts_with(w))
                && !foreign_dirs.iter().any(|d| overlaps(d, w))
                && !source_dirs.iter().any(|d| overlaps(d, w))
        });

        let mut claimed: HashSet<OsString> = snap
            .others
            .iter()
            .filter_map(|d| d.prep_job_id.as_deref())
            .map(OsString::from)
            .collect();
        if let Some(root) = datasets_root.as_deref() {
            let under = |p: &PathBuf| -> Option<OsString> {
                match p.strip_prefix(root).ok()?.components().next()? {
                    Component::Normal(n) => Some(n.to_os_string()),
                    _ => None,
                }
            };
            claimed.extend(foreign_files.iter().filter_map(under));
            claimed.extend(foreign_dirs.iter().filter_map(under));
            claimed.extend(source_dirs.iter().filter_map(under));
        }

        let export_dir = match (&outputs, &snap.dataset.export_dir) {
            (Some(o), Some(e)) => own_export(o, datasets_root.as_deref(), e, &other_exports),
            _ => None,
        };

        Self {
            loose: snap.dataset.prep_job_id.is_none(),
            outputs,
            datasets_root,
            work_dir,
            walkable,
            claimed,
            export_dir,
            own_sources,
            source_dirs,
            foreign_files,
            foreign_dirs,
            in_use,
        }
    }

    /// A guard whose only root is `work_dir` (already canonical) — for
    /// testing [`Self::check`] in isolation.
    #[cfg(test)]
    pub(super) fn with_work_dir_for_test(work_dir: PathBuf) -> Self {
        Self {
            work_dir: Some(work_dir),
            ..Self::default()
        }
    }

    pub(super) fn check(&self, raw: &Path) -> Verdict {
        if raw.as_os_str().is_empty() {
            return Verdict::Missing;
        }
        // A relative path would be resolved against the process's working
        // directory, which says nothing about where the frame really is.
        if !raw.is_absolute() {
            return Verdict::Skip(skipped(raw, SKIP_OUTSIDE));
        }
        let path = match std::fs::canonicalize(raw) {
            Ok(p) => p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Verdict::Missing,
            Err(e) => return Verdict::Skip(skipped(raw, &format!("{SKIP_ERROR}: {e}"))),
        };
        if self.own_sources.contains(&path) || self.source_dirs.iter().any(|d| path.starts_with(d))
        {
            return Verdict::Skip(skipped(raw, SKIP_SOURCE));
        }
        if self.foreign_files.contains(&path)
            || self.foreign_dirs.iter().any(|d| path.starts_with(d))
        {
            return Verdict::Skip(skipped(raw, SKIP_OTHER_DATASET));
        }
        if self.in_use.contains(&path) {
            return Verdict::Skip(skipped(raw, SKIP_IN_USE));
        }
        if self.boundary(&path).is_none() {
            return Verdict::Skip(skipped(raw, SKIP_OUTSIDE));
        }
        match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => Verdict::Delete {
                bytes: m.len(),
                path,
            },
            Ok(_) => Verdict::Skip(skipped(raw, SKIP_NOT_A_FILE)),
            Err(e) => Verdict::Skip(skipped(raw, &format!("{SKIP_ERROR}: {e}"))),
        }
    }

    /// The root folder a canonical `path` lies strictly inside — the limit
    /// up to which (exclusive) emptied folders may be pruned — or `None`
    /// when it is outside every root of this dataset.
    fn boundary(&self, path: &Path) -> Option<PathBuf> {
        if let Some(w) = self
            .work_dir
            .as_deref()
            .filter(|w| strictly_inside(path, w))
        {
            return Some(w.to_path_buf());
        }
        if let Some(e) = self
            .export_dir
            .as_deref()
            .filter(|e| strictly_inside(path, e))
        {
            return Some(e.to_path_buf());
        }
        if !self.loose {
            return None;
        }
        let root = self.datasets_root.as_deref()?;
        let mut rest = path.strip_prefix(root).ok()?.components();
        let (Some(Component::Normal(x)), Some(_)) = (rest.next(), rest.next()) else {
            return None;
        };
        (!self.claimed.contains(x)).then(|| root.join(x))
    }

    /// Nothing another dataset or any source folder needs lies at or inside
    /// `dir`, so an emptied `dir` may go.
    fn may_remove_dir(&self, dir: &Path) -> bool {
        !self
            .foreign_dirs
            .iter()
            .chain(&self.source_dirs)
            .any(|d| d.starts_with(dir))
    }

    /// Remove the folders that became empty because `deleted` (canonical
    /// files) went, from each file's folder upwards, stopping below the
    /// root the file lay in.
    pub(super) fn prune_after(&self, deleted: &[PathBuf]) {
        for file in deleted {
            let Some(stop) = self.boundary(file) else {
                continue;
            };
            let mut dir = file.parent();
            while let Some(d) = dir.filter(|d| strictly_inside(d, &stop)) {
                if !self.may_remove_dir(d) || std::fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
    }

    /// Remove the export folder once it is empty — only a dedicated folder at
    /// least two levels below outputs, never outputs itself or a first-level
    /// folder of it.
    pub(super) fn remove_export_dir_if_empty(&self) {
        let (Some(export), Some(outputs)) = (self.export_dir.as_deref(), self.outputs.as_deref())
        else {
            return;
        };
        let depth = export
            .strip_prefix(outputs)
            .map_or(0, |rest| rest.components().count());
        if depth >= 2 && self.may_remove_dir(export) {
            let _ = std::fs::remove_dir(export);
        }
    }
}

/// This dataset's export folder when the app may delete its numbered files:
/// strictly inside the outputs folder, not in or above the `datasets` folder
/// (that is where work folders live), and no other dataset's export folder.
fn own_export(
    outputs: &Path,
    datasets_root: Option<&Path>,
    export: &str,
    other_exports: &[PathBuf],
) -> Option<PathBuf> {
    let export = canonical(export)?;
    let inside = strictly_inside(&export, outputs);
    let near_work = datasets_root.is_some_and(|root| overlaps(&export, root));
    let shared = other_exports.contains(&export);
    (inside && !near_work && !shared && export.is_dir()).then_some(export)
}

pub(super) fn skipped(path: &Path, reason: &str) -> SkippedFile {
    SkippedFile {
        path: path.to_string_lossy().into_owned(),
        reason: reason.to_string(),
    }
}
