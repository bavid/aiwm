//! The library grouped by base family — the reverse direction of
//! [`super::resolve_package`]: per family, its base (or what would provide
//! one), its companions, and the LoRAs made for it.

use serde::Serialize;

use super::{
    base_needs, companion_needs, is_base, is_lora, match_member, need, own_stack, summarize,
    Catalog, FamilyRef, LibraryModel, Need, NeedRole, NeedStatus,
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
    /// The installed base made for this family.
    pub base: Option<GroupModel>,
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

#[derive(Debug, Clone, Serialize)]
pub struct LibraryPackages {
    /// In registry order; only families with a base or a LoRA installed.
    pub groups: Vec<LibraryGroup>,
    /// LoRAs whose base family could not be inferred.
    pub orphans: Vec<Model>,
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
    // The stack's own base file first (by content, whatever its inferred
    // family), then any base of this family.
    let stack_base = own_stack(family, catalog)
        .and_then(|s| catalog.members(s).into_iter().next())
        .and_then(|k| match_member(k, family.id, library));
    let base = stack_base
        .or_else(|| library.iter().filter(|m| is_base(&m.model)).find(of_family))
        .map(|m| GroupModel {
            model: m.model.clone(),
            family_source: m.family.map_or(FamilySource::Catalog, |(_, s)| s),
        });
    if base.is_none() && loras.is_empty() {
        return None;
    }
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
                base_needs(family, None, library, catalog)
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
    Some(LibraryGroup {
        family: FamilyRef::new(family, None),
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
    let groups = registry
        .iter()
        .filter_map(|family| group(family, registry, library, catalog))
        .collect();
    let orphans = library
        .iter()
        .filter(|m| is_lora(&m.model) && m.family.is_none())
        .map(|m| m.model.clone())
        .collect();
    LibraryPackages { groups, orphans }
}
