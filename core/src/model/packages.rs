//! Packages: a model plus everything it needs to run (Plan 14).
//!
//! [`resolve_package`] turns a pick — a Civitai LoRA or checkpoint, a file
//! already in the library — into a [`Package`]: the item, its base family,
//! and one [`Need`] per missing-or-present part (the base a LoRA needs, the
//! VAE / text encoders of the family's catalog stack), each marked installed,
//! in the catalog, findable on Civitai, or not runnable here.
//! [`library_packages`] is the reverse direction: the library grouped by base
//! family.
//!
//! Both are pure functions over the library (with each model's family
//! already inferred — [`LibraryModel`]), the family registry and the
//! catalog. Nothing is written; the only I/O is [`load_library`]'s
//! safetensors header reads, which the caller runs off the async runtime.
//! The Civitai lookup that fills a [`NeedStatus::Findable`] list happens at
//! the route, behind the offline gate.

mod library;
#[cfg(test)]
mod tests;

pub use library::{library_packages, GroupModel, LibraryGroup, LibraryPackages};

use serde::{Serialize, Serializer};

use crate::db::Model;
use crate::model::catalog::{KnownModel, ModelStack, KNOWN_MODELS, MODEL_STACKS};
use crate::model::family::{
    infer_family, works_with, ArchGroup, BaseFamily, FamilySource, Runnable,
};
use crate::model::SafetensorsHeader;
use crate::registry::CheckpointCandidate;

/// What kind of thing was picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// Needs a base (and the base's companions).
    Lora,
    /// Is a base; needs its companions.
    Checkpoint,
    /// Anything else that targets a base (an embedding, a ControlNet, …) —
    /// resolved like a LoRA.
    Other,
}

/// The pick a package is built around.
#[derive(Debug, Clone, Serialize)]
pub struct PackageItem {
    pub name: String,
    pub kind: ItemKind,
    /// The source's own base label (Civitai `baseModel`), when there is one.
    pub base_label: Option<String>,
    /// The library id when the item is already installed.
    pub model_id: Option<String>,
    pub size_bytes: Option<u64>,
    /// The item's base family and how it was decided — the caller maps the
    /// label ([`crate::model::family::family_for_civitai`]) or infers it
    /// ([`infer_family`]).
    #[serde(skip)]
    pub family: Option<(&'static BaseFamily, FamilySource)>,
}

/// A base family as a package reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FamilyRef {
    pub id: &'static str,
    pub label: &'static str,
    pub arch_group: ArchGroup,
    /// How the family was decided; `None` for a library group (its members
    /// each carry their own).
    pub source: Option<FamilySource>,
    pub runnable: Runnable,
}

impl FamilyRef {
    pub fn new(family: &'static BaseFamily, source: Option<FamilySource>) -> Self {
        Self {
            id: family.id,
            label: family.label,
            arch_group: family.arch_group,
            source,
            runnable: family.runnable,
        }
    }
}

/// What a need is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedRole {
    Base,
    Vae,
    TextEncoder,
    /// Any other catalog kind, verbatim.
    Other(&'static str),
}

impl NeedRole {
    /// The role of a catalog entry of this [`KnownModel::kind`].
    pub fn of_kind(kind: &'static str) -> Self {
        match kind {
            "checkpoint" | "diffusion_model" | "video" => Self::Base,
            "vae" => Self::Vae,
            "text_encoder" => Self::TextEncoder,
            other => Self::Other(other),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Vae => "vae",
            Self::TextEncoder => "text_encoder",
            Self::Other(kind) => kind,
        }
    }

    /// Whether a library model can fill this role, by its roles.
    fn filled_by(self, model: &Model) -> bool {
        match self {
            Self::Base => is_base(model),
            Self::Vae => has_role(model, "vae"),
            Self::TextEncoder => has_role(model, "text_encoder"),
            Self::Other(kind) => has_role(model, kind),
        }
    }
}

