//! Where a dataset's work folder goes (Plan 10): comparing a chosen folder
//! against the source and the model store, and the free-space preflight
//! before a prep run writes anything.
//!
//! Folder relations are decided on two forms of each path and a match in
//! either counts: the lexically normalised path (`.`/`..` folded) and the
//! resolved one (the nearest existing ancestor canonicalised, links and
//! junctions followed, the rest appended). On Windows both are compared
//! case-insensitively, and trailing dots and spaces of a component are
//! dropped (`src.` and `src ` name `src`), as Win32 does for any path that
//! is not a verbatim `\\?\` one.

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
    let verbatim = matches!(
        path.components().next(),
        Some(Component::Prefix(p)) if p.kind().is_verbatim()
    );
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(name) if !verbatim => {
                let name = win32_component(name);
                if !name.is_empty() {
                    out.push(name);
                }
            }
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

/// A path component as Win32 reads it: trailing dots and spaces dropped.
#[cfg(windows)]
fn win32_component(name: &std::ffi::OsStr) -> String {
    name.to_string_lossy()
        .trim_end_matches(['.', ' '])
        .to_string()
}

#[cfg(not(windows))]
fn win32_component(name: &std::ffi::OsStr) -> std::ffi::OsString {
    name.to_os_string()
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

/// A chosen `data_dir` checked against every existing dataset. The new work
/// folder will be `<data_dir>/<new prep job id>`, so the exact rule is:
///
/// - `data_dir` must not be, or lie inside, any dataset's `source_root` —
///   frames would be written into someone's source media;
/// - `data_dir` must not hold any dataset's `source_root` — the new work
///   folder's parent would then hold source media, and a later mix-up of
///   the two could only hurt the source;
/// - `data_dir` must not be, or lie inside, any dataset's work folder — the
///   recorded `work_dir`, or for older rows the derived
///   `<datasets_root>/<prep_job_id>` — the new folder would sit inside
///   another dataset's app-owned folder, which the housekeeping guard then
///   refuses to walk and deleting that dataset could reach into.
///
/// A `data_dir` that merely *holds* other datasets' work folders is fine:
/// that is what the default datasets root does, and the new folder is just
/// a sibling of theirs. Rows with neither `work_dir` nor a single-component
/// `prep_job_id` have no work folder to collide with. The error names the
/// conflicting dataset.
pub(crate) fn check_against_datasets(
    data_dir: &Path,
    datasets: &[crate::db::Dataset],
    datasets_root: &Path,
) -> Result<()> {
    match dataset_conflict(data_dir, datasets, datasets_root) {
        Some((dataset, why)) => Err(conflict(data_dir, &dataset, &why)),
        None => Ok(()),
    }
}

/// The first dataset a folder chosen to store app data in would collide
/// with, by the rules of [`check_against_datasets`]: `(dataset name, why)`,
/// the reason phrased to follow "it". Shared with the training-run location
/// check, which words its own message around it.
pub(crate) fn dataset_conflict(
    data_dir: &Path,
    datasets: &[crate::db::Dataset],
    datasets_root: &Path,
) -> Option<(String, String)> {
    for d in datasets {
        let source = Path::new(&d.source_root);
        if !d.source_root.is_empty() && same_or_inside(data_dir, source) {
            return Some((
                d.name.clone(),
                format!("lies inside its source folder {}", source.display()),
            ));
        }
        if !d.source_root.is_empty() && same_or_inside(source, data_dir) {
            return Some((
                d.name.clone(),
                format!("holds its source folder {}", source.display()),
            ));
        }
        if let Some(work) = work_folder_of(d, datasets_root) {
            if same_or_inside(data_dir, &work) {
                return Some((
                    d.name.clone(),
                    format!("lies inside its work folder {}", work.display()),
                ));
            }
        }
    }
    None
}

/// A dataset's work folder as the guard sees it (existing or not): the
/// recorded `work_dir`, else `<datasets_root>/<prep_job_id>` when the job id
/// is one plain path component.
pub(crate) fn work_folder_of(d: &crate::db::Dataset, datasets_root: &Path) -> Option<PathBuf> {
    if let Some(work) = d.work_dir.as_deref().filter(|w| !w.is_empty()) {
        return Some(PathBuf::from(work));
    }
    let job = d.prep_job_id.as_deref()?;
    let mut parts = Path::new(job).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(_)), None) => Some(datasets_root.join(job)),
        _ => None,
    }
}

fn conflict(data_dir: &Path, dataset: &str, why: &str) -> CoreError {
    CoreError::Config(format!(
        "frames cannot be stored in {}: it {why} of dataset \"{dataset}\" \u{2014} choose \
         another folder",
        data_dir.display()
    ))
}

