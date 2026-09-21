use std::collections::BTreeMap;

use super::*;
use crate::db::Model;
use crate::model::SafetensorsHeader;

fn id(f: Option<&'static BaseFamily>) -> Option<&'static str> {
    f.map(|f| f.id)
}

// ---- Civitai labels -------------------------------------------------------

/// Every `baseModel` label in the 60 most downloaded Civitai LoRAs on
/// 2026-09-21, with the family it must resolve to (`None` = unknown base,
/// reported as "not runnable here" by the resolver).
const SAMPLE_2026_09_21: &[(&str, Option<&str>)] = &[
    ("SD 1.5", Some("sd15")),
    ("Wan Video 2.2 I2V-A14B", Some("wan-14b")),
    ("Pony", Some("pony")),
    ("SDXL 1.0", Some("sdxl")),
    ("Krea 2", Some("krea2")),
    ("ZImageBase", None),
    ("Anima", None),
    ("Illustrious", Some("illustrious")),
    ("LTXV 2.3", Some("ltx2")),
    ("MiniMax H3", None),
    ("Wan Video 2.2 T2V-A14B", Some("wan-14b")),
    ("NoobAI", Some("noobai")),
    ("Flux.1 D", Some("flux1")),
    ("Other", None),
];

#[test]
fn every_label_of_the_2026_09_21_sample_maps_as_decided() {
    for (label, want) in SAMPLE_2026_09_21 {
        assert_eq!(id(family_for_civitai(label)), *want, "label {label:?}");
    }
}

/// The rest of Civitai's `BaseModel` enum (`GET /api/v1/enums`, 2026-09-21)
/// that this registry maps.
#[test]
fn further_civitai_labels_from_the_enum_map_to_their_family() {
    let cases = [
        ("SD 1.4", "sd15"),
        ("SD 1.5 LCM", "sd15"),
        ("SD 1.5 Hyper", "sd15"),
        ("SDXL 0.9", "sdxl"),
        ("SDXL 1.0 LCM", "sdxl"),
        ("SDXL Lightning", "sdxl"),
        ("SDXL Hyper", "sdxl"),
        ("SDXL Turbo", "sdxl"),
        ("Flux.1 S", "flux1-schnell"),
        ("Flux.1 Krea", "flux1-krea"),
        ("Flux.2 Klein 4B", "flux2-klein-4b"),
        ("Flux.2 Klein 4B-base", "flux2-klein-4b"),
        ("Flux.2 Klein 9B", "flux2-klein-9b"),
        ("Flux.2 Klein 9B-base", "flux2-klein-9b"),
        ("Wan Video 2.2 TI2V-5B", "wan22-5b"),
        ("Wan Video 14B t2v", "wan-14b"),
        ("Wan Video 14B i2v 480p", "wan-14b"),
        ("Wan Video 14B i2v 720p", "wan-14b"),
        ("LTXV", "ltxv"),
        ("LTXV2", "ltx2"),
    ];
    for (label, want) in cases {
        assert_eq!(id(family_for_civitai(label)), Some(want), "label {label:?}");
    }
}

#[test]
fn labels_whose_architecture_differs_or_is_unconfirmed_stay_unmapped() {
    for label in [
        "Pony V7",        // AuraFlow-based, not SDXL
        "SDXL Distilled", // pruned UNet — SDXL LoRAs don't fit it
        "Flux.1 Kontext",
        "Flux.2 D",
        "LTXV 2.5",
        "Wan Video 1.3B t2v",
        "Wan Video 2.5 T2V",
        "SD 2.1",
        "SD 3.5",
        "ZImageTurbo",
        "Qwen",
        "Chroma",
        "",
        "   ",
    ] {
        assert_eq!(id(family_for_civitai(label)), None, "label {label:?}");
    }
}

#[test]
fn civitai_matching_is_trimmed_and_case_insensitive() {
    assert_eq!(id(family_for_civitai("  sdxl 1.0 ")), Some("sdxl"));
    assert_eq!(id(family_for_civitai("PONY")), Some("pony"));
}

#[test]
fn every_label_and_hf_id_belongs_to_exactly_one_family() {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for f in FAMILIES {
        for key in f.civitai_labels.iter().chain(f.hf_base_models) {
            let prev = seen.insert(key.to_ascii_lowercase(), f.id);
            assert!(prev.is_none(), "{key:?} is in {prev:?} and {}", f.id);
        }
    }
    let ids: Vec<_> = FAMILIES.iter().map(|f| f.id).collect();
    let mut dedup = ids.clone();
    dedup.sort_unstable();
    dedup.dedup();
    assert_eq!(ids.len(), dedup.len(), "duplicate family id");
}

#[test]
fn stack_ids_point_at_real_catalog_stacks() {
    for f in FAMILIES {
        if let Some(stack) = f.stack_id {
            assert!(
                crate::model::MODEL_STACKS.iter().any(|s| s.id == stack),
                "{} names missing stack {stack}",
                f.id
            );
        }
    }
    assert_eq!(family_by_id("sd15").and_then(|f| f.stack_id), Some("sd15"));
    assert_eq!(family_by_id("sdxl").and_then(|f| f.stack_id), Some("sdxl"));
    assert_eq!(family_by_id("flux1").and_then(|f| f.stack_id), Some("flux"));
    assert_eq!(
        family_by_id("flux2-klein-9b").and_then(|f| f.stack_id),
        Some("flux2-klein")
    );
    assert_eq!(
        family_by_id("wan22-5b").and_then(|f| f.stack_id),
        Some("wan22")
    );
    assert_eq!(family_by_id("ltxv").and_then(|f| f.stack_id), Some("ltx"));
    assert_eq!(
        family_by_id("krea2").and_then(|f| f.stack_id),
        Some("krea2")
    );
    // Community checkpoints are found on Civitai, not pinned.
    for community in ["pony", "illustrious", "noobai"] {
        assert_eq!(family_by_id(community).and_then(|f| f.stack_id), None);
    }
}

#[test]
fn wan_14b_and_ltx2_are_known_but_not_runnable() {
    let wan = family_for_civitai("Wan Video 2.2 I2V-A14B");
    assert_eq!(
        wan.map(|f| f.runnable),
        Some(Runnable::No("does not fit 16 GB — this app runs the 5B"))
    );
    let ltx2 = family_for_civitai("LTXV 2.3");
    assert!(matches!(ltx2.map(|f| f.runnable), Some(Runnable::No(_))));
    for runnable in [
        "sd15",
        "sdxl",
        "pony",
        "flux1",
        "flux2-klein-4b",
        "wan22-5b",
        "ltxv",
        "krea2",
    ] {
        assert_eq!(
            family_by_id(runnable).map(|f| f.runnable),
            Some(Runnable::Yes),
            "{runnable}"
        );
    }
}

// ---- Hugging Face base_model ----------------------------------------------

#[test]
fn hf_base_models_map_with_or_without_the_tag_prefix() {
    let cases = [
        ("stabilityai/stable-diffusion-xl-base-1.0", Some("sdxl")),
        (
            "base_model:adapter:stabilityai/stable-diffusion-xl-base-1.0",
            Some("sdxl"),
        ),
        (
            "base_model:stable-diffusion-v1-5/stable-diffusion-v1-5",
            Some("sd15"),
        ),
        ("runwayml/stable-diffusion-v1-5", Some("sd15")),
        ("Black-Forest-Labs/FLUX.1-dev", Some("flux1")),
        ("black-forest-labs/FLUX.2-klein-4B", Some("flux2-klein-4b")),
        (
            "black-forest-labs/FLUX.2-klein-base-9B",
            Some("flux2-klein-9b"),
        ),
        ("Wan-AI/Wan2.2-TI2V-5B-Diffusers", Some("wan22-5b")),
        ("Wan-AI/Wan2.2-I2V-A14B", Some("wan-14b")),
        ("Lightricks/LTX-Video", Some("ltxv")),
        ("Lightricks/LTX-2.3", Some("ltx2")),
        ("Laxhar/noobai-XL-1.0", Some("noobai")),
        ("someone/unknown-model", None),
        ("", None),
    ];
    for (hf, want) in cases {
        assert_eq!(id(family_for_hf(hf)), want, "hf {hf:?}");
    }
}

// ---- works with / made for ------------------------------------------------

#[test]
fn a_pony_lora_works_with_sdxl_but_is_made_for_pony() {
    let pony = family_by_id("pony").map(|f| f.id);
    let (Some(pony), Some(sdxl)) = (family_by_id("pony"), family_by_id("sdxl")) else {
        panic!("registry is missing pony/sdxl: {pony:?}");
    };
    assert!(works_with(pony, sdxl));
    assert!(works_with(sdxl, pony));
    assert!(!made_for(pony, sdxl));
    assert!(made_for(pony, pony));
    assert_eq!(fit(pony, sdxl), Fit::WorksWith);
    assert_eq!(fit(pony, pony), Fit::MadeFor);
}

#[test]
fn different_architectures_do_not_work_together() {
    let get = |id: &str| family_by_id(id).unwrap_or_else(|| panic!("no {id}"));
    assert!(!works_with(get("sd15"), get("sdxl")));
    assert!(!works_with(get("flux1"), get("flux2-klein-4b")));
    assert!(!works_with(get("flux2-klein-4b"), get("flux2-klein-9b")));
    assert!(!works_with(get("wan22-5b"), get("wan-14b")));
    assert!(!works_with(get("ltxv"), get("ltx2")));
    assert_eq!(fit(get("sd15"), get("sdxl")), Fit::Incompatible);
    // Same architecture, different fine-tune.
    assert!(works_with(get("flux1-schnell"), get("flux1")));
    assert!(works_with(get("illustrious"), get("noobai")));
}

// ---- library strings --------------------------------------------------------

#[test]
fn existing_library_family_strings_normalize_onto_registry_ids() {
    let cases = [
        ("sdxl", Some("sdxl")),
        ("flux", Some("flux1")),
        ("flux2", Some("flux2-klein-9b")),
        ("flux2-klein-4b", Some("flux2-klein-4b")),
        ("flux2_klein_9b", Some("flux2-klein-9b")),
        ("wan", Some("wan22-5b")),
        ("ltx", Some("ltxv")),
        ("  SDXL ", Some("sdxl")),
        ("Pony", Some("pony")),
        ("sd15", Some("sd15")),
        ("qwen2", None),
        ("florence2", None),
        ("sd3", None),
        ("", None),
    ];
    for (s, want) in cases {
        assert_eq!(id(normalize_library_family(s)), want, "library {s:?}");
    }
}

// ---- header detection -------------------------------------------------------

fn header(tensors: &[(&str, &[u64])]) -> SafetensorsHeader {
    SafetensorsHeader {
        tensors: tensors
            .iter()
            .map(|(k, s)| ((*k).to_string(), s.to_vec()))
            .collect(),
        metadata: BTreeMap::new(),
    }
}

fn from_header(tensors: &[(&str, &[u64])]) -> Option<&'static str> {
    id(family_from_header(&header(tensors)))
}