impl Serialize for NeedRole {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// Where a need stands.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NeedStatus {
    /// In the library. `made_for` is false when it only *works with* the
    /// item (same architecture, another fine-tune).
    Installed {
        model_id: String,
        name: String,
        made_for: bool,
    },
    /// A pinned catalog file — known size and SHA-256.
    Catalog {
        known_model_id: &'static str,
        name: &'static str,
        size_bytes: u64,
        sha256: &'static str,
        url: &'static str,
    },
    /// No pinned file; Civitai has checkpoints for this base label. The
    /// route fills `candidates` (top 3 by downloads) unless offline — then
    /// `note` says why the list is empty.
    Findable {
        base_label: String,
        candidates: Vec<CheckpointCandidate>,
        note: Option<String>,
    },
    NotRunnable {
        reason: &'static str,
    },
}

/// One part of a package.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Need {
    pub role: NeedRole,
    pub label: String,
    /// A suggestion, not a requirement (the base a LoRA was made for, when
    /// a base it works with is already installed).
    pub optional: bool,
    pub status: NeedStatus,
}

/// The overall answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum Verdict {
    /// Everything required is installed.
    Ready,
    /// Something required is in the catalog or findable.
    NeedsDownload,
    NotRunnable(&'static str),
    /// The item's base family is not known.
    UnknownBase,
}

#[derive(Debug, Clone, Serialize)]
pub struct Package {
    pub item: PackageItem,
    pub family: Option<FamilyRef>,
    pub needs: Vec<Need>,
    /// Sum of the required catalog files still missing (findable ones have
    /// no size until picked).
    pub missing_bytes: u64,
    pub verdict: Verdict,
}

/// The catalog the resolver draws on — [`Catalog::builtin`] in production.
#[derive(Debug, Clone, Copy)]
pub struct Catalog<'a> {
    pub known: &'a [KnownModel],
    pub stacks: &'a [ModelStack],
}

impl Catalog<'static> {
    pub fn builtin() -> Self {
        Self {
            known: KNOWN_MODELS,
            stacks: MODEL_STACKS,
        }
    }
}

impl<'a> Catalog<'a> {
    fn stack(&self, id: &str) -> Option<&'a ModelStack> {
        self.stacks.iter().find(|s| s.id == id)
    }

    fn members(&self, stack: &ModelStack) -> Vec<&'a KnownModel> {
        stack
            .member_ids
            .iter()
            .filter_map(|id| self.known.iter().find(|k| k.id == *id))
            .collect()
    }
}

/// A library model with its family decided on read.
#[derive(Debug, Clone)]
pub struct LibraryModel {
    pub model: Model,
    pub family: Option<(&'static BaseFamily, FamilySource)>,
}

impl LibraryModel {
    pub fn infer(model: Model, header: Option<&SafetensorsHeader>) -> Self {
        let family = infer_family(&model, header);
        Self { model, family }
    }

    fn family_id(&self) -> Option<&'static str> {
        self.family.map(|(f, _)| f.id)
    }
}

/// Infer every model's family, reading a safetensors header only where the
/// row does not already record one. Blocking file I/O — run it in
/// `spawn_blocking`.
pub fn load_library(models: Vec<Model>) -> Vec<LibraryModel> {
    models
        .into_iter()
        .map(|model| {
            let needs_header = model.base_family.is_none()
                && model
                    .file_path
                    .to_ascii_lowercase()
                    .ends_with(".safetensors");
            let header = needs_header
                .then(|| {
                    crate::model::read_safetensors_header(std::path::Path::new(&model.file_path))
                })
                .and_then(|r| r.ok());
            LibraryModel::infer(model, header.as_ref())
        })
        .collect()
}

pub(crate) fn has_role(model: &Model, role: &str) -> bool {
    model.roles.iter().any(|r| r == role)
}

pub(crate) fn is_base(model: &Model) -> bool {
    has_role(model, "base_diffusion") || has_role(model, "base_video")
}

pub(crate) fn is_lora(model: &Model) -> bool {
    has_role(model, "lora")
}

