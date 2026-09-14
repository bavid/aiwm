//! What *kind* of model a file is — which decides where it lands in the
//! canonical store and how a runtime finds it.
//!
//! Chat / LLM models (`.gguf`) go to `<store>/llm/<slug>/` and llama.cpp reads
//! them by path. Image models (`.safetensors`, and `.gguf` for quantized Flux)
//! go to typed folders under `<store>/image/` that map 1:1 onto ComfyUI's
//! `models/<folder>` names — ComfyUI is pointed at them via
//! `extra_model_paths.yaml` (3.3), not a junction (the store and the ComfyUI
//! install are on different volumes).

/// Every model kind the importer understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// An LLM for llama.cpp (`chat`, `coding`, …).
    Chat,
    /// A full text-to-image checkpoint (SDXL and friends).
    Checkpoint,
    /// A bare diffusion transformer (Flux `unet`, GGUF-quantised).
    DiffusionModel,
    /// A VAE.
    Vae,
    /// A LoRA adapter.
    Lora,
    /// A text encoder (CLIP, T5, umt5).
    TextEncoder,
    /// A video diffusion model (Wan 2.2, LTX-2). Its encoder + VAE are imported
    /// separately as [`TextEncoder`](Self::TextEncoder) / [`Vae`](Self::Vae).
    VideoModel,
    /// A local text-to-speech model (Kokoro ONNX) — the Story Studio narrator.
    /// Consumed by the Python sidecar, not a runtime's own model-path scan, so
    /// it never appears in `extra_model_paths.yaml` or gets a runtime link;
    /// the capability reads `file_path` straight off the model row.
    VoiceModel,
    /// Kokoro's paired voice-embeddings file — meaningless without a matching
    /// [`VoiceModel`](Self::VoiceModel), same relationship [`TextEncoder`] has
    /// to a diffusion model. Also a `.bin` file, but never confused with the
    /// Pickle-format guard in `model::import::resolve_kind`: that guard only
    /// fires when no explicit `model_type` hint is given, and an import of
    /// this kind always is one (the catalog entry sets it).
    VoiceData,
}