#[test]
fn sdxl_and_sd15_checkpoints_are_told_apart() {
    // Key names from sd_xl_base_1.0 / v1-5-pruned-emaonly (HF headers).
    let sdxl = from_header(&[
        (
            "model.diffusion_model.input_blocks.0.0.weight",
            &[320, 4, 3, 3],
        ),
        ("model.diffusion_model.label_emb.0.0.weight", &[1280, 2816]),
        (
            "conditioner.embedders.1.model.transformer.resblocks.0.attn.in_proj_weight",
            &[3840, 1280],
        ),
    ]);
    assert_eq!(sdxl, Some("sdxl"));
    let sd15 = from_header(&[
        (
            "model.diffusion_model.input_blocks.0.0.weight",
            &[320, 4, 3, 3],
        ),
        (
            "cond_stage_model.transformer.text_model.embeddings.token_embedding.weight",
            &[49408, 768],
        ),
    ]);
    assert_eq!(sd15, Some("sd15"));
    // A bare UNet: the cross-attention width decides.
    let bare_xl = from_header(&[(
        "input_blocks.4.1.transformer_blocks.0.attn2.to_k.weight",
        &[640, 2048],
    )]);
    assert_eq!(bare_xl, Some("sdxl"));
    let bare_15 = from_header(&[(
        "input_blocks.1.1.transformer_blocks.0.attn2.to_k.weight",
        &[320, 768],
    )]);
    assert_eq!(bare_15, Some("sd15"));
}

