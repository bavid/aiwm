//! Family detection from a safetensors header and from a file name.
//!
//! Tensor names are compared in a canonical form — lowercase, `_` read as
//! `.` — so kohya's flattened LoRA names (`lora_unet_double_blocks_0_…`)
//! match the dotted checkpoint / diffusers / PEFT names (`double_blocks.0.…`)
//! with one set of patterns, whatever prefix a file carries
//! (`model.diffusion_model.`, `diffusion_model.`, `transformer.`, …).
//!
//! Where the patterns come from:
//! - checkpoints: ComfyUI's own architecture detection
//!   (`comfy/model_detection.py`, `comfy/supported_models.py`) — FLUX by
//!   `double_blocks` + `img_in` (FLUX.2 has 128 input channels, FLUX.1 64),
//!   FLUX.2 [klein] size by the text-encoder width of `txt_in` (Qwen3-4B:
//!   3 × 2560 = 7680, Qwen3-8B: 3 × 4096 = 12288); Wan by `head.modulation`
//!   (its last dimension is the width); LTX by `adaln_single`, LTX-2 by
//!   `audio_adaln_single`; SDXL by `conditioner.embedders.1.`, SD 1.x by
//!   `cond_stage_model.transformer.`.
//! - the widths: real headers read from Hugging Face on 2026-09-21
//!   (`sd_xl_base_1.0`, `v1-5-pruned-emaonly`, `flux1-schnell-fp8`, a FLUX.2
//!   klein 9B checkpoint, `wan2.2_ti2v_5B_fp16`, a Wan 2.2 14B,
//!   `ltx-video-2b-v0.9.5`) and the FLUX.2-klein-4B diffusers config.
//! - LoRAs: headers of published LoRAs of each family (kohya `lora_unet_` /
//!   `lora_te_` / `lora_te1_` + `lora_te2_`, diffusers/PEFT `lora_A`, XLabs'
//!   `processor.*_lora1.down`). A LoRA's width is the input width of its
//!   down projections; SD 1.x and SDXL differ in the cross-attention input
//!   (`attn2.to_k`: 768 vs 2048).

use std::collections::BTreeMap;

use super::{family_by_id, BaseFamily};
use crate::model::SafetensorsHeader;

/// The family a header's tensor names and shapes identify; `None` when
/// they don't pin one down.
pub fn family_from_header(header: &SafetensorsHeader) -> Option<&'static BaseFamily> {
    let tensors: Vec<(String, &[u64])> = header
        .tensors
        .iter()
        .map(|(name, shape)| (canonical(name), shape.as_slice()))
        .collect();
    let id = if tensors.iter().any(|(k, _)| is_down_projection(k)) {
        lora_family(&tensors)
    } else {
        checkpoint_family(&tensors)
    }?;
    family_by_id(id)
}

fn canonical(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', ".")
}

fn is_down_projection(key: &str) -> bool {
    key.ends_with("lora.down.weight")
        || key.ends_with("lora.a.weight")
        || (key.contains(".processor.") && key.ends_with(".down.weight"))
}

fn any_key(t: &[(String, &[u64])], needle: &str) -> bool {
    t.iter().any(|(k, _)| k.contains(needle))
}

// ---- checkpoints -------------------------------------------------------------

