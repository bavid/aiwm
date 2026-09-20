//! Group `finished_runs`: a run's folder as a whole when its result LoRA is
//! in the library, else its disposable files — never the final checkpoint.
//! Every folder is re-checked with the training purge check
//! ([`check_purge_target`]) right before anything in it goes, on the same
//! thread; a refusal skips the entry with its reason. That the run has
//! finished was checked before anything was touched
//! ([`super::refuse_if_busy`]).

use std::collections::HashMap;

use crate::db::{Database, Model, TrainingRun};
use crate::training::location::{check_purge_target, run_folder, PurgeContext};
use crate::Result;

use super::super::scan::runs::{disposable_files, library_copy};
use super::{
    blocking, canonical_dir, entry, not_offered, refused, remove_tree, EntryResult, ScanContext,
    Tally,
};

const GROUP: &str = "finished_runs";

pub(super) async fn apply(
    db: &Database,
    ctx: &ScanContext,
    ids: &[String],
    dry_run: bool,
) -> Result<Vec<EntryResult>> {
    let datasets = db.datasets().list().await?;
    let runs = db.training_runs().list().await?;
    let mut models: HashMap<String, Model> = HashMap::new();
    for run in runs.iter().filter(|r| ids.contains(&r.id)) {
        if let Some(model_id) = run.result_model_id.as_deref() {
            if let Some(model) = db.models().get(model_id).await? {
                models.insert(model_id.to_string(), model);
            }
        }
    }
    let ctx = ctx.clone();
    let ids = ids.to_vec();
    blocking(move || {
        let training_root = ctx.paths.training_dir();
        let outputs = ctx.paths.outputs_dir();
        let datasets_root = ctx.paths.datasets_dir();
        ids.iter()
            .map(|id| {
                let Some(run) = runs.iter().find(|r| &r.id == id) else {
                    return not_offered(GROUP, id);
                };
                let dir = run_folder(run, &training_root);
                if let Err(e) = std::fs::symlink_metadata(&dir) {
                    return refused(GROUP, id, &run.name, &dir, &e.to_string());
                }
                let others: Vec<TrainingRun> =
                    runs.iter().filter(|r| r.id != run.id).cloned().collect();
                let purge_ctx = PurgeContext {
                    store: &ctx.store,
                    outputs: &outputs,
                    datasets_root: &datasets_root,
                    training_root: &training_root,
                    datasets: &datasets,
                    other_runs: &others,
                };
                if let Err(e) = check_purge_target(&dir, run, &purge_ctx) {
                    return refused(GROUP, id, &run.name, &dir, &e.to_string());
                }
                let Some(root) = canonical_dir(&dir) else {
                    return refused(GROUP, id, &run.name, &dir, super::SKIP_NOT_A_FILE);
                };
                let model = run.result_model_id.as_deref().and_then(|m| models.get(m));
                let mut tally = Tally::default();
                let label = match library_copy(&ctx, model, &dir) {
                    Some(_) => {
                        remove_tree(&mut tally, &root, dry_run);
                        format!("{} \u{2014} whole folder", run.name)
                    }
                    None => {
                        let (files, _final_checkpoint) = disposable_files(run, &dir);
                        for f in &files {
                            tally.remove_file(&root, &f.path, dry_run);
                        }
                        format!(
                            "{} \u{2014} checkpoints, optimizer state, samples and log",
                            run.name
                        )
                    }
                };
                entry(GROUP, id, &label, tally, 0)
            })
            .collect()
    })
    .await
}
