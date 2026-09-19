//! LoRA overview and lineage (Plan 11): which library LoRAs came out of
//! training, and the chain of runs that made each one.
//!
//! Nothing here is stored. The two links already in the database are enough:
//! a finished run imports its adapter with `models.source = "training:<run
//! id>"`, and a run that continued a library LoRA records it in
//! `training_runs.init_lora_model_id` (migration `0020`). A LoRA's lineage
//! is therefore the walk model → its run → the LoRA that run started from →
//! *its* run → … until a run started from scratch, or until a link is
//! missing (a deleted run, a deleted LoRA). The walk is computed on read,
//! from one snapshot of the library, so a chain can never be half-updated.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::db::{Database, Dataset, Model, Preset, RunState, TrainingRun};
use crate::model::{lora_rank_from_header, ModelKind};
use crate::training::config::{training_folder, Hyperparams};
use crate::training::location::run_folder;
use crate::training::progress::scan_work_dir;
use crate::training::training_err;
use crate::Result;

/// The `models.source` prefix a run's import writes: `training:<run id>`.
pub const TRAINING_SOURCE_PREFIX: &str = "training:";

/// One library LoRA as the overview lists it, with what its whole lineage
/// adds up to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoraSummary {
    pub model_id: String,
    pub name: String,
    pub family: Option<String>,
    /// `true` when the LoRA came out of a training run here (`source =
    /// training:<run id>`); `false` for one the user imported by hand.
    pub trained: bool,
    /// The rank in the safetensors header — `None` when the file cannot be
    /// read or holds no LoRA weights. Never fatal: an unreadable file is a
    /// LoRA with an unknown rank, not a broken overview.
    pub rank: Option<u32>,
    pub size_bytes: i64,
    /// When the row entered the library (`models.imported_at`).
    pub created_at: String,
    /// How many runs contributed, oldest source to this LoRA.
    pub runs: usize,
    /// Steps of the *completed* runs in the lineage — a cancelled or failed
    /// run's steps went nowhere.
    pub total_steps: i64,
    /// Images/clips the lineage's runs were fed, summed; `None` when any run
    /// predates migration `0020` and has no count, because a partial sum
    /// shown as a total would be a lie.
    pub total_images: Option<i64>,
}

/// The dataset a lineage run trained on, as far as it is still known.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineageDataset {
    pub id: String,
    pub name: String,
    pub source_root: String,
    /// The captioner the dataset's prep job used, from that job's params;
    /// `None` when the job is gone or ran without one.
    pub captioner: Option<String>,
}

/// One run in a LoRA's history — everything the history view shows for it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineageRun {
    pub run_id: String,
    pub name: String,
    pub state: RunState,
    /// `None` when the dataset row has since been deleted.
    pub dataset: Option<LineageDataset>,
    pub image_count: Option<i64>,
    pub trigger_word: String,
    pub profile_family: String,
    pub preset: Preset,
    /// The stored overrides, parsed; `None` when the stored JSON does not
    /// parse (see `hyperparams_json` for what is there).
    pub hyperparams: Option<Hyperparams>,
    pub hyperparams_json: String,
    pub step: i64,
    pub total_steps: i64,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    /// `finished_at - started_at`, when both are set and parse.
    pub duration_secs: Option<i64>,
    pub result_model_id: Option<String>,
    pub result_model_name: Option<String>,
    pub init_lora_model_id: Option<String>,
    pub init_lora_name: Option<String>,
    /// Opaque tokens for `GET /training/runs/{id}/samples/{n}`, newest
    /// checkpoint first — the same tokens the run detail hands out.
    pub samples: Vec<String>,
}

/// A LoRA with its history, oldest run first.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoraLineage {
    pub lora: LoraSummary,
    pub runs: Vec<LineageRun>,
}

/// A library row is a LoRA when its file sits under `<store>/image/loras` —
/// the library records no kind of its own (see [`ModelKind::store_subdir`]).
///
/// The same rule as the runner's private `is_library_lora` in
/// `runner/init_lora.rs`; kept separate so the overview does not need a
/// [`crate::training::runner::Runner`].
pub fn is_library_lora(store_root: &Path, file_path: &str) -> bool {
    Path::new(file_path)
        .strip_prefix(store_root)
        .map(|rel| rel.starts_with(ModelKind::Lora.store_subdir()))
        .unwrap_or(false)
}

