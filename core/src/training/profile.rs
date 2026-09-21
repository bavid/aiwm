//! The training profile registry (spec §1 "Trainings-Profile"): one entry
//! per trainable model family, mapping it to an `ai-toolkit` architecture,
//! a VRAM strategy, the base weights the runtime needs staged locally, and
//! the three preset (fast/balanced/thorough) starting points a run can pick.
//!
//! A library model whose `family` has no entry here is "not yet trainable"
//! in the UI — adding support for a new family is a new [`PROFILES`] entry
//! plus tests, never a new subsystem.

use std::path::Path;

use crate::capability::dataset::CaptionOrder;
use crate::db::{DatasetMode, Model, Preset};

/// Plain-English VRAM headroom, shown in the UI before a run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    Comfortable,
    AtTheEdge,
}

impl Fit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Comfortable => "fits comfortably",
            Self::AtTheEdge => "at the edge",
        }
    }
}

/// What kind of dataset a profile trains from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataKind {
    Frames,
    Clips,
    Both,
}

impl DataKind {
    /// Whether a dataset in `mode` can feed this profile.
    pub fn accepts(self, mode: DatasetMode) -> bool {
        match (self, mode) {
            (Self::Both, _) => true,
            (Self::Frames, DatasetMode::Frames) => true,
            (Self::Clips, DatasetMode::Clips) => true,
            (Self::Frames, DatasetMode::Clips) | (Self::Clips, DatasetMode::Frames) => false,
        }
    }
}

/// Fixed `ai-toolkit` low-VRAM knobs for a profile, plus the plain-English
/// [`Fit`] shown to the user before they start a run.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct VramStrategy {
    pub quantize: bool,
    pub qtype: &'static str,
    pub quantize_te: bool,
    pub low_vram: bool,
    pub layer_offloading: bool,
    pub fit: Fit,
    pub reserve_mb: u64,
}

/// What a real training run on real hardware actually cost.
///
/// Deliberately not an estimate and not a projection: a profile may only
/// carry this once a run of that family has finished on this machine and the
/// numbers have been read off it. Everything else a profile claims (the VRAM
/// strategy, the preset step counts) is a starting point someone chose;
/// this is the part that was measured.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Measured {
    /// Wall-clock seconds per training step, averaged over the whole run,
    /// at the profile's **Fast** preset resolution.
    pub s_per_step: f64,
    /// Peak VRAM the **run itself** held, i.e. the card's measured peak minus
    /// what was already in use before it started.
    ///
    /// Deliberately not the raw card peak, even though that is the simpler
    /// measurement: this number exists to be compared against
    /// [`VramStrategy::reserve_mb`], and that is a demand for *free* VRAM, so
    /// the desktop's own hundreds of megabytes must not be counted on both
    /// sides. The raw figures behind each value are in the comment next to
    /// it, so nothing measured is thrown away.
    pub peak_vram_mb: u64,
    /// ISO date of the run the numbers come from.
    pub date: &'static str,
}

/// Rank/LR/resolution/step starting points for one of the three presets.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct PresetValues {
    pub steps: u32,
    pub lr: f64,
    pub rank: u32,
    pub resolution: u32,
    pub save_every: u32,
    pub sample_every: u32,
}

/// One base checkpoint the training runtime needs staged locally (via the
/// same local HF-cache mechanism as `sidecar/dia.py::_stage_local_hub_cache`)
/// before a run can start. `role` is the directory model's role string in
/// the library, e.g. `training_base_flux2_klein_4b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct BaseWeight {
    pub repo: &'static str,
    pub role: &'static str,
    pub required_files: &'static [&'static str],
    pub approx_gb: u32,
}

