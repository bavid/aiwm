//! Story Studio (Phase 1 -- docs/TODO.md "Story Studio"): a character-
//! consistent illustrated story/comic builder. Phase 1 is text-and-plain-
//! image only -- no character-consistency machinery yet (IP-Adapter is
//! Phase 2). This module owns `Story` itself; sibling modules own its child
//! entities (characters, npcs, locations, scenes, scene_images) -- one file
//! per entity, mirroring the rest of `db` (`sessions`, `documents`).

mod characters;
mod locations;
mod npcs;
mod scene_images;
mod scenes;

pub use characters::{
    Character, CharacterLogEntry, CharacterRelationship, CharacterRepo, CharacterUpdate,
    NewCharacter,
};
pub use locations::{Location, LocationRepo, LocationUpdate, NewLocation};
pub use npcs::{NewNpc, Npc, NpcRepo, NpcUpdate};
pub use scene_images::{SceneImage, SceneImageRepo};
pub use scenes::{DialogueLine, NewDialogueLine, NewScene, Scene, SceneRepo, SceneUpdate};

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// A stored Story: the top-level container for everything else in Story
/// Studio.
#[derive(Debug, Clone, Serialize)]
pub struct Story {
    pub id: String,
    pub name: String,
    /// Era/setting, freeform text.
    pub setting: String,
    /// Freeform text; feeds every generation prompt for this story's
    /// characters, locations, and scenes.
    pub art_style: String,
    pub premise: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct StoryRow {
    id: String,
    name: String,
    setting: String,
    art_style: String,
    premise: String,
    created_at: String,
}

impl From<StoryRow> for Story {
    fn from(r: StoryRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            setting: r.setting,
            art_style: r.art_style,
            premise: r.premise,
            created_at: r.created_at,
        }
    }
}

/// Body for [`StoryRepo::create`] / [`StoryRepo::update`] -- every editable
/// field, always supplied together (no partial-update semantics, matching
/// the rest of `db`).
#[derive(Debug, Clone, Default)]
pub struct StoryUpdate {
    pub name: String,
    pub setting: String,
    pub art_style: String,
    pub premise: String,
}

const SELECT_COLS: &str = "id, name, setting, art_style, premise, created_at";

#[derive(Debug)]
pub struct StoryRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> StoryRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, body: StoryUpdate) -> Result<Story> {
        let name = body.name.trim();
        if name.is_empty() {
            return Err(CoreError::Config("story name must not be empty".into()));
        }
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO stories (id, name, setting, art_style, premise, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(name)
        .bind(&body.setting)
        .bind(&body.art_style)
        .bind(&body.premise)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("story vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Story>> {
        let row = sqlx::query_as::<_, StoryRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM stories WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Story::from))
    }

    /// Every story, newest first.
    pub async fn list(&self) -> Result<Vec<Story>> {
        let rows = sqlx::query_as::<_, StoryRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM stories ORDER BY created_at DESC"
        )))
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Story::from).collect())
    }

    pub async fn update(&self, id: &str, body: StoryUpdate) -> Result<()> {
        let name = body.name.trim();
        if name.is_empty() {
            return Err(CoreError::Config("story name must not be empty".into()));
        }
        sqlx::query(
            "UPDATE stories SET name = $1, setting = $2, art_style = $3, premise = $4 \
             WHERE id = $5",
        )
        .bind(name)
        .bind(&body.setting)
        .bind(&body.art_style)
        .bind(&body.premise)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Deletes the story and cascades to every character, npc, location,
    /// scene (and the scene's own participants/dialogue/images/logs) --
    /// Story Studio's internal shape has no standalone value once its Story
    /// is gone (see the migration's own comment).
    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM stories WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    use super::StoryUpdate;

    fn body(name: &str) -> StoryUpdate {
        StoryUpdate {
            name: name.into(),
            setting: "1920s New Orleans".into(),
            art_style: "watercolor noir".into(),
            premise: "a jazz band stumbles onto a ghost story".into(),
        }
    }

    #[tokio::test]
    async fn create_and_get_round_trip() {
        let db = Database::connect_in_memory().await.unwrap();
        let s = db.stories().create(body("Midnight Revue")).await.unwrap();
        assert_eq!(s.name, "Midnight Revue");
        assert_eq!(s.art_style, "watercolor noir");

        let fetched = db.stories().get(&s.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, s.id);
        assert_eq!(fetched.premise, s.premise);
    }

    #[tokio::test]
    async fn create_rejects_a_blank_name() {
        let db = Database::connect_in_memory().await.unwrap();
        let err = db.stories().create(body("   ")).await.unwrap_err();
        assert!(matches!(err, crate::CoreError::Config(_)));
    }

    #[tokio::test]
    async fn list_orders_newest_first() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = db.stories().create(body("A")).await.unwrap();
        let b = db.stories().create(body("B")).await.unwrap();

        let all = db.stories().list().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, b.id);
        assert_eq!(all[1].id, a.id);
    }

    #[tokio::test]
    async fn update_replaces_every_field() {
        let db = Database::connect_in_memory().await.unwrap();
        let s = db.stories().create(body("Draft Title")).await.unwrap();

        db.stories()
            .update(
                &s.id,
                StoryUpdate {
                    name: "Final Title".into(),
                    setting: "far future Mars colony".into(),
                    art_style: "cel-shaded".into(),
                    premise: "the colony's air recycler starts talking back".into(),
                },
            )
            .await
            .unwrap();

        let fetched = db.stories().get(&s.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "Final Title");
        assert_eq!(fetched.setting, "far future Mars colony");
    }

    #[tokio::test]
    async fn update_rejects_a_blank_name() {
        let db = Database::connect_in_memory().await.unwrap();
        let s = db.stories().create(body("Keep me")).await.unwrap();
        let err = db.stories().update(&s.id, body("  ")).await.unwrap_err();
        assert!(matches!(err, crate::CoreError::Config(_)));
        // The original row is untouched.
        assert_eq!(
            db.stories().get(&s.id).await.unwrap().unwrap().name,
            "Keep me"
        );
    }

    #[tokio::test]
    async fn deleting_a_story_cascades_to_its_characters() {
        let db = Database::connect_in_memory().await.unwrap();
        let s = db.stories().create(body("Doomed")).await.unwrap();
        let c = db
            .characters()
            .create(super::NewCharacter {
                story_id: s.id.clone(),
                name: "Ada".into(),
                traits: String::new(),
                backstory: String::new(),
                alignment: String::new(),
            })
            .await
            .unwrap();

        db.stories().delete(&s.id).await.unwrap();

        assert!(db.stories().get(&s.id).await.unwrap().is_none());
        assert!(db.characters().get(&c.id).await.unwrap().is_none());
    }
}