#[test]
fn flux_checkpoints_are_told_apart_by_their_input_widths() {
    // flux1-schnell-fp8 (Comfy-Org): img_in [3072, 64], txt_in [3072, 4096].
    let flux1 = from_header(&[
        (
            "model.diffusion_model.double_blocks.0.img_mod.lin.weight",
            &[18432, 3072],
        ),
        ("model.diffusion_model.img_in.weight", &[3072, 64]),
        ("model.diffusion_model.txt_in.weight", &[3072, 4096]),
    ]);
    assert_eq!(flux1, Some("flux1"));
    // FLUX.2 klein 9B: img_in [4096, 128], txt_in [4096, 12288].
    let klein9 = from_header(&[
        (
            "model.diffusion_model.double_blocks.0.img_attn.qkv.weight",
            &[12288, 4096],
        ),
        (
            "model.diffusion_model.double_stream_modulation_img.lin.weight",
            &[24576, 4096],
        ),
        ("model.diffusion_model.img_in.weight", &[4096, 128]),
        ("model.diffusion_model.txt_in.weight", &[4096, 12288]),
    ]);
    assert_eq!(klein9, Some("flux2-klein-9b"));
    // FLUX.2 klein 4B: joint_attention_dim 7680 (diffusers config).
    let klein4 = from_header(&[
        ("double_blocks.0.img_attn.qkv.weight", &[9216, 3072]),
        ("img_in.weight", &[3072, 128]),
        ("txt_in.weight", &[3072, 7680]),
    ]);
    assert_eq!(klein4, Some("flux2-klein-4b"));
    // FLUX.2 [dev] (Mistral encoder, 15360) is not in the registry.
    let dev = from_header(&[
        ("double_blocks.0.img_attn.qkv.weight", &[18432, 6144]),
        ("img_in.weight", &[6144, 128]),
        ("txt_in.weight", &[6144, 15360]),
    ]);
    assert_eq!(dev, None);
}

#[test]
fn wan_checkpoints_are_told_apart_by_width() {
    // wan2.2_ti2v_5B_fp16: head.modulation [1, 2, 3072]; the 14B: 5120.
    let five = from_header(&[
        ("head.modulation", &[1, 2, 3072]),
        ("patch_embedding.weight", &[3072, 48, 1, 2, 2]),
        ("blocks.0.ffn.0.weight", &[14336, 3072]),
    ]);
    assert_eq!(five, Some("wan22-5b"));
    let fourteen = from_header(&[
        ("head.modulation", &[1, 2, 5120]),
        ("patch_embedding.weight", &[5120, 16, 1, 2, 2]),
    ]);
    assert_eq!(fourteen, Some("wan-14b"));
    let small = from_header(&[
        ("head.modulation", &[1, 2, 1536]),
        ("patch_embedding.weight", &[1536, 16, 1, 2, 2]),
    ]);
    assert_eq!(small, None);
}

