//! Rendering an `ai-toolkit` `config.yaml` from a [`TrainingProfile`] and a
//! run's settings (spec §4 "Config-Rendering"). Everything goes through
//! typed structs deriving `Serialize` and `serde_yaml_ng::to_string` — never
//! string concatenation, so the document is always syntactically valid YAML
//! and keys stay in the order ai-toolkit's own example config uses.
//!
//! `serde_yaml_ng` is used instead of the original `serde_yaml`: the latter
//! is deprecated and unmaintained; `serde_yaml_ng` is the maintained fork
//! with the same API.
//!
//! One caveat the typed-struct approach cannot paper over on its own: see
//! [`normalize_yaml_floats`] below for why `lr`/`caption_dropout_rate`/
//! `ema_decay`/`guidance_scale` get a post-render fix-up pass.

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::profile::{PresetValues, TrainingProfile};
use super::{training_err, training_refusal};
use crate::db::DatasetMode;
use crate::Result;

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

/// Sane bounds for per-run hyperparameter overrides — see
/// [`Hyperparams::validate`].
const LR_RANGE: RangeInclusive<f64> = 1e-6..=1e-2;
const RANK_RANGE: RangeInclusive<u32> = 4..=128;
const STEPS_RANGE: RangeInclusive<u32> = 50..=20_000;
const RESOLUTION_RANGE: RangeInclusive<u32> = 256..=2048;

/// Optional per-run overrides on top of a profile's preset, deserialized
/// from `TrainingRun::hyperparams_json`. Every field is optional — an
/// absent field keeps the preset's own value.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Hyperparams {
    pub steps: Option<u32>,
    pub lr: Option<f64>,
    pub rank: Option<u32>,
    pub resolution: Option<u32>,
}

impl Hyperparams {
    /// Reject a present-but-out-of-range field before it ever reaches
    /// [`merge_hyperparams`]/[`render_yaml`]. An absent field is never
    /// rejected — "not overridden" is always valid.
    pub fn validate(&self) -> Result<()> {
        if let Some(lr) = self.lr {
            if !LR_RANGE.contains(&lr) {
                return Err(training_refusal(format!(
                    "lr {lr} is outside the allowed range {}..={}",
                    LR_RANGE.start(),
                    LR_RANGE.end()
                )));
            }
        }
        if let Some(rank) = self.rank {
            if !RANK_RANGE.contains(&rank) {
                return Err(training_refusal(format!(
                    "rank {rank} is outside the allowed range {}..={}",
                    RANK_RANGE.start(),
                    RANK_RANGE.end()
                )));
            }
        }
        if let Some(steps) = self.steps {
            if !STEPS_RANGE.contains(&steps) {
                return Err(training_refusal(format!(
                    "steps {steps} is outside the allowed range {}..={}",
                    STEPS_RANGE.start(),
                    STEPS_RANGE.end()
                )));
            }
        }
        if let Some(resolution) = self.resolution {
            if !RESOLUTION_RANGE.contains(&resolution) {
                return Err(training_refusal(format!(
                    "resolution {resolution} is outside the allowed range {}..={}",
                    RESOLUTION_RANGE.start(),
                    RESOLUTION_RANGE.end()
                )));
            }
        }
        Ok(())
    }
}

/// Everything [`render_yaml`] needs, already resolved by the caller: the
/// profile, the merged preset (see [`merge_hyperparams`]), and the
/// filesystem paths for base weights / dataset / work dir. `overrides` is
/// the raw per-run input the caller merged into `preset` — passed through
/// so `render_yaml` can defensively re-validate it (see
/// [`Hyperparams::validate`]) even if the caller forgot to.
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
    pub overrides: Option<Hyperparams>,
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

fn network_block(preset: &PresetValues) -> NetworkBlock {
    NetworkBlock {
        kind: "lora",
        linear: preset.rank,
        linear_alpha: preset.rank,
    }
}

fn save_block(preset: &PresetValues) -> SaveBlock {
    SaveBlock {
        dtype: SAVE_DTYPE,
        save_every: preset.save_every,
        max_step_saves_to_keep: MAX_STEP_SAVES_TO_KEEP,
    }
}

