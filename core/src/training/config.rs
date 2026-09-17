//! Rendering an `ai-toolkit` `config.yaml` from a [`TrainingProfile`] and a
//! run's settings (spec §4 "Config-Rendering"). Everything goes through
//! typed structs deriving `Serialize` and `serde_yaml_ng::to_string` — never
//! string concatenation, so the document is always syntactically valid YAML
//! and keys stay in the order ai-toolkit's own example config uses.
//!
//! `serde_yaml_ng` is used instead of the original `serde_yaml`: the latter
//! is deprecated and unmaintained; `serde_yaml_ng` is the maintained fork
//! with the same API.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::profile::{PresetValues, TrainingProfile};
use crate::db::DatasetMode;
use crate::Result;

use super::training_err;

/// Wan 2.2 5B clip defaults for this rendering task (spec appendix: "Wan 2.2
/// 5B defaults"). Named constants rather than magic numbers scattered across
/// the dataset/sample blocks below.
const CLIP_NUM_FRAMES: u32 = 33;
const CLIP_FPS: u32 = 16;

/// Fixed ai-toolkit knobs this task's contract does not vary per profile or
/// run — kept as named constants rather than inline literals.
const SAVE_DTYPE: &str = "float16";
const MAX_STEP_SAVES_TO_KEEP: u32 = 4;
const CAPTION_EXT: &str = "txt";
const CAPTION_DROPOUT_RATE: f64 = 0.05;
const TRAIN_BATCH_SIZE: u32 = 1;
const GRADIENT_ACCUMULATION_STEPS: u32 = 1;
const OPTIMIZER: &str = "adamw8bit";
const EMA_DECAY: f64 = 0.99;
const TRAIN_DTYPE: &str = "bf16";
const DEVICE: &str = "cuda:0";
const SAMPLE_START_STEP: u32 = 0;
const SAMPLE_SEED: u32 = 42;
const SAMPLE_STEPS: u32 = 20;
const DEFAULT_GUIDANCE_SCALE: f64 = 4.0;
const SDXL_GUIDANCE_SCALE: f64 = 6.0;
const SDXL_ARCH: &str = "sdxl";

/// Optional per-run overrides on top of a profile's preset, deserialized
/// from `TrainingRun::hyperparams_json`. Every field is optional — an
/// absent field keeps the preset's own value.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize, Serialize)]
pub struct Hyperparams {
    pub steps: Option<u32>,
    pub lr: Option<f64>,
    pub rank: Option<u32>,
    pub resolution: Option<u32>,
}

/// Everything [`render_yaml`] needs, already resolved by the caller: the
/// profile, the merged preset (see [`merge_hyperparams`]), and the
/// filesystem paths for base weights / dataset / work dir.
#[derive(Debug, Clone, Copy)]
pub struct RenderInput<'a> {
    pub profile: &'a TrainingProfile,
    pub run_name: &'a str,
    pub trigger_word: &'a str,
    pub preset: PresetValues,
    pub base_dir: &'a Path,
    pub dataset_dir: &'a Path,
    pub work_dir: &'a Path,
    pub prompts: &'a [String],
    pub data_kind: DatasetMode,
}

/// Apply per-run hyperparameter overrides on top of a profile's preset
/// values. `save_every`/`sample_every` are not user-overridable in this
/// task's contract, so they always come from `base`.
pub fn merge_hyperparams(base: PresetValues, overrides: &Hyperparams) -> PresetValues {
    PresetValues {
        steps: overrides.steps.unwrap_or(base.steps),
        lr: overrides.lr.unwrap_or(base.lr),
        rank: overrides.rank.unwrap_or(base.rank),
        resolution: overrides.resolution.unwrap_or(base.resolution),
        save_every: base.save_every,
        sample_every: base.sample_every,
    }
}

/// `<work_dir>/config.yaml` — where [`render_yaml`]'s output is written.
pub fn config_path(work_dir: &Path) -> PathBuf {
    work_dir.join("config.yaml")
}

