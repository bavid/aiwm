//! Which image recipe a library model renders with, and the guard that stops
//! a diffusion-model-only file from reaching the single-file checkpoint
//! graph.
//!
//! The legacy `models.family` string decides first, exactly as before
//! ([`Recipe::for_family`]: `flux`, `flux2`, `krea2`). A row it leaves on the
//! checkpoint graph is then asked for its base family the way the Models →
//! Packages view decides it ([`infer_family`]: a recorded base family, the
//! catalog, the safetensors header, the file name) — so a Civitai Krea 2
//! fine-tune imported with no family at all still renders as Krea 2.
//!
//! What stays on the checkpoint graph with a base this app can't run (or
//! none it can tell) and a header without a single text-encoder tensor is
//! refused before anything is queued: `CheckpointLoaderSimple` would load it
//! and ComfyUI would fail with "clip input is invalid: None".

use std::path::Path;

use super::media::comfy_err;
use crate::db::Model;
use crate::model::family::{infer_family, ArchGroup, BaseFamily, Runnable};
use crate::model::{read_safetensors_header, SafetensorsHeader};
use crate::pipeline::Recipe;
use crate::Result;

/// Pick the recipe for `model` (file name `model_file`) and refuse a
/// checkpoint render that cannot work. Reads the safetensors header off the
/// async runtime when the legacy family leaves the choice open.
pub(crate) async fn route(model: &Model, model_file: &str) -> Result<Recipe> {
    let header = if needs_header(model, model_file) {
        read_header(model).await
    } else {
        None
    };
    let family = infer_family(model, header.as_ref()).map(|(f, _)| f);
    let recipe = recipe_for(model, model_file, family);
    if recipe == Recipe::Checkpoint {
        check_checkpoint_has_text_encoder(model, family, header.as_ref())?;
    }
    Ok(recipe)
}

/// The base family of `model`, reading its safetensors header when nothing
/// recorded decides it — for the per-family request defaults.
pub(crate) async fn base_family(model: &Model) -> Option<&'static BaseFamily> {
    let header = if model.base_family.is_none() && is_safetensors(&model.file_path) {
        read_header(model).await
    } else {
        None
    };
    infer_family(model, header.as_ref()).map(|(f, _)| f)
}

/// Whether the recipe choice may need the header: the legacy family left
/// the model on the checkpoint graph, and the file is one whose header can
/// be read.
fn needs_header(model: &Model, model_file: &str) -> bool {
    Recipe::for_family(model.family.as_deref(), model_file) == Recipe::Checkpoint
        && is_safetensors(model_file)
}

fn is_safetensors(file: &str) -> bool {
    file.to_ascii_lowercase().ends_with(".safetensors")
}

/// The header of `model`'s file, read on the blocking pool; `None` when it
/// can't be read (missing file, not a safetensors file) — a hint, never a
/// reason to fail the render.
async fn read_header(model: &Model) -> Option<SafetensorsHeader> {
    let path = model.file_path.clone();
    tokio::task::spawn_blocking(move || read_safetensors_header(Path::new(&path)).ok())
        .await
        .ok()
        .flatten()
}

/// [`Recipe::for_family`] on the legacy string first (unchanged behaviour
/// for every row that has one); a row it leaves on the checkpoint graph goes
/// to [`Recipe::Krea2`] when its base family is Krea 2.
fn recipe_for(model: &Model, model_file: &str, family: Option<&BaseFamily>) -> Recipe {
    match Recipe::for_family(model.family.as_deref(), model_file) {
        Recipe::Checkpoint if family.is_some_and(|f| f.arch_group == ArchGroup::Krea2) => {
            Recipe::Krea2
        }
        legacy => legacy,
    }
}

/// Where a single-file checkpoint keeps its text encoder(s): SD 1.x
/// `cond_stage_model.`, SDXL `conditioner.`, a ComfyUI-saved all-in-one
/// `text_encoders.`, a diffusers-style `text_encoder.` / bare CLIP
/// `text_model.`.
const TEXT_ENCODER_PREFIXES: &[&str] = &[
    "cond_stage_model.",
    "conditioner.",
    "text_encoders.",
    "text_encoder.",
    "text_model.",
];