/// One trainable model family: `ai-toolkit` architecture, VRAM strategy,
/// base weights, caption order for export, and the three presets.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct TrainingProfile {
    /// Matches the library's `models.family` value for a target model of
    /// this family (see [`find_for_family`]).
    pub family: &'static str,
    pub label: &'static str,
    /// The `ai-toolkit` architecture id (e.g. `flux2_klein_9b`, `wan22_5b`).
    pub arch: &'static str,
    pub data_kind: DataKind,
    pub vram: VramStrategy,
    pub base: BaseWeight,
    /// `train.noise_scheduler`/`sample.sampler` in the rendered config
    /// (`core::training::config`): `flowmatch` for the three flow-matching
    /// models, `ddpm` for SDXL — see the plan's appendix.
    pub noise_scheduler: &'static str,
    /// `sample.guidance_scale` in the rendered config: `4.0` for the
    /// flow-matching models, `6.0` for SDXL.
    pub guidance_scale: f64,
    pub caption_order: CaptionOrder,
    pub fast: PresetValues,
    pub balanced: PresetValues,
    pub thorough: PresetValues,
    pub license_note: &'static str,
    /// `Some` only for a family a real run has produced numbers for — see
    /// [`Measured`]. `None` means "nobody has run this here yet", which is
    /// what the UI should say rather than inventing a figure.
    pub measured: Option<Measured>,
}

/// What the FLUX.2 [klein] trainer actually opens under `name_or_path` — the
/// repo's **single-file** blob, and nothing else.
///
/// This is the opposite of what the diffusers layout suggests, and it cost a
/// failed run to find out (2026-09-17). In the pinned `ai-toolkit` commit,
/// `Flux2Model.load_model` does
/// `load_file(os.path.join(name_or_path, self.flux2_te_filename))`, and
/// `Flux2Klein4BModel`/`Flux2Klein9BModel` set that filename to
/// `flux-2-klein-base-{4,9}b.safetensors`. The `transformer/`,
/// `text_encoder/`, `vae/`, `tokenizer/` and `scheduler/` folders are never
/// read on this path: the klein text encoder comes from a *different* Hub
/// repo (`Qwen/Qwen3-4B` / `Qwen3-8B`) and the VAE from
/// `ai-toolkit/flux2_vae`, both fetched by the trainer at run time. Listing
/// the diffusers files here would refuse a download that is complete for the
/// trainer, and accept one missing the only file it opens — which is exactly
/// what happened: the first real run died on
/// `FileNotFoundError: No such file or directory: E:\AI\models\training\flux2-klein-4b`
/// with all five diffusers files present and verified.
const FLUX2_KLEIN_4B_REQUIRED_FILES: &[&str] = &["flux-2-klein-base-4b.safetensors"];

const FLUX2_KLEIN_9B_REQUIRED_FILES: &[&str] = &["flux-2-klein-base-9b.safetensors"];

