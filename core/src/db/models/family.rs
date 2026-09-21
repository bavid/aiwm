//! Persisting a model's base family together with how it was decided.

use serde::Serialize;

use super::ModelRepo;
use crate::model::family::{family_by_id, FamilySource};
use crate::{CoreError, Result};

/// What [`ModelRepo::set_family`] did.
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
    /// and the `source` that decided it. A family the user picked
    /// (`family_source = 'user'`) is only ever replaced by another user
    /// choice; anything else reports [`SetFamily::KeptUserChoice`].
    pub async fn set_family(
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
            "UPDATE models SET family = $1, family_source = $2
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

    async fn family_of(db: &Database, id: &str) -> (Option<String>, Option<String>) {
        let m = db.models().get(id).await.unwrap().unwrap();
        (m.family, m.family_source)
    }

    #[tokio::test]
    async fn a_new_row_has_no_family_source() {
        let db = Database::connect_in_memory().await.unwrap();
        let id = lora(&db).await;
        assert_eq!(family_of(&db, &id).await, (Some("sdxl".into()), None));
    }

    #[tokio::test]
    async fn set_family_writes_the_family_and_its_source() {
        let db = Database::connect_in_memory().await.unwrap();
        let id = lora(&db).await;
        let out = db
            .models()
            .set_family(&id, "pony", FamilySource::Header)
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
            .set_family(&id, "illustrious", FamilySource::User)
            .await
            .unwrap();
        for source in [
            FamilySource::Civitai,
            FamilySource::Hf,
            FamilySource::Catalog,
            FamilySource::Header,
            FamilySource::Name,
        ] {
            let out = db.models().set_family(&id, "sdxl", source).await.unwrap();
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
            .set_family(&id, "illustrious", FamilySource::User)
            .await
            .unwrap();
        let out = db
            .models()
            .set_family(&id, "noobai", FamilySource::User)
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
            .set_family(&id, "krea2", FamilySource::User)
            .await
            .is_err());
        assert_eq!(family_of(&db, &id).await, (Some("sdxl".into()), None));
        assert!(db
            .models()
            .set_family("no-such-model", "sdxl", FamilySource::User)
            .await
            .is_err());
    }
}
