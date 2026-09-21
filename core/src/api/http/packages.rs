//! Package routes (Plan 14) — thin wrappers over [`crate::api::packages`].

use axum::extract::{Path, Query, State};
use axum::Json;

use super::{ApiError, AppState};
use crate::api::packages::{self, BaseFamilyChoice, ResolveQuery, SavedFamily, SetBaseFamilyDto};
use crate::db::Model;
use crate::model::packages::{LibraryPackages, Package};

/// `GET /packages/resolve?source=civitai&model_id=…&version_id=…` or
/// `?source=library&model_id=…`.
pub(super) async fn resolve(
    State(app): AppState,
    Query(q): Query<ResolveQuery>,
) -> Result<Json<Package>, ApiError> {
    Ok(Json(packages::resolve(&app, q).await?))
}

/// `GET /packages/library`.
pub(super) async fn library(State(app): AppState) -> Result<Json<LibraryPackages>, ApiError> {
    Ok(Json(packages::library_packages(&app).await?))
}

/// `POST /models/{id}/base-family` — the user's choice.
pub(super) async fn set_base_family(
    State(app): AppState,
    Path(id): Path<String>,
    Json(body): Json<SetBaseFamilyDto>,
) -> Result<Json<Model>, ApiError> {
    Ok(Json(
        packages::set_base_family(&app, &id, &body.family).await?,
    ))
}

/// `POST /models/base-families` — persist a previewed batch.
pub(super) async fn save_base_families(
    State(app): AppState,
    Json(body): Json<Vec<BaseFamilyChoice>>,
) -> Result<Json<Vec<SavedFamily>>, ApiError> {
    Ok(Json(packages::save_base_families(&app, &body).await?))
}
