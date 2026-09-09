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
    /// A text encoder (CLIP, T5).
    TextEncoder,
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
            // Flux is shipped both ways.
            Self::DiffusionModel | Self::TextEncoder => ext == "safetensors" || ext == "gguf",
            Self::Checkpoint | Self::Vae | Self::Lora => ext == "safetensors",
        }
    }

    pub fn is_llm(self) -> bool {
        self == Self::Chat
    }

    /// The model role the importer assigns so `Auto` capability selection can
    /// find this model. Chat models get their roles from the user; image models
    /// are typed, so the kind implies the role.
    pub fn default_role(self) -> Option<&'static str> {
        match self {
            // The checkpoint / diffusion transformer a text-to-image job needs.
            Self::Checkpoint | Self::DiffusionModel => Some("base_diffusion"),
            Self::Chat | Self::Vae | Self::Lora | Self::TextEncoder => None,
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
        }
    }

    /// The ComfyUI `folder_paths` / `extra_model_paths.yaml` key, when this kind
    /// is a ComfyUI model.
    pub fn comfy_folder(self) -> Option<&'static str> {
        Some(match self {
            Self::Chat => return None,
            Self::Checkpoint => "checkpoints",
            Self::DiffusionModel => "diffusion_models",
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
        }
    }

    /// Every image kind — for building the ComfyUI model-paths config.
    pub const IMAGE_KINDS: [Self; 5] = [
        Self::Checkpoint,
        Self::DiffusionModel,
        Self::Vae,
        Self::Lora,
        Self::TextEncoder,
    ];
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
    fn checkpoints_carry_the_base_diffusion_role() {
        assert_eq!(ModelKind::Checkpoint.default_role(), Some("base_diffusion"));
        assert_eq!(
            ModelKind::DiffusionModel.default_role(),
            Some("base_diffusion")
        );
        assert_eq!(ModelKind::Vae.default_role(), None);
        assert_eq!(ModelKind::Chat.default_role(), None);
    }
}
