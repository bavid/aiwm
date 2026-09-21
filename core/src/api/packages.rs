//! Packages over the API (Plan 14): resolve a Civitai pick or a library model
//! into the package it needs, group the library by base family, and persist
//! base families (the user's choice, or a previewed batch of detections).
//!
//! Reading never writes: the library's families are inferred on read
//! ([`crate::model::packages::load_library`], header reads in
//! `spawn_blocking`). The one network call is the Civitai lookup behind the
//! offline gate — the model details for a Civitai pick, and the top
//! checkpoints for a findable base.

use serde::{Deserialize, Serialize};

use crate::db::{Database, Model, SetFamily};
use crate::model::family::{family_by_id, family_for_civitai, FamilySource, FAMILIES};
use crate::model::packages::{
    library_packages as group_library, load_library, resolve_package, Catalog, ItemKind,
    LibraryModel, LibraryPackages, NeedStatus, Package, PackageItem,
};
use crate::registry::RemoteModelDetails;
use crate::{App, CoreError, Result};

/// How many checkpoints a findable base lists.
const FINDABLE_CANDIDATES: usize = 3;

/// `GET /packages/resolve` query.
#[derive(Debug, Clone, Deserialize)]
pub struct ResolveQuery {
    /// `civitai` | `library`.
    pub source: String,
    /// Civitai's numeric model id, or the library model id.
    pub model_id: String,
    /// Civitai version id; the primary version when omitted.
    #[serde(default)]
    pub version_id: Option<String>,
    /// Include NSFW checkpoints in a findable base's list — only when the
    /// caller's own search had it on.
    #[serde(default)]
    pub nsfw: bool,
}

/// One row of `POST /models/base-families`.
#[derive(Debug, Clone, Deserialize)]
pub struct BaseFamilyChoice {
    pub model_id: String,
    pub family: String,
}

/// `POST /models/{id}/base-family` body.
#[derive(Debug, Clone, Deserialize)]
pub struct SetBaseFamilyDto {
    pub family: String,
}

/// What happened to one row of a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveOutcome {
    Written,
    /// The user picked this model's family — never replaced by a detection.
    KeptUserChoice,
    /// Not written; `reason` says why.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavedFamily {
    pub model_id: String,
    pub outcome: SaveOutcome,
    pub reason: Option<String>,
}

fn err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("packages: {msg}"))
}

/// The library with every family inferred, header reads off the runtime.
async fn library(app: &App) -> Result<Vec<LibraryModel>> {
    let models = app.db.models().list().await?;
    tokio::task::spawn_blocking(move || load_library(models))
        .await
        .map_err(|e| err(format!("library read worker failed: {e}")))
}

fn kind_of_hint(hint: Option<&str>) -> ItemKind {
    match hint.map(str::to_ascii_lowercase).as_deref() {
        Some("lora" | "locon" | "dora" | "lycoris") => ItemKind::Lora,
        Some("checkpoint") => ItemKind::Checkpoint,
        _ => ItemKind::Other,
    }
}

/// The package item for a Civitai model version.
fn civitai_item(
    details: &RemoteModelDetails,
    version_id: Option<&str>,
    library: &[LibraryModel],
) -> Result<PackageItem> {
    let version = match version_id.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => details
            .versions
            .iter()
            .find(|x| x.id == v)
            .ok_or_else(|| err(format!("version {v} is not part of this model")))?,
        None => details
            .versions
            .first()
            .ok_or_else(|| err("this model lists no versions"))?,
    };
    // Files are only known for the primary version (`details.revision`).
    let file = (version.id == details.revision)
        .then(|| {
            details
                .files
                .iter()
                .find(|f| f.path.ends_with(".safetensors"))
        })
        .flatten();
    let installed = file.and_then(|f| f.sha256.as_deref()).and_then(|sha| {
        library.iter().find(|m| {
            m.model
                .sha256
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case(sha))
        })
    });
    let base_label = version.base_model.clone();
    Ok(PackageItem {
        name: details
            .model
            .name
            .clone()
            .unwrap_or_else(|| details.model.id.clone()),
        kind: kind_of_hint(details.model.model_kind_hint.as_deref()),
        family: base_label
            .as_deref()
            .and_then(family_for_civitai)
            .map(|f| (f, FamilySource::Civitai)),
        base_label,
        model_id: installed.map(|m| m.model.id.clone()),
        size_bytes: file.map(|f| f.size),
    })
}

