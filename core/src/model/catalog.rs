//! A small curated catalogue of image models the tool knows how to run:
//! SDXL and the FLUX.1-dev GGUF stack (diffusion model + encoders + VAE).
//!
//! The MVP has no download manager (that is Phase 6), so "assisted import"
//! means: the UI shows this list with the Hugging Face source and the expected
//! SHA-256, the user downloads the file themselves, and [`import`](super::import)
//! stamps the catalogue metadata onto the model when the hash matches. Details
//! and recommended settings live in `docs/IMAGE_MODELS.md`.

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
    },
];

/// The catalogue entry whose file matches `sha256`, if any.
pub fn find_by_sha256(sha256: &str) -> Option<&'static KnownModel> {
    KNOWN_MODELS
        .iter()
        .find(|m| m.sha256.eq_ignore_ascii_case(sha256))
}

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
}
