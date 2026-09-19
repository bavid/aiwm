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
//! - it lies strictly inside one of this dataset's roots: its work folder,
//!   its export folder when that is a dedicated folder inside the outputs
//!   folder, or — for a row with neither prep job nor recorded work folder
//!   ("loose") — some `<datasets root>/<X>` that nothing else claims.
//!
//! **Work folders.** A row with a recorded `work_dir` (Plan 10, every
//! dataset prepared since migration 0019, possibly in a folder the user
//! chose) owns exactly that folder, canonicalised — with or without its prep
//! job. It is refused (treated as no work folder at all, so its files fall
//! back to the per-file rules and are reported as skipped) when it is a
//! drive root or a first-level folder, equals / holds / lies inside any
//! dataset's source folder or another dataset's work folder (either kind),
//! overlaps the model store, or holds the datasets root or the outputs
//! folder. A row without `work_dir` keeps the derived
//! `<datasets root>/<prep_job_id>`. Other datasets' recorded folders are
//! protected like their derived ones; one that cannot be located stops every
//! walk. **Training run folders** (the recorded `work_dir`, or the derived
//! `<training root>/<run id>`) are protected exactly like another dataset's
//! work folder: a work folder overlapping one is not the dataset's own, no
//! walk enters one, and no file inside one is deleted.
//!
//! A folder is walked (every file in it considered) only when it is this
//! dataset's work folder and nothing of another dataset and no source folder
//! lies inside it; otherwise deletion goes file by file over the dataset's
//! own frame rows. Links and junctions are never followed.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::capability::dataset::location::same_or_inside;
use crate::db::{Dataset, DatasetFrame};

