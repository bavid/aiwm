//! Per-family defaults for a text-to-image request — what a job gets for a
//! size, step count or CFG it does not state.
//!
//! Every checkpoint goes through the same graph ([`crate::pipeline::Recipe`]),
//! but the base resolution is baked into the weights: SDXL was trained at
//! 1024 px, Stable Diffusion 1.5 at 512 px, and SD 1.5 asked for 1024 px
//! repeats its subject across the canvas. So the defaults follow the
//! model's family ([`crate::model::family::infer_family`]); explicit values in
//! the job's params always win.

use crate::db::Model;
use crate::model::family::{infer_family, ArchGroup};

/// What an image request falls back to when its params leave a value out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDefaults {
    /// Width and height, in pixels.
    pub dim: u32,
    pub steps: u32,
    pub cfg: f64,
}

impl ImageDefaults {
    /// SDXL-class checkpoints and the FLUX recipes: 1024 px, 25 steps, CFG 7.
    pub const STANDARD: Self = Self {
        dim: 1024,
        steps: 25,
        cfg: 7.0,
    };

    /// Stable Diffusion 1.5: the values of ComfyUI's own SD 1.5 example
    /// workflow (`script_examples/basic_api_example.py`, ComfyUI v0.34.0 —
    /// `v1-5-pruned-emaonly.safetensors`, `EmptyLatentImage` 512×512,
    /// `KSampler` 20 steps, cfg 8, euler / normal).
    pub const SD15: Self = Self {
        dim: 512,
        steps: 20,
        cfg: 8.0,
    };

    /// The defaults for `model`'s family: [`Self::SD15`] for anything of the
    /// SD 1.5 architecture, [`Self::STANDARD`] otherwise (including a model
    /// whose family cannot be told).
    pub fn for_model(model: &Model) -> Self {
        if is_sd15(model) {
            Self::SD15
        } else {
            Self::STANDARD
        }
    }
}

/// Whether `model` is of the SD 1.5 architecture, by the same inference the
/// library uses (recorded family, catalog row, legacy string, file name).
pub fn is_sd15(model: &Model) -> bool {
    infer_family(model, None).is_some_and(|(family, _)| family.arch_group == ArchGroup::Sd15)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::image::ImageRequest;

    fn model(family: Option<&str>, base_family: Option<&str>, file: &str) -> Model {
        Model {
            id: "m".into(),
            publisher: None,
            name: file.into(),
            family: family.map(str::to_string),
            base_family: base_family.map(str::to_string),
            family_source: base_family.map(|_| "user".to_string()),
            format: "safetensors".into(),
            quant: None,
            arch: None,
            param_count: None,
            file_path: format!("E:\\AI\\models\\image\\checkpoints\\{file}"),
            sha256: None,
            size_bytes: 1,
            ctx_max: None,
            vram_estimate_mb: None,
            ram_estimate_mb: None,
            source: "manual".into(),
            source_revision: None,
            imported_at: "2026-09-21T00:00:00Z".into(),
            last_used_at: None,
            use_count: 0,
            n_layers: None,
            n_embd: None,
            n_heads: None,
            n_kv_heads: None,
            roles: vec!["base_diffusion".into()],
            runtimes: vec![],
        }
    }

    #[test]
    fn the_sd15_catalog_checkpoint_gets_512px_20_steps_cfg_8() {
        // What an import of the catalog file records: `family = "sd15"`.
        let m = model(Some("sd15"), None, "v1-5-pruned-emaonly.safetensors");
        assert!(is_sd15(&m));
        assert_eq!(
            ImageDefaults::for_model(&m),
            ImageDefaults {
                dim: 512,
                steps: 20,
                cfg: 8.0
            }
        );
    }

    #[test]
    fn an_sd15_fine_tune_is_told_by_its_recorded_family_or_its_name() {
        let recorded = model(None, Some("sd15"), "realisticVision_v60.safetensors");
        assert_eq!(ImageDefaults::for_model(&recorded), ImageDefaults::SD15);
        let named = model(None, None, "dreamshaper_8_sd15.safetensors");
        assert_eq!(ImageDefaults::for_model(&named), ImageDefaults::SD15);
    }

    #[test]
    fn sdxl_flux_and_unknown_checkpoints_keep_the_1024px_defaults() {
        for m in [
            model(Some("sdxl"), None, "sd_xl_base_1.0.safetensors"),
            model(None, Some("pony"), "ponyDiffusionV6XL.safetensors"),
            model(Some("flux"), None, "flux1-dev-Q8_0.gguf"),
            model(Some("flux2"), None, "flux-2-klein-9b-Q4_K_M.gguf"),
            model(None, None, "mystery.safetensors"),
        ] {
            assert!(!is_sd15(&m), "{}", m.name);
            assert_eq!(ImageDefaults::for_model(&m), ImageDefaults::STANDARD);
        }
        assert_eq!(ImageDefaults::STANDARD.dim, 1024);
    }

    #[test]
    fn a_request_for_an_sd15_model_falls_back_to_its_defaults() {
        let m = model(Some("sd15"), None, "v1-5-pruned-emaonly.safetensors");
        let r = ImageRequest::for_model(&serde_json::json!({ "prompt": "a cat" }), &m).unwrap();
        assert_eq!((r.width, r.height, r.steps, r.cfg), (512, 512, 20, 8.0));
    }

    #[test]
    fn explicit_values_win_over_the_family_defaults() {
        let m = model(Some("sd15"), None, "v1-5-pruned-emaonly.safetensors");
        let r = ImageRequest::for_model(
            &serde_json::json!({
                "prompt": "a cat",
                "width": 768,
                "height": 512,
                "steps": 30,
                "cfg": 6.5
            }),
            &m,
        )
        .unwrap();
        assert_eq!((r.width, r.height, r.steps, r.cfg), (768, 512, 30, 6.5));
    }

    #[test]
    fn a_request_for_an_sdxl_model_keeps_the_standard_defaults() {
        let m = model(Some("sdxl"), None, "sd_xl_base_1.0.safetensors");
        let r = ImageRequest::for_model(&serde_json::json!({ "prompt": "a cat" }), &m).unwrap();
        assert_eq!((r.width, r.height, r.steps, r.cfg), (1024, 1024, 25, 7.0));
    }
}
