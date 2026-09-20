//! Groups `media_retention` and `media_orphans`: the flat files directly in
//! the outputs folder (`<job_id>.<ext>`; subfolders — dataset work folders,
//! exports — are never flat files and never looked at here).
//!
//! A file with a `jobs.output_path` row is retention material when the
//! active [`super::RetentionPolicy`] marks it (the same selection the
//! retention sweep makes, over the files with rows only); a file without
//! any row is an orphan. A file a non-terminal job owns, and a file that is
//! a dataset frame or source (an in-place dataset), is neither.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use time::OffsetDateTime;

use super::super::outputs::{self, OutputFile};
use super::{
    capped, date_of, file_name, group, name_key, note, same_or_inside, CleanupEntry, CleanupGroup,
    Inventory, ProtectedNote, ScanContext,
};

pub(super) fn groups(
    ctx: &ScanContext,
    inv: &Inventory,
) -> (Vec<CleanupGroup>, Vec<ProtectedNote>) {
    let outputs = ctx.paths.outputs_dir();
    let empty = || {
        vec![
            group("media_retention", Vec::new()),
            group("media_orphans", Vec::new()),
        ]
    };
    if let Some(why) = ctx.inside_store_or_runtimes(&outputs) {
        return (
            empty(),
            vec![note(
                format!("Generated media (outputs) {}", outputs.display()),
                why,
            )],
        );
    }
    // An outputs folder that is a dataset's source folder holds the user's
    // media: nothing flat in it is the app's to offer.
    if let Some(info) = inv
        .datasets
        .iter()
        .find(|d| !d.dataset.source_root.is_empty())
        .filter(|d| same_or_inside(&outputs, Path::new(&d.dataset.source_root)))
    {
        return (
            empty(),
            vec![note(
                format!("Generated media (outputs) {}", outputs.display()),
                format!(
                    "lies inside the source folder of dataset \"{}\"",
                    info.dataset.name
                ),
            )],
        );
    }

    let rows = JobFiles::index(inv);
    let frame_names = frame_names_in(&outputs, inv);
    let mut with_rows = Vec::new();
    let mut orphans = Vec::new();
    let mut dataset_files = 0u64;
    for file in outputs::scan(&outputs) {
        let key = name_key(&file_name(&file.path));
        if frame_names.contains(&key) {
            dataset_files += 1;
            continue;
        }
        match rows.by_name.get(&key) {
            // A job that has not finished still owns its file.
            Some(true) => {}
            Some(false) => with_rows.push(file),
            None => orphans.push(file),
        }
    }
    let mut notes = Vec::new();
    if dataset_files > 0 {
        notes.push(note(
            format!("{dataset_files} media file(s) in {}", outputs.display()),
            "they are dataset frames or source files",
        ));
    }

    let retention = if ctx.policy.is_active() {
        outputs::files_to_delete(with_rows, ctx.policy, OffsetDateTime::now_utc())
    } else {
        Vec::new()
    };
    (
        vec![
            group("media_retention", entries(retention)),
            group("media_orphans", entries(orphans)),
        ],
        notes,
    )
}

/// The output file names the jobs table knows, keyed as names are compared,
/// with whether the owning job has not finished.
struct JobFiles {
    by_name: HashMap<String, bool>,
}

impl JobFiles {
    fn index(inv: &Inventory) -> Self {
        let mut by_name: HashMap<String, bool> = HashMap::new();
        for job in &inv.jobs {
            let Some(name) = job.output_path.as_deref().map(|p| file_name(Path::new(p))) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let active = !job.state.is_terminal();
            by_name
                .entry(name_key(&name))
                .and_modify(|a| *a |= active)
                .or_insert(active);
        }
        Self { by_name }
    }
}

/// Names of every frame or source file whose folder *is* the outputs
/// folder — an in-place dataset's files, sitting flat among the renders.
fn frame_names_in(outputs: &Path, inv: &Inventory) -> HashSet<String> {
    let mut is_outputs: HashMap<PathBuf, bool> = HashMap::new();
    let mut names = HashSet::new();
    for path in inv
        .datasets
        .iter()
        .flat_map(|d| d.frames.iter())
        .flat_map(|f| [f.frame_path.as_str(), f.source_path.as_str()])
        .filter(|p| !p.is_empty())
        .map(Path::new)
    {
        let Some(parent) = path.parent() else {
            continue;
        };
        let same = *is_outputs
            .entry(parent.to_path_buf())
            .or_insert_with(|| same_or_inside(parent, outputs) && same_or_inside(outputs, parent));
        if same {
            names.insert(name_key(&file_name(path)));
        }
    }
    names
}

/// One entry per file, by name, oldest first — what the user picks from.
fn entries(mut files: Vec<OutputFile>) -> Vec<CleanupEntry> {
    files.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    files
        .into_iter()
        .map(|f| {
            let name = file_name(&f.path);
            CleanupEntry {
                id: name.clone(),
                label: name,
                files: 1,
                bytes: f.bytes,
                rows: 0,
                detail: capped([format!("last modified {}", date_of(Some(f.modified)))]),
            }
        })
        .collect()
}
