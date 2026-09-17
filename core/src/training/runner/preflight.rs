//! Everything that must be true before a trainer process may be spawned,
//! and the resolving it does along the way: which training profile the chosen
//! model maps to, where its base weights and the dataset's export folder are,
//! and whether the GPU and the disk can take the run.
//!
//! Split out of [`super`] because it is the half of the runner with no
//! process in it — pure "may this start, and with what" decisions, every one
//! of them phrased as a sentence the user can act on.

use std::path::{Path, PathBuf};

use super::{Runner, BYTES_PER_GB, MIN_FREE_DISK_BYTES};
use crate::db::{DatasetMode, TrainingRun};
use crate::runtime::training::RUNTIME_ID as TRAINING_RUNTIME_ID;
use crate::scheduler::Scheduler;
use crate::training::bases::find_base;
use crate::training::config::Hyperparams;
use crate::training::profile::{find_for_model, find_staged_base, TrainingProfile};
use crate::training::{training_err, training_refusal};
use crate::Result;

/// Everything preflight resolved for one run — the profile it maps to and the
/// directories the rendered config points at.
#[derive(Debug, Clone)]
pub(super) struct Prepared {
    pub(super) profile: &'static TrainingProfile,
    pub(super) base_dir: PathBuf,
    pub(super) dataset_dir: PathBuf,
    pub(super) data_kind: DatasetMode,
    pub(super) hyperparams: Hyperparams,
    pub(super) prompts: Vec<String>,
}

impl Runner {
    /// Everything that must be true before a trainer process may be spawned.
    /// Every message is meant to be readable straight out of the UI.
    pub(super) async fn preflight(
        &self,
        target_model_id: Option<&str>,
        dataset_id: Option<&str>,
        hyperparams: Hyperparams,
        prompts: Vec<String>,
    ) -> Result<Prepared> {
        if !self.adapter.is_installed() {
            return Err(training_refusal(
                "the trainer is not installed yet — set it up in Settings first",
            ));
        }
        if self.adapter.env_broken() {
            return Err(training_refusal(
                "the trainer environment is broken — set it up again in Settings",
            ));
        }

        if let Some(other) = self.adapter.alive_run() {
            return Err(training_refusal(format!(
                "only one training can run at a time — run {other} is still going"
            )));
        }
        if let Some(other) = self.db.training_runs().list_recoverable().await?.first() {
            return Err(training_refusal(format!(
                "only one training can run at a time — \"{}\" is still going",
                other.name
            )));
        }

        let target_model_id = target_model_id
            .ok_or_else(|| training_refusal("this run has no target model to train on"))?;
        let model = self
            .db
            .models()
            .get(target_model_id)
            .await?
            .ok_or_else(|| training_refusal("the model you picked is no longer in the library"))?;
        let profile = find_for_model(model.family.as_deref(), &model.name, model.param_count)
            .ok_or_else(|| {
                training_refusal(format!("\"{}\" cannot be trained in AIWM yet", model.name))
            })?;

        let dataset_id =
            dataset_id.ok_or_else(|| training_refusal("this run has no dataset to learn from"))?;
        let dataset = self
            .db
            .datasets()
            .get(dataset_id)
            .await?
            .ok_or_else(|| training_refusal("the dataset you picked no longer exists"))?;
        if !profile.data_kind.accepts(dataset.mode) {
            return Err(training_refusal(format!(
                "\"{}\" learns from {} — the dataset \"{}\" holds {}",
                profile.label,
                describe_kind(profile.data_kind),
                dataset.name,
                dataset.mode.as_str()
            )));
        }
        let export = dataset
            .export_dir
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .ok_or_else(|| {
                training_refusal(format!(
                    "the dataset \"{}\" has not been exported yet — prepare it first",
                    dataset.name
                ))
            })?;
        let dataset_dir = PathBuf::from(export);
        if !dataset_dir.is_dir() {
            return Err(training_refusal(format!(
                "the export folder of \"{}\" is gone ({}) — export the dataset again",
                dataset.name,
                dataset_dir.display()
            )));
        }
        if media_count(&dataset_dir) == 0 {
            return Err(training_refusal(format!(
                "the export folder of \"{}\" contains no images or clips — export it again",
                dataset.name
            )));
        }

        let base_dir = self.base_weights_dir(profile).await?;

        let prompts: Vec<String> = prompts
            .into_iter()
            .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|p| !p.is_empty())
            .collect();
        if prompts.is_empty() {
            return Err(training_refusal(
                "at least one sample prompt is required so you can see what the run learns",
            ));
        }
        hyperparams.validate()?;

        self.check_disk()?;

        self.release_gpu().await;
        let free = self.scheduler.free_mb();
        if free < profile.vram.reserve_mb {
            return Err(training_refusal(format!(
                "not enough free video memory for \"{}\": it needs {} MB and only {} MB is \
                 free{}",
                profile.label,
                profile.vram.reserve_mb,
                free,
                self.pinned_note()
            )));
        }