#[test]
fn ltx_checkpoints_split_into_the_2b_and_ltx2() {
    // ltx-video-2b-v0.9.5: attn2.to_k [2048, 2048].
    let two_b = from_header(&[
        (
            "model.diffusion_model.adaln_single.emb.timestep_embedder.linear_1.bias",
            &[2048],
        ),
        (
            "model.diffusion_model.transformer_blocks.0.attn2.to_k.weight",
            &[2048, 2048],
        ),
    ]);
    assert_eq!(two_b, Some("ltxv"));
    let ltx2 = from_header(&[
        ("adaln_single.emb.timestep_embedder.linear_1.bias", &[4096]),
        ("audio_adaln_single.linear.weight", &[4096, 4096]),
        ("transformer_blocks.0.attn2.to_k.weight", &[4096, 4096]),
    ]);
    assert_eq!(ltx2, Some("ltx2"));
    let thirteen_b = from_header(&[
        ("adaln_single.emb.timestep_embedder.linear_1.bias", &[4096]),
        ("transformer_blocks.0.attn2.to_k.weight", &[4096, 4096]),
    ]);
    assert_eq!(thirteen_b, None);
    // PixArt shares adaln_single but has pos_embed.proj.
    let pixart = from_header(&[
        ("adaln_single.emb.timestep_embedder.linear_1.bias", &[1152]),
        ("pos_embed.proj.bias", &[1152]),
    ]);
    assert_eq!(pixart, None);
}

#[test]
fn sd_loras_are_told_apart_by_text_encoders_and_cross_attention() {
    // kohya SD 1.5 (te only, no te1/te2), from an HF SD 1.5 LoRA.
    let sd15 = from_header(&[
        (
            "lora_te_text_model_encoder_layers_0_mlp_fc1.lora_down.weight",
            &[32, 768],
        ),
        (
            "lora_unet_down_blocks_0_attentions_0_proj_in.lora_down.weight",
            &[32, 320, 1, 1],
        ),
    ]);
    assert_eq!(sd15, Some("sd15"));
    let sdxl_te = from_header(&[
        (
            "lora_te1_text_model_encoder_layers_0_mlp_fc1.lora_down.weight",
            &[8, 768],
        ),
        (
            "lora_te2_text_model_encoder_layers_0_mlp_fc1.lora_down.weight",
            &[8, 1280],
        ),
    ]);
    assert_eq!(sdxl_te, Some("sdxl"));
    // UNet-only (lcm-lora-sdxl / lcm-lora-sdv1-5): attn2 to_k input width.
    let unet_xl = from_header(&[(
        "lora_unet_up_blocks_0_attentions_0_transformer_blocks_1_attn2_to_k.lora_down.weight",
        &[64, 2048],
    )]);
    assert_eq!(unet_xl, Some("sdxl"));
    let unet_15 = from_header(&[(
        "lora_unet_up_blocks_1_attentions_0_transformer_blocks_0_attn2_to_k.lora_down.weight",
        &[64, 768],
    )]);
    assert_eq!(unet_15, Some("sd15"));
    // diffusers/PEFT naming.
    let peft_xl = from_header(&[
        (
            "unet.down_blocks.1.attentions.0.transformer_blocks.0.attn2.to_k.lora_A.weight",
            &[16, 2048],
        ),
        (
            "text_encoder_2.text_model.encoder.layers.0.mlp.fc1.lora_A.weight",
            &[16, 1280],
        ),
    ]);
    assert_eq!(peft_xl, Some("sdxl"));
}

