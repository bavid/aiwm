//! Continuing an existing library LoRA (Plan 11): everything preflight must
//! establish about `StartRequest::init_lora_model_id` before a run may be
//! created with it.
//!
//! Why this is strict: ai-toolkit's `network.pretrained_lora_path` fails
//! **silently**. A missing file is printed and then trained past from
//! scratch; a rank that differs from the target's is zero-padded or
//! truncated (`toolkit/network_mixins.py`); a LoRA of another architecture
//! has its keys dropped and initialises to a near-no-op. None of that is an
//! error on the trainer's side, so every one of those cases is a refusal on
//! ours, phrased as a sentence the Training tab can show as-is.

use std::path::{Path, PathBuf};

use super::settle::library_family;
use super::Runner;
use crate::db::Model;
use crate::model::{lora_rank_from_header, ModelKind};
use crate::training::profile::TrainingProfile;
use crate::training::training_refusal;
use crate::Result;

impl Runner {
    /// The file of the library LoRA `model_id`, once it is established that
    /// a run on `profile` at `rank` can actually continue it: the row and
    /// the file exist, the row is a LoRA (by its store folder, the only
    /// thing that makes a library row a LoRA), it belongs to the family the
    /// run trains, its recorded rank is the run's, and the run that produced
    /// it is not still writing it.
    pub(super) async fn check_init_lora(
        &self,
        model_id: &str,
        profile: &TrainingProfile,
        rank: u32,
    ) -> Result<PathBuf> {
        let model = self.db.models().get(model_id).await?.ok_or_else(|| {
            training_refusal("the LoRA you chose to continue is no longer in the library")
        })?;
        let path = PathBuf::from(&model.file_path);

        if !self.is_library_lora(&path) {
            return Err(training_refusal(format!(
                "\"{}\" is not a LoRA — only a LoRA from the library can be continued",
                model.name
            )));
        }
        if !path.is_file() {
            return Err(training_refusal(format!(
                "the file of \"{}\" is gone ({}) — it cannot be continued",
                model.name,
                path.display()
            )));
        }
        check_family(&model, profile)?;
        check_rank(&model, &path, rank)?;
        self.check_source_run_settled(&model).await?;
        Ok(path)
    }

    /// A row is a LoRA when its file sits under `<store>/image/loras` — the
    /// library records no kind of its own (see `ModelKind::store_subdir`).
    fn is_library_lora(&self, path: &Path) -> bool {
        path.strip_prefix(&self.store_root)
            .map(|rel| rel.starts_with(ModelKind::Lora.store_subdir()))
            .unwrap_or(false)
    }

    /// A LoRA whose `source` names a run that has not settled yet is still
    /// being written; continuing it would copy a moving target.
    async fn check_source_run_settled(&self, model: &Model) -> Result<()> {
        let Some(run_id) = model.source.strip_prefix("training:") else {
            return Ok(());
        };
        let Some(run) = self.db.training_runs().get(run_id).await? else {
            // The run was deleted from the history; the LoRA is whole.
            return Ok(());
        };
        if run.state.is_terminal() {
            return Ok(());
        }
        Err(training_refusal(format!(
            "\"{}\" is still being trained by \"{}\" ({}) — wait until that run has finished",
            model.name,
            run.name,
            run.state.as_str()
        )))
    }
}

/// The LoRA's recorded `family` must be the one the run's profile infers
/// as — ai-toolkit drops every key of a foreign architecture and starts
/// from what is left, which is nothing.
fn check_family(model: &Model, profile: &TrainingProfile) -> Result<()> {
    let wanted = library_family(profile.family);
    match model.family.as_deref() {
        Some(have) if have == wanted => Ok(()),
        Some(have) => Err(training_refusal(format!(
            "\"{}\" is a {have} LoRA — it cannot be continued on \"{}\" ({wanted})",
            model.name, profile.label
        ))),
        None => Err(training_refusal(format!(
            "\"{}\" has no family recorded — set it to {wanted} in the library first, so it \
             is known to fit \"{}\"",
            model.name, profile.label
        ))),
    }
}