        Ok(Prepared {
            profile,
            base_dir,
            dataset_dir,
            data_kind: dataset.mode,
            hyperparams,
            prompts,
        })
    }

    /// Re-derive [`Prepared`] for an existing row (start/resume of a run the
    /// user created earlier, possibly in a previous session).
    pub(super) async fn prepare_from_row(&self, run: &TrainingRun) -> Result<Prepared> {
        let hyperparams: Hyperparams = serde_json::from_str(&run.hyperparams_json)
            .map_err(|e| training_err(format!("this run's fine settings are unreadable: {e}")))?;
        let prompts: Vec<String> = serde_json::from_str(&run.sample_prompts_json)
            .map_err(|e| training_err(format!("this run's sample prompts are unreadable: {e}")))?;
        self.preflight(
            run.target_model_id.as_deref(),
            run.dataset_id.as_deref(),
            hyperparams,
            prompts,
        )
        .await
    }

    /// The library's directory model for `profile.base.role` whose folder
    /// actually holds every file the trainer needs. The completeness rule
    /// itself lives in [`find_staged_base`], shared with the profile list, so
    /// the UI's "base weights ready" badge and this check cannot drift apart.
    async fn base_weights_dir(&self, profile: &TrainingProfile) -> Result<PathBuf> {
        let candidates = self.db.models().for_role(profile.base.role).await?;
        if candidates.is_empty() {
            return Err(training_refusal(format!(
                "the base weights for \"{}\" are not in the library yet — download {} first",
                profile.label, profile.base.repo
            )));
        }
        let dir = find_staged_base(profile, &candidates)
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                training_refusal(format!(
                    "the base weights for \"{}\" are incomplete — download {} again",
                    profile.label, profile.base.repo
                ))
            })?;

        // Complete is not the same as intact. `find_staged_base` only asks
        // whether the files are there; this asks whether they are the files
        // we pinned, at their pinned sizes, with the smallest of the weights
        // hashed. A snapshot with no pinned hashes yet passes trivially --
        // see `crate::training::bases`.
        if let Some(base) = find_base(profile.family) {
            (self.verify_base)(&dir, base, false)?;
        }
        Ok(dir)
    }

    /// Free every runtime's non-pinned models so the trainer gets the GPU.
    /// Best effort: a runtime that refuses is logged, never fatal — the VRAM
    /// check right after this is what actually decides.
    async fn release_gpu(&self) {
        for rt in self.runtimes.all() {
            if rt.id() == TRAINING_RUNTIME_ID {
                continue;
            }
            for loaded in rt.loaded_models() {
                if self.scheduler.is_pinned(&loaded.model_id) {
                    continue;
                }
                if let Err(e) = rt.unload_model(&loaded.model_id).await {
                    tracing::warn!(
                        runtime = rt.id(),
                        model = %loaded.model_id,
                        error = %e,
                        "could not free a model before starting a training run"
                    );
                }
            }
        }
    }

    /// The trailing half of the "not enough VRAM" message: which pinned
    /// (agent-owned) models are still holding the card.
    fn pinned_note(&self) -> String {
        let held: Vec<String> = self
            .runtimes
            .all()
            .iter()
            .flat_map(|rt| rt.loaded_models())
            .filter(|m| self.scheduler.is_pinned(&m.model_id))
            .map(|m| m.model_id.clone())
            .collect();
        if held.is_empty() {
            String::new()
        } else {
            format!(
                " — {} is still loaded for an open session; close it first",
                held.join(", ")
            )
        }
    }

    /// Warn (never block) when the work dir's volume is nearly full. Uses the
    /// same `sysinfo`-based helper the storage report uses, so this needs no
    /// extra dependency and no `unsafe` Win32 call.
    fn check_disk(&self) -> Result<()> {
        let Some((free, _total)) = (self.free_space)(&self.data_dir) else {
            // A volume we cannot measure is not a volume we may refuse: the
            // probe misses network and mounted-folder paths, and a run the
            // user could have completed is worse than a disk-full failure
            // they can read straight off the trainer's log.
            tracing::debug!("could not determine free disk space for the training folder");
            return Ok(());
        };
        if free >= MIN_FREE_DISK_BYTES {
            return Ok(());
        }
        Err(training_refusal(format!(
            "only {} GB free on {}, at least {} GB needed for the checkpoints and \
             preview images this run writes",
            free / BYTES_PER_GB,
            volume_label(&self.data_dir),
            MIN_FREE_DISK_BYTES / BYTES_PER_GB
        )))
    }
}

/// The volume a path lives on, for the disk message: `E:` on Windows, the
/// whole path anywhere it has no drive prefix.
pub(super) fn volume_label(path: &Path) -> String {
    match path.components().next() {
        Some(std::path::Component::Prefix(prefix)) => {
            prefix.as_os_str().to_string_lossy().into_owned()
        }
        _ => path.display().to_string(),
    }
}

pub(super) fn describe_kind(kind: crate::training::profile::DataKind) -> &'static str {
    use crate::training::profile::DataKind;
    match kind {
        DataKind::Frames => "images",
        DataKind::Clips => "video clips",
        DataKind::Both => "images or video clips",
    }
}

/// Exported media files (`NNNN.<ext>`) in a dataset's export folder — the
/// captions next to them are `.txt` and do not count.
pub(super) fn media_count(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            let numbered = path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()));
            let media = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| !x.eq_ignore_ascii_case("txt"));
            numbered && media && path.is_file()
        })
        .count()
}