/// `<work_dir>/output` — ai-toolkit's `training_folder`, where it writes
/// `<name>/<name>_<step>.safetensors` and `<name>/samples/`.
pub fn training_folder(work_dir: &Path) -> PathBuf {
    work_dir.join("output")
}

#[derive(Debug, Clone, Serialize)]
struct RootDoc {
    job: &'static str,
    config: ConfigBlock,
    meta: MetaBlock,
}

#[derive(Debug, Clone, Serialize)]
struct ConfigBlock {
    name: String,
    process: Vec<ProcessBlock>,
}

#[derive(Debug, Clone, Serialize)]
struct ProcessBlock {
    #[serde(rename = "type")]
    kind: &'static str,
    training_folder: String,
    device: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    trigger_word: Option<String>,
    network: NetworkBlock,
    save: SaveBlock,
    datasets: Vec<DatasetBlock>,
    train: TrainBlock,
    model: ModelBlock,
    sample: SampleBlock,
}

#[derive(Debug, Clone, Serialize)]
struct NetworkBlock {
    #[serde(rename = "type")]
    kind: &'static str,
    linear: u32,
    linear_alpha: u32,
}

#[derive(Debug, Clone, Serialize)]
struct SaveBlock {
    dtype: &'static str,
    save_every: u32,
    max_step_saves_to_keep: u32,
}

