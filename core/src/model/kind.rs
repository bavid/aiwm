//! What *kind* of model a file is — which decides where it lands in the
//! canonical store and how a runtime finds it.
//!
//! Chat / LLM models (`.gguf`) go to `<store>/llm/<slug>/` and llama.cpp reads
//! them by path. Image models (`.safetensors`, and `.gguf` for quantized Flux)
//! go to typed folders under `<store>/image/` that map 1:1 onto ComfyUI's
//! `models/<folder>` names — ComfyUI is pointed at them via
//! `extra_model_paths.yaml` (3.3), not a junction (the store and the ComfyUI
//! install are on different volumes).

/// The model-library role [`ModelKind::WdTagger`] imports under — shared
/// with `capability::dataset::captioner`'s registry entry so the two never
/// drift apart (the captioner resolves models by this exact string).
pub const WD_TAGGER_ROLE: &str = "vision_wd_tagger";

/// The model-library role [`ModelKind::Florence2Engine`] imports under —
/// re-exported as `capability::dataset::caption::FLORENCE2_ROLE`, which
/// resolves the captioner's directory by this exact string.
pub const FLORENCE2_ROLE: &str = "vision_florence2";

/// The model-library role [`ModelKind::QwenVlEngine`] imports under — the
/// Qwen2.5-VL escalation model `capability::dataset::caption` resolves.
pub const QWEN_VL_ROLE: &str = "vision_qwen2_5_vl";

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
    /// One file of Nari Labs' Dia-1.6B narrator (`config.json`,
    /// `model-0000N-of-00002.safetensors`, …) — nine files that must land as
    /// *co-located siblings with their original Hugging Face filenames* in
    /// one directory, because `DiaForConditionalGeneration::from_pretrained`
    /// reads a directory, not a single path. Unlike every other kind, the
    /// destination filename is never hash-suffixed on a naming collision
    /// (`model::import::unique_destination`) — the fixed per-kind
    /// subdirectory already guarantees nothing else lands next to it.
    DiaEngine,
    /// One file of Dia's separate audio codec, `descript/dac_44khz` — three
    /// files with the same collision-free-by-construction treatment as
    /// [`DiaEngine`](Self::DiaEngine), but in their own directory: both
    /// repos ship a `config.json` and the codec is a distinct Hugging Face
    /// repo (MIT, not Dia's Apache-2.0), so they can never share a folder.
    /// Imported locally (rather than left for `AutoProcessor` to fetch on
    /// first use) to keep the narrator fully offline once both are in.
    DiaCodec,
    /// A CLIP vision encoder (`ComfyUI`'s core `CLIPVisionLoader`) — the
    /// image half of IP-Adapter-style character consistency (Story Studio
    /// Phase 2). Distinct from [`TextEncoder`](Self::TextEncoder): that role
    /// feeds `CLIPTextEncode` (prompts), this one feeds `CLIPVisionEncode`
    /// (reference images), and the two are never interchangeable files.
    ClipVision,
    /// An IP-Adapter weight file (`ComfyUI_IPAdapter_plus`'s
    /// `IPAdapterModelLoader`) — the identity/style-lock half of Story
    /// Studio Phase 2 character consistency, paired with a
    /// [`ClipVision`](Self::ClipVision) encoder.
    IpAdapter,
    /// SmilingWolf's WD Danbooru tagger — `model.onnx` + `selected_tags.csv`,
    /// co-located, consumed by the Python sidecar (`vision.tag_frame`).
    /// Never a ComfyUI model.
    WdTagger,
    /// One file of `microsoft/Florence-2-large` (weights, configs,
    /// tokenizer and the `trust_remote_code` Python files) — the same
    /// directory shape as [`DiaEngine`](Self::DiaEngine): the sidecar's
    /// `AutoModelForCausalLM.from_pretrained(<dir>, trust_remote_code=True)`
    /// reads co-located siblings by their original Hugging Face names, so
    /// the destination is never hash-suffixed.
    Florence2Engine,
    /// One file of `Qwen/Qwen2.5-VL-7B-Instruct` (five weight shards plus
    /// index, configs, tokenizer and chat template) — directory-shaped like
    /// [`Florence2Engine`](Self::Florence2Engine), read by
    /// `Qwen2_5_VLForConditionalGeneration.from_pretrained(<dir>)`.
    QwenVlEngine,
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
            "dia_engine" => Self::DiaEngine,
            "dia_codec" => Self::DiaCodec,
            "clip_vision" | "clip_vision_model" => Self::ClipVision,
            "ip_adapter" | "ipadapter" => Self::IpAdapter,
            "wd_tagger" => Self::WdTagger,
            "florence2_engine" => Self::Florence2Engine,
            "qwen_vl_engine" => Self::QwenVlEngine,
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
            Self::Checkpoint | Self::Vae | Self::Lora | Self::ClipVision | Self::IpAdapter => {
                ext == "safetensors"
            }
            Self::VoiceModel => ext == "onnx",
            Self::VoiceData => ext == "bin",
            // A mix of small JSON config/tokenizer files and one or two
            // safetensors weight shards -- both real extensions, in the same
            // directory, for both the Dia engine and its DAC codec.
            Self::DiaEngine | Self::DiaCodec => ext == "json" || ext == "safetensors",
            Self::WdTagger => ext == "onnx" || ext == "csv",
            // `.py` is Florence-2's remote code; `model::import` only lets a
            // `.py` in when its SHA-256 is a pinned catalog entry.
            Self::Florence2Engine => matches!(ext.as_str(), "json" | "safetensors" | "py"),
            // `merges.txt` is half of Qwen2's BPE tokenizer.
            Self::QwenVlEngine => matches!(ext.as_str(), "json" | "safetensors" | "txt"),
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
            Self::DiaEngine => Some("dia_engine"),
            Self::DiaCodec => Some("dia_codec"),
            Self::ClipVision => Some("clip_vision"),
            Self::IpAdapter => Some("ip_adapter"),
            Self::WdTagger => Some(WD_TAGGER_ROLE),
            Self::Florence2Engine => Some(FLORENCE2_ROLE),
            Self::QwenVlEngine => Some(QWEN_VL_ROLE),
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
            // Their own fixed subdirectories, not the flat "voice" folder --
            // each is a directory of co-located sibling files consumed as a
            // whole (see the type doc on `DiaEngine`/`DiaCodec`), and needs a
            // home no other kind's file could ever land in under the same
            // name.
            Self::DiaEngine => "voice/dia-engine",
            Self::DiaCodec => "voice/dia-codec",
            Self::ClipVision => "image/clip_vision",
            Self::IpAdapter => "image/ipadapter",
            Self::WdTagger => "vision/wd-tagger",
            Self::Florence2Engine => "vision/florence2-large",
            Self::QwenVlEngine => "vision/qwen2.5-vl-7b",
        }
    }

    /// The ComfyUI `folder_paths` / `extra_model_paths.yaml` key, when this kind
    /// is a ComfyUI model. `None` for anything ComfyUI never loads itself
    /// (an LLM for llama.cpp, or a voice file for the Python sidecar).
    pub fn comfy_folder(self) -> Option<&'static str> {
        Some(match self {
            Self::Chat
            | Self::VoiceModel
            | Self::VoiceData
            | Self::DiaEngine
            | Self::DiaCodec
            | Self::WdTagger
            | Self::Florence2Engine
            | Self::QwenVlEngine => return None,
            Self::Checkpoint => "checkpoints",
            Self::DiffusionModel | Self::VideoModel => "diffusion_models",
            Self::Vae => "vae",
            Self::Lora => "loras",
            Self::TextEncoder => "text_encoders",
            Self::ClipVision => "clip_vision",
            Self::IpAdapter => "ipadapter",
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
            Self::DiaEngine => "dia_engine",
            Self::DiaCodec => "dia_codec",
            Self::ClipVision => "clip_vision",
            Self::IpAdapter => "ip_adapter",
            Self::WdTagger => "wd_tagger",
            Self::Florence2Engine => "florence2_engine",
            Self::QwenVlEngine => "qwen_vl_engine",
        }
    }

    /// Image kinds that live under `<store>/image/` — for the ComfyUI
    /// model-paths config.
    pub const IMAGE_KINDS: [Self; 7] = [
        Self::Checkpoint,
        Self::DiffusionModel,
        Self::Vae,
        Self::Lora,
        Self::TextEncoder,
        Self::ClipVision,
        Self::IpAdapter,
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

    #[test]
    fn dia_kinds_parse_from_hints_and_land_in_their_own_dedicated_subdirs() {
        assert_eq!(
            ModelKind::from_hint("dia_engine"),
            Some(ModelKind::DiaEngine)
        );
        assert_eq!(ModelKind::from_hint("dia_codec"), Some(ModelKind::DiaCodec));

        // Nested under "voice/", but each in its own directory -- never the
        // flat "voice" folder Kokoro's two files share.
        assert_eq!(ModelKind::DiaEngine.store_subdir(), "voice/dia-engine");
        assert_eq!(ModelKind::DiaCodec.store_subdir(), "voice/dia-codec");
        assert_ne!(
            ModelKind::DiaEngine.store_subdir(),
            ModelKind::DiaCodec.store_subdir()
        );
    }

    #[test]
    fn dia_kinds_accept_their_real_file_extensions_only() {
        assert!(ModelKind::DiaEngine.accepts_ext("json"));
        assert!(ModelKind::DiaEngine.accepts_ext("safetensors"));
        assert!(!ModelKind::DiaEngine.accepts_ext("onnx"));
        assert!(!ModelKind::DiaEngine.accepts_ext("bin"));
        assert!(ModelKind::DiaCodec.accepts_ext("json"));
        assert!(ModelKind::DiaCodec.accepts_ext("safetensors"));
        assert!(!ModelKind::DiaCodec.accepts_ext("gguf"));
    }

    #[test]
    fn dia_kinds_are_invisible_to_every_runtimes_own_model_scan() {
        // Same reasoning as the Kokoro voice kinds above -- the Python
        // sidecar reads a directory path straight off the resolved model
        // rows, never a runtime's own model-path scan.
        assert!(!ModelKind::DiaEngine.is_llm());
        assert!(!ModelKind::DiaCodec.is_llm());
        assert_eq!(ModelKind::DiaEngine.comfy_folder(), None);
        assert_eq!(ModelKind::DiaCodec.comfy_folder(), None);
    }

    #[test]
    fn dia_kinds_carry_their_own_distinct_roles() {
        assert_eq!(ModelKind::DiaEngine.default_role(), Some("dia_engine"));
        assert_eq!(ModelKind::DiaCodec.default_role(), Some("dia_codec"));
    }

    #[test]
    fn clip_vision_and_ip_adapter_parse_from_hints() {
        assert_eq!(
            ModelKind::from_hint("clip_vision"),
            Some(ModelKind::ClipVision)
        );
        assert_eq!(
            ModelKind::from_hint("clip_vision_model"),
            Some(ModelKind::ClipVision)
        );
        assert_eq!(
            ModelKind::from_hint("ip_adapter"),
            Some(ModelKind::IpAdapter)
        );
        assert_eq!(
            ModelKind::from_hint("ipadapter"),
            Some(ModelKind::IpAdapter)
        );
    }

    #[test]
    fn wd_tagger_is_a_sidecar_kind_with_its_own_folder() {
        assert_eq!(ModelKind::WdTagger.default_role(), Some(WD_TAGGER_ROLE));
        assert_eq!(WD_TAGGER_ROLE, "vision_wd_tagger");
        assert_eq!(ModelKind::WdTagger.store_subdir(), "vision/wd-tagger");
        assert_eq!(ModelKind::WdTagger.comfy_folder(), None);
        assert_eq!(ModelKind::WdTagger.as_str(), "wd_tagger");
        assert_eq!(ModelKind::from_hint("wd_tagger"), Some(ModelKind::WdTagger));
        assert!(ModelKind::WdTagger.accepts_ext("onnx"));
        assert!(ModelKind::WdTagger.accepts_ext("csv"));
        assert!(!ModelKind::WdTagger.accepts_ext("safetensors"));
        assert!(!ModelKind::WdTagger.is_llm());
    }

    #[test]
    fn captioner_engine_kinds_are_directory_shaped_sidecar_kinds_with_their_own_folders() {
        assert_eq!(
            ModelKind::from_hint("florence2_engine"),
            Some(ModelKind::Florence2Engine)
        );
        assert_eq!(
            ModelKind::from_hint("qwen_vl_engine"),
            Some(ModelKind::QwenVlEngine)
        );
        assert_eq!(ModelKind::Florence2Engine.as_str(), "florence2_engine");
        assert_eq!(ModelKind::QwenVlEngine.as_str(), "qwen_vl_engine");

        assert_eq!(FLORENCE2_ROLE, "vision_florence2");
        assert_eq!(QWEN_VL_ROLE, "vision_qwen2_5_vl");
        assert_eq!(
            ModelKind::Florence2Engine.default_role(),
            Some(FLORENCE2_ROLE)
        );
        assert_eq!(ModelKind::QwenVlEngine.default_role(), Some(QWEN_VL_ROLE));

        assert_eq!(
            ModelKind::Florence2Engine.store_subdir(),
            "vision/florence2-large"
        );
        assert_eq!(
            ModelKind::QwenVlEngine.store_subdir(),
            "vision/qwen2.5-vl-7b"
        );
        for k in [ModelKind::Florence2Engine, ModelKind::QwenVlEngine] {
            assert_eq!(k.comfy_folder(), None);
            assert!(!k.is_llm());
            assert!(k.accepts_ext("json"));
            assert!(k.accepts_ext("safetensors"));
            assert!(!k.accepts_ext("bin"), "never the Pickle weights");
            assert!(!k.accepts_ext("gguf"));
        }
        // Florence-2 ships its architecture as `trust_remote_code` Python
        // files; Qwen2.5-VL's BPE tokenizer needs `merges.txt`.
        assert!(ModelKind::Florence2Engine.accepts_ext("py"));
        assert!(!ModelKind::Florence2Engine.accepts_ext("txt"));
        assert!(ModelKind::QwenVlEngine.accepts_ext("txt"));
        assert!(!ModelKind::QwenVlEngine.accepts_ext("py"));
    }

    #[test]
    fn clip_vision_and_ip_adapter_are_safetensors_only_and_route_to_their_own_comfy_folders() {
        assert!(ModelKind::ClipVision.accepts_ext("safetensors"));
        assert!(!ModelKind::ClipVision.accepts_ext("gguf"));
        assert!(ModelKind::IpAdapter.accepts_ext("safetensors"));
        assert!(!ModelKind::IpAdapter.accepts_ext("bin"));

        assert_eq!(ModelKind::ClipVision.store_subdir(), "image/clip_vision");
        assert_eq!(ModelKind::ClipVision.comfy_folder(), Some("clip_vision"));
        assert_eq!(ModelKind::IpAdapter.store_subdir(), "image/ipadapter");
        assert_eq!(ModelKind::IpAdapter.comfy_folder(), Some("ipadapter"));

        assert_eq!(ModelKind::ClipVision.default_role(), Some("clip_vision"));
        assert_eq!(ModelKind::IpAdapter.default_role(), Some("ip_adapter"));

        // Both are ComfyUI-loaded image models, so they must ride along in
        // IMAGE_KINDS (which drives `extra_model_paths.yaml`) same as every
        // other image kind.
        assert!(ModelKind::IMAGE_KINDS.contains(&ModelKind::ClipVision));
        assert!(ModelKind::IMAGE_KINDS.contains(&ModelKind::IpAdapter));
    }
}
