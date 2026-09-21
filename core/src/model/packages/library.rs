//! The library grouped by base family — the reverse direction of
//! [`super::resolve_package`]: per family, its base (or what would provide
//! one), its companions, and the LoRAs made for it.

use serde::Serialize;

use super::{
    base_needs, companion_needs, has_role, is_base, is_lora, match_exact, need, own_stack,
    summarize, Catalog, FamilyRef, ItemKind, LibraryModel, Need, NeedRole, NeedStatus,
};
use crate::db::Model;
use crate::model::family::{BaseFamily, FamilySource, Runnable};

/// A library model inside a group, with how its family was decided (a weak
/// `header` / `name` guess shows as such, and is what "Save detected
/// families" persists).
#[derive(Debug, Clone, Serialize)]
pub struct GroupModel {
    pub model: Model,
    pub family_source: FamilySource,
}

/// One base family's part of the library.
#[derive(Debug, Clone, Serialize)]
pub struct LibraryGroup {
    pub family: FamilyRef,
    /// The preferred installed base made for this family — the first of
    /// `checkpoints`.
    pub base: Option<GroupModel>,
    /// Every installed checkpoint of this family: the catalog stack's own
    /// base first, then by use (most used first), then library order.
    pub checkpoints: Vec<GroupModel>,
    /// No checkpoint installed and the required base is only findable on
    /// Civitai: the user has to pick one (there is no size to download yet,
    /// so `missing_bytes` alone would read "0 B").
    pub base_choice_needed: bool,
    /// One line for the card, e.g. that a training base is installed but
    /// image generation needs a single-file checkpoint.
    pub note: Option<String>,
    /// When `base` is `None`: what would provide one — a base it works with
    /// that is installed plus a suggestion, the catalog file, a findable
    /// checkpoint, or why it cannot run here. Empty when `base` is set.
    pub base_needs: Vec<Need>,
    /// The stack's VAE / text encoders, installed or in the catalog.
    pub companions: Vec<Need>,
    pub loras: Vec<GroupModel>,
    /// Required catalog bytes still missing.
    pub missing_bytes: u64,
    /// Base and every companion installed, and runnable here.
    pub complete: bool,
}

/// A checkpoint or LoRA whose base family could not be inferred.
#[derive(Debug, Clone, Serialize)]
pub struct UnknownModel {
    pub model: Model,
    /// [`ItemKind::Checkpoint`] or [`ItemKind::Lora`].
    pub kind: ItemKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryPackages {
    /// In registry order; only families with a base or a LoRA installed.
    pub groups: Vec<LibraryGroup>,
    /// Every checkpoint and LoRA with no inferable base family (an
    /// unsupported base such as Krea 2, or a file that says nothing), in
    /// library order. Companions and training folders are not listed.
    pub unknown: Vec<UnknownModel>,
}

fn member(m: &LibraryModel) -> Option<GroupModel> {
    m.family.map(|(_, source)| GroupModel {
        model: m.model.clone(),
        family_source: source,
    })
}

fn group(
    family: &'static BaseFamily,
    registry: &[BaseFamily],
    library: &[LibraryModel],
    catalog: &Catalog<'static>,
) -> Option<LibraryGroup> {
    let of_family = |m: &&LibraryModel| m.family_id() == Some(family.id);
    let loras: Vec<GroupModel> = library
        .iter()
        .filter(|m| is_lora(&m.model))
        .filter(of_family)
        .filter_map(member)
        .collect();
    let checkpoints = checkpoints(family, library, catalog);
    if checkpoints.is_empty() && loras.is_empty() {
        return None;
    }
    let base = checkpoints.first().cloned();
    let (base_needs, companions) = match family.runnable {
        Runnable::No(reason) => (
            vec![need(
                NeedRole::Base,
                family.label,
                false,
                NeedStatus::NotRunnable { reason },
            )],
            Vec::new(),
        ),
        Runnable::Yes => (
            if base.is_some() {
                Vec::new()
            } else {
                base_needs(family, None, registry, library, catalog)
            },
            companion_needs(family, registry, library, catalog),
        ),
    };
    let everything: Vec<Need> = base_needs.iter().chain(&companions).cloned().collect();
    let (missing_bytes, _) = summarize(&everything);
    let complete = matches!(family.runnable, Runnable::Yes)
        && base.is_some()
        && companions
            .iter()
            .all(|n| matches!(n.status, NeedStatus::Installed { .. }));
    let base_choice_needed = base.is_none()
        && base_needs
            .iter()
            .any(|n| !n.optional && matches!(n.status, NeedStatus::Findable { .. }));
    let note = base
        .is_none()
        .then(|| training_base_note(family, library))
        .flatten();
    Some(LibraryGroup {
        family: FamilyRef::new(family, None),
        checkpoints,
        base_choice_needed,
        note,
        base,
        base_needs,
        companions,
        loras,
        missing_bytes,
        complete,
    })
}

/// Group the library by base family. Pure — see the module doc of
/// [`super`].
pub fn library_packages(
    library: &[LibraryModel],
    registry: &'static [BaseFamily],
    catalog: &Catalog<'static>,
) -> LibraryPackages {
    let groups: Vec<LibraryGroup> = registry
        .iter()
        .filter_map(|family| group(family, registry, library, catalog))
        .collect();
    // A catalog base matched by content sits in its group even without an
    // inferred family.
    let placed = |m: &LibraryModel| {
        groups
            .iter()
            .flat_map(|g| &g.checkpoints)
            .any(|c| c.model.id == m.model.id)
    };
    let unknown = library
        .iter()
        .filter(|m| m.family.is_none() && !placed(m))
        .filter_map(|m| {
            let kind = if is_lora(&m.model) {
                ItemKind::Lora
            } else if is_base(&m.model) {
                ItemKind::Checkpoint
            } else {
                return None;
            };
            Some(UnknownModel {
                model: m.model.clone(),
                kind,
            })
        })
        .collect();
    LibraryPackages { groups, unknown }
}

/// Every installed checkpoint of `family`: the stack's own base file first
/// (by content or catalog id, whatever its inferred family), then the other
/// checkpoints of the family, most used first (library order breaks ties).
fn checkpoints(
    family: &'static BaseFamily,
    library: &[LibraryModel],
    catalog: &Catalog<'static>,
) -> Vec<GroupModel> {
    let stack_base = own_stack(family, catalog)
        .and_then(|s| catalog.members(s).into_iter().next())
        .and_then(|k| match_exact(k, library))
        .filter(|m| is_base(&m.model));
    let mut others: Vec<&LibraryModel> = library
        .iter()
        .filter(|m| is_base(&m.model) && m.family_id() == Some(family.id))
        .filter(|m| stack_base.is_none_or(|s| s.model.id != m.model.id))
        .collect();
    others.sort_by_key(|m| std::cmp::Reverse(m.model.use_count));
    stack_base
        .into_iter()
        .chain(others)
        .map(|m| GroupModel {
            model: m.model.clone(),
            family_source: m.family.map_or(FamilySource::Catalog, |(_, s)| s),
        })
        .collect()
}

/// For a family without an installed checkpoint: a line saying the
/// trainer's base (a Diffusers folder, role `training_base_<family>` —
/// `training::profile`) is installed but is not a checkpoint image
/// generation can load.
fn training_base_note(family: &BaseFamily, library: &[LibraryModel]) -> Option<String> {
    let role = format!("training_base_{}", family.id.replace('-', "_"));
    library.iter().any(|m| has_role(&m.model, &role)).then(|| {
        format!(
            "Your training base is installed; image generation with {} needs a \
                 single-file checkpoint.",
            family.label
        )
    })
}