fn checkpoint_family(t: &[(String, &[u64])]) -> Option<&'static str> {
    let shape = |suffix: &str| t.iter().find(|(k, _)| k.ends_with(suffix)).map(|(_, s)| *s);
    let dim = |suffix: &str, i: usize| shape(suffix).and_then(|s| s.get(i).copied());

    if any_key(t, "adaln.single.emb.timestep.embedder.linear.1.bias") {
        if any_key(t, "pos.embed.proj.bias") {
            return None; // PixArt
        }
        if any_key(t, "audio.adaln.single") {
            return Some("ltx2");
        }
        return (dim("transformer.blocks.0.attn2.to.k.weight", 0) == Some(2048)).then_some("ltxv");
    }
    if any_key(t, "double.blocks.") {
        if let Some(channels) = dim("img.in.weight", 1) {
            return match (channels, dim("txt.in.weight", 1)) {
                (64, _) => Some("flux1"),
                (128, Some(7680)) => Some("flux2-klein-4b"),
                (128, Some(12288)) => Some("flux2-klein-9b"),
                _ => None,
            };
        }
    }
    if let Some(modulation) = shape("head.modulation") {
        return match modulation.last() {
            Some(3072) => Some("wan22-5b"),
            Some(5120) => Some("wan-14b"),
            _ => None,
        };
    }
    let unet = any_key(t, "input.blocks.") || any_key(t, "output.blocks.");
    if unet {
        let cross = t
            .iter()
            .find(|(k, _)| k.ends_with("attn2.to.k.weight"))
            .and_then(|(_, s)| s.get(1).copied());
        match cross {
            Some(2048) => return Some("sdxl"),
            Some(768) => return Some("sd15"),
            _ => {}
        }
    }
    if any_key(t, "conditioner.embedders.1.model.") {
        return Some("sdxl");
    }
    if any_key(t, "cond.stage.model.transformer.") && !any_key(t, "conditioner.") {
        return Some("sd15");
    }
    None
}

// ---- LoRAs -------------------------------------------------------------------

/// Input widths of the down projections, most frequent first.
fn down_widths(t: &[(String, &[u64])]) -> BTreeMap<u64, usize> {
    let mut counts = BTreeMap::new();
    for (k, s) in t {
        if is_down_projection(k) {
            if let Some(width) = s.get(1) {
                *counts.entry(*width).or_insert(0) += 1;
            }
        }
    }
    counts
}

/// Modules whose input is the model width in every transformer family here:
/// attention inputs (`qkv`, `to_q`, Wan's `q`/`k`) and FLUX's `linear1`.
const WIDTH_MODULES: &[&str] = &[
    "attn.qkv.",
    "attn.to.q.",
    "attn.to.qkv",
    "qkv.lora",
    ".linear1.",
    ".self.attn.q.",
    ".cross.attn.q.",
];

/// The model width a LoRA was trained against: the input width of an
/// attention (or FLUX `linear1`) down projection, else the most common down
/// projection input — ties go to the narrower, since the MLP inputs are
/// wider than the model.
fn model_width(t: &[(String, &[u64])], widths: &BTreeMap<u64, usize>) -> Option<u64> {
    let direct = t
        .iter()
        .find(|(k, _)| is_down_projection(k) && WIDTH_MODULES.iter().any(|m| k.contains(m)))
        .and_then(|(_, s)| s.get(1).copied());
    direct.or_else(|| {
        widths
            .iter()
            .max_by_key(|(width, count)| (**count, std::cmp::Reverse(**width)))
            .map(|(width, _)| *width)
    })
}

/// Order matters: a kohya FLUX LoRA may carry CLIP-L keys under
/// `lora_te1_`, and CLIP's own layers are named `self_attn` like Wan's —
/// so FLUX first, the SD text-encoder/UNet rules next, Wan last.
fn lora_family(t: &[(String, &[u64])]) -> Option<&'static str> {
    let widths = down_widths(t);
    let flux = [
        "double.blocks.",
        "single.blocks.",
        "single.transformer.blocks.",
        "stream.modulation",
    ]
    .iter()
    .any(|n| any_key(t, n));
    if flux {
        return flux_lora_family(t, &widths);
    }
    if let Some(sd) = sd_lora_family(t) {
        return sd;
    }
    if any_key(t, ".self.attn.") || any_key(t, ".cross.attn.") {
        return match model_width(t, &widths) {
            Some(3072) => Some("wan22-5b"),
            Some(5120) => Some("wan-14b"),
            _ => None,
        };
    }
    None
}