#[test]
fn flux_loras_are_told_apart_by_width_and_flux2_only_modules() {
    // Flux2-Klein-9B-consistency-V2 (HF): width 4096.
    let klein9 = from_header(&[
        (
            "diffusion_model.double_blocks.0.img_attn.qkv.lora_A.weight",
            &[64, 4096],
        ),
        (
            "diffusion_model.double_blocks.0.img_attn.proj.lora_A.weight",
            &[64, 4096],
        ),
        (
            "diffusion_model.single_blocks.0.linear2.lora_A.weight",
            &[64, 16384],
        ),
    ]);
    assert_eq!(klein9, Some("flux2-klein-9b"));
    // tryon-klein-4b (HF): width 3072, linear2 in 12288, img_mlp.2 in 9216.
    let klein4 = from_header(&[
        (
            "diffusion_model.double_blocks.0.img_attn.qkv.lora_A.weight",
            &[32, 3072],
        ),
        (
            "diffusion_model.double_blocks.0.img_attn.proj.lora_A.weight",
            &[32, 3072],
        ),
        (
            "diffusion_model.double_blocks.0.img_mlp.2.lora_A.weight",
            &[32, 9216],
        ),
        (
            "diffusion_model.single_blocks.0.linear2.lora_A.weight",
            &[32, 12288],
        ),
    ]);
    assert_eq!(klein4, Some("flux2-klein-4b"));
    // diffusers FLUX.2 4B naming (pixel-art-lora): global modulation.
    let klein4_diffusers = from_header(&[
        (
            "transformer.transformer_blocks.1.attn.to_q.lora_A.weight",
            &[64, 3072],
        ),
        (
            "transformer.single_transformer_blocks.0.attn.to_out.lora_A.weight",
            &[64, 3072],
        ),
        (
            "transformer.double_stream_modulation_img.linear.lora_A.weight",
            &[64, 3072],
        ),
    ]);
    assert_eq!(klein4_diffusers, Some("flux2-klein-4b"));
    // kohya FLUX.1: per-block modulation and a 15360-wide linear2.
    let flux1 = from_header(&[
        (
            "lora_unet_double_blocks_0_img_attn_qkv.lora_down.weight",
            &[16, 3072],
        ),
        (
            "lora_unet_double_blocks_0_img_mod_lin.lora_down.weight",
            &[16, 3072],
        ),
        (
            "lora_unet_single_blocks_0_linear2.lora_down.weight",
            &[16, 15360],
        ),
    ]);
    assert_eq!(flux1, Some("flux1"));
    // XLabs format (flux-RealismLora) only ever existed for FLUX.1.
    let xlabs = from_header(&[
        (
            "double_blocks.0.processor.qkv_lora1.down.weight",
            &[16, 3072],
        ),
        (
            "double_blocks.0.processor.proj_lora1.down.weight",
            &[16, 3072],
        ),
    ]);
    assert_eq!(xlabs, Some("flux1"));
    // Width 3072 with nothing that tells FLUX.1 from klein 4B.
    let ambiguous = from_header(&[
        (
            "diffusion_model.double_blocks.0.img_attn.qkv.lora_A.weight",
            &[16, 3072],
        ),
        (
            "diffusion_model.double_blocks.0.img_attn.proj.lora_A.weight",
            &[16, 3072],
        ),
    ]);
    assert_eq!(ambiguous, None);
}

#[test]
fn wan_loras_are_told_apart_by_width() {
    // wan22_5b_i2v_crush_it_lora (HF): 3072; 80s_fantasy wan 2.2 14B: 5120.
    let five = from_header(&[
        (
            "diffusion_model.blocks.0.cross_attn.k.lora_A.weight",
            &[32, 3072],
        ),
        (
            "diffusion_model.blocks.0.self_attn.q.lora_A.weight",
            &[32, 3072],
        ),
        ("diffusion_model.blocks.0.ffn.2.lora_A.weight", &[32, 14336]),
    ]);
    assert_eq!(five, Some("wan22-5b"));
    let fourteen = from_header(&[
        (
            "diffusion_model.blocks.0.cross_attn.k.lora_A.weight",
            &[32, 5120],
        ),
        (
            "diffusion_model.blocks.0.self_attn.q.lora_A.weight",
            &[32, 5120],
        ),
    ]);
    assert_eq!(fourteen, Some("wan-14b"));
    let kohya = from_header(&[(
        "lora_unet_blocks_0_cross_attn_k.lora_down.weight",
        &[16, 5120],
    )]);
    assert_eq!(kohya, Some("wan-14b"));
}

#[test]
fn an_unrecognised_header_says_nothing() {
    assert_eq!(from_header(&[]), None);
    assert_eq!(
        from_header(&[("model.layers.0.self_attn.q_proj.weight", &[4096, 4096])]),
        None
    );
}

// ---- file names -------------------------------------------------------------

#[test]
fn file_names_are_the_weakest_hint() {
    let cases = [
        ("flux-2-klein-4b-fp8.safetensors", Some("flux2-klein-4b")),
        ("flux2_klein_9b_Q4_K_M.gguf", Some("flux2-klein-9b")),
        ("flux1-dev-Q8_0.gguf", Some("flux1")),
        ("flux1-schnell-fp8.safetensors", Some("flux1-schnell")),
        ("ponyDiffusionV6XL.safetensors", Some("pony")),
        ("illustriousXL_v01.safetensors", Some("illustrious")),
        ("noobaiXLNAIXL_vPred10.safetensors", Some("noobai")),
        ("sd_xl_base_1.0.safetensors", Some("sdxl")),
        ("add-detail-xl.safetensors", Some("sdxl")),
        ("v1-5-pruned-emaonly.safetensors", Some("sd15")),
        ("wan2.2_ti2v_5B_fp16.safetensors", Some("wan22-5b")),
        (
            "wan2.2_t2v_high_noise_14B_fp8_scaled.safetensors",
            Some("wan-14b"),
        ),
        ("ltx-video-2b-v0.9.5.safetensors", Some("ltxv")),
        ("ltx-2.3-22b-dev.safetensors", Some("ltx2")),
        ("my_style.safetensors", None),
    ];
    for (name, want) in cases {
        assert_eq!(id(family_from_name(name)), want, "name {name:?}");
    }
}

// ---- inference order ----------------------------------------------------------

