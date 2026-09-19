//! A small curated catalogue of image **and video** models the tool knows how to
//! run: SDXL, the FLUX.1-dev GGUF stack, the Wan 2.2 TI2V-5B video stack, and
//! LTX-Video 0.9.x.
//!
//! The MVP has no download manager (that is Phase 6), so "assisted import"
//! means: the UI shows this list with the Hugging Face source and the expected
//! SHA-256, the user downloads the file themselves, and [`import`](super::import)
//! stamps the catalogue metadata onto the model when the hash matches. Details
//! and recommended settings live in `docs/IMAGE_MODELS.md` /
//! `docs/VIDEO_MODELS.md`.

/// One curated model. All fields are compile-time constants — this is a
/// hand-maintained list, not user data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct KnownModel {
    /// Stable slug (also `models.source_revision` as `catalog:<id>` on a match).
    pub id: &'static str,
    pub name: &'static str,
    /// [`crate::model::ModelKind::as_str`] value.
    pub kind: &'static str,
    /// `models.family` on a match (`None` for the format-agnostic encoders).
    pub family: Option<&'static str>,
    pub publisher: &'static str,
    /// Hugging Face `owner/repo`.
    pub repo: &'static str,
    /// File within the repo.
    pub file: &'static str,
    /// Direct download URL.
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
    pub license: &'static str,
    /// One line for the UI — what it is / when to pick it.
    pub note: &'static str,
    /// The curated "pick this one" base model for its role (one `true` among
    /// the `checkpoint`/`diffusion_model` entries, one among the `video`
    /// entries; always `false` for required companions — VAE/text encoder —
    /// which aren't alternatives to each other).
    pub is_default: bool,
    /// `"image"`, `"video"`, `"voice"` or `"training"` (dataset captioners)
    /// — which stack / Models-tab category this entry belongs to. Not
    /// derivable from `kind`/`family` alone: a VAE/text-encoder companion has
    /// `family: None`, and both stacks have one.
    pub media: &'static str,
}

