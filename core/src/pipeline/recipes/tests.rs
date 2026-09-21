//! Cross-recipe invariants: the node-id map in [`super::ids`] is only worth
//! having if every recipe actually respects it, so this renders all fifteen
//! of them — each with a full five-LoRA chain where it takes one — and checks
//! the reservations hold.
//!
//! Per-recipe wiring lives next to each recipe (`recipes::*::tests`); whole
//! graphs live in `core/tests/pipeline_goldens.rs`.

use serde_json::Value;

use super::ids::{HIRES_IDS, LORA_IDS, MAX_LORAS};
use crate::pipeline::{
    checkpoint_ipadapter_txt2img, checkpoint_txt2img, flux2_klein_edit,
    flux2_klein_reference_txt2img, flux2_klein_reference_txt2img_safetensors, flux2_klein_txt2img,
    flux2_klein_txt2img_safetensors, flux_txt2img, krea2_txt2img, ltx_video, rtx_upscale_image,
    rtx_upscale_video, wan_ti2v, EditInputs, Flux2KleinModels, FluxModels, IpAdapterSpec,
    Krea2Models, LoraSpec, LtxModels, Txt2ImgInputs, UpscaleImageInputs, UpscaleResize,
    UpscaleVideoInputs, VideoInputs, WanModels,
};

fn five_loras() -> Vec<LoraSpec<'static>> {
    let files = [
        "a.safetensors",
        "b.safetensors",
        "c.safetensors",
        "d.safetensors",
        "e.safetensors",
    ];
    assert_eq!(files.len(), MAX_LORAS, "the request parser's own cap");
    files
        .iter()
        .map(|file| LoraSpec {
            file,
            strength: 0.8,
        })
        .collect()
}

fn txt2img() -> Txt2ImgInputs<'static> {
    Txt2ImgInputs {
        positive: "a lighthouse at dawn",
        negative: "blurry",
        width: 1024,
        height: 1024,
        steps: 24,
        cfg: 6.5,
        sampler: "dpmpp_2m",
        scheduler: "karras",
        seed: 4242,
        filename_prefix: "job-ids",
        hires: None,
    }
}

fn klein() -> Flux2KleinModels<'static> {
    Flux2KleinModels {
        unet: "u",
        clip: "c",
        vae: "v",
    }
}