fn model(family: Option<&str>, family_source: Option<&str>, file: &str) -> Model {
    Model {
        id: "m".into(),
        publisher: None,
        name: file.into(),
        family: family.map(str::to_string),
        base_family: None,
        family_source: family_source.map(str::to_string),
        format: "safetensors".into(),
        quant: None,
        arch: None,
        param_count: None,
        file_path: format!("E:\\AI\\models\\loras\\{file}"),
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
        roles: vec!["lora".into()],
        runtimes: vec![],
    }
}

fn klein4_lora() -> SafetensorsHeader {
    header(&[
        (
            "diffusion_model.double_blocks.0.img_attn.qkv.lora_A.weight",
            &[32, 3072],
        ),
        (
            "diffusion_model.single_blocks.0.linear2.lora_A.weight",
            &[32, 12288],
        ),
    ])
}

fn inferred(m: &Model, h: Option<&SafetensorsHeader>) -> Option<(&'static str, FamilySource)> {
    infer_family(m, h).map(|(f, s)| (f.id, s))
}

/// A model with a recorded `base_family` (and `family_source`) next to its
/// legacy `family` string.
fn recorded(base_family: &str, source: Option<&str>, legacy: Option<&str>, file: &str) -> Model {
    Model {
        base_family: Some(base_family.into()),
        ..model(legacy, source, file)
    }
}

#[test]
fn a_recorded_base_family_beats_the_header_and_the_name() {
    let m = recorded(
        "pony",
        Some("civitai"),
        None,
        "flux2-klein-9b-thing.safetensors",
    );
    let h = klein4_lora();
    assert_eq!(
        inferred(&m, Some(&h)),
        Some(("pony", FamilySource::Civitai))
    );
    let user = recorded("sdxl", Some("user"), None, "x.safetensors");
    assert_eq!(
        inferred(&user, Some(&h)),
        Some(("sdxl", FamilySource::User))
    );
}

#[test]
fn a_recorded_base_family_beats_a_contradicting_legacy_family() {
    // The trainer stores both klein sizes as "flux2"; the recorded base
    // family is the authority, the legacy string stays as it is.
    let m = recorded("flux2-klein-4b", Some("hf"), Some("flux2"), "x.safetensors");
    assert_eq!(
        inferred(&m, None),
        Some(("flux2-klein-4b", FamilySource::Hf))
    );
    let catalog = Model {
        source_revision: Some("catalog:flux2-klein-9b-q4".into()),
        ..recorded("flux2-klein-4b", Some("user"), Some("flux2"), "x.gguf")
    };
    assert_eq!(
        inferred(&catalog, None),
        Some(("flux2-klein-4b", FamilySource::User))
    );
}

#[test]
fn a_family_source_without_a_base_family_records_nothing() {
    // family_source describes base_family; next to a bare legacy string it
    // is not a recorded family.
    let m = model(Some("pony"), Some("civitai"), "x.safetensors");
    assert_eq!(inferred(&m, None), Some(("pony", FamilySource::Name)));
}

#[test]
fn a_base_family_without_its_source_counts_as_the_weakest_hint() {
    let m = recorded("pony", None, None, "x.safetensors");
    assert_eq!(inferred(&m, None), Some(("pony", FamilySource::Name)));
}

#[test]
fn an_unambiguous_legacy_family_beats_the_header() {
    let m = model(Some("sdxl"), None, "x.safetensors");
    let h = klein4_lora();
    assert_eq!(inferred(&m, Some(&h)), Some(("sdxl", FamilySource::Name)));
}

#[test]
fn an_ambiguous_legacy_family_lets_the_header_decide() {
    // A trained klein-4B LoRA is stored as "flux2" (settle::library_family).
    let m = model(Some("flux2"), None, "my_lora.safetensors");
    let h = klein4_lora();
    assert_eq!(
        inferred(&m, Some(&h)),
        Some(("flux2-klein-4b", FamilySource::Header))
    );
    // Without a header it falls back to the catalog meaning of the string.
    assert_eq!(
        inferred(&m, None),
        Some(("flux2-klein-9b", FamilySource::Name))
    );
}

#[test]
fn a_catalog_row_keeps_its_catalog_family() {
    let mut m = model(Some("flux2"), None, "flux-2-klein-9b-Q4_K_M.gguf");
    m.source_revision = Some("catalog:flux2-klein-9b-q4".into());
    assert_eq!(
        inferred(&m, Some(&klein4_lora())),
        Some(("flux2-klein-9b", FamilySource::Catalog))
    );
}

#[test]
fn the_header_beats_the_file_name() {
    let m = model(None, None, "flux1-dev-lora.safetensors");
    assert_eq!(
        inferred(&m, Some(&klein4_lora())),
        Some(("flux2-klein-4b", FamilySource::Header))
    );
}

#[test]
fn the_file_name_is_used_last() {
    let m = model(None, None, "ponyDiffusionV6XL.safetensors");
    assert_eq!(inferred(&m, None), Some(("pony", FamilySource::Name)));
    let unknown = model(Some("qwen2"), None, "model.gguf");
    assert_eq!(inferred(&unknown, None), None);
}