/// The sample images ai-toolkit wrote alongside the latest checkpoint of the
/// run whose work directory is `run_dir`, newest checkpoint first. A folder
/// that cannot be scanned has no samples — the overview must never fail on
/// a run whose files are gone.
pub fn latest_sample_paths(run_dir: &Path, run_name: &str) -> Vec<PathBuf> {
    let dir = training_folder(run_dir).join(run_name);
    scan_work_dir(&dir, run_name)
        .map(|state| state.latest_samples)
        .unwrap_or_else(|e| {
            tracing::debug!(run = %run_name, error = %e, "could not scan a run's work directory");
            Vec::new()
        })
}

/// One read of everything a lineage walk can touch. Loaded once per
/// request so a chain is computed against a single consistent snapshot,
/// and so listing fifty LoRAs does not cost fifty walks' worth of queries.
struct Library {
    models: BTreeMap<String, Model>,
    runs: BTreeMap<String, TrainingRun>,
}

impl Library {
    async fn load(db: &Database) -> Result<Self> {
        let models = db
            .models()
            .list()
            .await?
            .into_iter()
            .map(|m| (m.id.clone(), m))
            .collect();
        let runs = db
            .training_runs()
            .list()
            .await?
            .into_iter()
            .map(|r| (r.id.clone(), r))
            .collect();
        Ok(Self { models, runs })
    }

    /// The run that produced `model`, if its `source` names one that still
    /// exists.
    fn run_behind(&self, model: &Model) -> Option<&TrainingRun> {
        let run_id = model.source.strip_prefix(TRAINING_SOURCE_PREFIX)?;
        self.runs.get(run_id)
    }

    /// The runs that led to `model_id`, oldest first: the model's own run,
    /// the run of the LoRA that one started from, and so on. A link that
    /// points nowhere ends the chain; a run seen twice (rows that loop)
    /// ends it too.
    fn chain(&self, model_id: &str) -> Vec<&TrainingRun> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut newest_first = Vec::new();
        let mut next_model = self.models.get(model_id);
        while let Some(run) = next_model.and_then(|m| self.run_behind(m)) {
            if !seen.insert(run.id.as_str()) {
                break;
            }
            newest_first.push(run);
            next_model = run
                .init_lora_model_id
                .as_deref()
                .and_then(|id| self.models.get(id));
        }
        newest_first.reverse();
        newest_first
    }

    fn summarize(&self, model: &Model) -> LoraSummary {
        let chain = self.chain(&model.id);
        let total_steps = chain
            .iter()
            .filter(|r| r.state == RunState::Completed)
            .map(|r| r.step)
            .sum();
        let total_images = chain
            .iter()
            .map(|r| r.image_count)
            .try_fold(0i64, |acc, n| n.map(|n| acc + n))
            .filter(|_| !chain.is_empty());
        LoraSummary {
            model_id: model.id.clone(),
            name: model.name.clone(),
            family: model.family.clone(),
            trained: model.source.starts_with(TRAINING_SOURCE_PREFIX),
            rank: rank_of(&model.file_path),
            size_bytes: model.size_bytes,
            created_at: model.imported_at.clone(),
            runs: chain.len(),
            total_steps,
            total_images,
        }
    }

    fn model_name(&self, id: Option<&str>) -> Option<String> {
        id.and_then(|id| self.models.get(id))
            .map(|m| m.name.clone())
    }
}

/// The rank in the file's header; an unreadable or non-LoRA file is simply
/// a LoRA of unknown rank.
fn rank_of(file_path: &str) -> Option<u32> {
    match lora_rank_from_header(Path::new(file_path)) {
        Ok(rank) => rank,
        Err(e) => {
            tracing::debug!(file = %file_path, error = %e, "could not read a LoRA's rank");
            None
        }
    }
}

/// Run the file-reading half of a request on the blocking pool — a header
/// per LoRA, a directory listing per run — so a library of many LoRAs never
/// stalls the async runtime (the same split as `purge_run_folder`: rows are
/// read `async`, files are read here). A pool task that does not finish is a
/// fault, not a refusal.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| training_err(format!("the LoRA overview task did not finish: {e}")))
}