impl ModelKind {
    /// Parse a UI / API hint. Accepts a few friendly aliases.
    pub fn from_hint(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "chat" | "llm" => Self::Chat,
            "checkpoint" | "checkpoints" => Self::Checkpoint,
            "diffusion_model" | "diffusion_models" | "unet" => Self::DiffusionModel,
            "vae" => Self::Vae,
            "lora" | "loras" => Self::Lora,
            "text_encoder" | "text_encoders" | "clip" => Self::TextEncoder,
            "video_model" | "video_models" | "video" => Self::VideoModel,
            "voice_model" | "voice" => Self::VoiceModel,
            "voice_data" | "voices" => Self::VoiceData,
            _ => return None,
        })
    }

    /// The default kind for a file with this extension when no hint is given.
    pub fn default_for_ext(ext: &str) -> Option<Self> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "gguf" => Self::Chat,
            "safetensors" => Self::Checkpoint,
            _ => return None,
        })
    }

    /// `true` for every extension this kind can be imported from.
    pub fn accepts_ext(self, ext: &str) -> bool {
        let ext = ext.to_ascii_lowercase();
        match self {
            Self::Chat => ext == "gguf",
            // Flux / Wan / LTX ship both ways.
            Self::DiffusionModel | Self::TextEncoder | Self::VideoModel => {
                ext == "safetensors" || ext == "gguf"
            }
            Self::Checkpoint | Self::Vae | Self::Lora => ext == "safetensors",
            Self::VoiceModel => ext == "onnx",
            Self::VoiceData => ext == "bin",
        }
    }

    pub fn is_llm(self) -> bool {
        self == Self::Chat
    }

    /// The model role the importer assigns so `Auto` capability + companion
    /// resolution can find this model. Chat models get their roles from the
    /// user; image models are typed, so the kind implies the role.
    pub fn default_role(self) -> Option<&'static str> {
        match self {
            // The checkpoint / diffusion transformer a text-to-image job needs.
            Self::Checkpoint | Self::DiffusionModel => Some("base_diffusion"),
            // The video diffusion model a text-to-video job needs.
            Self::VideoModel => Some("base_video"),
            // Flux / Wan companions, resolved by role in the capability body.
            Self::Vae => Some("vae"),
            Self::TextEncoder => Some("text_encoder"),
            // A LoRA is never required to run a stack, but it still needs a
            // role so the picker can list "every LoRA in the library" the
            // same way Auto/companion resolution lists VAEs and encoders.
            Self::Lora => Some("lora"),
            Self::Chat => None,
            Self::VoiceModel => Some("voice_model"),
            Self::VoiceData => Some("voice_data"),
        }
    }

    /// Path under the store root, e.g. `"llm"` or `"image/checkpoints"`.
    pub fn store_subdir(self) -> &'static str {
        match self {
            Self::Chat => "llm",
            Self::Checkpoint => "image/checkpoints",
            Self::DiffusionModel => "image/diffusion_models",
            Self::Vae => "image/vae",
            Self::Lora => "image/loras",
            Self::TextEncoder => "image/text_encoders",
            Self::VideoModel => "video/diffusion_models",
            Self::VoiceModel | Self::VoiceData => "voice",
        }
    }

    /// The ComfyUI `folder_paths` / `extra_model_paths.yaml` key, when this kind
    /// is a ComfyUI model. `None` for anything ComfyUI never loads itself
    /// (an LLM for llama.cpp, or a voice file for the Python sidecar).
    pub fn comfy_folder(self) -> Option<&'static str> {
        Some(match self {
            Self::Chat | Self::VoiceModel | Self::VoiceData => return None,
            Self::Checkpoint => "checkpoints",
            Self::DiffusionModel | Self::VideoModel => "diffusion_models",
            Self::Vae => "vae",
            Self::Lora => "loras",
            Self::TextEncoder => "text_encoders",
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Checkpoint => "checkpoint",
            Self::DiffusionModel => "diffusion_model",
            Self::Vae => "vae",
            Self::Lora => "lora",
            Self::TextEncoder => "text_encoder",
            Self::VideoModel => "video_model",
            Self::VoiceModel => "voice_model",
            Self::VoiceData => "voice_data",
        }
    }

    /// Image kinds that live under `<store>/image/` — for the ComfyUI
    /// model-paths config.
    pub const IMAGE_KINDS: [Self; 5] = [
        Self::Checkpoint,
        Self::DiffusionModel,
        Self::Vae,
        Self::Lora,
        Self::TextEncoder,
    ];

    /// Kinds that live under `<store>/video/`.
    pub const VIDEO_KINDS: [Self; 1] = [Self::VideoModel];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_and_aliases_parse() {
        assert_eq!(
            ModelKind::from_hint("Checkpoint"),
            Some(ModelKind::Checkpoint)
        );
        assert_eq!(
            ModelKind::from_hint("unet"),
            Some(ModelKind::DiffusionModel)
        );
        assert_eq!(ModelKind::from_hint("llm"), Some(ModelKind::Chat));
        assert_eq!(ModelKind::from_hint("nonsense"), None);
    }

    #[test]
    fn defaults_by_extension() {
        assert_eq!(ModelKind::default_for_ext("gguf"), Some(ModelKind::Chat));
        assert_eq!(
            ModelKind::default_for_ext("SAFETENSORS"),
            Some(ModelKind::Checkpoint)
        );
        assert_eq!(ModelKind::default_for_ext("ckpt"), None);
    }

    #[test]
    fn extension_acceptance() {
        assert!(ModelKind::Chat.accepts_ext("gguf"));
        assert!(!ModelKind::Chat.accepts_ext("safetensors"));
        assert!(ModelKind::Vae.accepts_ext("safetensors"));
        assert!(!ModelKind::Vae.accepts_ext("gguf"));
        assert!(ModelKind::DiffusionModel.accepts_ext("gguf"));
    }

    #[test]
    fn subdirs_and_comfy_folders_line_up() {
        assert_eq!(ModelKind::Checkpoint.store_subdir(), "image/checkpoints");
        assert_eq!(ModelKind::Checkpoint.comfy_folder(), Some("checkpoints"));
        assert_eq!(ModelKind::Chat.comfy_folder(), None);
        for k in ModelKind::IMAGE_KINDS {
            assert!(k.comfy_folder().is_some());
            assert!(k.store_subdir().starts_with("image/"));
        }
    }

    #[test]
    fn kinds_carry_a_role_for_auto_and_companion_resolution() {
        assert_eq!(ModelKind::Checkpoint.default_role(), Some("base_diffusion"));
        assert_eq!(
            ModelKind::DiffusionModel.default_role(),
            Some("base_diffusion")
        );
        assert_eq!(ModelKind::VideoModel.default_role(), Some("base_video"));
        assert_eq!(ModelKind::Vae.default_role(), Some("vae"));
        assert_eq!(ModelKind::TextEncoder.default_role(), Some("text_encoder"));
        assert_eq!(ModelKind::Lora.default_role(), Some("lora"));
        assert_eq!(ModelKind::Chat.default_role(), None);
    }

    #[test]
    fn video_model_routes_to_the_video_store_and_diffusion_folder() {
        assert_eq!(ModelKind::from_hint("video"), Some(ModelKind::VideoModel));
        assert_eq!(
            ModelKind::VideoModel.store_subdir(),
            "video/diffusion_models"
        );
        assert_eq!(
            ModelKind::VideoModel.comfy_folder(),
            Some("diffusion_models")
        );
        assert!(ModelKind::VideoModel.accepts_ext("safetensors"));
        assert!(ModelKind::VideoModel.accepts_ext("gguf"));
    }

    #[test]
    fn voice_kinds_parse_from_hints_and_land_in_their_own_store_folder() {
        assert_eq!(
            ModelKind::from_hint("voice_model"),
            Some(ModelKind::VoiceModel)
        );
        assert_eq!(ModelKind::from_hint("voice"), Some(ModelKind::VoiceModel));
        assert_eq!(
            ModelKind::from_hint("voice_data"),
            Some(ModelKind::VoiceData)
        );
        assert_eq!(ModelKind::from_hint("voices"), Some(ModelKind::VoiceData));

        assert_eq!(ModelKind::VoiceModel.store_subdir(), "voice");
        assert_eq!(ModelKind::VoiceData.store_subdir(), "voice");
    }

    #[test]
    fn voice_kinds_accept_only_their_own_extension() {
        assert!(ModelKind::VoiceModel.accepts_ext("onnx"));
        assert!(!ModelKind::VoiceModel.accepts_ext("bin"));
        assert!(ModelKind::VoiceData.accepts_ext("bin"));
        assert!(!ModelKind::VoiceData.accepts_ext("onnx"));
    }

    #[test]
    fn voice_kinds_are_invisible_to_every_runtimes_own_model_scan() {
        // Neither llama.cpp nor ComfyUI ever loads these -- only the Python
        // sidecar does, reading `file_path` straight off the model row -- so
        // they must never be scheduled for a runtime link or a ComfyUI
        // extra_model_paths.yaml entry.
        assert!(!ModelKind::VoiceModel.is_llm());
        assert_eq!(ModelKind::VoiceModel.comfy_folder(), None);
        assert_eq!(ModelKind::VoiceData.comfy_folder(), None);
    }

    #[test]
    fn voice_kinds_carry_their_own_distinct_roles() {
        assert_eq!(ModelKind::VoiceModel.default_role(), Some("voice_model"));
        assert_eq!(ModelKind::VoiceData.default_role(), Some("voice_data"));
    }
}