/// A chosen `data_dir` checked against every training run's folder — the
/// recorded `work_dir`, or for a row without one the derived
/// `<training_root>/<run id>` (skipped when `training_root` is empty). The
/// new work folder will be `<data_dir>/<new prep job id>`, so the rule is:
///
/// - `data_dir` must not be, or lie inside, any run's folder — the frames
///   would land inside the run, and purging the run (which removes its whole
///   folder) would delete them;
/// - `data_dir` *holding* run folders is fine: the new work folder is a
///   sibling of theirs, and neither can reach into the other.
///
/// The mirror rule (a run may not be stored inside a dataset's work folder)
/// is in [`crate::training::location::check_run_data_dir`]. The error names
/// the run.
pub(crate) fn check_against_runs(
    data_dir: &Path,
    runs: &[crate::db::TrainingRun],
    training_root: &Path,
) -> Result<()> {
    for run in runs {
        if run.work_dir.trim().is_empty() && training_root.as_os_str().is_empty() {
            continue;
        }
        let folder = crate::training::location::run_folder(run, training_root);
        if same_or_inside(data_dir, &folder) {
            return Err(CoreError::Config(format!(
                "frames cannot be stored in {}: it lies inside the folder {} of training run \
                 \"{}\" \u{2014} choose another folder",
                data_dir.display(),
                folder.display(),
                run.name
            )));
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Dataset, DatasetMode};

    fn dataset(name: &str, source: &Path, job: Option<&str>, work: Option<&Path>) -> Dataset {
        Dataset {
            id: format!("id-{name}"),
            name: name.into(),
            mode: DatasetMode::Frames,
            source_root: source.to_string_lossy().into_owned(),
            trigger_word: String::new(),
            prep_job_id: job.map(str::to_string),
            export_dir: None,
            work_dir: work.map(|w| w.to_string_lossy().into_owned()),
            created_at: String::new(),
        }
    }

    fn refused(r: Result<()>, name: &str) {
        let err = r.expect_err("the data_dir must be refused");
        assert!(matches!(err, CoreError::Config(_)), "{err}");
        assert!(err.to_string().contains(name), "names the dataset: {err}");
    }

    /// Layout: `<tmp>/datasets` (default root), `<tmp>/srcA` (A's source),
    /// A's recorded work folder `<tmp>/chosen/job-a`, B a legacy row whose
    /// folder is derived as `<tmp>/datasets/job-b`. Nothing needs to exist.
    struct Layout {
        tmp: tempfile::TempDir,
        root: PathBuf,
        datasets: Vec<Dataset>,
    }

    fn layout() -> Layout {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("datasets");
        let a = dataset(
            "Alpha",
            &tmp.path().join("srcA"),
            Some("job-a"),
            Some(&tmp.path().join("chosen").join("job-a")),
        );
        let b = dataset("Beta", &tmp.path().join("srcB"), Some("job-b"), None);
        Layout {
            tmp,
            root,
            datasets: vec![a, b],
        }
    }

    impl Layout {
        fn check(&self, data_dir: &Path) -> Result<()> {
            check_against_datasets(data_dir, &self.datasets, &self.root)
        }
    }

    #[test]
    fn a_data_dir_at_or_inside_another_datasets_source_is_refused() {
        let l = layout();
        refused(l.check(&l.tmp.path().join("srcA")), "Alpha");
        refused(l.check(&l.tmp.path().join("srcA").join("frames")), "Alpha");
    }

    #[test]
    fn a_data_dir_holding_another_datasets_source_is_refused() {
        let l = layout();
        // `<tmp>` holds srcA and srcB.
        refused(l.check(l.tmp.path()), "Alpha");
    }

    #[test]
    fn a_data_dir_at_or_inside_a_recorded_work_folder_is_refused() {
        let l = layout();
        let work = l.tmp.path().join("chosen").join("job-a");
        refused(l.check(&work), "Alpha");
        refused(l.check(&work.join("raw")), "Alpha");
    }

    #[test]
    fn a_data_dir_at_or_inside_a_derived_work_folder_is_refused() {
        let l = layout();
        refused(l.check(&l.root.join("job-b")), "Beta");
        refused(l.check(&l.root.join("job-b").join("deeper")), "Beta");
    }

    /// Holding other datasets' work folders is what the default root does:
    /// allowed, the new work folder `<data_dir>/<new job>` is a sibling.
    #[test]
    fn a_data_dir_merely_holding_work_folders_is_accepted() {
        let l = layout();
        assert!(l.check(&l.root).is_ok());
        assert!(l.check(&l.tmp.path().join("chosen")).is_ok());
        assert!(l.check(&l.tmp.path().join("elsewhere")).is_ok());
    }

    /// A row without prep job and without recorded folder has no work
    /// folder to collide with; a job id that is not one path component is
    /// never joined under the root.
    #[test]
    fn rows_without_a_work_folder_do_not_block() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("datasets");
        let loose = dataset("Loose", &tmp.path().join("s1"), None, None);
        let odd = dataset("Odd", &tmp.path().join("s2"), Some(r"..\escape"), None);
        let all = [loose, odd];
        assert!(check_against_datasets(&root.join("x"), &all, &root).is_ok());
        assert!(check_against_datasets(&tmp.path().join("escape"), &all, &root).is_ok());
    }

    #[test]
    fn the_store_check_works_when_the_store_does_not_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("no-store-yet");
        assert!(check_outside_store(&store.join("frames"), &store).is_err());
    }

    /// Win32 drops trailing dots and spaces from a path component, so
    /// `src.` and `src ` name the folder `src`.
    #[cfg(windows)]
    #[test]
    fn trailing_dots_and_spaces_name_the_same_folder() {
        let tmp = tempfile::tempdir().unwrap();
        // A source that does not exist: only the lexical form can match.
        let missing = tmp.path().join("missing-src");
        for spelled in [
            "missing-src.",
            "missing-src ",
            "missing-src. .",
            "missing-src.",
        ] {
            let dir = tmp.path().join(spelled).join("frames");
            assert!(same_or_inside(&dir, &missing), "{spelled:?}");
            assert!(check_data_dir(&dir, &missing).is_err(), "{spelled:?}");
        }
        // An existing source: the resolved form must agree.
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        for spelled in ["src.", "src ", "src.."] {
            let dir = tmp.path().join(spelled);
            assert!(check_data_dir(&dir, &src).is_err(), "{spelled:?}");
        }
        // A real sibling is not confused with it.
        assert!(check_data_dir(&tmp.path().join("src.d"), &src).is_ok());
    }
}