/// The catalogue. Order is display order (default first, then the Flux stack).
pub const KNOWN_MODELS: &[KnownModel] = &[
    KnownModel {
        id: "sdxl-base-1.0",
        name: "Stable Diffusion XL 1.0 (base)",
        kind: "checkpoint",
        family: Some("sdxl"),
        publisher: "Stability AI",
        repo: "stabilityai/stable-diffusion-xl-base-1.0",
        file: "sd_xl_base_1.0.safetensors",
        url: "https://huggingface.co/stabilityai/stable-diffusion-xl-base-1.0/resolve/main/sd_xl_base_1.0.safetensors",
        sha256: "31e35c80fc4829d14f90153f4c74cd59c90b779f6afe05a74cd6120b893f7e5b",
        size_bytes: 6_938_078_334,
        license: "CreativeML Open RAIL++-M (commercial use allowed)",
        note: "The default. Fits comfortably in 16 GB, huge LoRA/ControlNet ecosystem.",
        is_default: true,
        media: "image",
    },
    KnownModel {
        id: "flux1-dev-q8",
        name: "FLUX.1-dev — Q8_0 (GGUF)",
        kind: "diffusion_model",
        family: Some("flux"),
        publisher: "Black Forest Labs / city96",
        repo: "city96/FLUX.1-dev-gguf",
        file: "flux1-dev-Q8_0.gguf",
        url: "https://huggingface.co/city96/FLUX.1-dev-gguf/resolve/main/flux1-dev-Q8_0.gguf",
        sha256: "129032f32224bf7138f16e18673d8008ba5f84c1ec74063bf4511a8bb4cf553d",
        size_bytes: 12_708_281_504,
        license: "FLUX.1 [dev] Non-Commercial License",
        note: "Best prompt fidelity + in-image text. Needs the T5, CLIP-L and VAE below.",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "flux1-dev-q4",
        name: "FLUX.1-dev — Q4_K_S (GGUF)",
        kind: "diffusion_model",
        family: Some("flux"),
        publisher: "Black Forest Labs / city96",
        repo: "city96/FLUX.1-dev-gguf",
        file: "flux1-dev-Q4_K_S.gguf",
        url: "https://huggingface.co/city96/FLUX.1-dev-gguf/resolve/main/flux1-dev-Q4_K_S.gguf",
        sha256: "75bb19459b5240c9f373b5af527584af15b675867fa142efadf7478d6abbf62b",
        size_bytes: 6_805_988_640,
        license: "FLUX.1 [dev] Non-Commercial License",
        note: "Smaller Flux for tighter VRAM; some quality loss vs Q8.",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "t5xxl-fp8",
        name: "T5-XXL — fp8 (Flux text encoder)",
        kind: "text_encoder",
        family: None,
        publisher: "Comfy-Org",
        repo: "comfyanonymous/flux_text_encoders",
        file: "t5xxl_fp8_e4m3fn.safetensors",
        url: "https://huggingface.co/comfyanonymous/flux_text_encoders/resolve/main/t5xxl_fp8_e4m3fn.safetensors",
        sha256: "7d330da4816157540d6bb7838bf63a0f02f573fc48ca4d8de34bb0cbfd514f09",
        size_bytes: 4_893_934_904,
        license: "Apache-2.0",
        note: "Flux prompt encoder. ComfyUI offloads it after encoding, so it is not resident during sampling.",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "t5xxl-q8-gguf",
        name: "T5-XXL — Q8_0 (GGUF text encoder)",
        kind: "text_encoder",
        family: None,
        publisher: "city96",
        repo: "city96/t5-v1_1-xxl-encoder-gguf",
        file: "t5-v1_1-xxl-encoder-Q8_0.gguf",
        url: "https://huggingface.co/city96/t5-v1_1-xxl-encoder-gguf/resolve/main/t5-v1_1-xxl-encoder-Q8_0.gguf",
        sha256: "9ec60f6028534b7fe5af439fcb535d75a68592a9ca3fcdeb175ef89e3ee99825",
        size_bytes: 5_061_584_064,
        license: "Apache-2.0",
        note: "Alternative to the fp8 T5; slightly smaller, needs the GGUF node (already installed).",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "clip-l",
        name: "CLIP-L (Flux text encoder)",
        kind: "text_encoder",
        family: None,
        publisher: "Comfy-Org",
        repo: "comfyanonymous/flux_text_encoders",
        file: "clip_l.safetensors",
        url: "https://huggingface.co/comfyanonymous/flux_text_encoders/resolve/main/clip_l.safetensors",
        sha256: "660c6f5b1abae9dc498ac2d21e1347d2abdb0cf6c0c0c8576cd796491d9a6cdd",
        size_bytes: 246_144_152,
        license: "MIT",
        note: "The second Flux encoder. Small.",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "flux-vae",
        name: "FLUX.1 VAE (ae)",
        kind: "vae",
        family: Some("flux"),
        publisher: "Black Forest Labs",
        repo: "second-state/FLUX.1-dev-GGUF",
        file: "ae.safetensors",
        url: "https://huggingface.co/second-state/FLUX.1-dev-GGUF/resolve/main/ae.safetensors",
        sha256: "afc8e28272cd15db3919bacdb6918ce9c1ed22e96cb12c4d5ed0fba823529e38",
        size_bytes: 335_304_388,
        license: "FLUX.1 [dev] Non-Commercial License",
        note: "The Flux autoencoder. Byte-identical across every Flux re-upload.",
        is_default: false,
        media: "image",
    },
    // --- FLUX.2 [klein] 9B (7.x) ---
    KnownModel {
        id: "flux2-klein-9b-q4",
        name: "FLUX.2 [klein] 9B — Q4_K_M (GGUF)",
        kind: "diffusion_model",
        family: Some("flux2"),
        publisher: "Black Forest Labs / unsloth",
        repo: "unsloth/FLUX.2-klein-9B-GGUF",
        file: "flux-2-klein-9b-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/FLUX.2-klein-9B-GGUF/resolve/main/flux-2-klein-9b-Q4_K_M.gguf",
        sha256: "5489463ed96056b0bb5472abb5d1bba7055e48d574e37877acb43b407465e26f",
        size_bytes: 5_909_829_920,
        license: "FLUX.2 [dev] Non-Commercial License",
        note: "Fast (BFL's distilled tier — good at ~4-8 steps, low CFG). Picked over \
               Q8 by default: its Qwen3-8B text encoder alone is ~8.7 GB, so Q4 keeps the \
               combined footprint comfortably inside 16 GB. Needs the encoder + VAE below.",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "flux2-klein-9b-q8",
        name: "FLUX.2 [klein] 9B — Q8_0 (GGUF)",
        kind: "diffusion_model",
        family: Some("flux2"),
        publisher: "Black Forest Labs / unsloth",
        repo: "unsloth/FLUX.2-klein-9B-GGUF",
        file: "flux-2-klein-9b-Q8_0.gguf",
        url: "https://huggingface.co/unsloth/FLUX.2-klein-9B-GGUF/resolve/main/flux-2-klein-9b-Q8_0.gguf",
        sha256: "dfa8908dd58c5af6479b944f80558e226ee40a8fe0903729309e8d1b98fe5297",
        size_bytes: 9_978_304_800,
        license: "FLUX.2 [dev] Non-Commercial License",
        note: "Best quality, but combined with the ~8.7 GB text encoder this is tight on \
               a 16 GB card (~19.6 GB together) — only pick this if nothing else is resident.",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "flux2-klein-9b-fp8",
        name: "FLUX.2 [klein] 9B — fp8 (safetensors)",
        kind: "diffusion_model",
        family: Some("flux2"),
        publisher: "Black Forest Labs (community fp8 repack)",
        repo: "silveroxides/FLUX.2-dev-fp8_scaled",
        file: "flux-2-klein-9b-fp8mixed.safetensors",
        url: "https://huggingface.co/silveroxides/FLUX.2-dev-fp8_scaled/resolve/main/flux-2-klein-9b-fp8mixed.safetensors",
        sha256: "55c1ef1fc05e9e855f22b063cbb9d67a955e53c85715a2ec0c475addb0f41850",
        size_bytes: 9_433_065_776,
        license: "FLUX.2 [dev] Non-Commercial License",
        note: "Same model as the Q8 GGUF above, but a plain .safetensors file — no \
               ComfyUI-GGUF custom node needed, just ComfyUI's built-in loader. The \
               official Black Forest Labs repo hosting this file is access-gated (needs \
               an HF login + license click-through); this community repack isn't. Needs \
               the same encoder + VAE below.",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "qwen3-8b-flux2-encoder",
        name: "Qwen3-8B — fp8 mixed (FLUX.2 text encoder)",
        kind: "text_encoder",
        family: None,
        publisher: "Comfy-Org",
        repo: "Comfy-Org/vae-text-encorder-for-flux-klein-9b",
        file: "qwen_3_8b_fp8mixed.safetensors",
        url: "https://huggingface.co/Comfy-Org/vae-text-encorder-for-flux-klein-9b/resolve/main/split_files/text_encoders/qwen_3_8b_fp8mixed.safetensors",
        sha256: "abad16806e0cbabc54e0325d6565847443fe396d5f0be38bb3cd3fe75a1201d6",
        size_bytes: 8_664_848_742,
        license: "Apache-2.0",
        note: "FLUX.2's prompt encoder — a full Qwen3-8B, much bigger than FLUX.1's T5. \
               ComfyUI offloads it after encoding, so it is not resident during sampling.",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "flux2-vae",
        name: "FLUX.2 VAE",
        kind: "vae",
        family: Some("flux2"),
        publisher: "Black Forest Labs / Comfy-Org",
        repo: "Comfy-Org/vae-text-encorder-for-flux-klein-9b",
        file: "flux2-vae.safetensors",
        url: "https://huggingface.co/Comfy-Org/vae-text-encorder-for-flux-klein-9b/resolve/main/split_files/vae/flux2-vae.safetensors",
        sha256: "868fe7b343cc8f3a19dbcfcafbc3d5f888802be3f89bd81b65b3621a066ce8f3",
        size_bytes: 336_211_292,
        license: "FLUX.2 [dev] Non-Commercial License",
        note: "FLUX.2's autoencoder — not compatible with FLUX.1's VAE.",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "flux2-klein-edit-vae",
        name: "FLUX.2 Edit VAE (small decoder)",
        kind: "vae",
        family: Some("flux2"),
        publisher: "Black Forest Labs",
        repo: "black-forest-labs/FLUX.2-small-decoder",
        file: "full_encoder_small_decoder.safetensors",
        url: "https://huggingface.co/black-forest-labs/FLUX.2-small-decoder/resolve/main/full_encoder_small_decoder.safetensors",
        sha256: "ea4273f02d1fafbf8e1d1c2cf6018ed8748652eb0bf34f2dd91171f16f15ab62",
        size_bytes: 249_519_092,
        license: "Apache-2.0",
        note: "Only for editing an existing image (not text-to-image) with FLUX.2 [klein] \
               9B — a different VAE from the plain generation one above, with a fuller \
               encoder path for re-encoding a real photo. Small (~238 MB).",
        is_default: false,
        media: "image",
    },
    KnownModel {
        id: "flux2-klein-realistic-detail-lora",
        name: "Realistic Detail LoRA (FLUX.2 Klein 9B)",
        kind: "lora",
        family: Some("flux2"),
        publisher: "SOLRICKS",
        repo: "SOLRICKS/Flux2-Klein-9B-Realistic-Detail",
        // The real file name has spaces; url-encoded here so `url` ends with
        // `file` verbatim (the invariant `every_entry_is_internally_consistent`
        // checks) -- the friendly `name` above is what the UI actually shows.
        file: "Flux2%20Klein%209B%20Realistic%20Detail%20LoRA.safetensors",
        url: "https://huggingface.co/SOLRICKS/Flux2-Klein-9B-Realistic-Detail/resolve/main/Flux2%20Klein%209B%20Realistic%20Detail%20LoRA.safetensors",
        sha256: "3f04d5531ce7d11c9250a0db60a222a542a36acf51638f7b9b1cd444424f8bcd",
        size_bytes: 165_704_488,
        license: "Custom (\u{201c}other\u{201d} on Hugging Face — check the repo before commercial use)",
        note: "Pushes toward photographic realism — fine skin/organic texture, less \u{201c}AI-clean.\u{201d} \
               Trigger word srx_detail; start around strength 0.8 (1.0 can oversaturate skin texture).",
        is_default: false,
        media: "image",
    },
    // --- video (Phase 4) ---
    KnownModel {
        id: "wan22-ti2v-5b",
        name: "Wan 2.2 TI2V-5B — fp16 (video, default)",
        kind: "video",
        family: Some("wan"),
        publisher: "Alibaba / Comfy-Org",
        repo: "Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
        file: "wan2.2_ti2v_5B_fp16.safetensors",
        url: "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_ti2v_5B_fp16.safetensors",
        sha256: "456f901338bd9eadbded3828b819109a9b68e8a525ca5cf8d0049a69fcfeca1e",
        size_bytes: 9_999_658_848,
        license: "Apache-2.0 (commercial use allowed)",
        note: "The default video model. One model for text→video and image→video. Needs the umt5 encoder and the Wan VAE below.",
        is_default: true,
        media: "video",
    },
    KnownModel {
        id: "wan-umt5-xxl-fp8",
        name: "umt5-XXL — fp8 (Wan text encoder)",
        kind: "text_encoder",
        family: None,
        publisher: "Comfy-Org",
        repo: "Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
        file: "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
        url: "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors",
        sha256: "c3355d30191f1f066b26d93fba017ae9809dce6c627dda5f6a66eaa651204f68",
        size_bytes: 6_735_906_897,
        license: "Apache-2.0",
        note: "Wan's prompt encoder (multilingual T5). Offloaded to the CPU during sampling.",
        is_default: false,
        media: "video",
    },
    KnownModel {
        id: "wan22-vae",
        name: "Wan 2.2 VAE",
        kind: "vae",
        family: Some("wan"),
        publisher: "Alibaba / Comfy-Org",
        repo: "Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
        file: "wan2.2_vae.safetensors",
        url: "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/vae/wan2.2_vae.safetensors",
        sha256: "e40321bd36b9709991dae2530eb4ac303dd168276980d3e9bc4b6e2b75fed156",
        size_bytes: 1_409_400_960,
        license: "Apache-2.0",
        note: "The Wan 2.2 autoencoder. Import as VAE.",
        is_default: false,
        media: "video",
    },
    KnownModel {
        id: "ltx-video-2b-095",
        name: "LTX-Video 2B v0.9.5 (video, second template)",
        kind: "video",
        family: Some("ltx"),
        publisher: "Lightricks",
        repo: "Lightricks/LTX-Video",
        file: "ltx-video-2b-v0.9.5.safetensors",
        url: "https://huggingface.co/Lightricks/LTX-Video/resolve/main/ltx-video-2b-v0.9.5.safetensors",
        sha256: "720d15c9f19f7d0f6b2a92bbbc34410e2cfb2f6856a100b38f734fbf973d4adf",
        size_bytes: 6_340_729_500,
        license: "LTXV License (OpenRAIL-M-style; check the repo for commercial terms)",
        note: "Fast, light. One .safetensors carries model + VAE — only needs a t5xxl encoder (the fp8 one above works). Wants long, descriptive prompts.",
        is_default: false,
        media: "video",
    },
    // --- voice (Story Studio narrator, WP-9) ---
    // Not Hugging Face — the working ONNX export + combined voices file for
    // the `kokoro-onnx` package (what the sidecar actually loads) live as
    // GitHub release assets. Verified by downloading and hashing both files
    // directly (no published checksum exists upstream).
    KnownModel {
        id: "kokoro-v1.0-int8",
        name: "Kokoro 82M — int8 (smaller, noisier)",
        kind: "voice_model",
        family: Some("kokoro"),
        publisher: "hexgrad / thewh1teagle",
        repo: "thewh1teagle/kokoro-onnx",
        file: "kokoro-v1.0.int8.onnx",
        url: "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.1/kokoro-v1.0.int8.onnx",
        sha256: "ae315a79b623f244700e4afb9246c46a26066782e049ba174bf3ba433970ee9c",
        size_bytes: 114_119_327,
        license: "Apache-2.0",
        note: "Quantized (spectral correlation 0.916 against fp32 per the release notes) — a \
               reported, reproducible background-noise artifact traces back to this quantization. \
               Smaller/faster, but fp32 below is the recommended pick. Needs the matching voices \
               file below.",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "kokoro-v1.0-fp32",
        name: "Kokoro 82M — full precision (voice, default)",
        kind: "voice_model",
        family: Some("kokoro"),
        publisher: "hexgrad / thewh1teagle",
        repo: "thewh1teagle/kokoro-onnx",
        file: "kokoro-v1.0.onnx",
        url: "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.1/kokoro-v1.0.onnx",
        sha256: "beb0d1848dee9a49da392cc3df26958d46cfa35d321edf434f52949153f0df3a",
        size_bytes: 325_505_369,
        license: "Apache-2.0",
        note: "The un-quantized export — a bit larger, but clean (0.999 spectral correlation, per \
               the release notes, vs. int8's 0.916). Needs the matching voices file below.",
        is_default: true,
        media: "voice",
    },
    KnownModel {
        id: "kokoro-voices-v1.0",
        name: "Kokoro voices (54 English voices)",
        kind: "voice_data",
        family: Some("kokoro"),
        publisher: "hexgrad / thewh1teagle",
        repo: "thewh1teagle/kokoro-onnx",
        file: "voices-v1.0.bin",
        url: "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.1/voices-v1.0.bin",
        sha256: "bca610b8308e8d99f32e6fe4197e7ec01679264efed0cac9140fe9c29f1fbf7d",
        size_bytes: 28_214_398,
        license: "Apache-2.0",
        note: "54 American/British English voice embeddings — required by either Kokoro model \
               above. Not a standalone model; pairs with whichever one you import.",
        is_default: false,
        media: "voice",
    },
    // --- Dia 1.6B (second, more expressive narrator engine) ---
    // Nine co-located files that must keep their exact Hugging Face names in
    // one directory (`DiaForConditionalGeneration::from_pretrained` reads a
    // directory, not a single path) -- see `ModelKind::DiaEngine`. Real
    // capability, stated honestly: recognized non-verbal tags
    // ((laughs)/(sighs)/(clears throat)/…) and [S1]/[S2] speaker turns for
    // actual dialogue -- NOT freeform emotional stage directions like
    // "(angry)", which no engine here can perform from text alone.
    KnownModel {
        id: "dia-1.6b-config",
        name: "Dia 1.6B — model config",
        kind: "dia_engine",
        family: Some("dia"),
        publisher: "Nari Labs",
        repo: "nari-labs/Dia-1.6B-0626",
        file: "config.json",
        url: "https://huggingface.co/nari-labs/Dia-1.6B-0626/resolve/main/config.json",
        sha256: "4f4e9c50e6898fa79d6fe16e9991bebef07a4d3dd6926eee5eb7f7e39c88e04f",
        size_bytes: 1396,
        license: "Apache-2.0",
        note: "Dia's encoder/decoder architecture config -- required by every file below.",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "dia-1.6b-generation-config",
        name: "Dia 1.6B — generation defaults",
        kind: "dia_engine",
        family: Some("dia"),
        publisher: "Nari Labs",
        repo: "nari-labs/Dia-1.6B-0626",
        file: "generation_config.json",
        url: "https://huggingface.co/nari-labs/Dia-1.6B-0626/resolve/main/generation_config.json",
        sha256: "df7c1f7e4dc1c3e35811b7b2e49abd89d8a8b9599ec732eb6b0d17d678548ac4",
        size_bytes: 238,
        license: "Apache-2.0",
        note: "Dia's shipped sampling / max-new-tokens defaults.",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "dia-1.6b-audio-tokenizer-config",
        name: "Dia 1.6B — audio tokenizer pointer",
        kind: "dia_engine",
        family: Some("dia"),
        publisher: "Nari Labs",
        repo: "nari-labs/Dia-1.6B-0626",
        file: "audio_tokenizer_config.json",
        url: "https://huggingface.co/nari-labs/Dia-1.6B-0626/resolve/main/audio_tokenizer_config.json",
        sha256: "1552f8bdccdfe0a98a8e93f9eb369de315e66a07b1530eba0be41513e2ee8f5e",
        size_bytes: 98,
        license: "Apache-2.0",
        note: "Tiny pointer config for Dia's separate audio codec (the DAC files below).",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "dia-1.6b-preprocessor-config",
        name: "Dia 1.6B — preprocessor config",
        kind: "dia_engine",
        family: Some("dia"),
        publisher: "Nari Labs",
        repo: "nari-labs/Dia-1.6B-0626",
        file: "preprocessor_config.json",
        url: "https://huggingface.co/nari-labs/Dia-1.6B-0626/resolve/main/preprocessor_config.json",
        sha256: "7dfa85b765a7c5b3fb3907abf43ece88cd9edcbf39402e3e32e056b3275e5433",
        size_bytes: 243,
        license: "Apache-2.0",
        note: "Settings `AutoProcessor` needs to build Dia's input side.",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "dia-1.6b-special-tokens-map",
        name: "Dia 1.6B — special tokens map",
        kind: "dia_engine",
        family: Some("dia"),
        publisher: "Nari Labs",
        repo: "nari-labs/Dia-1.6B-0626",
        file: "special_tokens_map.json",
        url: "https://huggingface.co/nari-labs/Dia-1.6B-0626/resolve/main/special_tokens_map.json",
        sha256: "4373f3b7455d5a34196fca520de10a0af2af9d7cc05f6177b4be73a8bcfb3f74",
        size_bytes: 277,
        license: "Apache-2.0",
        note: "Text tokenizer special-token metadata (BOS/EOS/pad/…).",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "dia-1.6b-tokenizer-config",
        name: "Dia 1.6B — tokenizer config",
        kind: "dia_engine",
        family: Some("dia"),
        publisher: "Nari Labs",
        repo: "nari-labs/Dia-1.6B-0626",
        file: "tokenizer_config.json",
        url: "https://huggingface.co/nari-labs/Dia-1.6B-0626/resolve/main/tokenizer_config.json",
        sha256: "6c84315ff3c5214a5846bc3c08cb23fe01146906e4abf395519316732d602e64",
        size_bytes: 803,
        license: "Apache-2.0",
        note: "Text tokenizer config -- how `[S1]`/`[S2]` speaker tags and non-verbal tags \
               like (laughs)/(sighs) get encoded.",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "dia-1.6b-weights-index",
        name: "Dia 1.6B — weights shard index",
        kind: "dia_engine",
        family: Some("dia"),
        publisher: "Nari Labs",
        repo: "nari-labs/Dia-1.6B-0626",
        file: "model.safetensors.index.json",
        url: "https://huggingface.co/nari-labs/Dia-1.6B-0626/resolve/main/model.safetensors.index.json",
        sha256: "89664ffd667582a41e71c9aa1a4ba9de8187ec9dcd1e7711d97ec57d6e32dec5",
        size_bytes: 30_833,
        license: "Apache-2.0",
        note: "Maps every tensor to whichever of the two weight shards below holds it -- \
               required alongside both for `from_pretrained` to load them as one model.",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "dia-1.6b-weights-part1",
        name: "Dia 1.6B — weights (shard 1 of 2)",
        kind: "dia_engine",
        family: Some("dia"),
        publisher: "Nari Labs",
        repo: "nari-labs/Dia-1.6B-0626",
        file: "model-00001-of-00002.safetensors",
        url: "https://huggingface.co/nari-labs/Dia-1.6B-0626/resolve/main/model-00001-of-00002.safetensors",
        sha256: "9cf8f82d9f87d408f5da9cc0d73a3e9c4d9ca3633a4df3be4d283277b7ff1b1a",
        size_bytes: 4_993_046_400,
        license: "Apache-2.0",
        note: "The bulk of Dia's 1.6B parameters (bf16), ~4.65 GB. Needs shard 2 and the \
               index above to load.",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "dia-1.6b-weights-part2",
        name: "Dia 1.6B — weights (shard 2 of 2)",
        kind: "dia_engine",
        family: Some("dia"),
        publisher: "Nari Labs",
        repo: "nari-labs/Dia-1.6B-0626",
        file: "model-00002-of-00002.safetensors",
        url: "https://huggingface.co/nari-labs/Dia-1.6B-0626/resolve/main/model-00002-of-00002.safetensors",
        sha256: "54bf3a47ac13e28ba6193659d57bc63a46875685b211b071f782c32efc834343",
        size_bytes: 1_451_637_544,
        license: "Apache-2.0",
        note: "The rest of Dia's weights (bf16), ~1.35 GB. Needs shard 1 and the index above \
               to load.",
        is_default: false,
        media: "voice",
    },
    // Dia's audio tokenizer (DAC) is a *separate* Hugging Face repo --
    // `AutoProcessor.from_pretrained` would otherwise fetch it over the
    // network the first time Dia runs, which this project's offline-first
    // rule doesn't allow. Its own directory: both repos ship a
    // `config.json` / `preprocessor_config.json`, so they'd collide by
    // filename if merged into Dia's own folder.
    KnownModel {
        id: "dac-44khz-config",
        name: "DAC 44kHz codec — config",
        kind: "dia_codec",
        family: None,
        publisher: "Descript",
        repo: "descript/dac_44khz",
        file: "config.json",
        url: "https://huggingface.co/descript/dac_44khz/resolve/main/config.json",
        sha256: "4eb55fb9af1990b8d608184ad29b70e358589719af7ea8d3c06998f7c2264a64",
        size_bytes: 541,
        license: "MIT",
        note: "descript-audio-codec (DAC) 44kHz config -- Dia's audio tokenizer/detokenizer \
               backend, a separate repo from Dia itself.",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "dac-44khz-preprocessor-config",
        name: "DAC 44kHz codec — preprocessor config",
        kind: "dia_codec",
        family: None,
        publisher: "Descript",
        repo: "descript/dac_44khz",
        file: "preprocessor_config.json",
        url: "https://huggingface.co/descript/dac_44khz/resolve/main/preprocessor_config.json",
        sha256: "c7d295758ce5777d6d88fef1996e94adc8ef3e2237ddfc5ecc24d1407aaddd7d",
        size_bytes: 206,
        license: "MIT",
        note: "DAC's own preprocessor settings -- same filename as Dia's preprocessor \
               config above, a different repo, hence its own directory.",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "dac-44khz-weights",
        name: "DAC 44kHz codec — weights",
        kind: "dia_codec",
        family: None,
        publisher: "Descript",
        repo: "descript/dac_44khz",
        file: "model.safetensors",
        url: "https://huggingface.co/descript/dac_44khz/resolve/main/model.safetensors",
        sha256: "6128ebff483a41422b0164d079a3773b0d8d82e64c4293d775994cbf8baf913a",
        size_bytes: 306_507_276,
        license: "MIT",
        note: "The codec's own weights (~292 MB) -- turns Dia's output tokens into the \
               actual 44.1kHz waveform.",
        is_default: false,
        media: "voice",
    },
    KnownModel {
        id: "wd-eva02-large-tagger-v3-model",
        name: "WD EVA02-Large Tagger v3 — model",
        kind: "wd_tagger",
        family: None,
        publisher: "SmilingWolf",
        repo: "SmilingWolf/wd-eva02-large-tagger-v3",
        file: "model.onnx",
        url: "https://huggingface.co/SmilingWolf/wd-eva02-large-tagger-v3/resolve/b25b82a03f7282e41aa2f257a52c7583b710bd1c/model.onnx",
        sha256: "9e768793060c7939b277ccb382783e8670e8a042d29d77aa736be0c8cc898bfc",
        size_bytes: 1_260_435_999,
        license: "Apache-2.0",
        note: "Danbooru-style tag captioner (rating, character and general tags, explicit \
               tags included) for anime/illustration datasets. 0.3B parameters, runs on the \
               CPU via onnxruntime. Needs the tag list below in the same folder. Pinned to \
               commit b25b82a03f7282e41aa2f257a52c7583b710bd1c.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "wd-eva02-large-tagger-v3-tags",
        name: "WD EVA02-Large Tagger v3 — tag list",
        kind: "wd_tagger",
        family: None,
        publisher: "SmilingWolf",
        repo: "SmilingWolf/wd-eva02-large-tagger-v3",
        file: "selected_tags.csv",
        url: "https://huggingface.co/SmilingWolf/wd-eva02-large-tagger-v3/resolve/b25b82a03f7282e41aa2f257a52c7583b710bd1c/selected_tags.csv",
        sha256: "298633d94d0031d2081c0893f29c82eab7f0df00b08483ba8f29d1e979441217",
        size_bytes: 308_468,
        license: "Apache-2.0",
        note: "The tag vocabulary the tagger's outputs map onto. Required companion of the \
               model above. Pinned to commit b25b82a03f7282e41aa2f257a52c7583b710bd1c.",
        is_default: false,
        media: "training",
    },
    // --- Florence-2 large (prose captioner, Dataset tab) ---
    // Ten co-located files read as one directory by
    // `AutoModelForCausalLM.from_pretrained(<dir>, trust_remote_code=True)` --
    // see `ModelKind::Florence2Engine`. The three `.py` files are remote code
    // the sidecar executes, so every URL is pinned to one commit (never
    // `main`) and each SHA-256/size below was computed from the file actually
    // downloaded at that commit (the weights' hash also matches the Hub's LFS
    // metadata).
    KnownModel {
        id: "florence2-large-model",
        name: "Florence-2 large — weights",
        kind: "florence2_engine",
        family: Some("florence2"),
        publisher: "Microsoft",
        repo: "microsoft/Florence-2-large",
        file: "model.safetensors",
        url: "https://huggingface.co/microsoft/Florence-2-large/resolve/21a599d414c4d928c9032694c424fb94458e3594/model.safetensors",
        sha256: "4f38ce741c6b71188fe2b3419a55e11917a8a7b321ae2e63c61da0191b0ebad7",
        size_bytes: 1_553_563_458,
        license: "MIT",
        note: "Florence-2-large weights (0.77B parameters, ~1.55 GB) -- the prose captioner \
               the Dataset tab uses by default. Needs every other Florence-2 file below in \
               the same folder. Pinned to commit 21a599d414c4d928c9032694c424fb94458e3594.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "florence2-large-config",
        name: "Florence-2 large — model config",
        kind: "florence2_engine",
        family: Some("florence2"),
        publisher: "Microsoft",
        repo: "microsoft/Florence-2-large",
        file: "config.json",
        url: "https://huggingface.co/microsoft/Florence-2-large/resolve/21a599d414c4d928c9032694c424fb94458e3594/config.json",
        sha256: "6f8a8f92a74ce18b5c1e5646b4a8222477dd1ac49f31b953e75d6fc3e0f8583a",
        size_bytes: 2_445,
        license: "MIT",
        note: "Architecture config; its auto_map points AutoModelForCausalLM at the pinned \
               modeling_florence2.py below. Pinned to commit \
               21a599d414c4d928c9032694c424fb94458e3594.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "florence2-large-configuration-florence2",
        name: "Florence-2 large — remote code (config)",
        kind: "florence2_engine",
        family: Some("florence2"),
        publisher: "Microsoft",
        repo: "microsoft/Florence-2-large",
        file: "configuration_florence2.py",
        url: "https://huggingface.co/microsoft/Florence-2-large/resolve/21a599d414c4d928c9032694c424fb94458e3594/configuration_florence2.py",
        sha256: "de2e45a975b3582de05d2f4d963a3e9f9a3d20dccf78d28e0052932a0be93bdf",
        size_bytes: 15_119,
        license: "MIT",
        note: "Florence2Config -- Python the sidecar runs via trust_remote_code, so it is \
               pinned by hash like the weights. Pinned to commit \
               21a599d414c4d928c9032694c424fb94458e3594.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "florence2-large-modeling-florence2",
        name: "Florence-2 large — remote code (model)",
        kind: "florence2_engine",
        family: Some("florence2"),
        publisher: "Microsoft",
        repo: "microsoft/Florence-2-large",
        file: "modeling_florence2.py",
        url: "https://huggingface.co/microsoft/Florence-2-large/resolve/21a599d414c4d928c9032694c424fb94458e3594/modeling_florence2.py",
        sha256: "5162bf465e61b6e29cc113a467630ec3cb56ed8e4d46eb6207157f10fb9b8a24",
        size_bytes: 127_455,
        license: "MIT",
        note: "Florence2ForConditionalGeneration -- Python the sidecar runs via \
               trust_remote_code, pinned by hash. Pinned to commit \
               21a599d414c4d928c9032694c424fb94458e3594.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "florence2-large-processing-florence2",
        name: "Florence-2 large — remote code (processor)",
        kind: "florence2_engine",
        family: Some("florence2"),
        publisher: "Microsoft",
        repo: "microsoft/Florence-2-large",
        file: "processing_florence2.py",
        url: "https://huggingface.co/microsoft/Florence-2-large/resolve/21a599d414c4d928c9032694c424fb94458e3594/processing_florence2.py",
        sha256: "c655782a9e4347965c735ea54cbc4e98fdbc02155ffd1ce2ecd61f42c45eda28",
        size_bytes: 48_674,
        license: "MIT",
        note: "Florence2Processor (task prompts, post-processing) -- Python run via \
               trust_remote_code, pinned by hash. Pinned to commit \
               21a599d414c4d928c9032694c424fb94458e3594.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "florence2-large-preprocessor-config",
        name: "Florence-2 large — preprocessor config",
        kind: "florence2_engine",
        family: Some("florence2"),
        publisher: "Microsoft",
        repo: "microsoft/Florence-2-large",
        file: "preprocessor_config.json",
        url: "https://huggingface.co/microsoft/Florence-2-large/resolve/21a599d414c4d928c9032694c424fb94458e3594/preprocessor_config.json",
        sha256: "2f5921bbc53c7cc04251e1027b45b1cec726276be6db23d1bb40641bfbe2cf29",
        size_bytes: 806,
        license: "MIT",
        note: "Image preprocessing settings; its auto_map points AutoProcessor at \
               processing_florence2.py. Pinned to commit \
               21a599d414c4d928c9032694c424fb94458e3594.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "florence2-large-generation-config",
        name: "Florence-2 large — generation defaults",
        kind: "florence2_engine",
        family: Some("florence2"),
        publisher: "Microsoft",
        repo: "microsoft/Florence-2-large",
        file: "generation_config.json",
        url: "https://huggingface.co/microsoft/Florence-2-large/resolve/21a599d414c4d928c9032694c424fb94458e3594/generation_config.json",
        sha256: "30e9865458ecc8ee931eeeb43f44f1d169c5ab95be39e0072142a7a6b8f31990",
        size_bytes: 51,
        license: "MIT",
        note: "Shipped beam-search defaults. Pinned to commit \
               21a599d414c4d928c9032694c424fb94458e3594.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "florence2-large-tokenizer",
        name: "Florence-2 large — tokenizer",
        kind: "florence2_engine",
        family: Some("florence2"),
        publisher: "Microsoft",
        repo: "microsoft/Florence-2-large",
        file: "tokenizer.json",
        url: "https://huggingface.co/microsoft/Florence-2-large/resolve/21a599d414c4d928c9032694c424fb94458e3594/tokenizer.json",
        sha256: "847bbeab6174d66a88898f729d52fa8d355fafe1bea101cf960dd404581df70e",
        size_bytes: 1_355_863,
        license: "MIT",
        note: "The fast BART tokenizer the processor loads. Pinned to commit \
               21a599d414c4d928c9032694c424fb94458e3594.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "florence2-large-tokenizer-config",
        name: "Florence-2 large — tokenizer config",
        kind: "florence2_engine",
        family: Some("florence2"),
        publisher: "Microsoft",
        repo: "microsoft/Florence-2-large",
        file: "tokenizer_config.json",
        url: "https://huggingface.co/microsoft/Florence-2-large/resolve/21a599d414c4d928c9032694c424fb94458e3594/tokenizer_config.json",
        sha256: "79ffcf43af8ebda99d165f61d243180da2e2639952e41e71e11611c18770489c",
        size_bytes: 34,
        license: "MIT",
        note: "Tokenizer settings (max length). Pinned to commit \
               21a599d414c4d928c9032694c424fb94458e3594.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "florence2-large-vocab",
        name: "Florence-2 large — vocabulary",
        kind: "florence2_engine",
        family: Some("florence2"),
        publisher: "Microsoft",
        repo: "microsoft/Florence-2-large",
        file: "vocab.json",
        url: "https://huggingface.co/microsoft/Florence-2-large/resolve/21a599d414c4d928c9032694c424fb94458e3594/vocab.json",
        sha256: "394fdc63c71aabe0a9b97117f5d62fb5fcc4d59b2b3ea929a3929e6a53217b3c",
        size_bytes: 1_099_884,
        license: "MIT",
        note: "BART vocabulary. Pinned to commit 21a599d414c4d928c9032694c424fb94458e3594.",
        is_default: false,
        media: "training",
    },
    // --- Qwen2.5-VL 7B Instruct (optional two-frame "second opinion") ---
    // Fourteen co-located files for `Qwen2_5_VLForConditionalGeneration` /
    // `AutoProcessor.from_pretrained(<dir>)` -- built into `transformers`, no
    // remote code. Pinned to one commit; every SHA-256/size computed from the
    // downloaded file (the five shards also match the Hub's LFS metadata).
    KnownModel {
        id: "qwen2.5-vl-7b-model-00001-of-00005",
        name: "Qwen2.5-VL 7B Instruct — weights (shard 1 of 5)",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "model-00001-of-00005.safetensors",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/model-00001-of-00005.safetensors",
        sha256: "e97b877e47fde53a6c6e77aafb36e58e91ee9d95c4a3eeac6f1b5c0e6a1c986e",
        size_bytes: 3_900_233_256,
        license: "Apache-2.0",
        note: "Qwen2.5-VL-7B-Instruct weights (bf16), ~3.9 GB -- the optional \
               second-opinion captioner that compares two frames when a Florence-2 caption \
               looks unsure. Needs all five shards and the index. Pinned to commit \
               cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-model-00002-of-00005",
        name: "Qwen2.5-VL 7B Instruct — weights (shard 2 of 5)",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "model-00002-of-00005.safetensors",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/model-00002-of-00005.safetensors",
        sha256: "a9a300a43b4724eee2abe7c18ceb26768d0ab011eb0cad19d9bfd2476a24d024",
        size_bytes: 3_864_726_320,
        license: "Apache-2.0",
        note: "Qwen2.5-VL-7B-Instruct weights (bf16), ~3.9 GB. Needs all five shards and \
               the index. Pinned to commit cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-model-00003-of-00005",
        name: "Qwen2.5-VL 7B Instruct — weights (shard 3 of 5)",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "model-00003-of-00005.safetensors",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/model-00003-of-00005.safetensors",
        sha256: "111223d173e00bbee81cba1216fad28668df3476706b7fd26f4d5b50f8b3a507",
        size_bytes: 3_864_726_424,
        license: "Apache-2.0",
        note: "Qwen2.5-VL-7B-Instruct weights (bf16), ~3.9 GB. Needs all five shards and \
               the index. Pinned to commit cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-model-00004-of-00005",
        name: "Qwen2.5-VL 7B Instruct — weights (shard 4 of 5)",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "model-00004-of-00005.safetensors",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/model-00004-of-00005.safetensors",
        sha256: "ef47f634fa57d46ee134edcc09f34085a47da1e16c12a2abe0d67118be6d72ed",
        size_bytes: 3_864_733_680,
        license: "Apache-2.0",
        note: "Qwen2.5-VL-7B-Instruct weights (bf16), ~3.9 GB. Needs all five shards and \
               the index. Pinned to commit cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-model-00005-of-00005",
        name: "Qwen2.5-VL 7B Instruct — weights (shard 5 of 5)",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "model-00005-of-00005.safetensors",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/model-00005-of-00005.safetensors",
        sha256: "0c859795ad3a627a9b95bcb762e059d5b768a4a36fdd4affeff269d93fdecc67",
        size_bytes: 1_089_994_880,
        license: "Apache-2.0",
        note: "Qwen2.5-VL-7B-Instruct weights (bf16), ~1.1 GB. Needs all five shards and \
               the index. Pinned to commit cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-model-index",
        name: "Qwen2.5-VL 7B Instruct — weights shard index",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "model.safetensors.index.json",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/model.safetensors.index.json",
        sha256: "73b333b0b16e5286ddba615d2caebcd495cf7e616f52eb217a81781393d79de9",
        size_bytes: 57_619,
        license: "Apache-2.0",
        note: "Maps every tensor to the shard holding it -- required for from_pretrained to \
               load the five shards as one model. Pinned to commit \
               cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-config",
        name: "Qwen2.5-VL 7B Instruct — model config",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "config.json",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/config.json",
        sha256: "77d9ec7321cc572e3579e2c84799c9cadaded63c49ce93b101733349fc330c43",
        size_bytes: 1_374,
        license: "Apache-2.0",
        note: "Architecture config (Qwen2_5_VLForConditionalGeneration, built into \
               transformers -- no remote code). Pinned to commit \
               cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-generation-config",
        name: "Qwen2.5-VL 7B Instruct — generation defaults",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "generation_config.json",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/generation_config.json",
        sha256: "0a3aea82869fe29f20dc95ccf3e2bcff380eca1f5ad6447a4a4b37110b08e43e",
        size_bytes: 216,
        license: "Apache-2.0",
        note: "Shipped sampling defaults. Pinned to commit \
               cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-preprocessor-config",
        name: "Qwen2.5-VL 7B Instruct — preprocessor config",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "preprocessor_config.json",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/preprocessor_config.json",
        sha256: "f2058c716eef96ccaed1cc1e2d0c08306b62586d535b28d9d08e691b2fab7ca0",
        size_bytes: 350,
        license: "Apache-2.0",
        note: "Image preprocessing settings AutoProcessor needs. Pinned to commit \
               cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-chat-template",
        name: "Qwen2.5-VL 7B Instruct — chat template",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "chat_template.json",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/chat_template.json",
        sha256: "ad60d90252ed0b0705ba14e2d0ad0fec0beac1ea955642b54059b36052d8bc96",
        size_bytes: 1_050,
        license: "Apache-2.0",
        note: "The multi-image chat template the processor applies. Pinned to commit \
               cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-tokenizer",
        name: "Qwen2.5-VL 7B Instruct — tokenizer",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "tokenizer.json",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/tokenizer.json",
        sha256: "c0382117ea329cdf097041132f6d735924b697924d6f6fc3945713e96ce87539",
        size_bytes: 7_031_645,
        license: "Apache-2.0",
        note: "The fast Qwen2 tokenizer. Pinned to commit \
               cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-tokenizer-config",
        name: "Qwen2.5-VL 7B Instruct — tokenizer config",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "tokenizer_config.json",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/tokenizer_config.json",
        sha256: "4abd3520120e266da84c0864fee064d1fb10806f02225911a47253dd38dc5f56",
        size_bytes: 5_702,
        license: "Apache-2.0",
        note: "Tokenizer settings and special tokens. Pinned to commit \
               cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-vocab",
        name: "Qwen2.5-VL 7B Instruct — vocabulary",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "vocab.json",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/vocab.json",
        sha256: "ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910",
        size_bytes: 2_776_833,
        license: "Apache-2.0",
        note: "Qwen2 BPE vocabulary. Pinned to commit \
               cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
    KnownModel {
        id: "qwen2.5-vl-7b-merges",
        name: "Qwen2.5-VL 7B Instruct — BPE merges",
        kind: "qwen_vl_engine",
        family: Some("qwen2.5-vl"),
        publisher: "Alibaba Cloud (Qwen)",
        repo: "Qwen/Qwen2.5-VL-7B-Instruct",
        file: "merges.txt",
        url: "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/cc594898137f460bfe9f0759e9844b3ce807cfb5/merges.txt",
        sha256: "599bab54075088774b1733fde865d5bd747cbcc7a547c5bc12610e874e26f5e3",
        size_bytes: 1_671_839,
        license: "Apache-2.0",
        note: "Qwen2 BPE merge rules. Pinned to commit \
               cc594898137f460bfe9f0759e9844b3ce807cfb5.",
        is_default: false,
        media: "training",
    },
];

/// The catalogue entry whose file matches `sha256`, if any.
pub fn find_by_sha256(sha256: &str) -> Option<&'static KnownModel> {
    KNOWN_MODELS
        .iter()
        .find(|m| m.sha256.eq_ignore_ascii_case(sha256))
}

