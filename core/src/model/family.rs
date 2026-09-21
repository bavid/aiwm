//! Base-model families: one vocabulary for "which base does this need".
//!
//! [`FAMILIES`] is the registry of the bases AIWM knows. Each entry carries
//! the Civitai `baseModel` labels and Hugging Face `base_model` ids that mean
//! it, its [`ArchGroup`] (Pony / Illustrious / NoobAI are fine-tunes of the
//! SDXL architecture, so a LoRA for one *works with* the others), the catalog
//! stack that provides it, and whether it can run here at all.
//!
//! Labels are matched exactly (trimmed, case-insensitive) — a label that is
//! not in the table is unknown, never guessed. The Civitai labels come from
//! `GET https://civitai.com/api/v1/enums` (`BaseModel`), read 2026-09-21;
//! labels whose architecture could not be confirmed stay unmapped.
//!
//! [`infer_family`] decides the family of a model already in the library
//! without writing anything: a recorded family first, then the safetensors
//! header ([`family_from_header`]), then the file name ([`family_from_name`]).

mod detect;
#[cfg(test)]
mod tests;

pub use detect::{family_from_header, family_from_name};

use serde::Serialize;

use crate::db::Model;

/// Whether this app can run a family's models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum Runnable {
    Yes,
    /// Not here — and why, in words for the UI.
    No(&'static str),
}

/// Families whose weights share a layout: a LoRA trained on one member loads
/// on any other member of the same group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ArchGroup {
    #[serde(rename = "sd15")]
    Sd15,
    #[serde(rename = "sdxl")]
    Sdxl,
    #[serde(rename = "flux1")]
    Flux1,
    #[serde(rename = "flux2-klein-4b")]
    Flux2Klein4b,
    #[serde(rename = "flux2-klein-9b")]
    Flux2Klein9b,
    #[serde(rename = "wan22-5b")]
    Wan22Ti2v5b,
    #[serde(rename = "wan-14b")]
    Wan14b,
    #[serde(rename = "ltxv")]
    Ltxv,
    #[serde(rename = "ltx2")]
    Ltx2,
}

/// One base family AIWM knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BaseFamily {
    /// Stable registry id (what `models.base_family` holds once persisted).
    pub id: &'static str,
    /// Display name.
    pub label: &'static str,
    /// Civitai `baseModel` values that mean this family.
    pub civitai_labels: &'static [&'static str],
    /// Hugging Face `base_model` repo ids that mean this family.
    pub hf_base_models: &'static [&'static str],
    pub arch_group: ArchGroup,
    /// The [`crate::model::ModelStack`] that provides the base and its
    /// companions; `None` when AIWM has no pinned stack for it.
    pub stack_id: Option<&'static str>,
    pub runnable: Runnable,
}

const WAN_14B_REASON: &str = "does not fit 16 GB — this app runs the 5B";
const LTX2_REASON: &str = "LTX-2 is a different, much larger audio-video model — \
                           this app runs LTX-Video 0.9.5 (2B)";