#[test]
fn a_recorded_family_the_registry_does_not_know_falls_through() {
    let m = recorded("zimage", Some("civitai"), None, "sd_xl_thing.safetensors");
    assert_eq!(inferred(&m, None), Some(("sdxl", FamilySource::Name)));
}

#[test]
fn family_source_round_trips_through_its_column_value() {
    for s in [
        FamilySource::Civitai,
        FamilySource::Hf,
        FamilySource::Catalog,
        FamilySource::Header,
        FamilySource::Name,
        FamilySource::User,
    ] {
        assert_eq!(FamilySource::parse(s.as_str()), Some(s));
    }
    assert_eq!(FamilySource::parse(" USER "), Some(FamilySource::User));
    assert_eq!(FamilySource::parse("guess"), None);
}

// ---- real-run findings (2026-09-21) ---------------------------------------------

/// A library row shaped like the real one: legacy `family`, the catalog tag
/// the import recorded, its roles.
fn real_row(file: &str, legacy: Option<&str>, catalog: Option<&str>, roles: &[&str]) -> Model {
    Model {
        source_revision: catalog.map(|c| format!("catalog:{c}")),
        roles: roles.iter().map(|r| (*r).to_string()).collect(),
        ..model(legacy, None, file)
    }
}

fn sdxl_checkpoint_header() -> SafetensorsHeader {
    header(&[
        ("model.diffusion_model.label_emb.0.0.weight", &[1280, 2816]),
        (
            "conditioner.embedders.1.model.transformer.resblocks.0.attn.in_proj_weight",
            &[3840, 1280],
        ),
    ])
}

#[test]
fn a_catalog_text_encoder_has_no_base_family_whatever_its_legacy_string() {
    // Both real rows carry `models.family = 'sdxl'` from an old import.
    let t5 = real_row(
        "t5xxl_fp8_e4m3fn.safetensors",
        Some("sdxl"),
        Some("t5xxl-fp8"),
        &["text_encoder"],
    );
    assert_eq!(inferred(&t5, None), None);
    let umt5 = real_row(
        "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
        Some("sdxl"),
        Some("wan-umt5-xxl-fp8"),
        &["text_encoder"],
    );
    assert_eq!(inferred(&umt5, None), None);
}

#[test]
fn a_catalog_row_takes_its_family_from_the_catalog_entry() {
    // A wrong legacy string on a catalog row loses to the entry's own family.
    let base = real_row(
        "sd_xl_base_1.0.safetensors",
        Some("flux"),
        Some("sdxl-base-1.0"),
        &["base_diffusion"],
    );
    assert_eq!(inferred(&base, None), Some(("sdxl", FamilySource::Catalog)));
    let vae = real_row("ae.safetensors", Some("sdxl"), Some("flux-vae"), &["vae"]);
    assert_eq!(inferred(&vae, None), Some(("flux1", FamilySource::Catalog)));
    // A catalog id this build does not know keeps the stored string.
    let gone = real_row("x.safetensors", Some("sdxl"), Some("retired-entry"), &[]);
    assert_eq!(inferred(&gone, None), Some(("sdxl", FamilySource::Catalog)));
}

#[test]
fn a_subfamily_in_the_name_beats_a_generic_stored_sdxl() {
    for (file, want) in [
        ("hassakuXLIllustrious_v34.safetensors", "illustrious"),
        ("ponyDiffusionV6XL_v6StartWithThisOne.safetensors", "pony"),
        ("noobaiXLNAIXL_vPred10.safetensors", "noobai"),
        ("NoobAI-XL-v1.1.safetensors", "noobai"),
        ("sdxl_pony_style.safetensors", "pony"),
        ("animagineXLV31_v31.safetensors", "sdxl"),
        ("unnamedixlRealisticModel_v7.safetensors", "sdxl"),
    ] {
        let m = real_row(file, Some("sdxl"), None, &["base_diffusion"]);
        assert_eq!(
            inferred(&m, None),
            Some((want, FamilySource::Name)),
            "file {file:?}"
        );
    }
}

#[test]
fn a_subfamily_word_inside_another_word_does_not_flip_sdxl() {
    for file in [
        "ponytailHelperXL.safetensors",
        "caponyXL_v2.safetensors",
        "epicponyness.safetensors",
        "illustriousnessXL.safetensors",
        "PONYTAILXL.safetensors",
    ] {
        let m = real_row(file, Some("sdxl"), None, &["lora"]);
        assert_eq!(
            inferred(&m, None),
            Some(("sdxl", FamilySource::Name)),
            "file {file:?}"
        );
    }
}