/// A base image/video model bundled with every companion file it needs to
/// actually run — a checkpoint is complete on its own, but Flux/Wan/LTX need
/// a VAE and one or two text encoders too. References [`KnownModel::id`]s
/// rather than duplicating their data, so there is exactly one place that
/// knows a file's URL/SHA-256/size.
#[derive(Debug, Clone, Copy)]
pub struct ModelStack {
    pub id: &'static str,
    pub label: &'static str,
    /// `"image"`, `"video"`, `"voice"` or `"training"` — the Models-tab
    /// catalog category the stack is listed under.
    pub media: &'static str,
    /// [`KnownModel::id`]s that make up one working setup — base model first,
    /// then companions (display order).
    pub member_ids: &'static [&'static str],
    /// One line for the UI — what it is, how many files, why that many.
    pub note: &'static str,
    /// The curated "pick this one" stack for its media type (one per media).
    pub is_default: bool,
}

/// The stacks. Order is display order (default first per media type).
pub const MODEL_STACKS: &[ModelStack] = &[
    ModelStack {
        id: "sdxl",
        label: "Stable Diffusion XL",
        media: "image",
        member_ids: &["sdxl-base-1.0"],
        note: "One file — the checkpoint carries its own VAE and text encoder.",
        is_default: true,
    },
    ModelStack {
        id: "flux",
        label: "FLUX.1-dev",
        media: "image",
        member_ids: &["flux1-dev-q8", "t5xxl-fp8", "clip-l", "flux-vae"],
        note: "Best prompt fidelity + in-image text. Four files: the diffusion \
               model plus its T5 and CLIP-L text encoders and its VAE.",
        is_default: false,
    },
    ModelStack {
        id: "flux2-klein",
        label: "FLUX.2 [klein] 9B",
        media: "image",
        member_ids: &[
            "flux2-klein-9b-q4",
            "qwen3-8b-flux2-encoder",
            "flux2-vae",
            "flux2-klein-edit-vae",
        ],
        note: "Fast (sub-second at 4 steps), fits a 16 GB card. Four files: the \
               diffusion model, its Qwen3 text encoder, its VAE, and the second VAE that \
               editing an existing image needs. The realistic-detail LoRA is a separate, \
               optional download from the Discover tab.",
        is_default: false,
    },
    ModelStack {
        id: "flux2-klein-safetensors",
        label: "FLUX.2 [klein] 9B (safetensors)",
        media: "image",
        member_ids: &[
            "flux2-klein-9b-fp8",
            "qwen3-8b-flux2-encoder",
            "flux2-vae",
            "flux2-klein-edit-vae",
        ],
        note: "Same model as the GGUF stack above, as a plain .safetensors file instead \
               — no ComfyUI-GGUF custom node required. Four files: the diffusion model, \
               its Qwen3 text encoder, its VAE, and the second VAE that editing an existing \
               image needs.",
        is_default: false,
    },
    ModelStack {
        id: "wan22",
        label: "Wan 2.2 TI2V-5B",
        media: "video",
        member_ids: &["wan22-ti2v-5b", "wan-umt5-xxl-fp8", "wan22-vae"],
        note: "The default video setup. Three files: the model, its umt5 text \
               encoder, and its VAE.",
        is_default: true,
    },
    ModelStack {
        id: "ltx",
        label: "LTX-Video 0.9.5 (2B)",
        media: "video",
        member_ids: &["ltx-video-2b-095", "t5xxl-fp8"],
        note: "Fast and light. Two files: model + VAE bundled in one, plus a \
               shared T5 text encoder.",
        is_default: false,
    },
    ModelStack {
        id: "kokoro-en",
        label: "Kokoro (English narrator)",
        media: "voice",
        member_ids: &["kokoro-v1.0-fp32", "kokoro-voices-v1.0"],
        note: "The Story Studio narrator. Two files: the model and its voice \
               embeddings — 54 English voices to pick a narrator preset from.",
        is_default: true,
    },
    ModelStack {
        id: "dia",
        label: "Dia 1.6B (expressive narrator)",
        media: "voice",
        member_ids: &[
            "dia-1.6b-config",
            "dia-1.6b-generation-config",
            "dia-1.6b-audio-tokenizer-config",
            "dia-1.6b-preprocessor-config",
            "dia-1.6b-special-tokens-map",
            "dia-1.6b-tokenizer-config",
            "dia-1.6b-weights-index",
            "dia-1.6b-weights-part1",
            "dia-1.6b-weights-part2",
            "dac-44khz-config",
            "dac-44khz-preprocessor-config",
            "dac-44khz-weights",
        ],
        note: "A second, more expressive narrator engine (Nari Labs' Dia-1.6B). Real \
               non-verbal tags -- (laughs), (sighs), (clears throat), and more -- plus \
               [S1]/[S2] speaker turns for actual dialogue, not just single-voice narration. \
               No freeform emotional stage directions like \"(angry)\": that isn't a real \
               model capability. Twelve files (~6.8 GB): Dia's own weights/config/tokenizer \
               (nine files) plus its separate audio codec, descript/dac_44khz (three files) \
               — both directories are required together, and imported locally so nothing \
               calls home for the codec at runtime. Slower than Kokoro; needs ~4.4-6 GB VRAM.",
        is_default: false,
    },
    // --- Training & captioning tools (dataset captioners) ---
    ModelStack {
        id: "wd-tagger",
        label: "WD EVA02-Large Tagger v3 (Danbooru tags)",
        media: "training",
        member_ids: &[
            "wd-eva02-large-tagger-v3-model",
            "wd-eva02-large-tagger-v3-tags",
        ],
        note: "The recommended dataset captioner: Danbooru-style tags for LoRA training \
               captions. Runs on the CPU (onnxruntime), so it never competes with training \
               for VRAM. Two files (~1.3 GB): the model and its tag list.",
        is_default: true,
    },
    ModelStack {
        id: "florence2-large",
        label: "Florence-2 large (prose captions)",
        media: "training",
        member_ids: &[
            "florence2-large-model",
            "florence2-large-config",
            "florence2-large-configuration-florence2",
            "florence2-large-modeling-florence2",
            "florence2-large-processing-florence2",
            "florence2-large-preprocessor-config",
            "florence2-large-generation-config",
            "florence2-large-tokenizer",
            "florence2-large-tokenizer-config",
            "florence2-large-vocab",
        ],
        note: "Sentence-style captions instead of tags. Ten files (~1.56 GB), pinned to one \
               revision because three of them are Python the model runs on load. Needs ~2 GB \
               VRAM.",
        is_default: false,
    },
    ModelStack {
        id: "qwen2.5-vl-7b",
        label: "Qwen2.5-VL 7B Instruct (second opinion)",
        media: "training",
        member_ids: &[
            "qwen2.5-vl-7b-model-00001-of-00005",
            "qwen2.5-vl-7b-model-00002-of-00005",
            "qwen2.5-vl-7b-model-00003-of-00005",
            "qwen2.5-vl-7b-model-00004-of-00005",
            "qwen2.5-vl-7b-model-00005-of-00005",
            "qwen2.5-vl-7b-model-index",
            "qwen2.5-vl-7b-config",
            "qwen2.5-vl-7b-generation-config",
            "qwen2.5-vl-7b-preprocessor-config",
            "qwen2.5-vl-7b-chat-template",
            "qwen2.5-vl-7b-tokenizer",
            "qwen2.5-vl-7b-tokenizer-config",
            "qwen2.5-vl-7b-vocab",
            "qwen2.5-vl-7b-merges",
        ],
        note: "Optional and large: re-captions a frame together with a later one when a \
               Florence-2 caption looks unsure, describing what changes between them. \
               Fourteen files (~16.6 GB download); loaded 4-bit it needs ~6 GB VRAM.",
        is_default: false,
    },
];