fn dataset_block(input: &RenderInput<'_>, is_clips: bool) -> DatasetBlock {
    DatasetBlock {
        folder_path: input.dataset_dir.to_string_lossy().into_owned(),
        caption_ext: CAPTION_EXT,
        caption_dropout_rate: CAPTION_DROPOUT_RATE,
        shuffle_tokens: false,
        cache_latents_to_disk: true,
        resolution: vec![input.preset.resolution],
        num_frames: is_clips.then_some(CLIP_NUM_FRAMES),
        do_i2v: is_clips.then_some(true),
    }
}

fn train_block(input: &RenderInput<'_>) -> TrainBlock {
    TrainBlock {
        batch_size: TRAIN_BATCH_SIZE,
        steps: input.preset.steps,
        gradient_accumulation_steps: GRADIENT_ACCUMULATION_STEPS,
        train_unet: true,
        train_text_encoder: false,
        gradient_checkpointing: true,
        noise_scheduler: input.profile.noise_scheduler,
        optimizer: OPTIMIZER,
        lr: input.preset.lr,
        ema_config: EmaConfigBlock {
            use_ema: true,
            ema_decay: EMA_DECAY,
        },
        dtype: TRAIN_DTYPE,
    }
}

fn model_block(input: &RenderInput<'_>) -> ModelBlock {
    let vram = &input.profile.vram;
    ModelBlock {
        name_or_path: input.base_dir.to_string_lossy().into_owned(),
        arch: input.profile.arch,
        quantize: vram.quantize,
        qtype: vram.quantize.then_some(vram.qtype),
        quantize_te: vram.quantize_te,
        low_vram: vram.low_vram,
        layer_offloading: vram.layer_offloading.then_some(true),
    }
}

fn sample_block(input: &RenderInput<'_>, is_clips: bool, prompts: Vec<String>) -> SampleBlock {
    SampleBlock {
        sampler: input.profile.noise_scheduler,
        sample_every: input.preset.sample_every,
        sample_start_step: SAMPLE_START_STEP,
        width: input.preset.resolution,
        height: input.preset.resolution,
        prompts,
        neg: "",
        seed: SAMPLE_SEED,
        walk_seed: true,
        guidance_scale: input.profile.guidance_scale,
        sample_steps: SAMPLE_STEPS,
        num_frames: is_clips.then_some(CLIP_NUM_FRAMES),
        fps: is_clips.then_some(CLIP_FPS),
    }
}

/// Collapse a raw sample prompt into a single YAML-safe line: every run of
/// whitespace (including `\n`/`\r`/`\t`) becomes one space, and leading/
/// trailing whitespace is trimmed. Returns `None` if nothing survives.
///
/// This is what guarantees the invariant [`normalize_yaml_floats`] depends
/// on — see that function's doc comment.
fn clean_prompt(raw: &str) -> Option<String> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// Render the ai-toolkit `config.yaml` document for one run.
pub fn render_yaml(input: &RenderInput<'_>) -> Result<String> {
    if let Some(overrides) = &input.overrides {
        overrides.validate()?;
    }

    let prompts: Vec<String> = input
        .prompts
        .iter()
        .filter_map(|p| clean_prompt(p))
        .collect();
    if prompts.is_empty() {
        return Err(training_refusal("at least one sample prompt is required"));
    }

    let is_clips = input.data_kind == DatasetMode::Clips;

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
                network: network_block(&input.preset),
                save: save_block(&input.preset),
                datasets: vec![dataset_block(input, is_clips)],
                train: train_block(input),
                model: model_block(input),
                sample: sample_block(input, is_clips, prompts),
            }],
        },
        meta: MetaBlock {
            name: "[name]".to_string(),
            version: "1.0",
        },
    };

    let yaml = serde_yaml_ng::to_string(&doc)
        .map_err(|e| training_err(format!("render config yaml: {e}")))?;
    Ok(normalize_yaml_floats(&yaml))
}

/// The training-relevant float keys ai-toolkit's Python side parses as
/// numbers — see [`normalize_yaml_floats`].
const YAML_FLOAT_KEYS: [&str; 4] = ["lr", "caption_dropout_rate", "ema_decay", "guidance_scale"];

