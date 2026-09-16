//! Characters belong to a Story. Phase 1 stores a portrait as a plain
//! `job_id` pointing at a `job_type=image` job -- no automatic
//! consistency yet (Phase 2, IP-Adapter). Inventory is a small freeform
//! list of strings (JSON column, never queried structurally -- YAGNI).
//! Relationships are normalized into their own table (see the migration's
//! comment) so a later "who talked with whom" graph never needs a
//! restructure.

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::now_rfc3339;
use crate::{CoreError, Result};

#[derive(Debug, Clone, Serialize)]
pub struct Character {
    pub id: String,
    pub story_id: String,
    pub name: String,
    pub traits: String,
    pub backstory: String,
    /// Role/alignment hint, freeform, e.g. "elf, friendly, skilled".
    pub alignment: String,
    /// A `job_type=image` job id, or `None` until a portrait is picked.
    pub portrait_job_id: Option<String>,
    pub inventory: Vec<String>,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct CharacterRow {
    id: String,
    story_id: String,
    name: String,
    traits: String,
    backstory: String,
    alignment: String,
    portrait_job_id: Option<String>,
    inventory: String,
    created_at: String,
}

impl TryFrom<CharacterRow> for Character {
    type Error = CoreError;

    fn try_from(r: CharacterRow) -> Result<Self> {
        Ok(Self {
            id: r.id,
            story_id: r.story_id,
            name: r.name,
            traits: r.traits,
            backstory: r.backstory,
            alignment: r.alignment,
            portrait_job_id: r.portrait_job_id,
            inventory: parse_inventory(&r.inventory)?,
            created_at: r.created_at,
        })
    }
}

fn parse_inventory(json: &str) -> Result<Vec<String>> {
    serde_json::from_str(json).map_err(|e| CoreError::Db(format!("bad inventory json: {e}")))
}

fn encode_inventory(items: &[String]) -> Result<String> {
    serde_json::to_string(items).map_err(|e| CoreError::Db(format!("serialize inventory: {e}")))
}

/// Body for [`CharacterRepo::create`].
#[derive(Debug, Clone)]
pub struct NewCharacter {
    pub story_id: String,
    pub name: String,
    pub traits: String,
    pub backstory: String,
    pub alignment: String,
}

/// Body for [`CharacterRepo::update`] -- text fields only; portrait and
/// inventory have their own dedicated setters.
#[derive(Debug, Clone, Default)]
pub struct CharacterUpdate {
    pub name: String,
    pub traits: String,
    pub backstory: String,
    pub alignment: String,
}

/// One character's relationship to another, from this character's point of
/// view.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct CharacterRelationship {
    pub id: String,
    pub character_id: String,
    pub related_character_id: String,
    pub note: String,
    pub created_at: String,
}

/// One append-only Character Log entry, auto-written when the character
/// participates in a new Scene (see `SceneRepo`).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct CharacterLogEntry {
    pub id: String,
    pub character_id: String,
    pub scene_id: String,
    pub created_at: String,
    pub text: String,
}

const SELECT_COLS: &str =
    "id, story_id, name, traits, backstory, alignment, portrait_job_id, inventory, created_at";

