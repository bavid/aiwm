//! Where a training run is stored (Plan 10): checking a chosen "Store run
//! in" folder before the run's own folder `<data_dir>/<run_id>` is created
//! in it. Path comparisons are the dataset location's
//! ([`crate::capability::dataset::location`]): lexical and resolved forms,
//! case-insensitive on Windows.

use std::path::{Path, PathBuf};

use crate::capability::dataset::location::{dataset_conflict, is_volume_root, same_or_inside};
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