/// The registry. Order is display order.
pub const FAMILIES: &[BaseFamily] = &[
    // SD 1.5 is supported; its curated stack arrives with the SD 1.5 task of
    // Plan 14, so there is no stack to name yet.
    BaseFamily {
        id: "sd15",
        label: "Stable Diffusion 1.5",
        civitai_labels: &["SD 1.5", "SD 1.4", "SD 1.5 LCM", "SD 1.5 Hyper"],
        hf_base_models: &[
            "stable-diffusion-v1-5/stable-diffusion-v1-5",
            "runwayml/stable-diffusion-v1-5",
            "CompVis/stable-diffusion-v1-4",
        ],
        arch_group: ArchGroup::Sd15,
        stack_id: None,
        runnable: Runnable::Yes,
    },
    BaseFamily {
        id: "sdxl",
        label: "Stable Diffusion XL",
        civitai_labels: &[
            "SDXL 1.0",
            "SDXL 0.9",
            "SDXL 1.0 LCM",
            "SDXL Lightning",
            "SDXL Hyper",
            "SDXL Turbo",
        ],
        hf_base_models: &["stabilityai/stable-diffusion-xl-base-1.0"],
        arch_group: ArchGroup::Sdxl,
        stack_id: Some("sdxl"),
        runnable: Runnable::Yes,
    },
    BaseFamily {
        id: "pony",
        label: "Pony Diffusion (SDXL)",
        civitai_labels: &["Pony"],
        hf_base_models: &["AstraliteHeart/pony-diffusion-v6"],
        arch_group: ArchGroup::Sdxl,
        stack_id: None,
        runnable: Runnable::Yes,
    },
    BaseFamily {
        id: "illustrious",
        label: "Illustrious (SDXL)",
        civitai_labels: &["Illustrious"],
        hf_base_models: &["OnomaAIResearch/Illustrious-xl-early-release-v0"],
        arch_group: ArchGroup::Sdxl,
        stack_id: None,
        runnable: Runnable::Yes,
    },
    BaseFamily {
        id: "noobai",
        label: "NoobAI (SDXL)",
        civitai_labels: &["NoobAI"],
        hf_base_models: &["Laxhar/noobai-XL-1.0"],
        arch_group: ArchGroup::Sdxl,
        stack_id: None,
        runnable: Runnable::Yes,
    },
    BaseFamily {
        id: "flux1",
        label: "FLUX.1 [dev]",
        civitai_labels: &["Flux.1 D"],
        hf_base_models: &["black-forest-labs/FLUX.1-dev"],
        arch_group: ArchGroup::Flux1,
        stack_id: Some("flux"),
        runnable: Runnable::Yes,
    },
    BaseFamily {
        id: "flux1-schnell",
        label: "FLUX.1 [schnell]",
        civitai_labels: &["Flux.1 S"],
        hf_base_models: &["black-forest-labs/FLUX.1-schnell"],
        arch_group: ArchGroup::Flux1,
        stack_id: None,
        runnable: Runnable::Yes,
    },
    BaseFamily {
        id: "flux1-krea",
        label: "FLUX.1 Krea [dev]",
        civitai_labels: &["Flux.1 Krea"],
        hf_base_models: &["black-forest-labs/FLUX.1-Krea-dev"],
        arch_group: ArchGroup::Flux1,
        stack_id: None,
        runnable: Runnable::Yes,
    },
    // No catalog stack pins the 4B; the library and the trainer use it.
    BaseFamily {
        id: "flux2-klein-4b",
        label: "FLUX.2 [klein] 4B",
        civitai_labels: &["Flux.2 Klein 4B", "Flux.2 Klein 4B-base"],
        hf_base_models: &[
            "black-forest-labs/FLUX.2-klein-4B",
            "black-forest-labs/FLUX.2-klein-base-4B",
        ],
        arch_group: ArchGroup::Flux2Klein4b,
        stack_id: None,
        runnable: Runnable::Yes,
    },
    BaseFamily {
        id: "flux2-klein-9b",
        label: "FLUX.2 [klein] 9B",
        civitai_labels: &["Flux.2 Klein 9B", "Flux.2 Klein 9B-base"],
        hf_base_models: &[
            "black-forest-labs/FLUX.2-klein-9B",
            "black-forest-labs/FLUX.2-klein-base-9B",
        ],
        arch_group: ArchGroup::Flux2Klein9b,
        stack_id: Some("flux2-klein"),
        runnable: Runnable::Yes,
    },
    BaseFamily {
        id: "wan22-5b",
        label: "Wan 2.2 TI2V-5B",
        civitai_labels: &["Wan Video 2.2 TI2V-5B"],
        hf_base_models: &["Wan-AI/Wan2.2-TI2V-5B", "Wan-AI/Wan2.2-TI2V-5B-Diffusers"],
        arch_group: ArchGroup::Wan22Ti2v5b,
        stack_id: Some("wan22"),
        runnable: Runnable::Yes,
    },
    // Wan 2.1 14B and Wan 2.2 A14B share the 5120-wide transformer.
    BaseFamily {
        id: "wan-14b",
        label: "Wan 14B",
        civitai_labels: &[
            "Wan Video 2.2 I2V-A14B",
            "Wan Video 2.2 T2V-A14B",
            "Wan Video 14B t2v",
            "Wan Video 14B i2v 480p",
            "Wan Video 14B i2v 720p",
        ],
        hf_base_models: &[
            "Wan-AI/Wan2.2-T2V-A14B",
            "Wan-AI/Wan2.2-I2V-A14B",
            "Wan-AI/Wan2.1-T2V-14B",
            "Wan-AI/Wan2.1-I2V-14B-480P",
            "Wan-AI/Wan2.1-I2V-14B-720P",
        ],
        arch_group: ArchGroup::Wan14b,
        stack_id: None,
        runnable: Runnable::No(WAN_14B_REASON),
    },
    BaseFamily {
        id: "ltxv",
        label: "LTX-Video (2B)",
        civitai_labels: &["LTXV"],
        hf_base_models: &["Lightricks/LTX-Video"],
        arch_group: ArchGroup::Ltxv,
        stack_id: Some("ltx"),
        runnable: Runnable::Yes,
    },
    BaseFamily {
        id: "ltx2",
        label: "LTX-2",
        civitai_labels: &["LTXV2", "LTXV 2.3"],
        hf_base_models: &["Lightricks/LTX-2", "Lightricks/LTX-2.3"],
        arch_group: ArchGroup::Ltx2,
        stack_id: None,
        runnable: Runnable::No(LTX2_REASON),
    },
];