/// A curated LLM recommendation — chat or coding. Unlike [`KnownModel`] this
/// only pins a Hugging Face **repo** and a preferred quant, not a single
/// file's hash: GGUF quant repos get re-uploaded/re-quantized over time, and
/// the download manager (6.4) already verifies whatever file is actually
/// fetched against `lfs.oid` from the live HF API (`core::registry`) — the
/// same way Discovery does. `typical_vram_mb` is a rough, documented estimate
/// (weights plus ~8k ctx plus runtime overhead, see `docs/AGENT_MODELS.md`)
/// shown before the real file is looked up; [`crate::compat::verdict_from_total_mb`]
/// turns it into an honest 🟢/🟡/🔴 against the user's actual VRAM budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct FeaturedModel {
    pub id: &'static str,
    /// The role this fills once imported: `"chat"` or `"coding"`.
    pub role: &'static str,
    pub label: &'static str,
    /// Hugging Face `owner/repo` — a GGUF quant repo, resolved live via
    /// `GET /registry/models/{repo}` when the user asks to see it.
    pub repo: &'static str,
    /// Substring to find the recommended quant in the repo's file list
    /// (e.g. `"Q5_K_M"`).
    pub quant_hint: &'static str,
    pub typical_vram_mb: u32,
    pub license: &'static str,
    /// One line for the UI — what it is / when to pick it.
    pub note: &'static str,
    /// Roles to check on import (`chat` alone, or `chat` + `coding` for an
    /// agent model — see `docs/AGENT_MODELS.md`).
    pub import_roles: &'static [&'static str],
    /// The curated "pick this one" model for its role — one `true` per role.
    pub is_default: bool,
}

