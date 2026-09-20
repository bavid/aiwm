//! Group `finished_runs`: the work folders of training runs in a terminal
//! state. A folder is offered as a whole only when the run's result LoRA is
//! in the library (`result_model_id` names a model whose file exists,
//! outside the run folder); otherwise the final checkpoint is the only copy
//! of the result and stays (listed under *protected*), and only the numbered
//! checkpoints below it, `optimizer.pt`, the samples and `train.log` are
//! offered. Every folder is first re-checked with the training purge check
//! ([`crate::training::location::check_purge_target`]) — a folder it
//! refuses is protected with its reason. A run that has not finished is
//! protected too.

use std::path::{Path, PathBuf};

use crate::training::location::{check_purge_target, run_folder, PurgeContext};
use crate::training::progress::checkpoint_step;

use super::{
    capped, display, file_name, note, same_or_inside, walk_files, walk_totals, CleanupEntry,
    CleanupGroup, FileInfo, Inventory, ProtectedNote, ScanContext,
};

pub(super) fn group(ctx: &ScanContext, inv: &Inventory) -> (CleanupGroup, Vec<ProtectedNote>) {
    let datasets: Vec<crate::db::Dataset> =
        inv.datasets.iter().map(|d| d.dataset.clone()).collect();
    let training_root = ctx.paths.training_dir();
    let outputs = ctx.paths.outputs_dir();
    let datasets_root = ctx.paths.datasets_dir();
    let mut entries = Vec::new();
    let mut notes = Vec::new();
    for run in &inv.runs {
        if !run.state.is_terminal() {
            notes.push(note(
                format!("Training run \"{}\"", run.name),
                format!(
                    "the run is {} \u{2014} its folder stays until it finishes",
                    run.state.as_str()
                ),
            ));
            continue;
        }
        let dir = run_folder(run, &training_root);
        match std::fs::symlink_metadata(&dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                notes.push(note(
                    format!("Training run \"{}\"", run.name),
                    format!("its folder {} cannot be inspected: {e}", display(&dir)),
                ));
                continue;
            }
            Ok(_) => {}
        }
        let others: Vec<crate::db::TrainingRun> = inv
            .runs
            .iter()
            .filter(|r| r.id != run.id)
            .cloned()
            .collect();
        let purge_ctx = PurgeContext {
            store: &ctx.store,
            outputs: &outputs,
            datasets_root: &datasets_root,
            training_root: &training_root,
            datasets: &datasets,
            other_runs: &others,
        };
        if let Err(e) = check_purge_target(&dir, run, &purge_ctx) {
            notes.push(note(
                format!("Training run \"{}\"", run.name),
                e.to_string(),
            ));
            continue;
        }
        let model = run
            .result_model_id
            .as_deref()
            .and_then(|id| inv.result_models.get(id));
        match library_copy(ctx, model, &dir) {
            Some(model_name) => entries.push(whole_folder(run, &dir, &model_name)),
            None => {
                let (entry, protected) = partial(run, &dir);
                notes.extend(protected);
                entries.extend(entry);
            }
        }
    }
    (super::group("finished_runs", entries), notes)
}

/// The name of the library model that holds this run's result — only when
/// its file exists and lies outside the run folder (a row pointing into the
/// folder would make the folder the only copy). `model` is the row the
/// run's `result_model_id` names, when there is one. Shared with the apply:
/// the same decision says whether the whole folder may go.
pub(in crate::cleanup) fn library_copy(
    ctx: &ScanContext,
    model: Option<&crate::db::Model>,
    dir: &Path,
) -> Option<String> {
    let model = model?;
    let file = Path::new(&model.file_path);
    if model.file_path.is_empty() || !file.is_absolute() {
        return None;
    }
    let present = file.is_file() || file.is_dir();
    let outside = !same_or_inside(file, dir);
    // Belt and braces: a "library" file must not sit in a folder the scan
    // would offer either.
    let not_in_outputs = !same_or_inside(file, &ctx.paths.outputs_dir());
    (present && outside && not_in_outputs).then(|| model.name.clone())
}

fn whole_folder(run: &crate::db::TrainingRun, dir: &Path, model_name: &str) -> CleanupEntry {
    let (files, bytes) = walk_totals(dir);
    CleanupEntry {
        id: run.id.clone(),
        label: format!("{} \u{2014} whole folder", run.name),
        files,
        bytes,
        rows: 0,
        detail: vec![
            display(dir),
            format!("the result LoRA \"{model_name}\" is in the library"),
        ],
    }
}

/// What a finished run without a library copy can lose: every checkpoint
/// below the final one, the optimizer state, the samples and the log. The
/// final checkpoint (the unsuffixed save, else the highest-numbered one) is
/// protected as the only copy of the result.
fn partial(run: &crate::db::TrainingRun, dir: &Path) -> (Option<CleanupEntry>, Vec<ProtectedNote>) {
    let (deletable, final_checkpoint) = disposable_files(run, dir);
    let notes = final_checkpoint
        .iter()
        .map(|p| {
            note(
                format!("Training run \"{}\": {}", run.name, file_name(p)),
                "the only copy of the result \u{2014} the LoRA is not in the library",
            )
        })
        .collect();
    if deletable.is_empty() {
        return (None, notes);
    }
    let entry = CleanupEntry {
        id: run.id.clone(),
        label: format!(
            "{} \u{2014} checkpoints, optimizer state, samples and log",
            run.name
        ),
        files: deletable.len() as u64,
        bytes: deletable.iter().map(|f| f.bytes).sum(),
        rows: 0,
        detail: capped(deletable.iter().map(|f| file_name(&f.path))),
    };
    (Some(entry), notes)
}

/// The files of a finished run's folder that may go without a library copy
/// — every disposable file (see [`is_disposable`]) except the final
/// checkpoint — and that final checkpoint (the unsuffixed save, else the
/// highest-numbered one), which is the only copy of the result. Shared with
/// the apply, which re-computes it right before deleting: the final
/// checkpoint is never in the first list.
pub(in crate::cleanup) fn disposable_files(
    run: &crate::db::TrainingRun,
    dir: &Path,
) -> (Vec<FileInfo>, Option<PathBuf>) {
    let files = walk_files(dir);
    let final_checkpoint: Option<PathBuf> = files
        .iter()
        .filter_map(|f| checkpoint_step(&file_name(&f.path), &run.name).map(|rank| (rank, f)))
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, f)| f.path.clone());
    let deletable = files
        .into_iter()
        .filter(|f| Some(&f.path) != final_checkpoint.as_ref())
        .filter(|f| is_disposable(&f.path, dir, &run.name))
        .collect();
    (deletable, final_checkpoint)
}

/// A checkpoint of this run, `optimizer.pt`, anything under a `samples`
/// folder, or `train.log`. The config and anything unknown stay.
fn is_disposable(path: &Path, dir: &Path, run_name: &str) -> bool {
    let name = file_name(path);
    if checkpoint_step(&name, run_name).is_some() || name == "optimizer.pt" || name == "train.log" {
        return true;
    }
    path.strip_prefix(dir)
        .ok()
        .map(|rel| {
            rel.components()
                .any(|c| c.as_os_str().eq_ignore_ascii_case("samples"))
        })
        .unwrap_or(false)
}
