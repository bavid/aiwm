//! Persisting a model's base family (`models.base_family`, a registry id)
//! together with how it was decided (`models.family_source`).
//!
//! `models.family` — the legacy/runtime string the recipes, the trainer
//! preflight and the LoRA pickers compare against — is never written here.

use serde::Serialize;

use super::ModelRepo;
use crate::model::family::{family_by_id, FamilySource};
use crate::{CoreError, Result};

/// What [`ModelRepo::set_base_family`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetFamily {
    Written,
    /// The row carries the user's own choice and the new value is not one —
    /// nothing was changed.
    KeptUserChoice,
}

impl ModelRepo<'_> {
    /// Record `family` (a registry id, [`crate::model::family::FAMILIES`])
    /// as the model's `base_family`, and the `source` that decided it. A
    /// family the user picked
    /// (`family_source = 'user'`) is only ever replaced by another user
    /// choice; anything else reports [`SetFamily::KeptUserChoice`].
    pub async fn set_base_family(
        &self,
        model_id: &str,
        family: &str,
        source: FamilySource,
    ) -> Result<SetFamily> {
        let family = family_by_id(family)
            .ok_or_else(|| CoreError::Config(format!("unknown base family {family:?}")))?;
        // One statement, so a concurrent user choice cannot slip in between
        // a check and the write.
        let res = sqlx::query(
            "UPDATE models SET base_family = $1, family_source = $2
             WHERE id = $3 AND (family_source IS NULL OR family_source <> 'user' OR $2 = 'user')",
        )
        .bind(family.id)
        .bind(source.as_str())
        .bind(model_id)
        .execute(self.pool)
        .await?;
        if res.rows_affected() > 0 {
            return Ok(SetFamily::Written);
        }
        let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM models WHERE id = $1")
            .bind(model_id)
            .fetch_optional(self.pool)
            .await?;
        match exists {
            Some(_) => Ok(SetFamily::KeptUserChoice),
            None => Err(CoreError::Config(format!(
                "model {model_id} is not in the library"
            ))),
        }
    }

    /// Record where a downloaded model came from: `origin`
    /// (`civitai:<model>/<version>`, `hf:<repo>@<rev>`) becomes its `source`
    /// when the import left the default `manual` there, and `base_family`
    /// (when the source's label mapped to one) is written through
    /// [`Self::set_base_family`] — so a user choice survives a re-download.
    pub async fn record_origin(
        &self,
        model_id: &str,
        origin: Option<&str>,
        base_family: Option<(&str, FamilySource)>,
    ) -> Result<()> {
        if let Some(origin) = origin.map(str::trim).filter(|o| !o.is_empty()) {
            sqlx::query("UPDATE models SET source = $1 WHERE id = $2 AND source = 'manual'")
                .bind(origin)
                .bind(model_id)
                .execute(self.pool)
                .await?;
        }
        if let Some((family, source)) = base_family {
            self.set_base_family(model_id, family, source).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SetFamily;
    use crate::db::{Database, NewModel};
    use crate::model::family::FamilySource;

    async fn lora(db: &Database) -> String {
        db.models()
            .insert(NewModel {
                name: "style".into(),
                family: Some("sdxl".into()),
                format: "safetensors".into(),
                file_path: "E:\\AI\\models\\loras\\style.safetensors".into(),
                size_bytes: 1,
                source: "manual".into(),
                roles: vec!["lora".into()],
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id
    }

    /// `(base_family, family_source)` — and the legacy `family` column must
    /// still hold what the import wrote.
    async fn family_of(db: &Database, id: &str) -> (Option<String>, Option<String>) {
        let m = db.models().get(id).await.unwrap().unwrap();
        assert_eq!(
            m.family.as_deref(),
            Some("sdxl"),
            "models.family is never rewritten"
        );
        (m.base_family, m.family_source)
    }

    #[tokio::test]
    async fn a_new_row_has_no_base_family() {
        let db = Database::connect_in_memory().await.unwrap();
        let id = lora(&db).await;
        assert_eq!(family_of(&db, &id).await, (None, None));
    }

    #[tokio::test]
    async fn set_base_family_writes_the_base_family_and_its_source_not_family() {
        let db = Database::connect_in_memory().await.unwrap();
        let id = lora(&db).await;
        let out = db
            .models()
            .set_base_family(&id, "pony", FamilySource::Header)
            .await
            .unwrap();
        assert_eq!(out, SetFamily::Written);
        assert_eq!(
            family_of(&db, &id).await,
            (Some("pony".into()), Some("header".into()))
        );
    }

    #[tokio::test]
    async fn a_user_choice_is_never_overwritten_by_another_source() {
        let db = Database::connect_in_memory().await.unwrap();
        let id = lora(&db).await;
        db.models()
            .set_base_family(&id, "illustrious", FamilySource::User)
            .await
            .unwrap();
        for source in [
            FamilySource::Civitai,
            FamilySource::Hf,
            FamilySource::Catalog,
            FamilySource::Header,
            FamilySource::Name,
        ] {
            let out = db
                .models()
                .set_base_family(&id, "sdxl", source)
                .await
                .unwrap();
            assert_eq!(out, SetFamily::KeptUserChoice, "{source:?}");
            assert_eq!(
                family_of(&db, &id).await,
                (Some("illustrious".into()), Some("user".into())),
                "{source:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_user_can_change_their_own_choice() {
        let db = Database::connect_in_memory().await.unwrap();
        let id = lora(&db).await;
        db.models()
            .set_base_family(&id, "illustrious", FamilySource::User)
            .await
            .unwrap();
        let out = db
            .models()
            .set_base_family(&id, "noobai", FamilySource::User)
            .await
            .unwrap();
        assert_eq!(out, SetFamily::Written);
        assert_eq!(
            family_of(&db, &id).await,
            (Some("noobai".into()), Some("user".into()))
        );
    }

    #[tokio::test]
    async fn an_unknown_family_or_model_is_an_error() {
        let db = Database::connect_in_memory().await.unwrap();
        let id = lora(&db).await;
        assert!(db
            .models()
            .set_base_family(&id, "zimage", FamilySource::User)
            .await
            .is_err());
        assert_eq!(family_of(&db, &id).await, (None, None));
        assert!(db
            .models()
            .set_base_family("no-such-model", "sdxl", FamilySource::User)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn record_origin_fills_a_manual_source_and_the_base_family() {
        let db = Database::connect_in_memory().await.unwrap();
        let id = lora(&db).await;
        db.models()
            .record_origin(
                &id,
                Some("civitai:1/2"),
                Some(("pony", FamilySource::Civitai)),
            )
            .await
            .unwrap();
        let m = db.models().get(&id).await.unwrap().unwrap();
        assert_eq!(m.source, "civitai:1/2");
        assert_eq!(
            family_of(&db, &id).await,
            (Some("pony".into()), Some("civitai".into()))
        );
    }

    #[tokio::test]
    async fn record_origin_keeps_a_known_source_and_a_user_choice() {
        let db = Database::connect_in_memory().await.unwrap();
        let id = lora(&db).await;
        db.models()
            .set_family_and_source(&id, Some("sdxl"), "training:run-1")
            .await
            .unwrap();
        db.models()
            .set_base_family(&id, "illustrious", FamilySource::User)
            .await
            .unwrap();
        db.models()
            .record_origin(
                &id,
                Some("civitai:1/2"),
                Some(("pony", FamilySource::Civitai)),
            )
            .await
            .unwrap();
        let m = db.models().get(&id).await.unwrap().unwrap();
        assert_eq!(m.source, "training:run-1");
        assert_eq!(
            family_of(&db, &id).await,
            (Some("illustrious".into()), Some("user".into()))
        );
    }
}