/// The family's own catalog stack.
fn own_stack<'a>(family: &BaseFamily, catalog: &Catalog<'a>) -> Option<&'a ModelStack> {
    family.stack_id.and_then(|id| catalog.stack(id))
}

/// The stack whose companions (VAE, text encoders) a family's models run
/// with: its own, else the first family of the same architecture that has
/// one (FLUX.1 Krea runs with FLUX.1-dev's encoders and VAE). Returns the
/// family that stack belongs to as well.
fn companion_stack<'a>(
    family: &'static BaseFamily,
    registry: &[BaseFamily],
    catalog: &Catalog<'a>,
) -> Option<(&'a ModelStack, &'static str)> {
    if let Some(stack) = own_stack(family, catalog) {
        return Some((stack, family.id));
    }
    registry
        .iter()
        .filter(|f| works_with(family, f))
        .find_map(|f| {
            f.stack_id
                .and_then(|id| catalog.stack(id))
                .map(|s| (s, f.id))
        })
}

/// The library model that is this catalog file, in order of evidence:
/// its SHA-256; the catalog id recorded at import (`catalog:<id>` in
/// `source_revision` or `source`); then a model filling the same role whose
/// family is `family_id`.
fn match_member<'l>(
    member: &KnownModel,
    family_id: &str,
    library: &'l [LibraryModel],
) -> Option<&'l LibraryModel> {
    let by_sha = library.iter().find(|m| {
        m.model
            .sha256
            .as_deref()
            .is_some_and(|s| s.trim().eq_ignore_ascii_case(member.sha256))
    });
    let catalog_tag = format!("catalog:{}", member.id);
    let by_catalog = || {
        library.iter().find(|m| {
            m.model.source_revision.as_deref() == Some(catalog_tag.as_str())
                || m.model.source == catalog_tag
        })
    };
    let role = NeedRole::of_kind(member.kind);
    let by_role = || {
        library
            .iter()
            .find(|m| role.filled_by(&m.model) && m.family_id() == Some(family_id))
    };
    by_sha.or_else(by_catalog).or_else(by_role)
}

fn installed(m: &LibraryModel, made_for: bool) -> NeedStatus {
    NeedStatus::Installed {
        model_id: m.model.id.clone(),
        name: m.model.name.clone(),
        made_for,
    }
}

fn catalog_status(k: &'static KnownModel) -> NeedStatus {
    NeedStatus::Catalog {
        known_model_id: k.id,
        name: k.name,
        size_bytes: k.size_bytes,
        sha256: k.sha256,
        url: k.url,
    }
}

fn need(role: NeedRole, label: impl Into<String>, optional: bool, status: NeedStatus) -> Need {
    Need {
        role,
        label: label.into(),
        optional,
        status,
    }
}

/// The base a LoRA (or other add-on) of `family` needs: the exact base
/// installed → made for it; only a same-architecture base installed → works
/// with it, plus an optional suggestion of its own base; nothing installed →
/// its own base from the catalog, or findable on Civitai.
fn base_needs(
    family: &'static BaseFamily,
    base_label: Option<&str>,
    library: &[LibraryModel],
    catalog: &Catalog<'static>,
) -> Vec<Need> {
    let own_base = own_stack(family, catalog).and_then(|s| catalog.members(s).into_iter().next());
    let made_for = own_base
        .and_then(|k| match_member(k, family.id, library))
        .or_else(|| {
            library
                .iter()
                .find(|m| is_base(&m.model) && m.family_id() == Some(family.id))
        });
    if let Some(m) = made_for {
        return vec![need(
            NeedRole::Base,
            family.label,
            false,
            installed(m, true),
        )];
    }
    let suggestion = |optional: bool| match own_base {
        Some(k) => need(NeedRole::Base, family.label, optional, catalog_status(k)),
        None => need(
            NeedRole::Base,
            format!("{} checkpoint", family.label),
            optional,
            NeedStatus::Findable {
                base_label: base_label
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .or_else(|| family.civitai_labels.first().copied())
                    .unwrap_or(family.label)
                    .to_string(),
                candidates: Vec::new(),
                note: None,
            },
        ),
    };
    let works_with_base = library
        .iter()
        .find(|m| is_base(&m.model) && m.family.is_some_and(|(f, _)| works_with(family, f)));
    match works_with_base {
        Some(m) => vec![
            need(NeedRole::Base, family.label, false, installed(m, false)),
            suggestion(true),
        ],
        None => vec![suggestion(false)],
    }
}

