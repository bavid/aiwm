//! The guard's "unclaimed under the datasets root" rule applied to every
//! dataset at once (Plan 13's cleanup scan): which direct children of the
//! datasets root no dataset claims. Reports only; deleting one still goes
//! through [`super::guard`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::db::Database;
use crate::Result;

use super::{blocking, guard, DataRoots};

/// Folders directly under the datasets root that no dataset claims.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnclaimedFolders {
    /// Real folders (canonical paths) nothing claims — the work folders of
    /// datasets whose rows are gone.
    pub folders: Vec<PathBuf>,
    /// Links and junctions directly under the root: never followed, never
    /// listed as folders.
    pub links: Vec<PathBuf>,
}

/// The guard's "unclaimed under the datasets root" rule applied to every
/// dataset at once: the direct children `<datasets root>/<X>` that no
/// dataset's derived (`<root>/<prep_job_id>`) or recorded work folder, no
/// frame or source file, no export folder, no source folder and no training
/// run's folder lies in (see [`guard::claimed_under`]). Nothing outside the
/// datasets root is ever guessed; a root that does not exist yields nothing.
pub async fn unclaimed_work_folders(db: &Database, roots: &DataRoots) -> Result<UnclaimedFolders> {
    let datasets = db.datasets().list().await?;
    let frame_paths = db.dataset_frames().list_all_paths().await?;
    let runs = db.training_runs().list().await?;
    let roots = roots.clone();
    blocking(move || {
        let Ok(root) = std::fs::canonicalize(&roots.datasets) else {
            return UnclaimedFolders::default();
        };
        // Every folder that claims its `<X>`, as stored and resolved: a
        // folder that cannot be resolved still claims where it says it is.
        let raw_folders: HashSet<PathBuf> = frame_paths
            .iter()
            .flat_map(|(frame, source)| [frame.as_str(), source.as_str()])
            .filter(|p| !p.is_empty())
            .filter_map(|p| Path::new(p).parent().map(Path::to_path_buf))
            .chain(datasets.iter().map(|d| PathBuf::from(&d.source_root)))
            .chain(
                datasets
                    .iter()
                    .filter_map(|d| d.work_dir.as_deref().map(PathBuf::from)),
            )
            .chain(
                datasets
                    .iter()
                    .filter_map(|d| d.export_dir.as_deref().map(PathBuf::from)),
            )
            .chain(
                runs.iter()
                    .map(|r| crate::training::location::run_folder(r, &roots.training)),
            )
            .collect();
        let resolved: Vec<PathBuf> = raw_folders
            .iter()
            .filter_map(|p| std::fs::canonicalize(p).ok())
            .collect();
        let claimed = guard::claimed_under(
            Some(&root),
            datasets.iter().filter_map(|d| d.prep_job_id.as_deref()),
            raw_folders.iter().chain(&resolved).map(PathBuf::as_path),
        );
        let Ok(entries) = std::fs::read_dir(&root) else {
            return UnclaimedFolders::default();
        };
        let mut out = UnclaimedFolders::default();
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let path = entry.path();
            if meta.is_symlink() || crate::cleanup::is_reparse_point(&meta) {
                out.links.push(path);
                continue;
            }
            if meta.is_dir() && !claimed.contains(&entry.file_name()) {
                out.folders.push(path);
            }
        }
        out.folders.sort();
        out.links.sort();
        out
    })
    .await
}