/// How a model's family was decided — the `models.family_source` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilySource {
    /// Civitai's `baseModel` at download time.
    Civitai,
    /// Hugging Face `base_model` at download time.
    Hf,
    /// A curated catalog entry (`source_revision = catalog:<id>`).
    Catalog,
    /// Tensor names/shapes in the safetensors header.
    Header,
    /// The file name — the weakest hint.
    Name,
    /// The user picked it; never overwritten by anything else.
    User,
}

impl FamilySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Civitai => "civitai",
            Self::Hf => "hf",
            Self::Catalog => "catalog",
            Self::Header => "header",
            Self::Name => "name",
            Self::User => "user",
        }
    }

    /// The column value back as a source; `None` for anything else.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        [
            Self::Civitai,
            Self::Hf,
            Self::Catalog,
            Self::Header,
            Self::Name,
            Self::User,
        ]
        .into_iter()
        .find(|v| v.as_str().eq_ignore_ascii_case(s))
    }
}

/// How a LoRA made for `item` relates to a base of family `base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// The exact base it was made for.
    MadeFor,
    /// Same architecture — loads and works, but was tuned on another base.
    WorksWith,
    Incompatible,
}

/// The registry entry with this id.
pub fn family_by_id(id: &str) -> Option<&'static BaseFamily> {
    let id = id.trim();
    FAMILIES.iter().find(|f| f.id.eq_ignore_ascii_case(id))
}

/// The family a Civitai `baseModel` label means; `None` when unknown.
pub fn family_for_civitai(label: &str) -> Option<&'static BaseFamily> {
    let label = label.trim();
    FAMILIES.iter().find(|f| {
        f.civitai_labels
            .iter()
            .any(|l| l.eq_ignore_ascii_case(label))
    })
}

/// Relations Hugging Face puts between `base_model:` and the repo id in a
/// model's tags (`base_model:adapter:owner/repo`).
const HF_RELATIONS: &[&str] = &["adapter:", "finetune:", "quantized:", "merge:"];

/// The family a Hugging Face `base_model` means — a bare `owner/repo` or
/// the `base_model:[relation:]owner/repo` tag form. Repo ids compare
/// case-insensitively, as the Hub resolves them.
pub fn family_for_hf(base_model: &str) -> Option<&'static BaseFamily> {
    let mut repo = base_model.trim();
    repo = strip_prefix_ci(repo, "base_model:").unwrap_or(repo);
    repo = HF_RELATIONS
        .iter()
        .find_map(|rel| strip_prefix_ci(repo, rel))
        .unwrap_or(repo);
    FAMILIES.iter().find(|f| {
        f.hf_base_models
            .iter()
            .any(|r| r.eq_ignore_ascii_case(repo))
    })
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// Whether a model of family `a` loads with a model of family `b`: the same
/// weight layout ([`ArchGroup`]).
pub fn works_with(a: &BaseFamily, b: &BaseFamily) -> bool {
    a.arch_group == b.arch_group
}

/// Whether `item` was made for exactly `base`.
pub fn made_for(item: &BaseFamily, base: &BaseFamily) -> bool {
    item.id == base.id
}