/// The registry. One entry per trainable model family — see the module doc.
///
/// `family` values for the two FLUX.2 [klein] sizes are **not** the
/// existing `models.family` string already used by today's inference-only
/// catalog entries (`KNOWN_MODELS` gives every `flux2-klein-9b-*` entry the
/// generic `family: Some("flux2")`, and has no 4B entry at all yet — see
/// the module-level "concerns" note surfaced in the task report). They use
/// the size-specific strings the plan's own test spec calls for
/// (`"flux2-klein-4b"` / `"flux2-klein-9b"`), which the later task that
/// registers the training-base directory models must adopt on those new
/// entries for `find_for_family` to resolve them correctly.
pub const PROFILES: &[TrainingProfile] = &[
    TrainingProfile {
        family: "flux2-klein-4b",
        label: "FLUX.2 [klein] 4B",
        arch: "flux2_klein_4b",
        data_kind: DataKind::Frames,
        vram: VramStrategy {
            quantize: true,
            qtype: "qfloat8",
            quantize_te: true,
            low_vram: false,
            layer_offloading: false,
            fit: Fit::Comfortable,
            reserve_mb: 12288,
        },
        base: BaseWeight {
            repo: "black-forest-labs/FLUX.2-klein-base-4B",
            role: "training_base_flux2_klein_4b",
            required_files: FLUX2_KLEIN_4B_REQUIRED_FILES,
            approx_gb: 16,
        },
        noise_scheduler: "flowmatch",
        guidance_scale: 4.0,
        caption_order: CaptionOrder::ProseFirst,
        fast: PresetValues {
            steps: 600,
            lr: 1e-4,
            rank: 16,
            resolution: 768,
            save_every: 200,
            sample_every: 200,
        },
        balanced: PresetValues {
            steps: 1500,
            lr: 1e-4,
            rank: 16,
            resolution: 1024,
            save_every: 250,
            sample_every: 250,
        },
        thorough: PresetValues {
            steps: 3000,
            lr: 8e-5,
            rank: 32,
            resolution: 1024,
            save_every: 250,
            sample_every: 250,
        },
        license_note: "Apache-2.0 base weights",
        // Measured on the first run that completed end to end on this
        // machine (RTX 4080 Super 16 GB, run
        // 01a0af0a-ec62-7d72-8e8f-b6c2c5b1d3d8): 600 Fast steps at 768 px
        // over a 50-image dataset in 16 min 55 s of training (1.69 s/step),
        // 20 min 18 s wall including model load, latent caching and four
        // sample rounds; 77 °C.
        //
        // VRAM: the card peaked at 12,340 MB of 16,376 MB with ~1,256 MB
        // already held by the desktop before the run started, so the run's
        // own peak was ~11,084 MB. An earlier run of the same config peaked
        // at 12,249 MB card / ~10,993 MB own and 1.65 s/step, so these are
        // steady numbers rather than one lucky sample.
        measured: Some(Measured {
            s_per_step: 1.69,
            peak_vram_mb: 11_084,
            date: "2026-09-17",
        }),
    },
    TrainingProfile {
        family: "flux2-klein-9b",
        label: "FLUX.2 [klein] 9B",
        arch: "flux2_klein_9b",
        data_kind: DataKind::Frames,
        vram: VramStrategy {
            quantize: true,
            qtype: "qfloat8",
            quantize_te: true,
            low_vram: true,
            layer_offloading: true,
            fit: Fit::AtTheEdge,
            reserve_mb: 15000,
        },
        base: BaseWeight {
            repo: "black-forest-labs/FLUX.2-klein-base-9B",
            role: "training_base_flux2_klein_9b",
            required_files: FLUX2_KLEIN_9B_REQUIRED_FILES,
            approx_gb: 30,
        },
        noise_scheduler: "flowmatch",
        guidance_scale: 4.0,
        caption_order: CaptionOrder::ProseFirst,
        fast: PresetValues {
            steps: 600,
            lr: 1e-4,
            rank: 16,
            resolution: 768,
            save_every: 200,
            sample_every: 200,
        },
        balanced: PresetValues {
            steps: 1500,
            lr: 1e-4,
            rank: 16,
            resolution: 1024,
            save_every: 250,
            sample_every: 250,
        },
        thorough: PresetValues {
            steps: 3000,
            lr: 8e-5,
            rank: 16,
            resolution: 1024,
            save_every: 250,
            sample_every: 250,
        },
        license_note: "FLUX non-commercial licence, and the repo is gated: the download \
                        fails with \"this repository requires approval\" until you accept \
                        the licence on the model page and sign in. Needs fp8 + layer \
                        offloading on 16 GB — unverified, because the gate blocked the \
                        first attempt (2026-09-17)",
        // No real run of this family here yet — the weights could not be
        // downloaded, see `license_note`.
        measured: None,
    },
    TrainingProfile {
        family: "sdxl",
        label: "SDXL",
        arch: "sdxl",
        data_kind: DataKind::Frames,
        vram: VramStrategy {
            quantize: false,
            qtype: "",
            quantize_te: false,
            low_vram: false,
            layer_offloading: false,
            fit: Fit::Comfortable,
            reserve_mb: 10240,
        },
        base: BaseWeight {
            repo: "stabilityai/stable-diffusion-xl-base-1.0",
            role: "training_base_sdxl",
            required_files: &[
                "model_index.json",
                "unet/diffusion_pytorch_model.fp16.safetensors",
                "text_encoder/model.fp16.safetensors",
                "text_encoder_2/model.fp16.safetensors",
                "vae/diffusion_pytorch_model.fp16.safetensors",
            ],
            approx_gb: 7,
        },
        noise_scheduler: "ddpm",
        guidance_scale: 6.0,
        caption_order: CaptionOrder::TagsFirst,
        fast: PresetValues {
            steps: 800,
            lr: 1e-4,
            rank: 16,
            resolution: 1024,
            save_every: 200,
            sample_every: 200,
        },
        balanced: PresetValues {
            steps: 2000,
            lr: 1e-4,
            rank: 16,
            resolution: 1024,
            save_every: 250,
            sample_every: 250,
        },
        thorough: PresetValues {
            steps: 4000,
            lr: 8e-5,
            rank: 32,
            resolution: 1024,
            save_every: 250,
            sample_every: 250,
        },
        license_note: "CreativeML Open RAIL++-M",
        // No real run of this family here yet — see `Measured`.
        measured: None,
    },
    TrainingProfile {
        family: "wan",
        label: "Wan 2.2 TI2V 5B",
        arch: "wan22_5b",
        data_kind: DataKind::Both,
        vram: VramStrategy {
            quantize: true,
            qtype: "qfloat8",
            quantize_te: true,
            low_vram: true,
            layer_offloading: false,
            fit: Fit::AtTheEdge,
            reserve_mb: 15000,
        },
        base: BaseWeight {
            repo: "Wan-AI/Wan2.2-TI2V-5B-Diffusers",
            role: "training_base_wan22_5b",
            required_files: &["model_index.json"],
            approx_gb: 20,
        },
        noise_scheduler: "flowmatch",
        guidance_scale: 4.0,
        caption_order: CaptionOrder::ProseFirst,
        fast: PresetValues {
            steps: 500,
            lr: 1e-4,
            rank: 16,
            resolution: 512,
            save_every: 100,
            sample_every: 100,
        },
        balanced: PresetValues {
            steps: 1200,
            lr: 1e-4,
            rank: 16,
            resolution: 640,
            save_every: 200,
            sample_every: 200,
        },
        thorough: PresetValues {
            steps: 2500,
            lr: 8e-5,
            rank: 32,
            resolution: 768,
            save_every: 250,
            sample_every: 250,
        },
        license_note: "Apache-2.0; the 16 GB setting is unverified until a real attempt \
                        (spec: open point)",
        // No real run of this family here yet — see `Measured`.
        measured: None,
    },
];