/// Sourced from `docs/AGENT_MODELS.md`'s researched candidates (coding) plus
/// the same family's base instruct models (chat) — all single-file GGUF
/// quants (no split parts), all verified to exist on Hugging Face.
pub const FEATURED_MODELS: &[FeaturedModel] = &[
    FeaturedModel {
        id: "qwen2.5-7b-instruct",
        role: "chat",
        label: "Qwen2.5-7B-Instruct",
        repo: "bartowski/Qwen2.5-7B-Instruct-GGUF",
        quant_hint: "Q5_K_M",
        typical_vram_mb: 8704,
        license: "Apache-2.0",
        note: "Strong general-purpose chat model with native tool-calling.",
        import_roles: &["chat"],
        is_default: true,
    },
    FeaturedModel {
        id: "qwen2.5-14b-instruct",
        role: "chat",
        label: "Qwen2.5-14B-Instruct",
        repo: "bartowski/Qwen2.5-14B-Instruct-GGUF",
        quant_hint: "Q4_K_M",
        typical_vram_mb: 12800,
        license: "Apache-2.0",
        note: "Bigger and sharper if you can spare the VRAM — fills most of a 16 GB card.",
        import_roles: &["chat"],
        is_default: false,
    },
    FeaturedModel {
        id: "qwen2.5-coder-7b-instruct",
        role: "coding",
        label: "Qwen2.5-Coder-7B-Instruct",
        repo: "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF",
        quant_hint: "Q5_K_M",
        typical_vram_mb: 8704,
        license: "Apache-2.0",
        note: "Best tool-calling/VRAM ratio for agent sessions (OpenCode) on 16 GB.",
        import_roles: &["chat", "coding"],
        is_default: true,
    },
    FeaturedModel {
        id: "qwen2.5-coder-14b-instruct",
        role: "coding",
        label: "Qwen2.5-Coder-14B-Instruct",
        repo: "bartowski/Qwen2.5-Coder-14B-Instruct-GGUF",
        quant_hint: "Q4_K_M",
        typical_vram_mb: 12800,
        license: "Apache-2.0",
        note: "Noticeably better code; fills most of a 16 GB card.",
        import_roles: &["chat", "coding"],
        is_default: false,
    },
    FeaturedModel {
        id: "hermes-3-llama-3.1-8b",
        role: "coding",
        label: "Hermes-3-Llama-3.1-8B",
        repo: "NousResearch/Hermes-3-Llama-3.1-8B-GGUF",
        quant_hint: "Q5_K_M",
        typical_vram_mb: 9216,
        license: "Llama-3.1 Community License",
        note: "Nous Research's own agent-tuned model — the natural pick for the Hermes runtime.",
        import_roles: &["chat", "coding"],
        is_default: false,
    },
];