use super::{
    DataRoots, SkippedFile, SKIP_ERROR, SKIP_IN_USE, SKIP_NOT_A_FILE, SKIP_OTHER_DATASET,
    SKIP_OUTSIDE, SKIP_SOURCE,
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
    /// Every training run: their folders are foreign to every dataset.
    pub(super) runs: Vec<crate::db::TrainingRun>,
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
    /// This dataset's canonical work folder when it exists and passes the
    /// rules (see the module docs).
    pub(super) work_dir: Option<PathBuf>,
    /// The work folder may be walked and removed as a whole.
    pub(super) walkable: bool,
    /// No prep job and no recorded work folder: frames may be deleted
    /// inside an unclaimed `<datasets_root>/<X>`.
    loose: bool,
    /// First components under `datasets_root` another dataset claims.
    claimed: HashSet<OsString>,
    /// Canonical export folder, only when it is this dataset's own.
    pub(super) export_dir: Option<PathBuf>,
    /// This dataset's source files near its roots (canonical), and all of
    /// them as stored (an in-place frame's path equals its source path).
    own_sources: HashSet<PathBuf>,
    own_source_raws: HashSet<String>,
    source_dirs: Vec<PathBuf>,
    /// Folders (as stored) of this dataset's sources / other datasets'
    /// files that exist but could not be resolved: nothing under them goes.
    unresolved_own: Vec<PathBuf>,
    unresolved_foreign: Vec<PathBuf>,
    /// Other datasets' frame/source files near this dataset's roots.
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

/// Many stored paths reduced to what the guard needs, without
/// canonicalising every file: frames live in a few hundred folders, so each
/// distinct *folder* is canonicalised once, and a file is canonicalised on
/// its own only when its folder lies inside one of `roots` (this dataset's
/// deletable folders) — usually never for another dataset's frames.
///
/// Accepted residual gap: a file whose folder is outside the roots but which
/// is itself a link into them is not recognised by its target. The prep
/// pipeline never creates links, and `check` resolves the file being deleted
/// itself, so the gap only matters for a hand-made link in another
/// dataset's folder pointing at one of this dataset's own files.
pub(super) struct PathIndex {
    /// Canonical folders the paths live in (folders that no longer exist
    /// are left out — their files are gone too).
    pub(super) folders: HashSet<PathBuf>,
    /// Canonical paths of the files whose folder lies inside a root.
    pub(super) near_roots: HashSet<PathBuf>,
    /// Folders (as stored) that exist but could not be resolved; everything
    /// under them is protected.
    pub(super) unresolved: Vec<PathBuf>,
    /// `false` when a path was relative or its folder could not be resolved,
    /// so where it really is is unknown — the work folder is then not walked.
    pub(super) all_located: bool,
}

/// What resolving one folder told us.
pub(super) enum FolderState {
    Resolved(PathBuf),
    /// The folder does not exist: its files are gone too.
    Gone,
    /// It may exist but cannot be resolved (permission denied, a broken
    /// reparse point, …): its files may still be there.
    Unresolvable,
}

/// Only `NotFound` means "gone"; every other error means "unknown".
pub(super) fn classify_folder(resolved: std::io::Result<PathBuf>) -> FolderState {
    match resolved {
        Ok(p) => FolderState::Resolved(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FolderState::Gone,
        Err(_) => FolderState::Unresolvable,
    }
}

fn index_paths<'a>(raws: impl Iterator<Item = &'a str>, roots: &[&Path]) -> PathIndex {
    let mut by_folder: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut all_absolute = true;
    for raw in raws.filter(|r| !r.is_empty()).collect::<HashSet<&str>>() {
        let path = Path::new(raw);
        if !path.is_absolute() {
            all_absolute = false;
            continue;
        }
        if let Some(folder) = path.parent() {
            by_folder
                .entry(folder.to_path_buf())
                .or_default()
                .push(raw.to_string());
        }
    }
    let index = index_folders(by_folder, roots, |p| std::fs::canonicalize(p));
    PathIndex {
        all_located: index.all_located && all_absolute,
        ..index
    }
}

/// The folder half of [`index_paths`], with the resolver injectable so an
/// unresolvable folder can be tested without changing file permissions.
pub(super) fn index_folders(
    by_folder: impl IntoIterator<Item = (PathBuf, Vec<String>)>,
    roots: &[&Path],
    resolve: impl Fn(&Path) -> std::io::Result<PathBuf>,
) -> PathIndex {
    let mut folders = HashSet::new();
    let mut near_roots = HashSet::new();
    let mut unresolved = Vec::new();
    for (raw_folder, files) in by_folder {
        let folder = match classify_folder(resolve(&raw_folder)) {
            FolderState::Resolved(folder) => folder,
            FolderState::Gone => continue,
            FolderState::Unresolvable => {
                unresolved.push(raw_folder);
                continue;
            }
        };
        if roots.iter().any(|r| folder.starts_with(r)) {
            near_roots.extend(files.iter().filter_map(|f| resolve(Path::new(f)).ok()));
        }
        folders.insert(folder);
    }
    PathIndex {
        all_located: unresolved.is_empty(),
        folders,
        near_roots,
        unresolved,
    }
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

/// A recorded work folder needs at least this many ordinary path components
/// (`E:\\frames\\<job>`): a drive root or a first-level folder is never one
/// dataset's own.
const MIN_WORK_FOLDER_DEPTH: usize = 2;

/// The rules a recorded work folder `w` (canonical) must pass on its own:
/// deep enough below its drive, not overlapping the model store (compared as
/// configured and resolved, so a store that does not exist yet counts), and not
/// holding an app root (the datasets root, the outputs folder). The rules
/// against source folders and other datasets' folders are in
/// [`WorkFolders::resolve`].
pub(super) fn unfit_work_folder(w: &Path, app_roots: &[&Path], models: Option<&Path>) -> bool {
    let depth = w
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    depth < MIN_WORK_FOLDER_DEPTH
        || models.is_some_and(|m| same_or_inside(m, w) || same_or_inside(w, m))
        || app_roots.iter().any(|r| r.starts_with(w))
}

/// Every dataset's work folder: the recorded `work_dir` (Plan 10) when the
/// row has one, else `<datasets_root>/<prep_job_id>` — plus every training
/// run's folder (recorded, or `<training root>/<run id>`), which counts as
/// another owner's folder.
struct WorkFolders {
    /// This dataset's, when it exists and passes every rule.
    own: Option<PathBuf>,
    /// Other datasets' (canonical, existing) — recorded ones whether or not
    /// they would pass the rules: protecting too much is safe.
    others: Vec<PathBuf>,
    /// Other datasets' recorded folders that may exist but cannot be
    /// resolved (or were stored relative): nothing under them goes, and no
    /// folder is walked.
    unresolved: Vec<PathBuf>,
}

impl WorkFolders {
    fn resolve(
        snap: &Snapshot,
        datasets_root: Option<&Path>,
        outputs: Option<&Path>,
        models: Option<&Path>,
        source_dirs: &[PathBuf],
        training_root: &Path,
    ) -> Self {
        let derived = |d: &Dataset| -> Option<PathBuf> {
            let root = datasets_root?;
            let id = single_component(d.prep_job_id.as_deref()?)?;
            let dir = std::fs::canonicalize(root.join(id)).ok()?;
            (dir.is_dir() && strictly_inside(&dir, root)).then_some(dir)
        };
        let mut others = Vec::new();
        let mut unresolved = Vec::new();
        for d in &snap.others {
            let Some(raw) = d.work_dir.as_deref() else {
                others.extend(derived(d));
                continue;
            };
            let path = Path::new(raw);
            if !path.is_absolute() {
                unresolved.push(path.to_path_buf());
                continue;
            }
            match classify_folder(std::fs::canonicalize(path)) {
                FolderState::Resolved(dir) => others.push(dir),
                FolderState::Gone => {}
                FolderState::Unresolvable => unresolved.push(path.to_path_buf()),
            }
        }
        // Training run folders are foreign to every dataset, exactly like
        // another dataset's recorded work folder: never this dataset's own,
        // never walked into, nothing inside them deleted.
        for run in &snap.runs {
            if run.work_dir.trim().is_empty() && training_root.as_os_str().is_empty() {
                continue;
            }
            let path = crate::training::location::run_folder(run, training_root);
            if !path.is_absolute() {
                unresolved.push(path);
                continue;
            }
            match classify_folder(std::fs::canonicalize(&path)) {
                FolderState::Resolved(dir) => others.push(dir),
                FolderState::Gone => {}
                FolderState::Unresolvable => unresolved.push(path),
            }
        }
        let own = match snap.dataset.work_dir.as_deref() {
            None => derived(&snap.dataset),
            Some(raw) => {
                let app_roots: Vec<&Path> =
                    [datasets_root, outputs].into_iter().flatten().collect();
                canonical(raw).filter(|w| {
                    w.is_dir()
                        && !unfit_work_folder(w, &app_roots, models)
                        && !source_dirs.iter().any(|d| overlaps(d, w))
                        && !others.iter().any(|o| overlaps(o, w))
                })
            }
        };
        Self {
            own,
            others,
            unresolved,
        }
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
    /// Everything except the in-use set; see [`Self::keeping`]. The outputs
    /// folder decides "own export" (an export folder must sit strictly inside
    /// it); the datasets root (`AppPaths::datasets_dir`, independently
    /// overridable) is where older rows' work folders are derived; the model
    /// store and both roots are never part of a recorded work folder.
    pub(super) fn build(roots: &DataRoots, snap: &Snapshot) -> Self {
        let outputs = std::fs::canonicalize(&roots.outputs).ok();
        let datasets_root = std::fs::canonicalize(&roots.datasets).ok();
        // The store as configured, not canonicalised: it is created lazily
        // and may not exist yet. `unfit_work_folder` compares it lexically
        // and resolved (nearest existing ancestor), so a missing store still
        // counts.
        let models = Some(roots.models.as_path()).filter(|m| !m.as_os_str().is_empty());

        let source_dirs: Vec<PathBuf> = canonical_all(
            std::iter::once(snap.dataset.source_root.as_str())
                .chain(snap.others.iter().map(|d| d.source_root.as_str())),
        )
        .into_iter()
        .collect();
        let other_exports: Vec<PathBuf> =
            canonical_all(snap.others.iter().filter_map(|d| d.export_dir.as_deref()))
                .into_iter()
                .collect();
        let works = WorkFolders::resolve(
            snap,
            datasets_root.as_deref(),
            outputs.as_deref(),
            models,
            &source_dirs,
            &roots.training,
        );
        let foreign_dirs: Vec<PathBuf> =
            other_exports.iter().cloned().chain(works.others).collect();
        let work_dir = works.own;
        let export_dir = match (&outputs, &snap.dataset.export_dir) {
            (Some(o), Some(e)) => own_export(o, datasets_root.as_deref(), e, &other_exports),
            _ => None,
        };
        // A recorded work folder names the dataset's folder even without its
        // prep job; only a legacy row without both is "loose".
        let loose = snap.dataset.prep_job_id.is_none() && snap.dataset.work_dir.is_none();

        // The folders files may be deleted from. Without a prep job that is
        // any unclaimed `<datasets_root>/<X>` — and a foreign file's folder
        // claims its whole X (below), so other datasets' files need no
        // per-file check there; this dataset's own sources still do.
        let roots: Vec<&Path> = [work_dir.as_deref(), export_dir.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        let mut own_roots = roots.clone();
        if loose {
            own_roots.extend(datasets_root.as_deref());
        }
        let foreign = index_paths(
            snap.foreign_frames
                .iter()
                .flat_map(|(frame, source)| [frame.as_str(), source.as_str()]),
            &roots,
        );
        let own = index_paths(
            snap.frames.iter().map(|f| f.source_path.as_str()),
            &own_roots,
        );

        let walkable = foreign.all_located
            && own.all_located
            && works.unresolved.is_empty()
            && work_dir.as_deref().is_some_and(|w| {
                !foreign.folders.iter().any(|f| f.starts_with(w))
                    && !own.folders.iter().any(|f| f.starts_with(w))
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
            claimed.extend(foreign.folders.iter().filter_map(under));
            claimed.extend(foreign_dirs.iter().filter_map(under));
            claimed.extend(source_dirs.iter().filter_map(under));
        }

        Self {
            loose,
            outputs,
            datasets_root,
            work_dir,
            walkable,
            claimed,
            export_dir,
            own_sources: own.near_roots,
            own_source_raws: snap
                .frames
                .iter()
                .map(|f| f.source_path.clone())
                .filter(|s| !s.is_empty())
                .collect(),
            source_dirs,
            unresolved_own: own.unresolved,
            unresolved_foreign: foreign
                .unresolved
                .into_iter()
                .chain(works.unresolved)
                .collect(),
            foreign_files: foreign.near_roots,
            foreign_dirs,
            in_use: HashSet::new(),
        }
    }

    /// The same guard, also refusing every file one of `staying` (frame
    /// paths of this dataset that are not being deleted) still shows —
    /// compared by canonical path.
    pub(super) fn keeping(self, staying: &[&str]) -> Self {
        Self {
            in_use: canonical_all(staying.iter().copied()),
            ..self
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
        let own_source = self.own_sources.contains(&path)
            || raw
                .to_str()
                .is_some_and(|r| self.own_source_raws.contains(r));
        // An unresolvable folder is matched both as stored and as resolved
        // here: where it really points is unknown.
        let under = |dirs: &[PathBuf]| {
            dirs.iter()
                .any(|d| raw.starts_with(d) || path.starts_with(d))
        };
        if own_source
            || self.source_dirs.iter().any(|d| path.starts_with(d))
            || under(&self.unresolved_own)
        {
            return Verdict::Skip(skipped(raw, SKIP_SOURCE));
        }
        if self.foreign_files.contains(&path)
            || self.foreign_dirs.iter().any(|d| path.starts_with(d))
            || under(&self.unresolved_foreign)
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