/// `Some(answer)` when the LoRA has SD text-encoder or UNet keys at all
/// (the answer itself may be "can't tell"); `None` when it is not an SD
/// LoRA.
fn sd_lora_family(t: &[(String, &[u64])]) -> Option<Option<&'static str>> {
    if any_key(t, "lora.te1.") || any_key(t, "lora.te2.") || any_key(t, "text.encoder.2.") {
        return Some(Some("sdxl"));
    }
    let unet = [
        "down.blocks.",
        "up.blocks.",
        "mid.block.",
        "input.blocks.",
        "output.blocks.",
    ]
    .iter()
    .any(|n| any_key(t, n));
    if unet {
        let cross = t
            .iter()
            .find(|(k, _)| k.contains("attn2.to.k.") && is_down_projection(k))
            .and_then(|(_, s)| s.get(1).copied());
        match cross {
            Some(2048) => return Some(Some("sdxl")),
            Some(768) => return Some(Some("sd15")),
            _ => {}
        }
    }
    // SD 1.x has a single text encoder: kohya `lora_te_`, diffusers
    // `text_encoder.` (SDXL's second one was caught above).
    if any_key(t, "lora.te.") || t.iter().any(|(k, _)| k.starts_with("text.encoder.")) {
        return Some(Some("sd15"));
    }
    unet.then_some(None)
}

/// FLUX.1 and FLUX.2 [klein] 4B are both 3072 wide; they differ in modules
/// only one of them has. FLUX.2 modulates globally
/// (`double_stream_modulation_*`, `single_stream_modulation`) and its MLP is
/// 3× wide (inputs of 9216 into `img_mlp.2`, 7680 into `txt_in`); FLUX.1
/// modulates per block (`img_mod.lin`, `norm1.linear`). The single-block
/// output `linear2` takes width + MLP width: 3072 + 9216 = 12288 on klein 4B,
/// 3072 + 12288 = 15360 on FLUX.1. XLabs' `processor` LoRA format only ever
/// existed for FLUX.1.
fn flux_lora_family(t: &[(String, &[u64])], widths: &BTreeMap<u64, usize>) -> Option<&'static str> {
    match model_width(t, widths)? {
        4096 => Some("flux2-klein-9b"),
        3072 => {
            let linear2 = t
                .iter()
                .find(|(k, _)| k.contains(".linear2.") && is_down_projection(k))
                .and_then(|(_, s)| s.get(1).copied());
            let flux2 = any_key(t, "stream.modulation")
                || widths.contains_key(&9216)
                || widths.contains_key(&7680)
                || linear2 == Some(12288);
            let flux1 = widths.contains_key(&15360)
                || any_key(t, "img.mod.lin.")
                || any_key(t, "txt.mod.lin.")
                || any_key(t, "norm1.linear.")
                || any_key(t, "norm1.context.linear.")
                || any_key(t, ".processor.")
                || t.iter().any(|(k, _)| {
                    k.contains(".modulation.lin.") && !k.contains("stream.modulation")
                });
            match (flux1, flux2) {
                (true, false) => Some("flux1"),
                (false, true) => Some("flux2-klein-4b"),
                _ => None,
            }
        }
        _ => None,
    }
}

// ---- file names --------------------------------------------------------------

/// The family a file (or display) name suggests — the weakest hint, used
/// only when nothing better is known. Letters and digits only, lowercase,
/// so `flux-2-klein-4b`, `flux2_klein_4B` and `Flux2Klein4b` read alike.
pub fn family_from_name(name: &str) -> Option<&'static BaseFamily> {
    let n: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let has = |needle: &str| n.contains(needle);
    let id = if has("flux2") || has("klein") {
        if has("4b") {
            "flux2-klein-4b"
        } else if has("9b") {
            "flux2-klein-9b"
        } else {
            return None;
        }
    } else if has("flux") {
        if has("schnell") {
            "flux1-schnell"
        } else if has("krea") {
            "flux1-krea"
        } else {
            "flux1"
        }
    } else if has("pony") {
        "pony"
    } else if has("illustrious") {
        "illustrious"
    } else if has("noobai") {
        "noobai"
    } else if has("ltx") {
        if has("ltx2") || has("ltxv2") {
            "ltx2"
        } else if has("2b") {
            "ltxv"
        } else {
            return None;
        }
    } else if has("wan2") {
        if has("14b") {
            "wan-14b"
        } else if has("5b") {
            "wan22-5b"
        } else {
            return None;
        }
    } else if has("xl") {
        "sdxl"
    } else if has("sd15") || n.starts_with("v15") {
        "sd15"
    } else {
        return None;
    };
    family_by_id(id)
}