/// A Colibri model (github.com/JustVugg/colibri) worth curating for this
/// project's hardware ceiling — see that adapter's own module doc for why
/// only the CPU-only, RAM-reachable end of Colibri's roster qualifies.
/// Unlike [`KnownModel`]/[`FeaturedModel`], AIWM does **not** download these:
/// the repo is a few dozen safetensors shards, and Hugging Face's own `hf`
/// CLI (with `hf_transfer` acceleration) fetches a directory like this far
/// faster than this project's single-stream downloader ever would — the UI
/// shows the exact command to run, then [`crate::model::register_directory_model`]
/// registers wherever it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ColibriModel {
    pub id: &'static str,
    pub label: &'static str,
    /// Hugging Face `owner/repo` to `hf download`.
    pub repo: &'static str,
    /// Colibri's own documented "you need this much RAM resident" figure —
    /// the number `capability::colibri`'s RAM preflight checks against.
    pub ram_estimate_mb: u32,
    /// Approximate total download size — shown before committing to it.
    pub disk_estimate_bytes: u64,
    pub license: &'static str,
    /// One line for the UI — what it is / the honest caveat.
    pub note: &'static str,
}

/// Only one entry: Qwen3.6-35B-A3B is the sole model in Colibri's roster that
/// is both real GGUF/llama.cpp couldn't already reach (a 35B MoE, 3B active)
/// *and* within reach of a 32 GB RAM machine (24 GB resident, tight but
/// possible) — every other family needs 167 GB-1.6 TB of disk and 16-32 GB+
/// RAM well beyond a single consumer box. OLMoE (7B) fits easily but needs a
/// local conversion script run against the original checkpoint (no
/// ready-made HF container), so it doesn't get a one-click catalog entry —
/// `docs/` covers it as a manual path instead.
pub const COLIBRI_MODELS: &[ColibriModel] = &[ColibriModel {
    id: "qwen3.6-35b-a3b-colibri",
    label: "Qwen3.6-35B-A3B (Colibri)",
    repo: "Kreuzzelg/qwen36-35b-a3b-colibri-i4-gs64",
    ram_estimate_mb: 24_576,
    disk_estimate_bytes: 20 * 1024 * 1024 * 1024,
    license: "Apache-2.0",
    note: "A 35B-parameter MoE (3B active) too large for GGUF/llama.cpp on a 16 GB card — \
           runs CPU-only via Colibri instead. Needs ~24 GB RAM resident; tight on a 32 GB \
           machine.",
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelKind;

    #[test]
    fn every_entry_is_internally_consistent() {
        for m in KNOWN_MODELS {
            assert_eq!(m.sha256.len(), 64, "{}: sha256 not 64 hex chars", m.id);
            assert!(m.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", m.id);
            assert!(m.size_bytes > 0, "{}", m.id);
            // A small allowlist of hosts trusted to serve immutable, versioned
            // release assets -- not "any https URL" (Hugging Face's own
            // `resolve/<rev>/` path is itself one such immutable-per-revision
            // scheme; a GitHub release asset URL is the same idea).
            let trusted_host = m.url.starts_with("https://huggingface.co/")
                || (m.url.starts_with("https://github.com/")
                    && m.url.contains("/releases/download/"));
            assert!(trusted_host, "{}: url is not from a trusted host", m.id);
            assert!(m.url.ends_with(m.file), "{}: url/file mismatch", m.id);
            // The declared kind must parse back to a real ModelKind.
            assert!(ModelKind::from_hint(m.kind).is_some(), "{}: bad kind", m.id);
            assert!(
                matches!(m.media, "image" | "video" | "voice" | "training"),
                "{}: bad media",
                m.id
            );
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = KNOWN_MODELS.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), KNOWN_MODELS.len());
    }

    #[test]
    fn find_by_sha256_is_case_insensitive() {
        let got =
            find_by_sha256("31E35C80FC4829D14F90153F4C74CD59C90B779F6AFE05A74CD6120B893F7E5B");
        assert_eq!(got.map(|m| m.id), Some("sdxl-base-1.0"));
        assert!(find_by_sha256("deadbeef").is_none());
    }

    #[test]
    fn exactly_one_default_base_model_per_group() {
        let base_image = KNOWN_MODELS
            .iter()
            .filter(|m| matches!(m.kind, "checkpoint" | "diffusion_model") && m.is_default)
            .count();
        assert_eq!(base_image, 1, "exactly one default image base model");

        let base_video = KNOWN_MODELS
            .iter()
            .filter(|m| m.kind == "video" && m.is_default)
            .count();
        assert_eq!(base_video, 1, "exactly one default video base model");

        let base_voice = KNOWN_MODELS
            .iter()
            .filter(|m| m.kind == "voice_model" && m.is_default)
            .count();
        assert_eq!(base_voice, 1, "exactly one default voice base model");

        // Companions (vae/text_encoder/voice_data/dia_engine/dia_codec) are
        // required, not alternatives -- none of them should be marked "the
        // pick". Dia's own 9+3 files have no single "the pick" at all: every
        // one of them is a required sibling, not an alternative to another.
        assert!(KNOWN_MODELS
            .iter()
            .filter(|m| matches!(
                m.kind,
                "vae"
                    | "text_encoder"
                    | "voice_data"
                    | "dia_engine"
                    | "dia_codec"
                    | "wd_tagger"
                    | "florence2_engine"
                    | "qwen_vl_engine"
            ))
            .all(|m| !m.is_default));
    }

    #[test]
    fn every_stack_member_id_resolves_to_a_real_known_model() {
        for s in MODEL_STACKS {
            assert!(!s.member_ids.is_empty(), "{}: empty stack", s.id);
            for id in s.member_ids {
                assert!(
                    KNOWN_MODELS.iter().any(|m| &m.id == id),
                    "{}: member {id:?} is not in KNOWN_MODELS",
                    s.id
                );
            }
        }
    }

    #[test]
    fn stack_ids_are_unique_and_the_base_model_matches_its_media() {
        let mut ids: Vec<_> = MODEL_STACKS.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MODEL_STACKS.len());

        // Only the *base* model (first member) has to match the stack's
        // declared media -- a companion can be shared across media (LTX's
        // video stack borrows Flux's image-tagged T5 text encoder).
        for s in MODEL_STACKS {
            let base_id = s.member_ids[0];
            let base = KNOWN_MODELS.iter().find(|m| m.id == base_id).unwrap();
            assert_eq!(
                base.media, s.media,
                "{}: base model {base_id:?} is {} media, stack is {}",
                s.id, base.media, s.media
            );
        }
    }

    #[test]
    fn exactly_one_default_stack_per_media() {
        for media in ["image", "video", "voice", "training"] {
            let count = MODEL_STACKS
                .iter()
                .filter(|s| s.media == media && s.is_default)
                .count();
            assert_eq!(
                count, 1,
                "media {media} should have exactly one default stack"
            );
        }
    }

    #[test]
    fn featured_ids_are_unique_and_every_entry_is_consistent() {
        let mut ids: Vec<_> = FEATURED_MODELS.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), FEATURED_MODELS.len());

        for m in FEATURED_MODELS {
            assert!(matches!(m.role, "chat" | "coding"), "{}: bad role", m.id);
            assert_eq!(
                m.repo.matches('/').count(),
                1,
                "{}: repo must be owner/repo",
                m.id
            );
            assert!(m.typical_vram_mb > 0, "{}", m.id);
            assert!(!m.quant_hint.is_empty(), "{}", m.id);
            assert!(!m.license.is_empty(), "{}", m.id);
            assert!(!m.note.is_empty(), "{}", m.id);
            assert!(
                m.import_roles.contains(&"chat"),
                "{}: chat.cpp only imports as chat",
                m.id
            );
        }
    }

    #[test]
    fn exactly_one_default_featured_model_per_role() {
        for role in ["chat", "coding"] {
            let count = FEATURED_MODELS
                .iter()
                .filter(|m| m.role == role && m.is_default)
                .count();
            assert_eq!(count, 1, "role {role} should have exactly one default");
        }
    }

    #[test]
    fn dia_stack_bundles_all_nine_engine_files_and_all_three_codec_files() {
        let stack = MODEL_STACKS.iter().find(|s| s.id == "dia").unwrap();
        assert_eq!(stack.media, "voice");
        assert!(!stack.is_default, "kokoro-en stays the default voice stack");

        let engine_count = stack
            .member_ids
            .iter()
            .filter(|id| {
                KNOWN_MODELS
                    .iter()
                    .find(|m| &m.id == *id)
                    .is_some_and(|m| m.kind == "dia_engine")
            })
            .count();
        let codec_count = stack
            .member_ids
            .iter()
            .filter(|id| {
                KNOWN_MODELS
                    .iter()
                    .find(|m| &m.id == *id)
                    .is_some_and(|m| m.kind == "dia_codec")
            })
            .count();
        // Every dia_engine/dia_codec catalog entry belongs to this one stack
        // -- so these counts double as "the whole catalogue is bundled".
        let total_engine = KNOWN_MODELS
            .iter()
            .filter(|m| m.kind == "dia_engine")
            .count();
        let total_codec = KNOWN_MODELS
            .iter()
            .filter(|m| m.kind == "dia_codec")
            .count();
        assert_eq!(engine_count, 9);
        assert_eq!(engine_count, total_engine);
        assert_eq!(codec_count, 3);
        assert_eq!(codec_count, total_codec);
        assert_eq!(stack.member_ids.len(), 12);
    }

    #[test]
    fn dia_catalog_entries_are_all_apache_or_mit_and_never_claim_freeform_emotion_tags() {
        for m in KNOWN_MODELS
            .iter()
            .filter(|m| matches!(m.kind, "dia_engine" | "dia_codec"))
        {
            assert!(
                matches!(m.license, "Apache-2.0" | "MIT"),
                "{}: unexpected license {}",
                m.id,
                m.license
            );
            // Dia cannot perform a freeform stage direction like "(angry)"
            // from text -- individual file notes must never imply otherwise.
            assert!(
                !m.note.to_lowercase().contains("(angry)"),
                "{}: note must not imply freeform emotion tags work",
                m.id
            );
        }
    }

    fn stack(id: &str) -> &'static ModelStack {
        MODEL_STACKS
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("stack {id:?} missing"))
    }

    fn member(id: &str) -> &'static KnownModel {
        KNOWN_MODELS.iter().find(|m| m.id == id).unwrap()
    }

    #[test]
    fn sha256_values_are_unique_so_an_import_matches_exactly_one_entry() {
        let mut shas: Vec<String> = KNOWN_MODELS
            .iter()
            .map(|m| m.sha256.to_ascii_lowercase())
            .collect();
        shas.sort_unstable();
        shas.dedup();
        assert_eq!(shas.len(), KNOWN_MODELS.len());
    }

    #[test]
    fn the_training_category_holds_the_three_captioner_stacks_wd_tagger_first() {
        let training: Vec<&str> = MODEL_STACKS
            .iter()
            .filter(|s| s.media == "training")
            .map(|s| s.id)
            .collect();
        assert_eq!(training, ["wd-tagger", "florence2-large", "qwen2.5-vl-7b"]);
        assert!(
            stack("wd-tagger").is_default,
            "the WD tagger is recommended"
        );
        assert!(!stack("florence2-large").is_default);
        assert!(!stack("qwen2.5-vl-7b").is_default);
    }

    #[test]
    fn the_wd_tagger_stack_is_the_two_existing_catalog_files() {
        let s = stack("wd-tagger");
        assert_eq!(
            s.member_ids,
            [
                "wd-eva02-large-tagger-v3-model",
                "wd-eva02-large-tagger-v3-tags"
            ]
        );
        let files: Vec<&str> = s.member_ids.iter().map(|id| member(id).file).collect();
        assert_eq!(files, ["model.onnx", "selected_tags.csv"]);
    }

    /// Every file of a directory-shaped captioner kind belongs to its one
    /// stack (nothing half-listed), and the stack carries exactly the files
    /// its `from_pretrained` loader reads.
    fn assert_directory_stack(stack_id: &str, kind: &str, expected_files: &[&str]) {
        let s = stack(stack_id);
        let mut files: Vec<&str> = s.member_ids.iter().map(|id| member(id).file).collect();
        files.sort_unstable();
        let mut expected = expected_files.to_vec();
        expected.sort_unstable();
        assert_eq!(files, expected, "{stack_id}");
        for id in s.member_ids {
            assert_eq!(member(id).kind, kind, "{id}");
            assert_eq!(member(id).media, "training", "{id}");
        }
        let total = KNOWN_MODELS.iter().filter(|m| m.kind == kind).count();
        assert_eq!(
            total,
            s.member_ids.len(),
            "{kind}: every file is in the stack"
        );
    }

    #[test]
    fn the_florence2_stack_bundles_weights_configs_tokenizer_and_remote_code() {
        assert_directory_stack(
            "florence2-large",
            "florence2_engine",
            &[
                "config.json",
                "configuration_florence2.py",
                "generation_config.json",
                "model.safetensors",
                "modeling_florence2.py",
                "preprocessor_config.json",
                "processing_florence2.py",
                "tokenizer.json",
                "tokenizer_config.json",
                "vocab.json",
            ],
        );
    }

    #[test]
    fn the_qwen_vl_stack_bundles_all_five_shards_and_the_processor_files() {
        assert_directory_stack(
            "qwen2.5-vl-7b",
            "qwen_vl_engine",
            &[
                "chat_template.json",
                "config.json",
                "generation_config.json",
                "merges.txt",
                "model-00001-of-00005.safetensors",
                "model-00002-of-00005.safetensors",
                "model-00003-of-00005.safetensors",
                "model-00004-of-00005.safetensors",
                "model-00005-of-00005.safetensors",
                "model.safetensors.index.json",
                "preprocessor_config.json",
                "tokenizer.json",
                "tokenizer_config.json",
                "vocab.json",
            ],
        );
    }

    /// Remote code and weights must never change under us: every file of a
    /// directory-shaped captioner (and the WD tagger pair) is fetched from
    /// one pinned 40-hex commit, never a moving branch like `main`.
    #[test]
    fn captioner_engine_urls_are_pinned_to_one_commit_per_repo() {
        for kind in ["florence2_engine", "qwen_vl_engine", "wd_tagger"] {
            let mut commits: Vec<&str> = Vec::new();
            for m in KNOWN_MODELS.iter().filter(|m| m.kind == kind) {
                let prefix = format!("https://huggingface.co/{}/resolve/", m.repo);
                let rest = m
                    .url
                    .strip_prefix(&prefix)
                    .unwrap_or_else(|| panic!("{}: unexpected url {}", m.id, m.url));
                let (commit, file) = rest.split_once('/').unwrap();
                assert_eq!(file, m.file, "{}", m.id);
                assert_eq!(commit.len(), 40, "{}: not a commit: {commit}", m.id);
                assert!(
                    commit
                        .chars()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                    "{}: not a commit: {commit}",
                    m.id
                );
                assert!(m.note.contains(commit), "{}: note names the commit", m.id);
                commits.push(commit);
            }
            commits.sort_unstable();
            commits.dedup();
            assert_eq!(
                commits.len(),
                1,
                "{kind}: one revision for the whole directory"
            );
        }
    }

    #[test]
    fn captioner_licenses_match_their_model_cards() {
        for m in KNOWN_MODELS {
            let expected = match m.kind {
                "florence2_engine" => "MIT",
                "qwen_vl_engine" | "wd_tagger" => "Apache-2.0",
                _ => continue,
            };
            assert_eq!(m.license, expected, "{}", m.id);
        }
    }

    #[test]
    fn colibri_ids_are_unique_and_every_entry_is_consistent() {
        let mut ids: Vec<_> = COLIBRI_MODELS.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), COLIBRI_MODELS.len());

        for m in COLIBRI_MODELS {
            assert_eq!(
                m.repo.matches('/').count(),
                1,
                "{}: repo must be owner/repo",
                m.id
            );
            assert!(m.ram_estimate_mb > 0, "{}", m.id);
            assert!(m.disk_estimate_bytes > 0, "{}", m.id);
            assert!(!m.license.is_empty(), "{}", m.id);
            assert!(!m.note.is_empty(), "{}", m.id);
        }
    }
}