/// `serde_yaml_ng` formats `f64` via `ryu`'s shortest-round-trip algorithm,
/// which switches to bare `<mantissa>e<±exp>` notation (no decimal point)
/// once the magnitude drops below roughly `1e-5` — e.g. `lr: 1e-6`. PyYAML's
/// YAML-1.1 float resolver (the reader ai-toolkit's Python side uses)
/// requires a literal `.` to recognize a scalar as a float, so a bare
/// `1e-6` round-trips back as a **string** there, silently breaking `lr`.
///
/// There is no way to avoid this through `serde`'s `Serializer` trait alone
/// — verified empirically, not assumed:
/// - `serialize_f64` always defers to `ryu::Buffer::format_finite`, which
///   uses this exponential form for any real magnitude below ~`1e-5`
///   regardless of how the value is rounded first (the decision is made on
///   the value's decimal exponent, not on precision).
/// - `serialize_str` on an already-correctly-formatted decimal string like
///   `"0.000001"` gets force-quoted by `serde_yaml_ng`'s own
///   `InferScalarStyle` visitor, because it round-trips through
///   `parse_f64` and the serializer quotes anything that would otherwise
///   read back as a non-string — the exact opposite of what we want here.
/// - `Value::Number` is a dead end too: its `Serialize` impl just calls
///   `serializer.serialize_f64(f)` on the same `f64`, reproducing the same
///   `ryu` output.
///
/// So this rewrites just the rendered lines for the four float keys ai
/// toolkit actually parses as numbers, after `serde_yaml_ng::to_string` has
/// produced an otherwise-correct document, into an explicit fixed-decimal
/// form. It runs unconditionally (not only when `ryu` chose exponential
/// notation) so every occurrence of these keys is guaranteed the same
/// "always has a dot, never an exponent" shape.
///
/// **Invariant this relies on:** it matches lines by looking for one of
/// [`YAML_FLOAT_KEYS`] immediately after the line's leading whitespace, up
/// to the first `": "`. That is only safe because every other string field
/// in the document is guaranteed single-line — in particular, sample
/// prompts are flattened by [`clean_prompt`] before they ever reach the
/// renderer. Without that guarantee, `serde_yaml_ng` would render a
/// multi-line prompt as a `|-` block scalar whose body lines are emitted
/// unquoted — the matching below strips leading whitespace before
/// comparing, so an embedded line like `lr: 5` inside such a prompt would
/// be indistinguishable from an actual `lr:` key and get corrupted into
/// `lr: 5.0`.
fn normalize_yaml_floats(yaml: &str) -> String {
    let mut out = yaml
        .lines()
        .map(|line| {
            let Some((prefix, rest)) = line.split_once(": ") else {
                return line.to_string();
            };
            let key = prefix.trim_start();
            if !YAML_FLOAT_KEYS.contains(&key) {
                return line.to_string();
            }
            match rest.parse::<f64>() {
                Ok(value) => format!("{prefix}: {}", format_plain_decimal(value)),
                Err(_) => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    out.push('\n');
    out
}

/// Format `v` as a plain decimal that always has a `.` and never an
/// exponent: fixed at 12 decimal places, trailing zeros trimmed, but at
/// least one digit is kept after the dot (`0.000001`, `0.0001`, `0.99`,
/// `4.0`).
fn format_plain_decimal(v: f64) -> String {
    let fixed = format!("{v:.12}");
    let trimmed = fixed.trim_end_matches('0');
    if let Some(stripped) = trimmed.strip_suffix('.') {
        format!("{stripped}.0")
    } else {
        trimmed.to_string()
    }
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
            overrides: None,
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
            overrides: None,
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
            overrides: None,
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

    #[test]
    fn lr_encoding_uses_a_plain_decimal_with_no_exponent() {
        let base_dir = Path::new(r"E:\Models\training\flux2-klein-4b");
        let dataset_dir = Path::new(r"E:\Data\training\anime\export");
        let work_dir = Path::new(r"E:\Data\training\runs\anime_style_v1");
        let prompts = vec!["ghibli_xy portrait".to_string()];
        let mut preset = find_for_family("flux2-klein-4b").unwrap().fast;
        preset.lr = 1e-6;
        let input = flux2_4b_input(
            preset,
            base_dir,
            dataset_dir,
            work_dir,
            "ghibli_xy",
            &prompts,
        );

        let yaml = render_yaml(&input).expect("render yaml");

        assert!(
            yaml.contains("lr: 0.000001\n"),
            "expected a bare `lr: 0.000001` line, got:\n{yaml}"
        );
        for key in YAML_FLOAT_KEYS {
            for line in yaml.lines() {
                let Some((prefix, rest)) = line.split_once(": ") else {
                    continue;
                };
                if prefix.trim_start() != key {
                    continue;
                }
                assert!(
                    !rest.contains("e-") && !rest.contains("e+"),
                    "{key}: {rest} still uses exponential notation"
                );
            }
        }

        // And it must read back as a Number (not a String) — the whole point.
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).expect("parse yaml");
        let lr = &value["config"]["process"][0]["train"]["lr"];
        assert!(lr.is_number(), "lr must parse back as a Number, got {lr:?}");
        assert_eq!(lr.as_f64(), Some(1e-6));
    }

    #[test]
    fn hyperparams_validate_rejects_out_of_range_values() {
        assert!(Hyperparams {
            lr: Some(1.0),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(Hyperparams {
            rank: Some(2),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(Hyperparams {
            steps: Some(10),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(Hyperparams {
            resolution: Some(64),
            ..Default::default()
        }
        .validate()
        .is_err());

        // In-range and absent fields are both fine.
        assert!(Hyperparams::default().validate().is_ok());
        assert!(Hyperparams {
            lr: Some(1e-4),
            rank: Some(16),
            steps: Some(600),
            resolution: Some(768),
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn render_yaml_rejects_an_out_of_range_lr_override() {
        let base_dir = Path::new(r"E:\Models\training\flux2-klein-4b");
        let dataset_dir = Path::new(r"E:\Data\training\anime\export");
        let work_dir = Path::new(r"E:\Data\training\runs\anime_style_v1");
        let prompts = vec!["ghibli_xy portrait".to_string()];
        let mut input = flux2_4b_input(
            find_for_family("flux2-klein-4b").unwrap().fast,
            base_dir,
            dataset_dir,
            work_dir,
            "ghibli_xy",
            &prompts,
        );
        input.overrides = Some(Hyperparams {
            lr: Some(1.0), // way outside [1e-6, 1e-2]
            ..Default::default()
        });

        let err = render_yaml(&input).expect_err("out-of-range lr must be rejected");
        assert!(err.to_string().contains("lr"));
    }

    #[test]
    fn multi_line_prompts_are_flattened_and_never_touched_by_the_float_normaliser() {
        let base_dir = Path::new(r"E:\Models\training\flux2-klein-4b");
        let dataset_dir = Path::new(r"E:\Data\training\anime\export");
        let work_dir = Path::new(r"E:\Data\training\runs\anime_style_v1");
        let prompts = vec!["portrait\nlr: 5\nmore".to_string()];
        let input = flux2_4b_input(
            find_for_family("flux2-klein-4b").unwrap().fast,
            base_dir,
            dataset_dir,
            work_dir,
            "ghibli_xy",
            &prompts,
        );

        let yaml = render_yaml(&input).expect("render yaml");

        // Flattened onto one quoted line — never a `|-` block scalar whose
        // body could be mistaken for a real `lr:` key.
        assert!(
            yaml.contains("- 'portrait lr: 5 more'\n"),
            "expected a single quoted, flattened prompt line, got:\n{yaml}"
        );
        assert!(!yaml.contains("|-"));

        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).expect("parse yaml");
        let prompt = &value["config"]["process"][0]["sample"]["prompts"][0];
        assert_eq!(prompt.as_str(), Some("portrait lr: 5 more"));

        // The float normaliser must not have touched the real `lr:` key.
        let lr = &value["config"]["process"][0]["train"]["lr"];
        assert_eq!(lr.as_f64(), Some(1e-4));
    }

    #[test]
    fn render_yaml_requires_at_least_one_non_empty_prompt() {
        let base_dir = Path::new(r"E:\Models\training\flux2-klein-4b");
        let dataset_dir = Path::new(r"E:\Data\training\anime\export");
        let work_dir = Path::new(r"E:\Data\training\runs\anime_style_v1");
        let prompts = vec!["   \n\t  ".to_string()];
        let input = flux2_4b_input(
            find_for_family("flux2-klein-4b").unwrap().fast,
            base_dir,
            dataset_dir,
            work_dir,
            "ghibli_xy",
            &prompts,
        );

        let err = render_yaml(&input).expect_err("an all-whitespace prompt must be rejected");
        assert!(err.to_string().contains("sample prompt"));

        let empty: Vec<String> = vec![];
        let input = flux2_4b_input(
            find_for_family("flux2-klein-4b").unwrap().fast,
            base_dir,
            dataset_dir,
            work_dir,
            "ghibli_xy",
            &empty,
        );
        assert!(render_yaml(&input).is_err());
    }
}
