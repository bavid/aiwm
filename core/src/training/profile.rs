//! The training profile registry (spec §1 "Trainings-Profile"): one entry
//! per trainable model family, mapping it to an `ai-toolkit` architecture,
//! a VRAM strategy, the base weights the runtime needs staged locally, and
//! the three preset (fast/balanced/thorough) starting points a run can pick.
//!
//! A library model whose `family` has no entry here is "not yet trainable"
//! in the UI — adding support for a new family is a new [`PROFILES`] entry
//! plus tests, never a new subsystem.

use crate::capability::dataset::CaptionOrder;
use crate::db::{DatasetMode, Preset};

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
    pub caption_order: CaptionOrder,
    pub fast: PresetValues,
    pub balanced: PresetValues,
    pub thorough: PresetValues,
    pub license_note: &'static str,
}

const FLUX2_KLEIN_REQUIRED_FILES: &[&str] = &[
    "model_index.json",
    "transformer/diffusion_pytorch_model.safetensors",
    "text_encoder/model-00001-of-00002.safetensors",
    "text_encoder/model-00002-of-00002.safetensors",
    "vae/diffusion_pytorch_model.safetensors",
];

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
            required_files: FLUX2_KLEIN_REQUIRED_FILES,
            approx_gb: 16,
        },
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
            required_files: FLUX2_KLEIN_REQUIRED_FILES,
            approx_gb: 30,
        },
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
        license_note: "FLUX non-commercial licence; needs fp8 + layer offloading on 16 GB — \
                        unverified until the first real run",
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
    },
];

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
}