/// The rank in the file must be the run's: ai-toolkit zero-pads a smaller
/// LoRA and truncates a larger one without a word.
fn check_rank(model: &Model, path: &Path, rank: u32) -> Result<()> {
    let found = lora_rank_from_header(path).map_err(|e| {
        training_refusal(format!(
            "the file of \"{}\" could not be read as a LoRA ({e})",
            model.name
        ))
    })?;
    match found {
        None => Err(training_refusal(format!(
            "\"{}\" is not a LoRA file — it holds no LoRA weights",
            model.name
        ))),
        Some(have) if have != rank => Err(training_refusal(format!(
            "\"{}\" has rank {have} — set the rank to {have} to continue it (this run asked \
             for {rank})",
            model.name
        ))),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::tests::{ready_fixture, recording_runner, reload, start_request, Fx, FAMILY};
    use super::super::{StartRequest, LOG_FILE};
    use crate::db::{DatasetMode, NewModel, NewTrainingRun, Preset, RunState};
    use crate::training::config::config_path;

    /// `<store>/image/loras` — where a library LoRA lives.
    fn lora_dir(fx: &Fx) -> PathBuf {
        fx.root.join("store").join("image").join("loras")
    }

    /// A minimal `.safetensors` whose header carries one `lora_A` down
    /// projection of `rank` — or, with `None`, a plain weight and no LoRA
    /// tensor at all.
    fn write_weights(path: &Path, rank: Option<u32>) {
        let header = match rank {
            Some(r) => serde_json::json!({
                "transformer.single_transformer_blocks.0.attn.to_q.lora_A.weight": {
                    "dtype": "BF16", "shape": [r, 3072], "data_offsets": [0, 8]
                },
                "transformer.single_transformer_blocks.0.attn.to_q.lora_B.weight": {
                    "dtype": "BF16", "shape": [3072, r], "data_offsets": [8, 16]
                }
            }),
            None => serde_json::json!({
                "transformer.single_transformer_blocks.0.attn.to_q.weight": {
                    "dtype": "BF16", "shape": [3072, 3072], "data_offsets": [0, 16]
                }
            }),
        };
        let json = serde_json::to_vec(&header).expect("encode the header");
        let mut bytes = (json.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(&json);
        bytes.extend(std::iter::repeat_n(0u8, 16));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("create the dir");
        std::fs::write(path, bytes).expect("write the weights");
    }

    async fn lora_row(
        fx: &Fx,
        name: &str,
        family: Option<&str>,
        path: &Path,
        source: &str,
    ) -> String {
        fx.db
            .models()
            .insert(NewModel {
                name: name.into(),
                family: family.map(str::to_string),
                format: "safetensors".into(),
                file_path: path.to_string_lossy().into_owned(),
                size_bytes: 1,
                source: source.into(),
                roles: vec!["lora".into()],
                ..NewModel::default()
            })
            .await
            .expect("insert the LoRA row")
            .id
    }

    /// A rank-16 flux2 LoRA in the store with its file on disk — the source
    /// every refusal test then breaks in exactly one way.
    async fn good_lora(fx: &Fx) -> (String, PathBuf) {
        let path = lora_dir(fx).join("Anime style v1_ab12cd34.safetensors");
        write_weights(&path, Some(16));
        let id = lora_row(fx, "Anime style v1", Some("flux2"), &path, "manual").await;
        (id, path)
    }

    fn continue_from(target: &str, ds: &str, lora: &str) -> StartRequest {
        StartRequest {
            init_lora_model_id: Some(lora.to_string()),
            ..start_request(target, ds)
        }
    }

    /// The refusal message for continuing `lora`, asserting nothing was
    /// created along the way.
    async fn refused(fx: &Fx, target: &str, ds: &str, lora: &str) -> String {
        let err = fx
            .runner
            .create_and_start(continue_from(target, ds, lora))
            .await
            .expect_err("preflight must refuse this LoRA");
        assert!(
            fx.db.training_runs().list().await.expect("list").is_empty(),
            "a refused continue must not leave a run row behind"
        );
        err.to_string()
    }

    fn rendered_pretrained_lora_path(work_dir: &Path) -> Option<String> {
        let yaml = std::fs::read_to_string(config_path(work_dir)).expect("the config was written");
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).expect("valid yaml");
        value["config"]["process"][0]["network"]["pretrained_lora_path"]
            .as_str()
            .map(str::to_string)
    }

    #[tokio::test]
    async fn refuses_a_lora_that_is_not_in_the_library() {
        let (fx, target, ds) = ready_fixture().await;

        let msg = refused(&fx, &target, &ds, "no-such-model").await;

        assert!(msg.contains("no longer in the library"), "{msg}");
    }

    #[tokio::test]
    async fn refuses_a_lora_whose_file_is_gone() {
        let (fx, target, ds) = ready_fixture().await;
        let path = lora_dir(&fx).join("vanished.safetensors");
        let lora = lora_row(&fx, "Vanished", Some("flux2"), &path, "manual").await;

        let msg = refused(&fx, &target, &ds, &lora).await;

        assert!(msg.contains("is gone"), "{msg}");
        assert!(msg.contains("Vanished"), "names the LoRA: {msg}");
    }

    #[tokio::test]
    async fn refuses_a_model_that_is_not_a_lora() {
        let (fx, target, ds) = ready_fixture().await;
        // Shaped like a LoRA inside, but filed as a checkpoint: what the
        // library calls it is the store folder it sits in.
        let path = fx
            .root
            .join("store")
            .join("image")
            .join("checkpoints")
            .join("Not a lora.safetensors");
        write_weights(&path, Some(16));
        let model = lora_row(&fx, "Not a lora", Some("flux2"), &path, "manual").await;

        let msg = refused(&fx, &target, &ds, &model).await;

        assert!(msg.contains("not a LoRA"), "{msg}");
    }

    #[tokio::test]
    async fn refuses_a_lora_of_another_family() {
        let (fx, target, ds) = ready_fixture().await;
        let path = lora_dir(&fx).join("sdxl_thing.safetensors");
        write_weights(&path, Some(16));
        let sdxl = lora_row(&fx, "SDXL thing", Some("sdxl"), &path, "manual").await;

        let msg = refused(&fx, &target, &ds, &sdxl).await;

        assert!(msg.contains("sdxl"), "names the LoRA's family: {msg}");
        assert!(msg.contains("flux2"), "names the run's family: {msg}");

        // No recorded family is a mismatch too — never a silent pass.
        let path = lora_dir(&fx).join("unknown_family.safetensors");
        write_weights(&path, Some(16));
        let unknown = lora_row(&fx, "Unknown family", None, &path, "manual").await;

        let msg = refused(&fx, &target, &ds, &unknown).await;

        assert!(msg.contains("family"), "{msg}");
    }

    #[tokio::test]
    async fn refuses_a_rank_that_differs_from_the_loras() {
        let (fx, target, ds) = ready_fixture().await;
        let path = lora_dir(&fx).join("rank32.safetensors");
        write_weights(&path, Some(32));
        let lora = lora_row(&fx, "Rank 32", Some("flux2"), &path, "manual").await;

        // The fast preset asks for rank 16.
        let msg = refused(&fx, &target, &ds, &lora).await;

        assert!(msg.contains("rank 32"), "names the LoRA's rank: {msg}");
        assert!(msg.contains("16"), "names the requested rank: {msg}");
    }

    #[tokio::test]
    async fn refuses_a_file_without_lora_weights() {
        let (fx, target, ds) = ready_fixture().await;
        let path = lora_dir(&fx).join("full_model.safetensors");
        write_weights(&path, None);
        let lora = lora_row(&fx, "Full model", Some("flux2"), &path, "manual").await;

        let msg = refused(&fx, &target, &ds, &lora).await;

        assert!(msg.contains("not a LoRA file"), "{msg}");
    }

    #[tokio::test]
    async fn refuses_a_lora_whose_run_is_still_going() {
        let (fx, target, ds) = ready_fixture().await;
        // A paused run: not "alive" for the one-at-a-time rule, so this is
        // the check that has to catch it.
        let earlier = fx
            .db
            .training_runs()
            .create(NewTrainingRun {
                name: "Earlier".into(),
                profile_family: FAMILY.into(),
                target_model_id: None,
                dataset_id: None,
                data_kind: DatasetMode::Frames,
                trigger_word: "tgr_xy".into(),
                preset: Preset::Fast,
                hyperparams_json: "{}".into(),
                sample_prompts_json: "[]".into(),
                work_dir: fx.root.join("elsewhere").to_string_lossy().into_owned(),
                init_lora_model_id: None,
                image_count: None,
            })
            .await
            .expect("create the earlier run");
        for next in [RunState::Running, RunState::Paused] {
            fx.db
                .training_runs()
                .set_state(&earlier.id, next)
                .await
                .expect("move the earlier run");
        }
        let path = lora_dir(&fx).join("mid_training.safetensors");
        write_weights(&path, Some(16));
        let lora = lora_row(
            &fx,
            "Mid training",
            Some("flux2"),
            &path,
            &format!("training:{}", earlier.id),
        )
        .await;

        let err = fx
            .runner
            .create_and_start(continue_from(&target, &ds, &lora))
            .await
            .expect_err("a LoRA whose run is paused must not be continued");

        let msg = err.to_string();
        assert!(msg.contains("Earlier"), "names the run: {msg}");
        assert!(msg.contains("paused"), "names its state: {msg}");
    }

    #[tokio::test]
    async fn a_continue_run_records_its_source_and_renders_pretrained_lora_path() {
        let (fx, target, ds) = ready_fixture().await;
        let (lora, path) = good_lora(&fx).await;
        let runner = recording_runner(&fx);

        // The fake interpreter cannot launch, so the run fails at the spawn
        // — after its row and its config were written.
        runner
            .create_and_start(continue_from(&target, &ds, &lora))
            .await
            .expect_err("the fake interpreter cannot actually launch");

        let runs = fx.db.training_runs().list().await.expect("list runs");
        let run = runs.first().expect("the run row exists");
        assert_eq!(
            run.init_lora_model_id.as_deref(),
            Some(lora.as_str()),
            "the source LoRA is recorded on the row"
        );
        assert_eq!(
            run.image_count,
            Some(1),
            "the export folder's one frame is what the trainer was fed"
        );
        assert_eq!(
            rendered_pretrained_lora_path(Path::new(&run.work_dir)).as_deref(),
            Some(path.to_string_lossy().as_ref()),
            "the config points ai-toolkit at the LoRA file"
        );
    }

    #[tokio::test]
    async fn a_from_scratch_run_renders_no_pretrained_lora_path() {
        let (fx, target, ds) = ready_fixture().await;
        let runner = recording_runner(&fx);

        runner
            .create_and_start(start_request(&target, &ds))
            .await
            .expect_err("the fake interpreter cannot actually launch");

        let runs = fx.db.training_runs().list().await.expect("list runs");
        let run = runs.first().expect("the run row exists");
        assert_eq!(run.init_lora_model_id, None);
        assert_eq!(
            rendered_pretrained_lora_path(Path::new(&run.work_dir)),
            None
        );
    }

    #[tokio::test]
    async fn resume_re_renders_the_pretrained_lora_path_from_the_row() {
        let (fx, target, ds) = ready_fixture().await;
        let (lora, path) = good_lora(&fx).await;
        let folder = fx.root.join("elsewhere").join("continued");
        let run = fx
            .db
            .training_runs()
            .create(NewTrainingRun {
                name: "continued".into(),
                profile_family: FAMILY.into(),
                target_model_id: Some(target.clone()),
                dataset_id: Some(ds.clone()),
                data_kind: DatasetMode::Frames,
                trigger_word: "tgr_xy".into(),
                preset: Preset::Fast,
                hyperparams_json: "{}".into(),
                sample_prompts_json: "[\"tgr_xy a cat\"]".into(),
                work_dir: folder.to_string_lossy().into_owned(),
                init_lora_model_id: Some(lora.clone()),
                image_count: Some(1),
            })
            .await
            .expect("create the run");
        std::fs::create_dir_all(&folder).expect("create the folder");
        std::fs::write(folder.join(LOG_FILE), "").expect("write the log");
        for next in [RunState::Running, RunState::Paused] {
            fx.db
                .training_runs()
                .set_state(&run.id, next)
                .await
                .expect("move the run");
        }
        let runner = recording_runner(&fx);

        runner
            .resume(&run.id)
            .await
            .expect_err("the fake interpreter cannot actually launch");

        assert_eq!(
            rendered_pretrained_lora_path(&folder).as_deref(),
            Some(path.to_string_lossy().as_ref()),
            "a resume must point at the same LoRA the run started from"
        );
        assert_eq!(
            reload(&fx, &run.id).await.init_lora_model_id.as_deref(),
            Some(lora.as_str())
        );
    }

    #[tokio::test]
    async fn image_count_is_the_export_folders_media_count() {
        let (fx, target, ds) = ready_fixture().await;
        let export = fx.root.join("export");
        for n in 2..=3 {
            std::fs::write(export.join(format!("{n:04}.png")), b"png").expect("write a frame");
            std::fs::write(export.join(format!("{n:04}.txt")), b"a cat").expect("write a caption");
        }
        let runner = recording_runner(&fx);

        runner
            .create_and_start(start_request(&target, &ds))
            .await
            .expect_err("the fake interpreter cannot actually launch");

        let runs = fx.db.training_runs().list().await.expect("list runs");
        let run = runs.first().expect("the run row exists");
        assert_eq!(
            run.image_count,
            Some(3),
            "three frames, captions not counted"
        );
    }
}