fn has_text_encoder(header: &SafetensorsHeader) -> bool {
    header
        .tensors
        .keys()
        .any(|k| TEXT_ENCODER_PREFIXES.iter().any(|p| k.starts_with(p)))
}

/// Refuse a checkpoint render of a file that holds only a diffusion model
/// when its base is unknown or not runnable here. `Ok` whenever that can't be
/// told (no header, an empty one) or the base is one this app runs.
fn check_checkpoint_has_text_encoder(
    model: &Model,
    family: Option<&BaseFamily>,
    header: Option<&SafetensorsHeader>,
) -> Result<()> {
    if family.is_some_and(|f| f.runnable == Runnable::Yes) {
        return Ok(());
    }
    let Some(header) = header.filter(|h| !h.tensors.is_empty()) else {
        return Ok(());
    };
    if has_text_encoder(header) {
        return Ok(());
    }
    Err(comfy_err(format!(
        "\u{201c}{}\u{201d} holds only a diffusion model (no text encoder), and its base \
         isn't one this app can run yet \u{2014} pick its base on Models \u{2192} Packages.",
        model.name
    )))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;

    use super::*;
    use crate::model::family::family_by_id;

    fn model(family: Option<&str>, base_family: Option<&str>, path: &str) -> Model {
        let name = path.rsplit(['\\', '/']).next().unwrap_or(path).to_string();
        Model {
            id: "m".into(),
            publisher: None,
            name,
            family: family.map(str::to_string),
            base_family: base_family.map(str::to_string),
            family_source: base_family.map(|_| "civitai".to_string()),
            format: "safetensors".into(),
            quant: None,
            arch: None,
            param_count: None,
            file_path: path.into(),
            sha256: None,
            size_bytes: 1,
            ctx_max: None,
            vram_estimate_mb: None,
            ram_estimate_mb: None,
            source: "civitai".into(),
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

    fn header(keys: &[&str]) -> SafetensorsHeader {
        SafetensorsHeader {
            tensors: keys
                .iter()
                .map(|k| ((*k).to_string(), vec![8, 8]))
                .collect(),
            metadata: BTreeMap::new(),
        }
    }

    const KREA2_KEYS: &[&str] = &[
        "blocks.0.attn.gate.weight",
        "blocks.0.attn.wq.weight",
        "txtfusion.projector.weight",
        "first.weight",
    ];

    /// A diffusion model of no family this app knows, and no text encoder.
    const ORPHAN_KEYS: &[&str] = &["layers.0.attn.qkv.weight", "final_layer.weight"];

    /// Write a real `.safetensors` file whose header lists `keys` (8×8 F8
    /// tensors each), so [`route`] reads it like a library file.
    pub(crate) fn write_safetensors(dir: &Path, file: &str, keys: &[&str]) -> String {
        let mut entries = serde_json::Map::new();
        for (i, k) in keys.iter().enumerate() {
            let start = i * 64;
            entries.insert(
                (*k).to_string(),
                serde_json::json!({
                    "dtype": "F8_E4M3",
                    "shape": [8, 8],
                    "data_offsets": [start, start + 64]
                }),
            );
        }
        let json = serde_json::to_vec(&serde_json::Value::Object(entries)).unwrap();
        let path = dir.join(file);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&(json.len() as u64).to_le_bytes()).unwrap();
        f.write_all(&json).unwrap();
        f.write_all(&vec![0u8; keys.len() * 64]).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn fam(id: &str) -> Option<&'static BaseFamily> {
        family_by_id(id)
    }

    #[test]
    fn rows_with_a_legacy_family_route_exactly_as_before() {
        let cases = [
            (Some("flux"), "flux1-dev-Q8_0.gguf", Recipe::FluxGguf),
            (
                Some("flux2"),
                "flux-2-klein-9b-Q4_K_M.gguf",
                Recipe::Flux2KleinGguf,
            ),
            (
                Some("flux2"),
                "flux-2-klein-9b-fp8mixed.safetensors",
                Recipe::Flux2KleinSafetensors,
            ),
            (
                Some("sdxl"),
                "sd_xl_base_1.0.safetensors",
                Recipe::Checkpoint,
            ),
            (
                Some("sd15"),
                "v1-5-pruned-emaonly.safetensors",
                Recipe::Checkpoint,
            ),
            (
                Some("krea2"),
                "krea2_turbo_fp8_scaled.safetensors",
                Recipe::Krea2,
            ),
        ];
        for (family, file, want) in cases {
            let m = model(family, None, &format!("E:\\m\\{file}"));
            let inferred = infer_family(&m, None).map(|(f, _)| f);
            assert_eq!(recipe_for(&m, file, inferred), want, "{family:?} {file}");
        }
    }

    #[test]
    fn a_row_without_a_legacy_family_routes_by_its_base_family() {
        let file = "lustifyNSFWCheckpoint_v10Krea2_2997637.safetensors";
        let m = model(None, None, &format!("E:\\m\\{file}"));
        assert_eq!(recipe_for(&m, file, fam("krea2")), Recipe::Krea2);
        // Any other base stays on the checkpoint graph.
        assert_eq!(recipe_for(&m, file, fam("sdxl")), Recipe::Checkpoint);
        assert_eq!(recipe_for(&m, file, None), Recipe::Checkpoint);
        // A legacy family still wins over the base family.
        let flux = model(Some("flux"), Some("krea2"), "E:\\m\\x.gguf");
        assert_eq!(recipe_for(&flux, "x.gguf", fam("krea2")), Recipe::FluxGguf);
    }

    #[test]
    fn a_diffusion_only_file_of_an_unknown_base_is_refused_on_the_checkpoint_graph() {
        let m = model(None, None, "E:\\m\\mystery_model_v1.safetensors");
        let err = check_checkpoint_has_text_encoder(&m, None, Some(&header(ORPHAN_KEYS)))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(
                "\u{201c}mystery_model_v1.safetensors\u{201d} holds only a diffusion model \
                 (no text encoder), and its base isn't one this app can run yet \u{2014} pick \
                 its base on Models \u{2192} Packages."
            ),
            "{err}"
        );
        // A known base that can't run here is refused the same way.
        assert!(
            check_checkpoint_has_text_encoder(&m, fam("wan-14b"), Some(&header(ORPHAN_KEYS)))
                .is_err()
        );
    }

    #[test]
    fn real_checkpoints_and_unreadable_headers_pass_the_guard() {
        let m = model(None, None, "E:\\m\\mystery.safetensors");
        // SD 1.x carries `cond_stage_model.`, SDXL `conditioner.`, a
        // ComfyUI-saved all-in-one `text_encoders.`.
        for te in [
            "cond_stage_model.transformer.text_model.embeddings.position_ids",
            "conditioner.embedders.0.transformer.text_model.final_layer_norm.weight",
            "text_encoders.clip_l.transformer.text_model.final_layer_norm.weight",
        ] {
            let h = header(&[ORPHAN_KEYS[0], te]);
            assert!(
                check_checkpoint_has_text_encoder(&m, None, Some(&h)).is_ok(),
                "{te}"
            );
        }
        // No header, or an empty one: can't tell, so don't block.
        assert!(check_checkpoint_has_text_encoder(&m, None, None).is_ok());
        assert!(check_checkpoint_has_text_encoder(&m, None, Some(&header(&[]))).is_ok());
        // A base this app runs is left to ComfyUI.
        assert!(
            check_checkpoint_has_text_encoder(&m, fam("sdxl"), Some(&header(ORPHAN_KEYS))).is_ok()
        );
    }

    #[tokio::test]
    async fn route_reads_the_header_of_a_file_with_no_recorded_family() {
        let dir = tempfile::tempdir().unwrap();
        // A name that gives nothing away: only the header says Krea 2.
        let path = write_safetensors(dir.path(), "civitai_2997637.safetensors", KREA2_KEYS);
        let m = model(None, None, &path);
        let r = route(&m, "civitai_2997637.safetensors").await.unwrap();
        assert_eq!(r, Recipe::Krea2);
        assert_eq!(base_family(&m).await.map(|f| f.id), Some("krea2"));

        let orphan = write_safetensors(dir.path(), "orphan.safetensors", ORPHAN_KEYS);
        let err = route(&model(None, None, &orphan), "orphan.safetensors")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("holds only a diffusion model"),
            "{err}"
        );

        // Missing file: nothing to read, nothing blocked.
        let gone = model(None, None, "E:\\nope\\gone.safetensors");
        let r = route(&gone, "gone.safetensors").await.unwrap();
        assert_eq!(r, Recipe::Checkpoint);
    }
}