#[test]
fn a_subfamily_name_never_overrides_a_user_catalog_or_header_result() {
    let user = recorded(
        "sdxl",
        Some("user"),
        Some("sdxl"),
        "hassakuXLIllustrious_v34.safetensors",
    );
    assert_eq!(inferred(&user, None), Some(("sdxl", FamilySource::User)));
    let catalog = real_row(
        "ponyRenamed_sd_xl_base.safetensors",
        Some("sdxl"),
        Some("sdxl-base-1.0"),
        &["base_diffusion"],
    );
    assert_eq!(
        inferred(&catalog, None),
        Some(("sdxl", FamilySource::Catalog))
    );
    // No stored string: the header decides, the name does not flip it.
    let header_only = real_row("hassakuXLIllustrious_v34.safetensors", None, None, &[]);
    assert_eq!(
        inferred(&header_only, Some(&sdxl_checkpoint_header())),
        Some(("sdxl", FamilySource::Header))
    );
    // Only the arch-group root flips: a stored `pony` stays pony.
    let pony = real_row("illustriousMix.safetensors", Some("pony"), None, &[]);
    assert_eq!(inferred(&pony, None), Some(("pony", FamilySource::Name)));
}

// ---- Krea 2 -------------------------------------------------------------------

#[test]
fn krea2_is_a_runnable_family_of_its_own() {
    let krea = family_by_id("krea2").expect("krea2 is in the registry");
    assert_eq!(krea.label, "Krea 2");
    assert_eq!(krea.civitai_labels, &["Krea 2"]);
    assert_eq!(krea.arch_group, ArchGroup::Krea2);
    assert_eq!(krea.runnable, Runnable::Yes);
    assert_eq!(id(family_for_civitai("Krea 2")), Some("krea2"));
    // FLUX.1 Krea [dev] is a different model with FLUX.1's architecture.
    assert_eq!(id(family_for_civitai("Flux.1 Krea")), Some("flux1-krea"));
    let flux_krea = family_by_id("flux1-krea").expect("flux1-krea");
    assert_eq!(fit(krea, flux_krea), Fit::Incompatible);
    assert_eq!(id(normalize_library_family("krea2")), Some("krea2"));
}

/// Keys of `lustifyNSFWCheckpoint_v10Krea2` (430 F8 tensors, no text
/// encoder, no VAE), under `prefix`.
fn krea2_diffusion_header(prefix: &str) -> SafetensorsHeader {
    SafetensorsHeader {
        tensors: [
            "blocks.0.attn.gate.weight",
            "blocks.0.attn.wq.weight",
            "blocks.0.mlp.up.weight",
            "txtfusion.layerwise_blocks.0.attn.wq.weight",
            "txtfusion.projector.weight",
            "first.weight",
            "last.linear.weight",
            "tmlp.0.weight",
        ]
        .iter()
        .map(|k| (format!("{prefix}{k}"), vec![8, 8]))
        .collect(),
        metadata: BTreeMap::new(),
    }
}

#[test]
fn a_krea2_diffusion_model_is_told_by_its_text_fusion_projector() {
    for prefix in ["", "diffusion_model.", "model.diffusion_model."] {
        let h = krea2_diffusion_header(prefix);
        assert_eq!(
            id(family_from_header(&h)),
            Some("krea2"),
            "prefix {prefix:?}"
        );
    }
}

#[test]
fn a_krea2_lora_is_told_by_its_keys_or_its_training_metadata() {
    let by_keys = from_header(&[
        (
            "diffusion_model.blocks.0.attn.gate.lora_A.weight",
            &[16, 3072],
        ),
        (
            "diffusion_model.blocks.0.attn.gate.lora_B.weight",
            &[3072, 16],
        ),
        (
            "diffusion_model.blocks.0.attn.wq.lora_A.weight",
            &[16, 3072],
        ),
    ]);
    assert_eq!(by_keys, Some("krea2"));

    let mut by_meta = header(&[("unet.some.module.lora_A.weight", &[16, 64])]);
    by_meta
        .metadata
        .insert("ss_base_model_version".into(), "krea2".into());
    assert_eq!(id(family_from_header(&by_meta)), Some("krea2"));
}

#[test]
fn krea2_names_do_not_capture_flux1_krea_and_vice_versa() {
    let cases = [
        (
            "lustifyNSFWCheckpoint_v10Krea2_2997637.safetensors",
            Some("krea2"),
        ),
        ("krea2_turbo_fp8_scaled.safetensors", Some("krea2")),
        ("my Krea 2 finetune.safetensors", Some("krea2")),
        ("krea-2-raw.safetensors", Some("krea2")),
        ("flux1-krea-dev_fp8_scaled.safetensors", Some("flux1-krea")),
        ("flux1-krea-dev.safetensors", Some("flux1-krea")),
        ("FLUX.1-Krea-dev-Q8_0.gguf", Some("flux1-krea")),
    ];
    for (name, want) in cases {
        assert_eq!(id(family_from_name(name)), want, "name {name:?}");
    }
}

#[test]
fn a_krea2_row_without_any_recorded_family_is_inferred_from_its_header() {
    let m = model(
        None,
        None,
        "lustifyNSFWCheckpoint_v10Krea2_2997637.safetensors",
    );
    let h = krea2_diffusion_header("");
    assert_eq!(
        inferred(&m, Some(&h)),
        Some(("krea2", FamilySource::Header))
    );
    // No header (unreadable file): the name still says it.
    assert_eq!(inferred(&m, None), Some(("krea2", FamilySource::Name)));
}