/// Stack members a model can run without. The FLUX.2 klein stack carries a
/// second VAE that only image *editing* uses (its catalog note: "Only for
/// editing an existing image (not text-to-image)"); counting it as required
/// would report a working text-to-image setup as incomplete and add ~238 MB to
/// every "download missing".
const OPTIONAL_COMPANIONS: &[&str] = &["flux2-klein-edit-vae"];

/// The companions of `family`'s stack (every member but the base), each
/// installed or in the catalog; the ones in [`OPTIONAL_COMPANIONS`] are marked
/// optional, so they neither count towards the missing bytes nor block `Ready`.
fn companion_needs(
    family: &'static BaseFamily,
    registry: &[BaseFamily],
    library: &[LibraryModel],
    catalog: &Catalog<'static>,
) -> Vec<Need> {
    let Some((stack, stack_family)) = companion_stack(family, registry, catalog) else {
        return Vec::new();
    };
    catalog
        .members(stack)
        .into_iter()
        .filter(|k| NeedRole::of_kind(k.kind) != NeedRole::Base)
        .map(|k| {
            let status = match match_member(k, stack_family, library) {
                Some(m) => installed(m, true),
                None => catalog_status(k),
            };
            need(
                NeedRole::of_kind(k.kind),
                k.name,
                OPTIONAL_COMPANIONS.contains(&k.id),
                status,
            )
        })
        .collect()
}

/// Required bytes still to fetch, and the verdict, for a set of needs.
fn summarize(needs: &[Need]) -> (u64, Verdict) {
    let required = || needs.iter().filter(|n| !n.optional);
    let missing_bytes = required()
        .filter_map(|n| match n.status {
            NeedStatus::Catalog { size_bytes, .. } => Some(size_bytes),
            _ => None,
        })
        .sum();
    let incomplete = required().any(|n| {
        matches!(
            n.status,
            NeedStatus::Catalog { .. } | NeedStatus::Findable { .. }
        )
    });
    let verdict = if incomplete {
        Verdict::NeedsDownload
    } else {
        Verdict::Ready
    };
    (missing_bytes, verdict)
}

/// Build the package for `item`. See the module doc.
pub fn resolve_package(
    item: PackageItem,
    library: &[LibraryModel],
    registry: &[BaseFamily],
    catalog: &Catalog<'static>,
) -> Package {
    let Some((family, source)) = item.family else {
        return Package {
            item,
            family: None,
            needs: Vec::new(),
            missing_bytes: 0,
            verdict: Verdict::UnknownBase,
        };
    };
    let family_ref = Some(FamilyRef::new(family, Some(source)));
    if let Runnable::No(reason) = family.runnable {
        return Package {
            item,
            family: family_ref,
            needs: vec![need(
                NeedRole::Base,
                family.label,
                false,
                NeedStatus::NotRunnable { reason },
            )],
            missing_bytes: 0,
            verdict: Verdict::NotRunnable(reason),
        };
    }
    let mut needs = Vec::new();
    if item.kind != ItemKind::Checkpoint {
        needs.extend(base_needs(
            family,
            item.base_label.as_deref(),
            library,
            catalog,
        ));
    }
    needs.extend(companion_needs(family, registry, library, catalog));
    let (missing_bytes, verdict) = summarize(&needs);
    Package {
        item,
        family: family_ref,
        needs,
        missing_bytes,
        verdict,
    }
}
