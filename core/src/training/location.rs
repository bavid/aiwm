//! Where a training run is stored (Plan 10): checking a chosen "Store run
//! in" folder before the run's own folder `<data_dir>/<run_id>` is created
//! in it. Path comparisons are the dataset location's
//! ([`crate::capability::dataset::location`]): lexical and resolved forms,
//! case-insensitive on Windows.

use std::path::{Path, PathBuf};

use crate::capability::dataset::location::{
    dataset_conflict, is_volume_root, same_or_inside, work_folder_of,
};
use crate::db::{Dataset, TrainingRun};
use crate::{CoreError, Result};

/// Everything a chosen folder is checked against.
#[derive(Debug, Clone, Copy)]
pub struct RunLocationContext<'a> {
    /// The canonical model store.
    pub store: &'a Path,
    /// Every dataset, for their source folders and work folders.
    pub datasets: &'a [Dataset],
    /// The datasets root, for older dataset rows without a recorded folder.
    pub datasets_root: &'a Path,
    /// Every training run, for their folders.
    pub runs: &'a [TrainingRun],
    /// The default training root, for a run row without a recorded folder.
    pub training_root: &'a Path,
}

/// A chosen "Store run in" folder, refused (as a
/// [`CoreError::Config`], a 400) unless:
///
/// - it is absolute and not a whole drive;
/// - it is not the model store or inside it — a run's checkpoints are not
///   library models, and the store's housekeeping must never meet them;
/// - it does not collide with any dataset by the dataset rules
///   ([`crate::capability::dataset::location::check_against_datasets`]):
///   not at, inside or holding a source folder (purging the run must never
///   be able to reach source media), not inside a dataset's work folder;
/// - it is not another run's folder or inside one — purging that run would
///   take this one with it.
///
/// Holding other runs' folders is fine: that is what the default training
/// root does, and the new run's folder is a sibling of theirs.
pub fn check_run_data_dir(data_dir: &Path, ctx: &RunLocationContext<'_>) -> Result<()> {
    if !data_dir.is_absolute() {
        return Err(CoreError::Config(format!(
            "the folder to store the training run in must be an absolute path, got {:?}",
            data_dir.display().to_string()
        )));
    }
    if is_volume_root(data_dir) {
        return Err(CoreError::Config(format!(
            "choose a folder to store the training run in, not the whole drive {}",
            data_dir.display()
        )));
    }
    if !ctx.store.as_os_str().is_empty() && same_or_inside(data_dir, ctx.store) {
        return Err(refused(
            data_dir,
            &format!("it lies inside the model store {}", ctx.store.display()),
        ));
    }
    if let Some((dataset, why)) = dataset_conflict(data_dir, ctx.datasets, ctx.datasets_root) {
        return Err(refused(
            data_dir,
            &format!("it {why} of dataset \"{dataset}\""),
        ));
    }
    for run in ctx.runs {
        let folder = run_folder(run, ctx.training_root);
        if same_or_inside(data_dir, &folder) {
            return Err(refused(
                data_dir,
                &format!(
                    "it lies inside the folder {} of training run \"{}\"",
                    folder.display(),
                    run.name
                ),
            ));
        }
    }
    Ok(())
}

/// A run's folder as recorded, or the derived `<training_root>/<id>` for a
/// row that never got one (see [`crate::training::runner::Runner::run_dir`]).
pub fn run_folder(run: &TrainingRun, training_root: &Path) -> PathBuf {
    let recorded = run.work_dir.trim();
    if recorded.is_empty() {
        training_root.join(&run.id)
    } else {
        PathBuf::from(recorded)
    }
}

fn refused(data_dir: &Path, why: &str) -> CoreError {
    CoreError::Config(format!(
        "the training run cannot be stored in {}: {why} \u{2014} choose another folder",
        data_dir.display()
    ))
}

/// Everything a run folder is re-checked against right before a purge.
#[derive(Debug, Clone, Copy)]
pub struct PurgeContext<'a> {
    pub store: &'a Path,
    pub outputs: &'a Path,
    pub datasets_root: &'a Path,
    pub training_root: &'a Path,
    pub datasets: &'a [Dataset],
    /// Every run except the one being purged.
    pub other_runs: &'a [TrainingRun],
}