/// The library directory that actually holds every file `profile`'s base
/// weights need, or `None` if no single candidate is complete.
///
/// "Complete" means *one* directory with all of [`BaseWeight::required_files`]
/// in it. Half a checkpoint in one folder and half in another is not a staged
/// base: the trainer is pointed at a single directory and would fail partway
/// in, long after the run looked like it had started. Both the preflight that
/// resolves the directory for a real run and the profile list that reports
/// `base_installed` to the UI go through here, so the badge in the Training
/// tab can never disagree with what pressing Start does.
pub fn find_staged_base<'a>(
    profile: &TrainingProfile,
    candidates: &'a [Model],
) -> Option<&'a Path> {
    candidates
        .iter()
        .map(|m| Path::new(m.file_path.as_str()))
        .find(|dir| {
            profile
                .base
                .required_files
                .iter()
                .all(|f| dir.join(f).is_file())
        })
}

/// Find the profile whose `family` matches the library's `models.family`
/// value for a target model. `None` for a family AIWM cannot train yet.
pub fn find_for_family(family: &str) -> Option<&'static TrainingProfile> {
    PROFILES.iter().find(|p| p.family == family)
}

/// The preset values a profile+preset combination resolves to.
pub fn preset_values(p: &TrainingProfile, preset: Preset) -> PresetValues {
    match preset {
        Preset::Fast => p.fast,
        Preset::Balanced => p.balanced,
        Preset::Thorough => p.thorough,
    }
}