#[derive(Debug)]
pub struct CharacterRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> CharacterRepo<'a> {
    pub(crate) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, body: NewCharacter) -> Result<Character> {
        let name = body.name.trim();
        if name.is_empty() {
            return Err(CoreError::Config("character name must not be empty".into()));
        }
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO characters \
             (id, story_id, name, traits, backstory, alignment, inventory, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, '[]', $7)",
        )
        .bind(&id)
        .bind(&body.story_id)
        .bind(name)
        .bind(&body.traits)
        .bind(&body.backstory)
        .bind(&body.alignment)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("character vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Character>> {
        let row: Option<CharacterRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM characters WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        row.map(Character::try_from).transpose()
    }

    /// Every character in a story, oldest first (roughly creation order --
    /// there is no separate manual ordering for the character roster).
    pub async fn list_for_story(&self, story_id: &str) -> Result<Vec<Character>> {
        let rows: Vec<CharacterRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM characters WHERE story_id = $1 ORDER BY created_at"
        )))
        .bind(story_id)
        .fetch_all(self.pool)
        .await?;
        rows.into_iter().map(Character::try_from).collect()
    }

    pub async fn update(&self, id: &str, body: CharacterUpdate) -> Result<()> {
        let name = body.name.trim();
        if name.is_empty() {
            return Err(CoreError::Config("character name must not be empty".into()));
        }
        sqlx::query(
            "UPDATE characters SET name = $1, traits = $2, backstory = $3, alignment = $4 \
             WHERE id = $5",
        )
        .bind(name)
        .bind(&body.traits)
        .bind(&body.backstory)
        .bind(&body.alignment)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM characters WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Pick (or clear, with `None`) this character's reference portrait --
    /// a `job_type=image` job already submitted through the ordinary Image
    /// capability.
    pub async fn set_portrait(&self, id: &str, job_id: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE characters SET portrait_job_id = $1 WHERE id = $2")
            .bind(job_id)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Replaces the whole inventory list.
    pub async fn set_inventory(&self, id: &str, items: &[String]) -> Result<()> {
        let json = encode_inventory(items)?;
        sqlx::query("UPDATE characters SET inventory = $1 WHERE id = $2")
            .bind(&json)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_relationship(
        &self,
        character_id: &str,
        related_character_id: &str,
        note: &str,
    ) -> Result<CharacterRelationship> {
        let id = Uuid::now_v7().to_string();
        let now = now_rfc3339();
        sqlx::query(
            "INSERT INTO character_relationships \
             (id, character_id, related_character_id, note, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&id)
        .bind(character_id)
        .bind(related_character_id)
        .bind(note)
        .bind(&now)
        .execute(self.pool)
        .await?;
        Ok(CharacterRelationship {
            id,
            character_id: character_id.to_string(),
            related_character_id: related_character_id.to_string(),
            note: note.to_string(),
            created_at: now,
        })
    }

    /// Every relationship `character_id` has declared, oldest first.
    pub async fn list_relationships(
        &self,
        character_id: &str,
    ) -> Result<Vec<CharacterRelationship>> {
        let rows = sqlx::query_as::<_, CharacterRelationship>(
            "SELECT id, character_id, related_character_id, note, created_at \
             FROM character_relationships WHERE character_id = $1 ORDER BY created_at",
        )
        .bind(character_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn remove_relationship(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM character_relationships WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// This character's append-only log, oldest first.
    pub async fn logs_for(&self, character_id: &str) -> Result<Vec<CharacterLogEntry>> {
        let rows = sqlx::query_as::<_, CharacterLogEntry>(
            "SELECT id, character_id, scene_id, created_at, text FROM character_logs \
             WHERE character_id = $1 ORDER BY created_at",
        )
        .bind(character_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::stories::StoryUpdate;
    use crate::db::Database;

    use super::{CharacterUpdate, NewCharacter};

    async fn db_with_story() -> (Database, String) {
        let db = Database::connect_in_memory().await.unwrap();
        let s = db
            .stories()
            .create(StoryUpdate {
                name: "Test Story".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        (db, s.id)
    }

    fn new_char(story_id: &str, name: &str) -> NewCharacter {
        NewCharacter {
            story_id: story_id.to_string(),
            name: name.to_string(),
            traits: "brave, curious".into(),
            backstory: "orphaned young, raised by sailors".into(),
            alignment: "human, friendly, skilled".into(),
        }
    }

    #[tokio::test]
    async fn create_and_get_round_trip() {
        let (db, story_id) = db_with_story().await;
        let c = db
            .characters()
            .create(new_char(&story_id, "Mira"))
            .await
            .unwrap();
        assert_eq!(c.name, "Mira");
        assert_eq!(c.story_id, story_id);
        assert!(c.inventory.is_empty());
        assert!(c.portrait_job_id.is_none());

        let fetched = db.characters().get(&c.id).await.unwrap().unwrap();
        assert_eq!(fetched.traits, "brave, curious");
    }

    #[tokio::test]
    async fn create_rejects_a_blank_name() {
        let (db, story_id) = db_with_story().await;
        let err = db
            .characters()
            .create(new_char(&story_id, "  "))
            .await
            .unwrap_err();
        assert!(matches!(err, crate::CoreError::Config(_)));
    }

    #[tokio::test]
    async fn list_for_story_only_returns_that_storys_characters() {
        let (db, story_a) = db_with_story().await;
        let story_b = db
            .stories()
            .create(StoryUpdate {
                name: "Other".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .id;

        db.characters()
            .create(new_char(&story_a, "Ada"))
            .await
            .unwrap();
        db.characters()
            .create(new_char(&story_b, "Bea"))
            .await
            .unwrap();

        let a_chars = db.characters().list_for_story(&story_a).await.unwrap();
        assert_eq!(a_chars.len(), 1);
        assert_eq!(a_chars[0].name, "Ada");
    }

    #[tokio::test]
    async fn update_replaces_the_text_fields() {
        let (db, story_id) = db_with_story().await;
        let c = db
            .characters()
            .create(new_char(&story_id, "Mira"))
            .await
            .unwrap();

        db.characters()
            .update(
                &c.id,
                CharacterUpdate {
                    name: "Mira Voss".into(),
                    traits: "cautious now".into(),
                    backstory: "survived the wreck".into(),
                    alignment: "human, wary, skilled".into(),
                },
            )
            .await
            .unwrap();

        let fetched = db.characters().get(&c.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "Mira Voss");
        assert_eq!(fetched.traits, "cautious now");
    }

    #[tokio::test]
    async fn set_portrait_then_clear() {
        let (db, story_id) = db_with_story().await;
        let c = db
            .characters()
            .create(new_char(&story_id, "Mira"))
            .await
            .unwrap();
        let job = db
            .jobs()
            .insert(crate::db::NewJob::new("image"))
            .await
            .unwrap();

        db.characters()
            .set_portrait(&c.id, Some(&job.id))
            .await
            .unwrap();
        assert_eq!(
            db.characters()
                .get(&c.id)
                .await
                .unwrap()
                .unwrap()
                .portrait_job_id
                .as_deref(),
            Some(job.id.as_str())
        );

        db.characters().set_portrait(&c.id, None).await.unwrap();
        assert!(db
            .characters()
            .get(&c.id)
            .await
            .unwrap()
            .unwrap()
            .portrait_job_id
            .is_none());
    }

    #[tokio::test]
    async fn deleting_the_referenced_job_clears_the_portrait() {
        let (db, story_id) = db_with_story().await;
        let c = db
            .characters()
            .create(new_char(&story_id, "Mira"))
            .await
            .unwrap();
        let job = db
            .jobs()
            .insert(crate::db::NewJob::new("image"))
            .await
            .unwrap();
        db.characters()
            .set_portrait(&c.id, Some(&job.id))
            .await
            .unwrap();

        db.jobs().delete(&job.id).await.unwrap();

        let fetched = db.characters().get(&c.id).await.unwrap().unwrap();
        assert!(
            fetched.portrait_job_id.is_none(),
            "the character must survive, unportraited"
        );
    }

    #[tokio::test]
    async fn set_inventory_replaces_the_whole_list() {
        let (db, story_id) = db_with_story().await;
        let c = db
            .characters()
            .create(new_char(&story_id, "Mira"))
            .await
            .unwrap();

        db.characters()
            .set_inventory(&c.id, &["rope".into(), "lantern".into()])
            .await
            .unwrap();
        let fetched = db.characters().get(&c.id).await.unwrap().unwrap();
        assert_eq!(fetched.inventory, vec!["rope", "lantern"]);

        db.characters()
            .set_inventory(&c.id, &["map".into()])
            .await
            .unwrap();
        let fetched = db.characters().get(&c.id).await.unwrap().unwrap();
        assert_eq!(fetched.inventory, vec!["map"]);
    }

    #[tokio::test]
    async fn relationships_round_trip_and_remove() {
        let (db, story_id) = db_with_story().await;
        let a = db
            .characters()
            .create(new_char(&story_id, "Ada"))
            .await
            .unwrap();
        let b = db
            .characters()
            .create(new_char(&story_id, "Bea"))
            .await
            .unwrap();

        let rel = db
            .characters()
            .add_relationship(&a.id, &b.id, "trusts her with her life")
            .await
            .unwrap();

        let list = db.characters().list_relationships(&a.id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].related_character_id, b.id);

        db.characters().remove_relationship(&rel.id).await.unwrap();
        assert!(db
            .characters()
            .list_relationships(&a.id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn deleting_a_character_cascades_its_relationships() {
        let (db, story_id) = db_with_story().await;
        let a = db
            .characters()
            .create(new_char(&story_id, "Ada"))
            .await
            .unwrap();
        let b = db
            .characters()
            .create(new_char(&story_id, "Bea"))
            .await
            .unwrap();
        db.characters()
            .add_relationship(&a.id, &b.id, "note")
            .await
            .unwrap();

        db.characters().delete(&b.id).await.unwrap();

        assert!(db
            .characters()
            .list_relationships(&a.id)
            .await
            .unwrap()
            .is_empty());
    }
}