/// A run folder needs at least this many ordinary path components
/// (`E:\runs\<id>`): a drive root or a first-level folder is never one run's.
const MIN_RUN_FOLDER_DEPTH: usize = 2;

/// Right before a purge deletes `dir`, the folder `run`'s row points at:
/// refuse (nothing is deleted) unless it still looks like this run's own
/// folder and nothing else. Defence in depth — the start-time checks on both
/// sides (runs here, datasets in
/// [`crate::capability::dataset::location::check_against_runs`]) keep
/// datasets and runs from nesting, but the row is only a string and the disk
/// may have changed since. `dir` must:
///
/// - be absolute and at least [`MIN_RUN_FOLDER_DEPTH`] folders below its
///   drive;
/// - be named exactly after the run id (every run folder is
///   `<chosen or default folder>/<run_id>`);
/// - exist as a real folder, not a link or junction (never followed);
/// - not overlap the model store (be it, hold it or lie inside it);
/// - not be or hold the outputs folder, the datasets root or the training
///   root. Lying *inside* them is normal — every default run folder sits in
///   the training root, and a configured training root may sit under the
///   outputs folder;
/// - not overlap any dataset's source folder or work folder (recorded, or
///   derived `<datasets_root>/<prep_job_id>`), nor any other run's folder.
///
/// Links and junctions *inside* the folder are not followed by the deletion
/// itself: `std::fs::remove_dir_all` removes a link without walking into it
/// (pinned by `purging_a_run_leaves_a_junction_target_alone` in the handler
/// tests).
pub fn check_purge_target(dir: &Path, run: &TrainingRun, ctx: &PurgeContext<'_>) -> Result<()> {
    let refuse = |why: String| -> Result<()> {
        Err(CoreError::Config(format!(
            "the folder {} of training run \"{}\" was not deleted: {why}",
            dir.display(),
            run.name
        )))
    };
    if !dir.is_absolute() {
        return refuse("its path is not absolute".into());
    }
    let depth = dir
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .count();
    if depth < MIN_RUN_FOLDER_DEPTH {
        return refuse("it is too close to the drive root".into());
    }
    if dir.file_name() != Some(std::ffi::OsStr::new(&run.id)) {
        return refuse(format!("it is not named after the run ({})", run.id));
    }
    match std::fs::symlink_metadata(dir) {
        Err(e) => return refuse(format!("it cannot be inspected: {e}")),
        Ok(meta) if is_link(&meta) => {
            return refuse("it is a link or junction, not a folder of its own".into())
        }
        Ok(meta) if !meta.is_dir() => return refuse("it is not a folder".into()),
        Ok(_) => {}
    }
    if !ctx.store.as_os_str().is_empty() && overlaps(dir, ctx.store) {
        return refuse(format!(
            "it overlaps the model store {}",
            ctx.store.display()
        ));
    }
    for (root, label) in [
        (ctx.outputs, "outputs folder"),
        (ctx.datasets_root, "datasets folder"),
        (ctx.training_root, "training folder"),
    ] {
        if !root.as_os_str().is_empty() && same_or_inside(root, dir) {
            return refuse(format!("it is or holds the {label} {}", root.display()));
        }
    }
    for d in ctx.datasets {
        let source = Path::new(&d.source_root);
        if !d.source_root.is_empty() && overlaps(dir, source) {
            return refuse(format!(
                "it overlaps the source folder {} of dataset \"{}\"",
                source.display(),
                d.name
            ));
        }
        if let Some(work) = work_folder_of(d, ctx.datasets_root) {
            if overlaps(dir, &work) {
                return refuse(format!(
                    "it overlaps the work folder {} of dataset \"{}\"",
                    work.display(),
                    d.name
                ));
            }
        }
    }
    for other in ctx.other_runs {
        let folder = run_folder(other, ctx.training_root);
        if overlaps(dir, &folder) {
            return refuse(format!(
                "it overlaps the folder {} of training run \"{}\"",
                folder.display(),
                other.name
            ));
        }
    }
    Ok(())
}

/// Either folder is, holds or lies inside the other.
fn overlaps(a: &Path, b: &Path) -> bool {
    same_or_inside(a, b) || same_or_inside(b, a)
}