/// The package item for a library model.
fn library_item(m: &LibraryModel) -> PackageItem {
    let roles = &m.model.roles;
    let kind = if roles.iter().any(|r| r == "lora") {
        ItemKind::Lora
    } else if roles
        .iter()
        .any(|r| r == "base_diffusion" || r == "base_video")
    {
        ItemKind::Checkpoint
    } else {
        ItemKind::Other
    };
    PackageItem {
        name: m.model.name.clone(),
        kind,
        base_label: None,
        model_id: Some(m.model.id.clone()),
        size_bytes: u64::try_from(m.model.size_bytes).ok(),
        family: m.family,
    }
}

/// Fill every findable need with Civitai's top checkpoints for its label —
/// or, offline / on a failed lookup, leave the list empty with a note.
async fn fill_findable(app: &App, package: &mut Package, nsfw: bool) {
    for need in &mut package.needs {
        let NeedStatus::Findable {
            base_label,
            candidates,
            note,
        } = &mut need.status
        else {
            continue;
        };
        if app.offline() {
            *note = Some("offline mode is on — no checkpoint search".into());
            continue;
        }
        match app
            .civitai_registry
            .checkpoints_for_base(base_label, nsfw, FINDABLE_CANDIDATES)
            .await
        {
            Ok(fetched) => *candidates = fetched.data,
            Err(e) => {
                tracing::warn!(label = %base_label, error = %e, "checkpoint lookup failed");
                *note = Some(format!("checkpoint search failed: {e}"));
            }
        }
    }
}

/// `GET /packages/resolve`.
pub async fn resolve(app: &App, q: ResolveQuery) -> Result<Package> {
    let library = library(app).await?;
    let catalog = Catalog::builtin();
    let item = match q.source.trim() {
        "civitai" => {
            let details = app.civitai_registry.details(q.model_id.trim()).await?.data;
            civitai_item(&details, q.version_id.as_deref(), &library)?
        }
        "library" => {
            let m = library
                .iter()
                .find(|m| m.model.id == q.model_id.trim())
                .ok_or_else(|| err(format!("model {} is not in the library", q.model_id)))?;
            library_item(m)
        }
        other => return Err(err(format!("unknown source {other:?}"))),
    };
    let mut package = resolve_package(item, &library, FAMILIES, &catalog);
    fill_findable(app, &mut package, q.nsfw).await;
    Ok(package)
}

/// `GET /packages/library`. Findable bases stay without candidates here —
/// the view asks for them per group through `resolve`.
pub async fn library_packages(app: &App) -> Result<LibraryPackages> {
    let library = library(app).await?;
    Ok(group_library(&library, FAMILIES, &Catalog::builtin()))
}

/// `POST /models/{id}/base-family` — the user's own choice.
pub async fn set_base_family(app: &App, model_id: &str, family: &str) -> Result<Model> {
    app.db
        .models()
        .set_base_family(model_id, family, FamilySource::User)
        .await?;
    app.db
        .models()
        .get(model_id)
        .await?
        .ok_or_else(|| err(format!("model {model_id} is not in the library")))
}

/// `POST /models/base-families` — persist a previewed batch of detections.
pub async fn save_base_families(app: &App, batch: &[BaseFamilyChoice]) -> Result<Vec<SavedFamily>> {
    let library = library(app).await?;
    save_detected(&app.db, &library, batch).await
}

fn saved(model_id: &str, outcome: SaveOutcome, reason: Option<String>) -> SavedFamily {
    SavedFamily {
        model_id: model_id.to_string(),
        outcome,
        reason,
    }
}

