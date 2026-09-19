//! Storage-locations report (Plan 10): every folder the app writes to, with
//! a recursive size, free/total space on its volume, and whether the
//! Settings UI may point it elsewhere. Backs `GET /storage/locations`.
//!
//! The recursive walk never follows a symlink or (on Windows) a junction /
//! reparse point — it counts the entry and moves on, exactly like the
//! dataset-housekeeping guard's own walks (`capability::dataset::
//! housekeeping::guard`) — and an unreadable entry is counted in `skipped`
//! rather than failing the whole report.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::paths::AppPaths;
use crate::{CoreError, Result};

/// One location's report row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageLocation {
    /// Stable identifier (`outputs`, `datasets`, `training`, `models`,
    /// `runtimes`, `cache`, `downloads`).
    pub key: String,
    /// Short human label for the Settings UI.
    pub label: String,
    pub path: String,
    /// Whether this location can be pointed elsewhere in Settings.
    pub configurable: bool,
    pub exists: bool,
    pub bytes: u64,
    pub files: u64,
    /// Entries the walk could not read (permission denied, a broken reparse
    /// point, a race with something deleting files) — never fatal.
    pub skipped: u64,
    pub volume_free_bytes: Option<u64>,
    pub volume_total_bytes: Option<u64>,
}

/// One location before it is measured.
struct LocationSpec {
    key: &'static str,
    label: &'static str,
    path: PathBuf,
    configurable: bool,
}

/// Every location the app writes to, in display order. `store_path` is
/// `Config::store_path` (the model store) — not part of `AppPaths` itself.
fn location_specs(paths: &AppPaths, store_path: &Path) -> Vec<LocationSpec> {
    vec![
        LocationSpec {
            key: "outputs",
            label: "Generated media (outputs)",
            path: paths.outputs_dir(),
            configurable: true,
        },
        LocationSpec {
            key: "datasets",
            label: "Dataset work folders",
            path: paths.datasets_dir(),
            configurable: true,
        },
        LocationSpec {
            key: "training",
            label: "Training runs",
            path: paths.training_dir(),
            configurable: true,
        },
        LocationSpec {
            key: "models",
            label: "Model store",
            path: store_path.to_path_buf(),
            configurable: true,
        },
        LocationSpec {
            key: "runtimes",
            label: "Managed runtime installs",
            path: paths.runtimes_dir(),
            configurable: true,
        },
        LocationSpec {
            key: "cache",
            label: "Disposable cache",
            path: paths.cache_dir(),
            configurable: true,
        },
        LocationSpec {
            key: "downloads",
            label: "Download staging",
            path: paths.downloads_dir(),
            configurable: false,
        },
    ]
}

/// The full report: one row per [`location_specs`], each measured on disk.
/// Blocking (a recursive walk) — the caller runs this in `spawn_blocking`;
/// [`report`] does that for you.
fn measure_all(paths: &AppPaths, store_path: &Path) -> Vec<StorageLocation> {
    location_specs(paths, store_path)
        .into_iter()
        .map(measure_one)
        .collect()
}

fn measure_one(spec: LocationSpec) -> StorageLocation {
    let walked = walk(&spec.path);
    let volume_target = if walked.exists {
        spec.path.clone()
    } else {
        nearest_existing_ancestor(&spec.path)
    };
    let (volume_free_bytes, volume_total_bytes) = match super::volume_free(&volume_target) {
        Some((free, total)) => (Some(free), Some(total)),
        None => (None, None),
    };
    StorageLocation {
        key: spec.key.to_string(),
        label: spec.label.to_string(),
        path: spec.path.display().to_string(),
        configurable: spec.configurable,
        exists: walked.exists,
        bytes: walked.bytes,
        files: walked.files,
        skipped: walked.skipped,
        volume_free_bytes,
        volume_total_bytes,
    }
}

/// The nearest ancestor of `path` that exists — for free-space lookups
/// against a folder that hasn't been created yet. Falls back to `path`
/// itself if nothing on the way up exists either (nothing sensible left to
/// report, but never a panic).
fn nearest_existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path;
    loop {
        if current.exists() {
            return current.to_path_buf();
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent,
            _ => return current.to_path_buf(),
        }
    }
}

/// What a recursive walk of one folder found.
#[derive(Debug, Default, PartialEq, Eq)]
struct WalkTotals {
    exists: bool,
    bytes: u64,
    files: u64,
    skipped: u64,
}