/// On Windows any reparse point — symbolic links, junctions and every other
/// kind, whichever of them `std` reports as a symlink; elsewhere a symbolic
/// link.
#[cfg(windows)]
fn is_link(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DatasetMode, Preset, RunState};

    fn run(id: &str, name: &str, work_dir: &Path) -> TrainingRun {
        TrainingRun {
            id: id.into(),
            name: name.into(),
            profile_family: "flux2-klein-4b".into(),
            target_model_id: None,
            dataset_id: None,
            data_kind: DatasetMode::Frames,
            trigger_word: "t".into(),
            preset: Preset::Fast,
            hyperparams_json: "{}".into(),
            sample_prompts_json: "[]".into(),
            state: RunState::Cancelled,
            step: 0,
            total_steps: 0,
            last_loss: None,
            last_checkpoint_at: None,
            pid: None,
            work_dir: work_dir.to_string_lossy().into_owned(),
            result_model_id: None,
            error_text: None,
            created_at: String::new(),
            started_at: None,
            finished_at: None,
        }
    }

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

    /// `<tmp>/{store, outputs, outputs/datasets, training}`, the run folder
    /// `<tmp>/chosen/run-1` (created), and nothing else — every test adds the
    /// one conflict it is about.
    struct Layout {
        tmp: tempfile::TempDir,
        store: PathBuf,
        outputs: PathBuf,
        datasets_root: PathBuf,
        training: PathBuf,
        folder: PathBuf,
    }

    fn layout() -> Layout {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("chosen").join("run-1");
        std::fs::create_dir_all(&folder).unwrap();
        let outputs = tmp.path().join("outputs");
        Layout {
            store: tmp.path().join("store"),
            datasets_root: outputs.join("datasets"),
            outputs,
            training: tmp.path().join("training"),
            folder,
            tmp,
        }
    }

    impl Layout {
        fn check_with(
            &self,
            dir: &Path,
            datasets: &[Dataset],
            others: &[TrainingRun],
        ) -> Result<()> {
            check_purge_target(
                dir,
                &run("run-1", "Mine", dir),
                &PurgeContext {
                    store: &self.store,
                    outputs: &self.outputs,
                    datasets_root: &self.datasets_root,
                    training_root: &self.training,
                    datasets,
                    other_runs: others,
                },
            )
        }

        fn check(&self, dir: &Path) -> Result<()> {
            self.check_with(dir, &[], &[])
        }
    }

    fn refused(r: Result<()>, needle: &str) {
        let err = r.expect_err("the purge must be refused");
        assert!(matches!(err, CoreError::Config(_)), "{err}");
        assert!(err.to_string().contains(needle), "{needle:?} in: {err}");
    }

    #[test]
    fn the_runs_own_folder_may_be_purged() {
        let l = layout();
        assert!(l.check(&l.folder).is_ok());
        // Under the default training root too.
        let default = l.training.join("run-1");
        std::fs::create_dir_all(&default).unwrap();
        assert!(l.check(&default).is_ok());
    }

    #[test]
    fn a_relative_or_shallow_folder_is_refused() {
        let l = layout();
        refused(l.check(Path::new("run-1")), "absolute");
        let drive = l.tmp.path().ancestors().last().unwrap();
        refused(l.check(&drive.join("run-1")), "too close to the drive root");
    }

    #[test]
    fn a_folder_not_named_after_the_run_is_refused() {
        let l = layout();
        let other = l.tmp.path().join("chosen").join("run-10");
        std::fs::create_dir_all(&other).unwrap();
        refused(l.check(&other), "not named after the run");
    }

    #[test]
    fn a_folder_overlapping_the_model_store_is_refused() {
        let l = layout();
        let inside = l.store.join("run-1");
        std::fs::create_dir_all(&inside).unwrap();
        refused(l.check(&inside), "model store");
        let holding = l.tmp.path().join("run-1");
        std::fs::create_dir_all(holding.join("store")).unwrap();
        let l2 = Layout {
            store: holding.join("store"),
            ..layout()
        };
        refused(l2.check(&holding), "model store");
    }

    #[test]
    fn a_folder_holding_an_app_root_is_refused() {
        for (root, label) in [
            ("outputs", "outputs folder"),
            ("datasets", "datasets folder"),
            ("training", "training folder"),
        ] {
            let l = layout();
            let run_dir = l.tmp.path().join("x").join("run-1");
            std::fs::create_dir_all(&run_dir).unwrap();
            let inner = run_dir.join(root);
            let l = Layout {
                outputs: if root == "outputs" {
                    inner.clone()
                } else {
                    l.outputs.clone()
                },
                datasets_root: if root == "datasets" {
                    inner.clone()
                } else {
                    l.datasets_root.clone()
                },
                training: if root == "training" {
                    inner.clone()
                } else {
                    l.training.clone()
                },
                ..l
            };
            refused(l.check(&run_dir), label);
        }
    }

    #[test]
    fn a_folder_that_is_the_training_root_is_refused() {
        let l = layout();
        let root = l.tmp.path().join("t").join("run-1");
        std::fs::create_dir_all(&root).unwrap();
        let l = Layout {
            training: root.clone(),
            ..l
        };
        refused(l.check(&root), "training folder");
    }

    #[test]
    fn a_folder_overlapping_a_datasets_source_is_refused() {
        let l = layout();
        let inside_src = l.tmp.path().join("chosen");
        let holding = dataset("Held", &l.folder.join("media"), None, None);
        refused(l.check_with(&l.folder, &[holding], &[]), "Held");
        let around = dataset("Around", &inside_src, None, None);
        refused(l.check_with(&l.folder, &[around], &[]), "Around");
    }

    #[test]
    fn a_folder_overlapping_a_datasets_work_folder_is_refused() {
        let l = layout();
        let recorded = dataset(
            "Recorded",
            &l.tmp.path().join("src"),
            None,
            Some(&l.folder.join("frames")),
        );
        refused(l.check_with(&l.folder, &[recorded], &[]), "Recorded");
        let around = dataset(
            "Around",
            &l.tmp.path().join("src"),
            None,
            Some(&l.tmp.path().join("chosen")),
        );
        refused(l.check_with(&l.folder, &[around], &[]), "Around");
        // A legacy row: its folder is derived under the datasets root.
        let base = layout();
        let l = Layout {
            datasets_root: base.tmp.path().to_path_buf(),
            ..base
        };
        let legacy = dataset("Legacy", &l.tmp.path().join("src"), Some("chosen"), None);
        refused(l.check_with(&l.folder, &[legacy], &[]), "Legacy");
    }

    #[test]
    fn a_folder_overlapping_another_runs_folder_is_refused() {
        let l = layout();
        let nested = run("run-2", "Nested", &l.folder.join("run-2"));
        refused(l.check_with(&l.folder, &[], &[nested]), "Nested");
        let around = run("run-3", "Around", &l.tmp.path().join("chosen"));
        refused(l.check_with(&l.folder, &[], &[around]), "Around");
        // A row without a recorded folder: derived under the training root.
        let base = layout();
        let l = Layout {
            training: base.tmp.path().to_path_buf(),
            ..base
        };
        let derived = run("chosen", "Derived", Path::new(""));
        refused(l.check_with(&l.folder, &[], &[derived]), "Derived");
    }

    /// The run folder itself is a junction: never followed into.
    #[cfg(windows)]
    #[test]
    fn a_run_folder_that_is_a_junction_is_refused() {
        let l = layout();
        let target = l.tmp.path().join("elsewhere");
        std::fs::create_dir_all(&target).unwrap();
        let link = l.tmp.path().join("linked").join("run-1");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        if !make_junction(&link, &target) {
            eprintln!("skipped: cannot create a junction here");
            return;
        }
        refused(l.check(&link), "junction");
    }

    /// A file where the run folder should be is never deleted as one.
    #[test]
    fn a_file_named_after_the_run_is_refused() {
        let l = layout();
        let file = l.tmp.path().join("files").join("run-1");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"x").unwrap();
        refused(l.check(&file), "not a folder");
    }

    /// `mklink /J` needs no admin rights. `false` when it failed anyway.
    #[cfg(windows)]
    pub(crate) fn make_junction(link: &Path, target: &Path) -> bool {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}