#[derive(Debug, Clone, Serialize)]
struct DatasetBlock {
    folder_path: String,
    caption_ext: &'static str,
    caption_dropout_rate: f64,
    shuffle_tokens: bool,
    cache_latents_to_disk: bool,
    resolution: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_frames: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    do_i2v: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct TrainBlock {
    batch_size: u32,
    steps: u32,
    gradient_accumulation_steps: u32,
    train_unet: bool,
    train_text_encoder: bool,
    gradient_checkpointing: bool,
    noise_scheduler: &'static str,
    optimizer: &'static str,
    lr: f64,
    ema_config: EmaConfigBlock,
    dtype: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct EmaConfigBlock {
    use_ema: bool,
    ema_decay: f64,
}

#[derive(Debug, Clone, Serialize)]
struct ModelBlock {
    name_or_path: String,
    arch: &'static str,
    quantize: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    qtype: Option<&'static str>,
    quantize_te: bool,
    low_vram: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    layer_offloading: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct SampleBlock {
    sampler: &'static str,
    sample_every: u32,
    sample_start_step: u32,
    width: u32,
    height: u32,
    prompts: Vec<String>,
    neg: &'static str,
    seed: u32,
    walk_seed: bool,
    guidance_scale: f64,
    sample_steps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_frames: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fps: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct MetaBlock {
    name: String,
    version: &'static str,
}

/// `true` for the profiles ai-toolkit trains with the `ddpm` scheduler
/// instead of `flowmatch` (today: SDXL only — see the appendix).
fn is_ddpm(profile: &TrainingProfile) -> bool {
    profile.arch == SDXL_ARCH
}

fn noise_scheduler(profile: &TrainingProfile) -> &'static str {
    if is_ddpm(profile) {
        "ddpm"
    } else {
        "flowmatch"
    }
}

fn guidance_scale(profile: &TrainingProfile) -> f64 {
    if is_ddpm(profile) {
        SDXL_GUIDANCE_SCALE
    } else {
        DEFAULT_GUIDANCE_SCALE
    }
}

/// Render the ai-toolkit `config.yaml` document for one run.
pub fn render_yaml(input: &RenderInput<'_>) -> Result<String> {
    let is_clips = input.data_kind == DatasetMode::Clips;
    let scheduler = noise_scheduler(input.profile);
    let vram = &input.profile.vram;

    let doc = RootDoc {
        job: "extension",
        config: ConfigBlock {
            name: input.run_name.to_string(),
            process: vec![ProcessBlock {
                kind: "sd_trainer",
                training_folder: training_folder(input.work_dir)
                    .to_string_lossy()
                    .into_owned(),
                device: DEVICE,
                trigger_word: (!input.trigger_word.is_empty())
                    .then(|| input.trigger_word.to_string()),
                network: NetworkBlock {
                    kind: "lora",
                    linear: input.preset.rank,
                    linear_alpha: input.preset.rank,
                },
                save: SaveBlock {
                    dtype: SAVE_DTYPE,
                    save_every: input.preset.save_every,
                    max_step_saves_to_keep: MAX_STEP_SAVES_TO_KEEP,
                },
                datasets: vec![DatasetBlock {
                    folder_path: input.dataset_dir.to_string_lossy().into_owned(),
                    caption_ext: CAPTION_EXT,
                    caption_dropout_rate: CAPTION_DROPOUT_RATE,
                    shuffle_tokens: false,
                    cache_latents_to_disk: true,
                    resolution: vec![input.preset.resolution],
                    num_frames: is_clips.then_some(CLIP_NUM_FRAMES),
                    do_i2v: is_clips.then_some(true),
                }],
                train: TrainBlock {
                    batch_size: TRAIN_BATCH_SIZE,
                    steps: input.preset.steps,
                    gradient_accumulation_steps: GRADIENT_ACCUMULATION_STEPS,
                    train_unet: true,
                    train_text_encoder: false,
                    gradient_checkpointing: true,
                    noise_scheduler: scheduler,
                    optimizer: OPTIMIZER,
                    lr: input.preset.lr,
                    ema_config: EmaConfigBlock {
                        use_ema: true,
                        ema_decay: EMA_DECAY,
                    },
                    dtype: TRAIN_DTYPE,
                },
                model: ModelBlock {
                    name_or_path: input.base_dir.to_string_lossy().into_owned(),
                    arch: input.profile.arch,
                    quantize: vram.quantize,
                    qtype: vram.quantize.then_some(vram.qtype),
                    quantize_te: vram.quantize_te,
                    low_vram: vram.low_vram,
                    layer_offloading: vram.layer_offloading.then_some(true),
                },
                sample: SampleBlock {
                    sampler: scheduler,
                    sample_every: input.preset.sample_every,
                    sample_start_step: SAMPLE_START_STEP,
                    width: input.preset.resolution,
                    height: input.preset.resolution,
                    prompts: input.prompts.to_vec(),
                    neg: "",
                    seed: SAMPLE_SEED,
                    walk_seed: true,
                    guidance_scale: guidance_scale(input.profile),
                    sample_steps: SAMPLE_STEPS,
                    num_frames: is_clips.then_some(CLIP_NUM_FRAMES),
                    fps: is_clips.then_some(CLIP_FPS),
                },
            }],
        },
        meta: MetaBlock {
            name: "[name]".to_string(),
            version: "1.0",
        },
    };

    serde_yaml_ng::to_string(&doc).map_err(|e| training_err(format!("render config yaml: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::training::profile::find_for_family;

    fn flux2_4b_input<'a>(
        preset: PresetValues,
        base_dir: &'a Path,
        dataset_dir: &'a Path,
        work_dir: &'a Path,
        trigger_word: &'a str,
        prompts: &'a [String],
    ) -> RenderInput<'a> {
        RenderInput {
            profile: find_for_family("flux2-klein-4b").expect("flux2-klein-4b profile"),
            run_name: "anime_style_v1",
            trigger_word,
            preset,
            base_dir,
            dataset_dir,
            work_dir,
            prompts,
            data_kind: DatasetMode::Frames,
        }
    }

    #[test]
    fn renders_the_exact_yaml_for_a_flux2_klein_4b_frames_run() {
        let profile = find_for_family("flux2-klein-4b").expect("flux2-klein-4b profile");
        let preset = profile.fast;
        let base_dir = Path::new(r"E:\Models\training\flux2-klein-4b");
        let dataset_dir = Path::new(r"E:\Data\training\anime\export");
        let work_dir = Path::new(r"E:\Data\training\runs\anime_style_v1");
        let prompts = vec![
            "ghibli_xy portrait, cinematic lighting".to_string(),
            "ghibli_xy landscape".to_string(),
        ];
        let input = flux2_4b_input(
            preset,
            base_dir,
            dataset_dir,
            work_dir,
            "ghibli_xy",
            &prompts,
        );

        let yaml = render_yaml(&input).expect("render yaml");

        let expected = r#"job: extension
config:
  name: anime_style_v1
  process:
  - type: sd_trainer
    training_folder: E:\Data\training\runs\anime_style_v1\output
    device: cuda:0
    trigger_word: ghibli_xy
    network:
      type: lora
      linear: 16
      linear_alpha: 16
    save:
      dtype: float16
      save_every: 200
      max_step_saves_to_keep: 4
    datasets:
    - folder_path: E:\Data\training\anime\export
      caption_ext: txt
      caption_dropout_rate: 0.05
      shuffle_tokens: false
      cache_latents_to_disk: true
      resolution:
      - 768
    train:
      batch_size: 1
      steps: 600
      gradient_accumulation_steps: 1
      train_unet: true
      train_text_encoder: false
      gradient_checkpointing: true
      noise_scheduler: flowmatch
      optimizer: adamw8bit
      lr: 0.0001
      ema_config:
        use_ema: true
        ema_decay: 0.99
      dtype: bf16
    model:
      name_or_path: E:\Models\training\flux2-klein-4b
      arch: flux2_klein_4b
      quantize: true
      qtype: qfloat8
      quantize_te: true
      low_vram: false
    sample:
      sampler: flowmatch
      sample_every: 200
      sample_start_step: 0
      width: 768
      height: 768
      prompts:
      - ghibli_xy portrait, cinematic lighting
      - ghibli_xy landscape
      neg: ''
      seed: 42
      walk_seed: true
      guidance_scale: 4.0
      sample_steps: 20
meta:
  name: '[name]'
  version: '1.0'
"#;

        assert_eq!(yaml, expected);
    }

    #[test]
    fn clips_runs_add_video_keys_for_wan() {
        let profile = find_for_family("wan").expect("wan profile");
        let preset = profile.fast;
        let base_dir = Path::new(r"E:\Models\training\wan22-5b");
        let dataset_dir = Path::new(r"E:\Data\training\clips\export");
        let work_dir = Path::new(r"E:\Data\training\runs\clip_run");
        let prompts = vec!["a trg_wan clip of a cat walking".to_string()];
        let input = RenderInput {
            profile,
            run_name: "clip_run",
            trigger_word: "trg_wan",
            preset,
            base_dir,
            dataset_dir,
            work_dir,
            prompts: &prompts,
            data_kind: DatasetMode::Clips,
        };

        let yaml = render_yaml(&input).expect("render yaml");
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).expect("parse yaml");
        let process = &value["config"]["process"][0];

        assert_eq!(process["datasets"][0]["num_frames"], CLIP_NUM_FRAMES);
        assert_eq!(process["datasets"][0]["do_i2v"], true);
        assert_eq!(process["sample"]["num_frames"], CLIP_NUM_FRAMES);
        assert_eq!(process["sample"]["fps"], CLIP_FPS);

        // A frames (non-clips) run must not carry any of these keys at all.
        let flux_input = flux2_4b_input(
            find_for_family("flux2-klein-4b").unwrap().fast,
            base_dir,
            dataset_dir,
            work_dir,
            "ghibli_xy",
            &prompts,
        );
        let flux_yaml = render_yaml(&flux_input).expect("render yaml");
        assert!(!flux_yaml.contains("num_frames"));
        assert!(!flux_yaml.contains("do_i2v"));
        assert!(!flux_yaml.contains("fps"));
    }

    #[test]
    fn windows_paths_round_trip_through_yaml() {
        let base_dir = Path::new(r"E:\Models\training\flux2-klein-4b");
        let dataset_dir = Path::new(r"E:\Data\training\anime\export");
        let work_dir = Path::new(r"E:\Data\training\runs\weird name\anime_style_v1");
        let prompts = vec!["ghibli_xy portrait".to_string()];
        let input = flux2_4b_input(
            find_for_family("flux2-klein-4b").unwrap().fast,
            base_dir,
            dataset_dir,
            work_dir,
            "ghibli_xy",
            &prompts,
        );

        let yaml = render_yaml(&input).expect("render yaml");
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).expect("parse yaml");
        let process = &value["config"]["process"][0];

        assert_eq!(
            process["training_folder"].as_str().unwrap(),
            training_folder(work_dir).to_string_lossy()
        );
        assert_eq!(
            process["datasets"][0]["folder_path"].as_str().unwrap(),
            dataset_dir.to_string_lossy()
        );
        assert_eq!(
            process["model"]["name_or_path"].as_str().unwrap(),
            base_dir.to_string_lossy()
        );
    }

    #[test]
    fn sdxl_uses_ddpm_and_no_quantize_keys() {
        let profile = find_for_family("sdxl").expect("sdxl profile");
        let preset = profile.fast;
        let base_dir = Path::new(r"E:\Models\training\sdxl");
        let dataset_dir = Path::new(r"E:\Data\training\portraits\export");
        let work_dir = Path::new(r"E:\Data\training\runs\sdxl_run");
        let prompts = vec!["trg_sdxl portrait".to_string()];
        let input = RenderInput {
            profile,
            run_name: "sdxl_run",
            trigger_word: "trg_sdxl",
            preset,
            base_dir,
            dataset_dir,
            work_dir,
            prompts: &prompts,
            data_kind: DatasetMode::Frames,
        };

        let yaml = render_yaml(&input).expect("render yaml");
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).expect("parse yaml");
        let process = &value["config"]["process"][0];

        assert_eq!(process["train"]["noise_scheduler"], "ddpm");
        assert_eq!(process["sample"]["sampler"], "ddpm");
        assert_eq!(process["sample"]["guidance_scale"], 6.0);
        assert!(process["model"]["qtype"].is_null());
        assert!(process["model"]["layer_offloading"].is_null());
        assert!(!yaml.contains("qtype"));
        assert!(!yaml.contains("layer_offloading"));
    }

    #[test]
    fn hyperparam_overrides_win_over_presets() {
        let profile = find_for_family("flux2-klein-4b").expect("flux2-klein-4b profile");
        let overrides = Hyperparams {
            steps: Some(1234),
            lr: Some(5e-5),
            rank: Some(64),
            resolution: None,
        };

        let merged = merge_hyperparams(profile.fast, &overrides);

        assert_eq!(merged.steps, 1234);
        assert_eq!(merged.lr, 5e-5);
        assert_eq!(merged.rank, 64);
        // Not overridden: falls back to the preset.
        assert_eq!(merged.resolution, profile.fast.resolution);
        // Never overridable in this task's contract.
        assert_eq!(merged.save_every, profile.fast.save_every);
        assert_eq!(merged.sample_every, profile.fast.sample_every);
    }

    #[test]
    fn empty_trigger_word_is_omitted() {
        let base_dir = Path::new(r"E:\Models\training\flux2-klein-4b");
        let dataset_dir = Path::new(r"E:\Data\training\anime\export");
        let work_dir = Path::new(r"E:\Data\training\runs\anime_style_v1");
        let prompts = vec!["a portrait".to_string()];
        let input = flux2_4b_input(
            find_for_family("flux2-klein-4b").unwrap().fast,
            base_dir,
            dataset_dir,
            work_dir,
            "",
            &prompts,
        );

        let yaml = render_yaml(&input).expect("render yaml");

        assert!(!yaml.contains("trigger_word"));
    }

    #[test]
    fn config_and_training_folder_paths() {
        let work_dir = Path::new(r"E:\Data\training\runs\anime_style_v1");
        assert_eq!(config_path(work_dir), work_dir.join("config.yaml"));
        assert_eq!(training_folder(work_dir), work_dir.join("output"));
    }
}