/// Recursively sum the regular files under `dir`. Never follows a symlink or
/// (on Windows) a junction/reparse point — such an entry is simply not
/// descended into or counted as a file. An entry that cannot be read
/// (permission, a race) is counted in `skipped`, never fatal. A missing
/// `dir` reports `exists: false` and all-zero totals.
fn walk(dir: &Path) -> WalkTotals {
    if !dir.is_dir() {
        return WalkTotals::default();
    }
    let mut totals = WalkTotals {
        exists: true,
        ..WalkTotals::default()
    };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = match std::fs::read_dir(&current) {
            Ok(e) => e,
            Err(_) => {
                totals.skipped += 1;
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    totals.skipped += 1;
                    continue;
                }
            };
            // `DirEntry::metadata` does not follow a symlink (unlike
            // `Path::metadata`) on every platform we build for.
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => {
                    totals.skipped += 1;
                    continue;
                }
            };
            if meta.is_symlink() || is_reparse_point(&meta) {
                continue;
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                totals.bytes += meta.len();
                totals.files += 1;
            } else {
                totals.skipped += 1;
            }
        }
    }
    totals
}

/// A Windows junction/mount point is a reparse point but is not reported by
/// [`std::fs::Metadata::is_symlink`] (that only covers `IO_REPARSE_TAG_
/// SYMLINK`) — check the raw attribute bit directly. No `unsafe`: this is a
/// plain bitwise read of a value `std` already computed.
#[cfg(windows)]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    false
}

/// `GET /storage/locations` — every configured and fixed location, measured
/// on disk. Runs the recursive walk in `spawn_blocking` so a large outputs
/// or model-store folder never stalls the async runtime.
pub async fn report(paths: &AppPaths, store_path: &Path) -> Result<Vec<StorageLocation>> {
    let paths = paths.clone();
    let store_path = store_path.to_path_buf();
    tokio::task::spawn_blocking(move || measure_all(&paths, &store_path))
        .await
        .map_err(|e| CoreError::Config(format!("storage-locations task did not finish: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_counts_nested_files_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.bin"), vec![0u8; 10]).unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("b.bin"), vec![0u8; 20]).unwrap();
        let deeper = sub.join("deeper");
        std::fs::create_dir_all(&deeper).unwrap();
        std::fs::write(deeper.join("c.bin"), vec![0u8; 5]).unwrap();

        let totals = walk(tmp.path());

        assert!(totals.exists);
        assert_eq!(totals.bytes, 35);
        assert_eq!(totals.files, 3);
        assert_eq!(totals.skipped, 0);
    }

    #[test]
    fn walk_of_a_missing_folder_reports_not_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");

        let totals = walk(&missing);

        assert!(!totals.exists);
        assert_eq!(totals.bytes, 0);
        assert_eq!(totals.files, 0);
    }

    #[test]
    fn walk_does_not_follow_a_symlinked_file() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target.bin");
        std::fs::write(&target, vec![0u8; 1000]).unwrap();
        let link = tmp.path().join("link.bin");

        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_file(&target, &link);
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(&target, &link);

        if created.is_err() {
            // No symlink privilege on this machine (Windows needs Developer
            // Mode or admin) -- nothing more this test can prove here.
            return;
        }

        let totals = walk(tmp.path());

        // Only `target.bin` is counted; the symlink is skipped outright (not
        // walked, not sized), so total bytes must not double-count it.
        assert_eq!(totals.bytes, 1000);
        assert_eq!(totals.files, 1);
    }

    #[test]
    fn nearest_existing_ancestor_climbs_to_the_first_real_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("a").join("b").join("c");

        assert_eq!(nearest_existing_ancestor(&missing), tmp.path());
    }

    #[test]
    fn nearest_existing_ancestor_of_an_existing_path_is_itself() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(nearest_existing_ancestor(tmp.path()), tmp.path());
    }

    #[tokio::test]
    async fn report_covers_every_key_and_flags_configurability() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path().join("aiwm"));
        let store = tmp.path().join("models");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("weights.bin"), vec![0u8; 42]).unwrap();

        let rows = report(&paths, &store).await.unwrap();

        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "outputs",
                "datasets",
                "training",
                "models",
                "runtimes",
                "cache",
                "downloads"
            ]
        );
        for row in &rows {
            let expect_configurable = row.key != "downloads";
            assert_eq!(
                row.configurable, expect_configurable,
                "{} configurable flag",
                row.key
            );
        }
        let models_row = rows.iter().find(|r| r.key == "models").unwrap();
        assert!(models_row.exists);
        assert_eq!(models_row.bytes, 42);
        assert_eq!(models_row.files, 1);

        let outputs_row = rows.iter().find(|r| r.key == "outputs").unwrap();
        assert!(!outputs_row.exists, "nothing has written to outputs yet");
        assert_eq!(outputs_row.bytes, 0);
    }
}