/// [`made_for`] / [`works_with`] as one answer.
pub fn fit(item: &BaseFamily, base: &BaseFamily) -> Fit {
    if made_for(item, base) {
        Fit::MadeFor
    } else if works_with(item, base) {
        Fit::WorksWith
    } else {
        Fit::Incompatible
    }
}

/// `models.family` strings written before the registry, and the registry id
/// each one means on a catalog row. `flux`, `flux2`, `wan` and `ltx` are
/// ambiguous elsewhere: the import derives them from any file name
/// containing the word, and the trainer stores both FLUX.2 [klein] sizes as
/// `flux2` (`training::runner::settle::library_family`).
const LEGACY_FAMILIES: &[(&str, &str)] = &[
    ("flux", "flux1"),
    ("flux2", "flux2-klein-9b"),
    ("wan", "wan22-5b"),
    ("ltx", "ltxv"),
];

fn legacy_alias(family: &str) -> Option<&'static str> {
    LEGACY_FAMILIES
        .iter()
        .find(|(legacy, _)| legacy.eq_ignore_ascii_case(family))
        .map(|(_, id)| *id)
}

/// Map a `models.family` string onto the registry without changing what is
/// stored: a registry id (`_` read as `-`), or one of the legacy strings
/// above. Trimmed, case-insensitive; `None` for anything else (`qwen2`,
/// `florence2`, `sd3`, …).
pub fn normalize_library_family(family: &str) -> Option<&'static BaseFamily> {
    let family = family.trim().replace('_', "-");
    family_by_id(&family).or_else(|| legacy_alias(&family).and_then(family_by_id))
}

/// The family of a library model and how it was decided, without writing
/// anything. Order:
///
/// 1. a recorded `base_family` (a registry id, with how it was decided in
///    `family_source`) — authoritative, whatever the legacy string says;
/// 2. on a catalog row (`source_revision = catalog:<id>`), the catalog's
///    legacy family in its catalog meaning;
/// 3. a legacy `family` string that is unambiguous (`sdxl`, a registry id);
/// 4. the safetensors `header` ([`family_from_header`]);
/// 5. an ambiguous legacy string (`flux`, `flux2`, `wan`, `ltx` — see
///    [`LEGACY_FAMILIES`]) in its catalog meaning;
/// 6. the file name, then the display name ([`family_from_name`]).
///
/// A recorded base family the registry does not know falls through to the
/// next step; one without a readable source counts as
/// [`FamilySource::Name`]. Legacy strings count as [`FamilySource::Name`]:
/// before the registry, the import derived `family` from the file name.
pub fn infer_family(
    model: &Model,
    header: Option<&crate::model::SafetensorsHeader>,
) -> Option<(&'static BaseFamily, FamilySource)> {
    if let Some(family) = model.base_family.as_deref().and_then(family_by_id) {
        let source = model
            .family_source
            .as_deref()
            .and_then(FamilySource::parse)
            .unwrap_or(FamilySource::Name);
        return Some((family, source));
    }
    let stored = model
        .family
        .as_deref()
        .and_then(|f| normalize_library_family(f).map(|family| (f, family)));

    if let (Some((_, family)), true) = (stored, is_catalog_row(model)) {
        return Some((family, FamilySource::Catalog));
    }
    let ambiguous = stored.is_some_and(|(raw, _)| legacy_alias(raw.trim()).is_some());
    if let (Some((_, family)), false) = (stored, ambiguous) {
        return Some((family, FamilySource::Name));
    }
    if let Some(family) = header.and_then(family_from_header) {
        return Some((family, FamilySource::Header));
    }
    if let Some((_, family)) = stored {
        return Some((family, FamilySource::Name));
    }
    file_name(&model.file_path)
        .and_then(family_from_name)
        .or_else(|| family_from_name(&model.name))
        .map(|family| (family, FamilySource::Name))
}

fn is_catalog_row(model: &Model) -> bool {
    model
        .source_revision
        .as_deref()
        .is_some_and(|r| r.starts_with("catalog:"))
}

/// The last path component, for either separator (library paths are
/// Windows paths, tests may use either).
fn file_name(path: &str) -> Option<&str> {
    path.rsplit(['\\', '/']).next().filter(|n| !n.is_empty())
}
