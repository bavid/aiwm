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
    /// `"image"` or `"video"` — which stack this entry belongs to. Not
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
    /// `"image"` or `"video"`.
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
        member_ids: &["flux2-klein-9b-q4", "qwen3-8b-flux2-encoder", "flux2-vae"],
        note: "Fast (sub-second at 4 steps), fits a 16 GB card. Three files: the \
               diffusion model, its Qwen3 text encoder, and its VAE. The realistic-detail \
               LoRA is a separate, optional download from the Discover tab.",
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
            assert!(m.url.starts_with("https://huggingface.co/"), "{}", m.id);
            assert!(m.url.ends_with(m.file), "{}: url/file mismatch", m.id);
            // The declared kind must parse back to a real ModelKind.
            assert!(ModelKind::from_hint(m.kind).is_some(), "{}: bad kind", m.id);
            assert!(matches!(m.media, "image" | "video"), "{}: bad media", m.id);
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

        // Companions (vae/text_encoder) are required, not alternatives -- none
        // of them should be marked "the pick".
        assert!(KNOWN_MODELS
            .iter()
            .filter(|m| matches!(m.kind, "vae" | "text_encoder"))
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
        for media in ["image", "video"] {
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