fn video(start_image: Option<&'static str>) -> VideoInputs<'static> {
    VideoInputs {
        positive: "a lighthouse at dawn",
        negative: "blurry",
        width: 832,
        height: 480,
        length: 81,
        fps: 24,
        steps: 24,
        cfg: 6.5,
        seed: 4242,
        start_image,
        filename_prefix: "job-ids",
    }
}

/// One rendered recipe plus what the id map promises about it.
struct Rendered {
    name: &'static str,
    graph: Value,
    /// Whether this recipe takes a `hires` input, i.e. whether the 40–44
    /// window has to stay clear for the Hi-Res-Fix fragment.
    reserves_hires_ids: bool,
}

/// Every recipe, rendered once with representative inputs — a full five-LoRA
/// chain wherever a recipe takes LoRAs at all.
fn render_every_recipe() -> Vec<Rendered> {
    let loras = five_loras();
    let i = txt2img();
    let edit = EditInputs {
        instruction: "make the hair blonde",
        source_image: "src.png",
        steps: 8,
        cfg: 1.5,
        sampler: "euler",
        seed: 4242,
        filename_prefix: "job-ids",
    };
    let ip = IpAdapterSpec {
        clip_vision: "cv.safetensors",
        ipadapter_model: "ip.safetensors",
        reference_image: "portrait.png",
        weight: 0.8,
    };
    let flux = FluxModels {
        unet: "u",
        t5: "t",
        clip_l: "c",
        vae: "v",
    };
    let wan = WanModels {
        unet: "u",
        clip: "c",
        vae: "v",
    };
    let ltx = LtxModels {
        checkpoint: "ckpt.safetensors",
        t5: "t",
    };

    vec![
        Rendered {
            name: "checkpoint_txt2img",
            graph: checkpoint_txt2img(&i, "sd_xl_base_1.0.safetensors", &loras),
            reserves_hires_ids: true,
        },
        Rendered {
            name: "flux_txt2img",
            graph: flux_txt2img(&i, &flux, &loras),
            reserves_hires_ids: true,
        },
        Rendered {
            name: "flux2_klein_txt2img",
            graph: flux2_klein_txt2img(&i, &klein(), &loras),
            reserves_hires_ids: true,
        },
        Rendered {
            name: "flux2_klein_txt2img_safetensors",
            graph: flux2_klein_txt2img_safetensors(&i, &klein(), &loras),
            reserves_hires_ids: true,
        },
        Rendered {
            name: "krea2_txt2img",
            graph: krea2_txt2img(
                &i,
                &Krea2Models {
                    unet: "u",
                    clip: "c",
                    vae: "v",
                },
                &loras,
            ),
            reserves_hires_ids: true,
        },
        Rendered {
            name: "flux2_klein_edit",
            graph: flux2_klein_edit(&edit, &klein(), &loras),
            reserves_hires_ids: false,
        },
        Rendered {
            name: "checkpoint_ipadapter_txt2img",
            graph: checkpoint_ipadapter_txt2img(&i, "sd_xl_base_1.0.safetensors", &ip, &loras),
            reserves_hires_ids: false,
        },
        Rendered {
            name: "flux2_klein_reference_txt2img",
            graph: flux2_klein_reference_txt2img(&i, &klein(), "portrait.png", &loras),
            reserves_hires_ids: false,
        },
        Rendered {
            name: "flux2_klein_reference_txt2img_safetensors",
            graph: flux2_klein_reference_txt2img_safetensors(&i, &klein(), "portrait.png", &loras),
            reserves_hires_ids: false,
        },
        Rendered {
            name: "wan_ti2v_t2v",
            graph: wan_ti2v(&video(None), &wan, &loras),
            reserves_hires_ids: false,
        },
        Rendered {
            name: "wan_ti2v_i2v",
            graph: wan_ti2v(&video(Some("start.png")), &wan, &loras),
            reserves_hires_ids: false,
        },
        Rendered {
            name: "ltx_video_t2v",
            graph: ltx_video(&video(None), &ltx, &loras),
            reserves_hires_ids: false,
        },
        Rendered {
            name: "ltx_video_i2v",
            graph: ltx_video(&video(Some("start.png")), &ltx, &loras),
            reserves_hires_ids: false,
        },
        Rendered {
            name: "rtx_upscale_image",
            graph: rtx_upscale_image(&UpscaleImageInputs {
                source_image: "src.png",
                resize: UpscaleResize::ScaleBy(2.0),
                quality: "ULTRA",
                filename_prefix: "job-ids",
            }),
            reserves_hires_ids: false,
        },
        Rendered {
            name: "rtx_upscale_video",
            graph: rtx_upscale_video(&UpscaleVideoInputs {
                source_video: "src.mp4",
                resize: UpscaleResize::ScaleBy(1.5),
                quality: "ULTRA",
                filename_prefix: "job-ids",
            }),
            reserves_hires_ids: false,
        },
    ]
}

/// `(numeric id, class_type)` for every node in a rendered graph.
fn nodes(graph: &Value) -> Vec<(u32, String)> {
    graph
        .as_object()
        .expect("a recipe renders a JSON object of nodes")
        .iter()
        .map(|(id, node)| {
            let n = id
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("node id {id:?} is not a decimal number"));
            let class = node["class_type"]
                .as_str()
                .unwrap_or_else(|| panic!("node {id} has no class_type"))
                .to_string();
            (n, class)
        })
        .collect()
}

/// The two node classes a LoRA chain is built from: `LoraLoader` (model +
/// CLIP) and, for Krea 2, `LoraLoaderModelOnly`.
fn is_lora_loader(class: &str) -> bool {
    matches!(class, "LoraLoader" | "LoraLoaderModelOnly")
}

#[test]
fn no_recipe_puts_a_node_in_another_concerns_reserved_range() {
    for r in render_every_recipe() {
        for (id, class) in nodes(&r.graph) {
            if LORA_IDS.contains(&id) {
                assert!(
                    is_lora_loader(&class),
                    "{}: node {id} is a {class}, but {:?} is reserved for the LoRA chain",
                    r.name,
                    LORA_IDS
                );
            }
            if r.reserves_hires_ids {
                assert!(
                    !HIRES_IDS.contains(&id),
                    "{}: node {id} ({class}) sits in {:?}, reserved for the Hi-Res-Fix fragment \
                     in every recipe that takes a `hires` input",
                    r.name,
                    HIRES_IDS
                );
            }
        }
    }
}

#[test]
fn a_full_lora_chain_fills_exactly_the_reserved_range() {
    for r in render_every_recipe() {
        let chain: Vec<u32> = nodes(&r.graph)
            .into_iter()
            .filter(|(_, class)| is_lora_loader(class))
            .map(|(id, _)| id)
            .collect();
        if chain.is_empty() {
            // The two RTX upscale recipes never touch a model, so they take
            // no LoRAs at all.
            assert!(
                r.name.starts_with("rtx_upscale"),
                "{} rendered no LoraLoader despite being handed five LoRAs",
                r.name
            );
            continue;
        }
        let mut sorted = chain;
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            LORA_IDS.clone().collect::<Vec<u32>>(),
            "{}: a five-LoRA chain must fill exactly {:?}",
            r.name,
            LORA_IDS
        );
    }
}