/// Every library LoRA, newest first.
pub async fn list_loras(db: &Database, store_root: &Path) -> Result<Vec<LoraSummary>> {
    let library = Library::load(db).await?;
    let store_root = store_root.to_path_buf();
    blocking(move || {
        let mut loras: Vec<LoraSummary> = library
            .models
            .values()
            .filter(|m| is_library_lora(&store_root, &m.file_path))
            .map(|m| library.summarize(m))
            .collect();
        loras.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.model_id.cmp(&a.model_id))
        });
        loras
    })
    .await
}

/// A LoRA's history, oldest run first. `None` when `model_id` is not a
/// library LoRA (the route answers 404); an imported LoRA has `Some` with no
/// runs.
pub async fn lineage(
    db: &Database,
    store_root: &Path,
    training_root: &Path,
    model_id: &str,
) -> Result<Option<LoraLineage>> {
    let library = Library::load(db).await?;
    let Some(model) = library
        .models
        .get(model_id)
        .filter(|m| is_library_lora(store_root, &m.file_path))
        .cloned()
    else {
        return Ok(None);
    };
    // Rows first (async), then files (blocking): the chain's datasets and
    // their prep jobs come from the store; the samples and the rank come
    // from disk, so they go to the pool with an owned copy of the chain.
    let chain: Vec<TrainingRun> = library.chain(model_id).into_iter().cloned().collect();
    let mut datasets = Vec::with_capacity(chain.len());
    for run in &chain {
        datasets.push(dataset_of(db, run).await?);
    }
    let training_root = training_root.to_path_buf();
    blocking(move || {
        let runs = chain
            .iter()
            .zip(datasets)
            .map(|(run, dataset)| describe_run(&library, &training_root, run, dataset))
            .collect();
        Some(LoraLineage {
            lora: library.summarize(&model),
            runs,
        })
    })
    .await
}

/// The dataset `run` trained on, when its row is still there.
async fn dataset_of(db: &Database, run: &TrainingRun) -> Result<Option<LineageDataset>> {
    let Some(id) = run.dataset_id.as_deref() else {
        return Ok(None);
    };
    match db.datasets().get(id).await? {
        Some(ds) => Ok(Some(describe_dataset(db, ds).await?)),
        None => Ok(None),
    }
}

/// Everything the history shows for one run. Reads the run's sample folder,
/// so it belongs on the blocking pool.
fn describe_run(
    library: &Library,
    training_root: &Path,
    run: &TrainingRun,
    dataset: Option<LineageDataset>,
) -> LineageRun {
    let samples = (0..latest_sample_paths(&run_folder(run, training_root), &run.name).len())
        .map(|i| i.to_string())
        .collect();
    LineageRun {
        run_id: run.id.clone(),
        name: run.name.clone(),
        state: run.state,
        dataset,
        image_count: run.image_count,
        trigger_word: run.trigger_word.clone(),
        profile_family: run.profile_family.clone(),
        preset: run.preset,
        hyperparams: serde_json::from_str(&run.hyperparams_json).ok(),
        hyperparams_json: run.hyperparams_json.clone(),
        step: run.step,
        total_steps: run.total_steps,
        started_at: run.started_at.clone(),
        finished_at: run.finished_at.clone(),
        duration_secs: duration_secs(run.started_at.as_deref(), run.finished_at.as_deref()),
        result_model_id: run.result_model_id.clone(),
        result_model_name: library.model_name(run.result_model_id.as_deref()),
        init_lora_model_id: run.init_lora_model_id.clone(),
        init_lora_name: library.model_name(run.init_lora_model_id.as_deref()),
        samples,
    }
}

/// The dataset plus the captioner its prep job recorded in its params.
async fn describe_dataset(db: &Database, ds: Dataset) -> Result<LineageDataset> {
    let captioner = match ds.prep_job_id.as_deref() {
        Some(job_id) => db.jobs().get(job_id).await?.and_then(|job| {
            job.params
                .get("captioner")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        }),
        None => None,
    };
    Ok(LineageDataset {
        id: ds.id,
        name: ds.name,
        source_root: ds.source_root,
        captioner,
    })
}

/// Whole seconds between two RFC 3339 stamps; `None` unless both are there
/// and parse.
fn duration_secs(started_at: Option<&str>, finished_at: Option<&str>) -> Option<i64> {
    let started = OffsetDateTime::parse(started_at?, &Rfc3339).ok()?;
    let finished = OffsetDateTime::parse(finished_at?, &Rfc3339).ok()?;
    Some((finished - started).whole_seconds())
}