/// Resolve the profile for a *library model* the user picked as the
/// training target. An exact `family` match wins first — this is how the
/// training-base directory models (`flux2-klein-4b`/`flux2-klein-9b`, see
/// [`PROFILES`]'s doc comment) resolve. Today's inference-only library
/// entries all share the generic `"flux2"` family for FLUX.2 [klein]
/// regardless of size, and `"wan"` for every Wan 2.2 size, so those two
/// need the model's name/id or `param_count` to disambiguate:
/// - `"flux2"`: a `4b`/`9b` token in `name` (case-insensitive) wins; else
///   `param_count` (`< 6e9` → 4B, else 9B); else not trainable.
/// - `"wan"`: only the 5B is trainable here — a `5b` token in `name`, or
///   `param_count < 8e9`, resolves to `wan22_5b`; otherwise (the 14B, or
///   nothing to go on) `None`.
///
/// `None` means "not trainable" — the UI shows that in plain text rather
/// than failing.
/// Whether `name` has `token` as a whole, case-insensitive component —
/// splitting on every non-alphanumeric character, so `4b` matches
/// `…-4b-…`/`4b_fp8`/`4B` but not the `4b` inside `14b`.
fn has_size_token(name: &str, token: &str) -> bool {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part.eq_ignore_ascii_case(token))
}