/// Write each row whose family is still what the library infers, with the
/// source it was inferred from (`header`, `name`, …). A row the user decided
/// is kept; a row whose detection changed since the preview is skipped.
pub async fn save_detected(
    db: &Database,
    library: &[LibraryModel],
    batch: &[BaseFamilyChoice],
) -> Result<Vec<SavedFamily>> {
    let mut out = Vec::with_capacity(batch.len());
    for choice in batch {
        let id = choice.model_id.trim();
        let Some(m) = library.iter().find(|m| m.model.id == id) else {
            out.push(saved(
                id,
                SaveOutcome::Skipped,
                Some("not in the library".into()),
            ));
            continue;
        };
        let Some(wanted) = family_by_id(&choice.family) else {
            out.push(saved(
                id,
                SaveOutcome::Skipped,
                Some(format!("unknown base family {:?}", choice.family)),
            ));
            continue;
        };
        let Some((family, source)) = m.family else {
            out.push(saved(
                id,
                SaveOutcome::Skipped,
                Some("no family detected".into()),
            ));
            continue;
        };
        if source == FamilySource::User {
            out.push(saved(id, SaveOutcome::KeptUserChoice, None));
            continue;
        }
        if family.id != wanted.id {
            out.push(saved(
                id,
                SaveOutcome::Skipped,
                Some(format!("detected {} now, not {}", family.id, wanted.id)),
            ));
            continue;
        }
        let outcome = match db.models().set_base_family(id, family.id, source).await? {
            SetFamily::Written => SaveOutcome::Written,
            SetFamily::KeptUserChoice => SaveOutcome::KeptUserChoice,
        };
        out.push(saved(id, outcome, None));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewModel;

    async fn lora(db: &Database, file: &str) -> String {
        db.models()
            .insert(NewModel {
                name: file.into(),
                format: "safetensors".into(),
                file_path: format!("E:\\AI\\models\\loras\\{file}"),
                size_bytes: 1,
                source: "manual".into(),
                roles: vec!["lora".into()],
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id
    }

    async fn inferred(db: &Database) -> Vec<LibraryModel> {
        load_library(db.models().list().await.unwrap())
    }

    fn choice(id: &str, family: &str) -> BaseFamilyChoice {
        BaseFamilyChoice {
            model_id: id.into(),
            family: family.into(),
        }
    }

    #[tokio::test]
    async fn a_previewed_batch_writes_detections_and_skips_user_rows() {
        let db = Database::connect_in_memory().await.unwrap();
        let detected = lora(&db, "ponyStyle.safetensors").await;
        let chosen = lora(&db, "ponyOther.safetensors").await;
        let changed = lora(&db, "sdxl_thing.safetensors").await;
        // The preview showed "pony" for `chosen`; the user has since picked
        // Illustrious for it.
        db.models()
            .set_base_family(&chosen, "illustrious", FamilySource::User)
            .await
            .unwrap();
        let library = inferred(&db).await;

        let out = save_detected(
            &db,
            &library,
            &[
                choice(&detected, "pony"),
                choice(&chosen, "pony"),
                choice(&changed, "pony"),
                choice("gone", "pony"),
            ],
        )
        .await
        .unwrap();
        let outcomes: Vec<SaveOutcome> = out.iter().map(|s| s.outcome).collect();
        assert_eq!(
            outcomes,
            [
                SaveOutcome::Written,
                SaveOutcome::KeptUserChoice,
                SaveOutcome::Skipped,
                SaveOutcome::Skipped,
            ]
        );

        let get = |id: String| {
            let db = db.clone();
            async move {
                let m = db.models().get(&id).await.unwrap().unwrap();
                (m.base_family, m.family_source)
            }
        };
        assert_eq!(
            get(detected).await,
            (Some("pony".into()), Some("name".into()))
        );
        assert_eq!(
            get(chosen).await,
            (Some("illustrious".into()), Some("user".into()))
        );
        assert_eq!(get(changed).await, (None, None));
    }

    #[tokio::test]
    async fn a_civitai_item_takes_the_asked_versions_base() {
        use crate::registry::{Gated, RemoteFormat, RemoteModel, RemoteVersion};
        let details = RemoteModelDetails {
            model: RemoteModel {
                id: "1".into(),
                name: Some("Style".into()),
                author: None,
                downloads: 0,
                likes: 0,
                trending_score: None,
                created_at: None,
                last_modified: None,
                pipeline_tag: None,
                library_name: None,
                gated: Gated::No,
                license: None,
                base_model: None,
                tags: vec![],
                param_count: None,
                arch: None,
                ctx_max: None,
                precision: None,
                format: RemoteFormat::Safetensors,
                nsfw: false,
                preview_image_url: None,
                previews: vec![],
                allow_commercial_use: vec![],
                model_kind_hint: Some("LORA".into()),
                base_model_family: Some("Pony, SDXL 1.0".into()),
            },
            revision: "11".into(),
            files: vec![],
            versions: vec![
                RemoteVersion {
                    id: "11".into(),
                    name: None,
                    base_model: Some("Pony".into()),
                },
                RemoteVersion {
                    id: "10".into(),
                    name: None,
                    base_model: Some("SDXL 1.0".into()),
                },
            ],
        };
        let item = civitai_item(&details, Some("10"), &[]).unwrap();
        assert_eq!(item.kind, ItemKind::Lora);
        assert_eq!(item.base_label.as_deref(), Some("SDXL 1.0"));
        assert_eq!(item.family.map(|(f, _)| f.id), Some("sdxl"));
        let primary = civitai_item(&details, None, &[]).unwrap();
        assert_eq!(primary.family.map(|(f, _)| f.id), Some("pony"));
        assert!(civitai_item(&details, Some("99"), &[]).is_err());
    }
}