pub fn find_for_model(
    family: Option<&str>,
    name: &str,
    param_count: Option<i64>,
) -> Option<&'static TrainingProfile> {
    const FLUX2_4B_PARAM_THRESHOLD: i64 = 6_000_000_000;
    const WAN_5B_PARAM_THRESHOLD: i64 = 8_000_000_000;

    let family = family?;

    match family {
        "flux2" => {
            let resolved = if has_size_token(name, "9b") {
                "flux2-klein-9b"
            } else if has_size_token(name, "4b") {
                "flux2-klein-4b"
            } else {
                match param_count {
                    Some(count) if count < FLUX2_4B_PARAM_THRESHOLD => "flux2-klein-4b",
                    Some(_) => "flux2-klein-9b",
                    None => return None,
                }
            };
            find_for_family(resolved)
        }
        "wan" => {
            let is_5b = has_size_token(name, "5b")
                || match param_count {
                    Some(count) => count < WAN_5B_PARAM_THRESHOLD,
                    None => false,
                };
            if is_5b {
                find_for_family("wan")
            } else {
                None
            }
        }
        other => find_for_family(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// A library row standing for a staged base-weights directory: only
    /// `file_path` matters to [`find_staged_base`].
    fn dir_model(dir: &Path) -> Model {
        Model {
            id: "m".into(),
            publisher: None,
            name: "base weights".into(),
            family: None,
            base_family: None,
            family_source: None,
            format: "dir".into(),
            quant: None,
            arch: None,
            param_count: None,
            file_path: dir.to_string_lossy().into_owned(),
            sha256: None,
            size_bytes: 0,
            ctx_max: None,
            vram_estimate_mb: None,
            ram_estimate_mb: None,
            source: "manual".into(),
            source_revision: None,
            imported_at: String::new(),
            last_used_at: None,
            use_count: 0,
            n_layers: None,
            n_embd: None,
            n_heads: None,
            n_kv_heads: None,
            roles: vec![],
            runtimes: vec![],
        }
    }

    fn touch(dir: &Path, relative: &str) {
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&path, b"x").expect("write file");
    }

    #[test]
    fn find_staged_base_needs_every_required_file_under_one_candidate() {
        let profile = find_for_family("sdxl").expect("sdxl profile");
        let tmp = tempfile::tempdir().expect("tempdir");

        // The first candidate is a half-finished download: one required file
        // present, the rest missing.
        let partial = tmp.path().join("partial");
        std::fs::create_dir_all(&partial).expect("create partial dir");
        touch(&partial, profile.base.required_files[0]);

        let complete = tmp.path().join("complete");
        std::fs::create_dir_all(&complete).expect("create complete dir");
        for file in profile.base.required_files {
            touch(&complete, file);
        }

        // Only the partial one: not staged at all.
        let only_partial = vec![dir_model(&partial)];
        assert_eq!(find_staged_base(profile, &only_partial), None);

        // Both, partial first: the complete one wins rather than the first hit.
        let both = vec![dir_model(&partial), dir_model(&complete)];
        assert_eq!(find_staged_base(profile, &both), Some(complete.as_path()));

        // A file missing from an otherwise complete directory disqualifies it
        // -- the trainer would fail partway in, not at the start.
        std::fs::remove_file(complete.join(profile.base.required_files[1]))
            .expect("remove one required file");
        assert_eq!(find_staged_base(profile, &both), None);
    }

    #[test]
    fn find_staged_base_is_none_without_candidates() {
        let profile = find_for_family("sdxl").expect("sdxl profile");
        assert_eq!(find_staged_base(profile, &[]), None);
    }

    #[test]
    fn every_profile_has_a_unique_family_and_arch() {
        let mut families = HashSet::new();
        let mut archs = HashSet::new();
        for p in PROFILES {
            assert!(families.insert(p.family), "duplicate family: {}", p.family);
            assert!(archs.insert(p.arch), "duplicate arch: {}", p.arch);
        }
        assert_eq!(PROFILES.len(), 4, "expected 4 seeded profiles");
    }

    #[test]
    fn presets_scale_steps_monotonically() {
        for p in PROFILES {
            assert!(
                p.fast.steps < p.balanced.steps,
                "{}: fast steps ({}) must be less than balanced ({})",
                p.family,
                p.fast.steps,
                p.balanced.steps
            );
            assert!(
                p.balanced.steps < p.thorough.steps,
                "{}: balanced steps ({}) must be less than thorough ({})",
                p.family,
                p.balanced.steps,
                p.thorough.steps
            );
            assert!(
                p.thorough.lr <= p.fast.lr,
                "{}: thorough lr ({}) must not be higher than fast ({})",
                p.family,
                p.thorough.lr,
                p.fast.lr
            );
        }
    }

    #[test]
    fn find_for_family_matches_library_family_strings() {
        assert_eq!(
            find_for_family("flux2-klein-4b").map(|p| p.arch),
            Some("flux2_klein_4b")
        );
        assert_eq!(
            find_for_family("flux2-klein-9b").map(|p| p.arch),
            Some("flux2_klein_9b")
        );
        assert_eq!(find_for_family("sdxl").map(|p| p.arch), Some("sdxl"));
        assert_eq!(find_for_family("wan").map(|p| p.arch), Some("wan22_5b"));
        assert_eq!(find_for_family("not-a-real-family"), None);
    }

    #[test]
    fn video_profiles_accept_clips_and_image_profiles_do_not() {
        let wan = find_for_family("wan").expect("wan profile");
        assert!(wan.data_kind.accepts(DatasetMode::Clips));
        assert!(wan.data_kind.accepts(DatasetMode::Frames));

        for family in ["flux2-klein-4b", "flux2-klein-9b", "sdxl"] {
            let profile = find_for_family(family).expect("image profile");
            assert!(
                !profile.data_kind.accepts(DatasetMode::Clips),
                "{family}: image profile must not accept clips"
            );
            assert!(profile.data_kind.accepts(DatasetMode::Frames));
        }
    }

    #[test]
    fn fit_labels_are_plain_english() {
        assert_eq!(Fit::Comfortable.label(), "fits comfortably");
        assert_eq!(Fit::AtTheEdge.label(), "at the edge");
    }

    #[test]
    fn base_required_files_are_relative_paths_without_leading_slashes() {
        for p in PROFILES {
            assert!(
                !p.base.required_files.is_empty(),
                "{}: required_files must not be empty",
                p.family
            );
            for file in p.base.required_files {
                assert!(
                    !file.starts_with('/') && !file.starts_with('\\'),
                    "{}: {} must be a relative path with no leading slash",
                    p.family,
                    file
                );
            }
        }
    }

    #[test]
    fn preset_values_resolves_each_preset() {
        let sdxl = find_for_family("sdxl").expect("sdxl profile");
        assert_eq!(preset_values(sdxl, Preset::Fast), sdxl.fast);
        assert_eq!(preset_values(sdxl, Preset::Balanced), sdxl.balanced);
        assert_eq!(preset_values(sdxl, Preset::Thorough), sdxl.thorough);
    }

    #[test]
    fn find_for_model_reads_the_size_token_from_the_name_first() {
        assert_eq!(
            find_for_model(Some("flux2"), "flux2-klein-9b-fp8", None).map(|p| p.arch),
            Some("flux2_klein_9b")
        );
        assert_eq!(
            find_for_model(Some("flux2"), "FLUX.2 [klein] 4B", None).map(|p| p.arch),
            Some("flux2_klein_4b")
        );
    }

    #[test]
    fn find_for_model_falls_back_to_param_count_for_generic_flux2_names() {
        assert_eq!(
            find_for_model(Some("flux2"), "flux2-klein", Some(8_900_000_000)).map(|p| p.arch),
            Some("flux2_klein_9b")
        );
        assert_eq!(
            find_for_model(Some("flux2"), "flux2-klein", Some(4_000_000_000)).map(|p| p.arch),
            Some("flux2_klein_4b")
        );
    }

    #[test]
    fn find_for_model_is_none_for_flux2_with_no_size_signal() {
        assert_eq!(find_for_model(Some("flux2"), "mystery", None), None);
    }

    #[test]
    fn find_for_model_matches_sdxl_regardless_of_name() {
        assert_eq!(
            find_for_model(Some("sdxl"), "anything", None).map(|p| p.arch),
            Some("sdxl")
        );
    }

    #[test]
    fn find_for_model_accepts_only_the_wan_5b_size() {
        assert_eq!(
            find_for_model(Some("wan"), "wan2.2-ti2v-5b-fp16", None).map(|p| p.arch),
            Some("wan22_5b")
        );
        assert_eq!(
            find_for_model(Some("wan"), "wan-mystery", Some(5_000_000_000)).map(|p| p.arch),
            Some("wan22_5b")
        );
        // The 14B is a real Wan 2.2 size but has no profile here.
        assert_eq!(
            find_for_model(Some("wan"), "wan2.2-t2v-14b", Some(14_000_000_000)),
            None
        );
        assert_eq!(find_for_model(Some("wan"), "wan-mystery", None), None);
    }

    #[test]
    fn find_for_model_returns_none_without_a_family() {
        assert_eq!(find_for_model(None, "anything", None), None);
    }

    #[test]
    fn find_for_model_matches_the_size_specific_family_exactly() {
        assert_eq!(
            find_for_model(Some("flux2-klein-9b"), "irrelevant name", None).map(|p| p.arch),
            Some("flux2_klein_9b")
        );
    }

    #[test]
    fn find_for_model_size_matching_is_whole_token_not_substring() {
        // "14b" must not be mistaken for a "4b" (or, for Wan, "5b") substring.
        assert_eq!(
            find_for_model(Some("flux2"), "flux2-klein-14b-experimental", None),
            None
        );
        assert_eq!(find_for_model(Some("wan"), "wan2.2-t2v-14b", None), None);
        assert_eq!(
            find_for_model(Some("wan"), "Wan2.2-TI2V-5B-Diffusers", None).map(|p| p.arch),
            Some("wan22_5b")
        );
    }

    #[test]
    fn reserve_and_fit_match_the_spec_table() {
        let expected: &[(&str, u64, Fit)] = &[
            ("flux2-klein-4b", 12288, Fit::Comfortable),
            ("flux2-klein-9b", 15000, Fit::AtTheEdge),
            ("sdxl", 10240, Fit::Comfortable),
            ("wan", 15000, Fit::AtTheEdge),
        ];
        for (family, reserve_mb, fit) in expected {
            let profile = find_for_family(family).expect("seeded profile");
            assert_eq!(profile.vram.reserve_mb, *reserve_mb, "{family}: reserve_mb");
            assert_eq!(profile.vram.fit, *fit, "{family}: fit");
        }
    }

    #[test]
    fn the_9b_note_warns_that_its_weights_are_gated() {
        // Unlike the 4B, whose weights download without any sign-in, the 9B
        // repo is approval-gated: `hf download` answers "Access denied. This
        // repository requires approval." (confirmed 2026-09-17). Nothing in
        // the app can work around that, so the note has to say so -- a user
        // who reads only "needs fp8 + layer offloading" will otherwise spend
        // the download before finding out.
        let note = find_for_family("flux2-klein-9b")
            .expect("9B profile")
            .license_note;
        assert!(
            note.contains("gated") || note.contains("approval"),
            "the 9B note must warn about the gate: {note}"
        );
    }

    #[test]
    fn only_a_profile_that_has_actually_been_run_carries_a_measurement() {
        // `measured` is evidence, not an estimate: it may only be present for
        // a family a real run has produced numbers for on real hardware.
        // Today that is the 4B and nothing else.
        for profile in PROFILES {
            match profile.family {
                "flux2-klein-4b" => {
                    let m = profile.measured.expect("the 4B has been run for real");
                    assert!(m.s_per_step > 0.0, "seconds per step must be positive");
                    assert!(
                        m.peak_vram_mb > 0 && m.peak_vram_mb < 16_376,
                        "a peak measured on a 16 GB card must fit in one: {}",
                        m.peak_vram_mb
                    );
                    assert_eq!(m.date.len(), 10, "an ISO date, e.g. 2026-09-17");
                }
                other => assert!(
                    profile.measured.is_none(),
                    "{other}: no real run has measured this profile yet"
                ),
            }
        }
    }

    #[test]
    fn the_4b_reserve_covers_what_the_real_run_actually_used() {
        // The reserve is what preflight demands be *free* before it will
        // start, and `peak_vram_mb` is what the run itself held. If the
        // measurement ever exceeds the reserve, the profile is letting a run
        // start that cannot fit, and the user gets an OOM half an hour in
        // instead of a refusal in the first second.
        let profile = find_for_family("flux2-klein-4b").expect("4B profile");
        let measured = profile.measured.expect("the 4B has been measured");
        assert!(
            measured.peak_vram_mb <= profile.vram.reserve_mb,
            "measured peak {} MB exceeds the {} MB the profile reserves",
            measured.peak_vram_mb,
            profile.vram.reserve_mb
        );
        // ...and the reserve must not be so far above the measurement that
        // it refuses runs that would have been fine. 1.5x is generous for a
        // figure that varied by ~100 MB across two runs.
        assert!(
            profile.vram.reserve_mb <= measured.peak_vram_mb * 3 / 2,
            "the {} MB reserve is far above the {} MB actually needed",
            profile.vram.reserve_mb,
            measured.peak_vram_mb
        );
    }

    #[test]
    fn the_flux2_klein_sizes_require_only_their_single_file_blob() {
        // Verified against the pinned ai-toolkit commit and against a real
        // failed run (see the module note): `Flux2Model.load_model` joins
        // `name_or_path` with `flux2_te_filename` and calls `load_file` on
        // the result, and the klein subclasses set that filename to the
        // repo's single-file blob. The diffusers subfolders are never opened
        // on this path -- requiring them would refuse a download that is
        // complete for the trainer, and permit one that is missing the only
        // file it actually reads.
        let expected: &[(&str, &str)] = &[
            ("flux2-klein-4b", "flux-2-klein-base-4b.safetensors"),
            ("flux2-klein-9b", "flux-2-klein-base-9b.safetensors"),
        ];
        for (family, blob) in expected {
            let profile = find_for_family(family).expect("klein profile");
            assert_eq!(
                profile.base.required_files,
                &[*blob],
                "{family}: the trainer reads exactly one local file"
            );
        }
    }

    #[test]
    fn noise_scheduler_and_guidance_scale_match_the_spec_table() {
        // flowmatch/4.0 for the three flow-matching models, ddpm/6.0 for
        // SDXL — see `core::training::config` and the plan's appendix.
        let expected: &[(&str, &str, f64)] = &[
            ("flux2-klein-4b", "flowmatch", 4.0),
            ("flux2-klein-9b", "flowmatch", 4.0),
            ("sdxl", "ddpm", 6.0),
            ("wan", "flowmatch", 4.0),
        ];
        for (family, noise_scheduler, guidance_scale) in expected {
            let profile = find_for_family(family).expect("seeded profile");
            assert_eq!(
                profile.noise_scheduler, *noise_scheduler,
                "{family}: noise_scheduler"
            );
            assert_eq!(
                profile.guidance_scale, *guidance_scale,
                "{family}: guidance_scale"
            );
        }
    }
}
