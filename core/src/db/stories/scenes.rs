//! Scenes belong to a Story. A Scene bundles narrative text, dialogue lines
//! (always rendered as UI overlay text -- never baked into a generated
//! image, a hard requirement), its participants, and an optional location.
//! Creating or editing a Scene's participant list auto-appends a Character
//! Log entry for every *newly added* participant (see `character_logs` in
//! the migration) -- re-saving a scene with the same participants does not
//! spam the log.

use std::collections::HashSet;

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::now_rfc3339;
use crate::Result;

#[derive(Debug, Clone, Serialize)]
pub struct Scene {
    pub id: String,
    pub story_id: String,
    pub location_id: Option<String>,
    pub narrative: String,
    /// The short scenario prompt that led to this scene, e.g. "5 travelers
    /// meet in a tavern, suddenly a shivering roar in the mountain".
    pub redline: String,
    /// Ordering within the story's timeline; assigned on creation, stable
    /// across edits (Phase 1 has no manual reorder).
    pub position: i64,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct SceneRow {
    id: String,
    story_id: String,
    location_id: Option<String>,
    narrative: String,
    redline: String,
    position: i64,
    created_at: String,
}

impl From<SceneRow> for Scene {
    fn from(r: SceneRow) -> Self {
        Self {
            id: r.id,
            story_id: r.story_id,
            location_id: r.location_id,
            narrative: r.narrative,
            redline: r.redline,
            position: r.position,
            created_at: r.created_at,
        }
    }
}

/// One dialogue line as stored -- always UI overlay text, never composited
/// into the generated image.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct DialogueLine {
    pub id: String,
    pub scene_id: String,
    pub character_id: String,
    pub position: i64,
    pub text: String,
}

/// A dialogue line as supplied by a caller (no id/scene_id/position yet --
/// position is assigned from list order).
#[derive(Debug, Clone)]
pub struct NewDialogueLine {
    pub character_id: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct NewScene {
    pub story_id: String,
    pub location_id: Option<String>,
    pub narrative: String,
    pub redline: String,
    pub participant_ids: Vec<String>,
    pub dialogue: Vec<NewDialogueLine>,
}

/// Body for [`SceneRepo::update`] -- full replace of every editable field,
/// including participants and dialogue (matching the rest of `db`'s
/// no-partial-update convention). `position` is never editable here.
#[derive(Debug, Clone, Default)]
pub struct SceneUpdate {
    pub location_id: Option<String>,
    pub narrative: String,
    pub redline: String,
    pub participant_ids: Vec<String>,
    pub dialogue: Vec<NewDialogueLine>,
}

const SELECT_COLS: &str = "id, story_id, location_id, narrative, redline, position, created_at";

/// The Character Log text auto-written for a participant of this scene.
fn auto_log_text(redline: &str, narrative: &str) -> String {
    let redline = redline.trim();
    if !redline.is_empty() {
        return format!("Appeared in a scene: {redline}");
    }
    let narrative = narrative.trim();
    if narrative.is_empty() {
        return "Appeared in a scene.".to_string();
    }
    const MAX_CHARS: usize = 140;
    let truncated = narrative.chars().count() > MAX_CHARS;
    let snippet: String = narrative.chars().take(MAX_CHARS).collect();
    format!(
        "Appeared in a scene: {snippet}{}",
        if truncated { "\u{2026}" } else { "" }
    )
}

/// De-duplicates while preserving first-seen order (a caller sending the
/// same character id twice must not trip the `scene_participants` PK, or
/// double-log the character).
fn dedup(ids: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert((*id).clone()))
        .cloned()
        .collect()
}

#[derive(Debug)]
pub struct SceneRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SceneRepo<'a> {
    pub(crate) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, body: NewScene) -> Result<Scene> {
        let id = Uuid::now_v7().to_string();
        let now = now_rfc3339();
        let next_position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM scenes WHERE story_id = $1",
        )
        .bind(&body.story_id)
        .fetch_one(self.pool)
        .await?;

        sqlx::query(
            "INSERT INTO scenes (id, story_id, location_id, narrative, redline, position, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&id)
        .bind(&body.story_id)
        .bind(&body.location_id)
        .bind(&body.narrative)
        .bind(&body.redline)
        .bind(next_position)
        .bind(&now)
        .execute(self.pool)
        .await?;

        let participants = dedup(&body.participant_ids);
        for character_id in &participants {
            self.insert_participant(&id, character_id).await?;
            self.append_log(
                character_id,
                &id,
                &auto_log_text(&body.redline, &body.narrative),
            )
            .await?;
        }
        for (position, line) in body.dialogue.iter().enumerate() {
            self.insert_dialogue_line(&id, position, line).await?;
        }

        self.get(&id)
            .await?
            .ok_or_else(|| crate::CoreError::Db("scene vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Scene>> {
        let row: Option<SceneRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM scenes WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Scene::from))
    }

    /// Every scene in a story, in timeline order.
    pub async fn list_for_story(&self, story_id: &str) -> Result<Vec<Scene>> {
        let rows: Vec<SceneRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM scenes WHERE story_id = $1 ORDER BY position"
        )))
        .bind(story_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Scene::from).collect())
    }

    /// This scene's participant character ids, in the order they were added.
    pub async fn participants(&self, scene_id: &str) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT character_id FROM scene_participants WHERE scene_id = $1 ORDER BY rowid",
        )
        .bind(scene_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// This scene's dialogue lines, in display order.
    pub async fn dialogue(&self, scene_id: &str) -> Result<Vec<DialogueLine>> {
        let rows = sqlx::query_as::<_, DialogueLine>(
            "SELECT id, scene_id, character_id, position, text FROM scene_dialogue_lines \
             WHERE scene_id = $1 ORDER BY position",
        )
        .bind(scene_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    /// Full replace of a scene's editable fields, participants included.
    /// Character Log entries are appended only for participants that were
    /// not already on this scene.
    pub async fn update(&self, id: &str, body: SceneUpdate) -> Result<()> {
        sqlx::query(
            "UPDATE scenes SET location_id = $1, narrative = $2, redline = $3 WHERE id = $4",
        )
        .bind(&body.location_id)
        .bind(&body.narrative)
        .bind(&body.redline)
        .bind(id)
        .execute(self.pool)
        .await?;

        let before: HashSet<String> = self.participants(id).await?.into_iter().collect();
        sqlx::query("DELETE FROM scene_participants WHERE scene_id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        let participants = dedup(&body.participant_ids);
        for character_id in &participants {
            self.insert_participant(id, character_id).await?;
            if !before.contains(character_id) {
                self.append_log(
                    character_id,
                    id,
                    &auto_log_text(&body.redline, &body.narrative),
                )
                .await?;
            }
        }

        sqlx::query("DELETE FROM scene_dialogue_lines WHERE scene_id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        for (position, line) in body.dialogue.iter().enumerate() {
            self.insert_dialogue_line(id, position, line).await?;
        }

        Ok(())
    }

    /// Deletes the scene, cascading to its participants, dialogue, images,
    /// and Character Log entries.
    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM scenes WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    async fn insert_participant(&self, scene_id: &str, character_id: &str) -> Result<()> {
        sqlx::query("INSERT INTO scene_participants (scene_id, character_id) VALUES ($1, $2)")
            .bind(scene_id)
            .bind(character_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    async fn insert_dialogue_line(
        &self,
        scene_id: &str,
        position: usize,
        line: &NewDialogueLine,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO scene_dialogue_lines (id, scene_id, character_id, position, text) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(scene_id)
        .bind(&line.character_id)
        .bind(i64::try_from(position).unwrap_or(i64::MAX))
        .bind(&line.text)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    async fn append_log(&self, character_id: &str, scene_id: &str, text: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO character_logs (id, character_id, scene_id, created_at, text) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(character_id)
        .bind(scene_id)
        .bind(now_rfc3339())
        .bind(text)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::stories::{NewCharacter, StoryUpdate};
    use crate::db::Database;

    use super::{NewDialogueLine, NewScene, SceneUpdate};

    async fn setup() -> (Database, String, String, String) {
        let db = Database::connect_in_memory().await.unwrap();
        let story = db
            .stories()
            .create(StoryUpdate {
                name: "Test Story".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let a = db
            .characters()
            .create(NewCharacter {
                story_id: story.id.clone(),
                name: "Ada".into(),
                traits: String::new(),
                backstory: String::new(),
                alignment: String::new(),
            })
            .await
            .unwrap();
        let b = db
            .characters()
            .create(NewCharacter {
                story_id: story.id.clone(),
                name: "Bea".into(),
                traits: String::new(),
                backstory: String::new(),
                alignment: String::new(),
            })
            .await
            .unwrap();
        (db, story.id, a.id, b.id)
    }

    fn scene(story_id: &str, participants: Vec<String>) -> NewScene {
        NewScene {
            story_id: story_id.to_string(),
            location_id: None,
            narrative: "The travelers gather by the fire.".into(),
            redline: "5 travelers meet in a tavern".into(),
            participant_ids: participants,
            dialogue: vec![],
        }
    }

    #[tokio::test]
    async fn create_assigns_increasing_positions() {
        let (db, story_id, _a, _b) = setup().await;
        let s1 = db.scenes().create(scene(&story_id, vec![])).await.unwrap();
        let s2 = db.scenes().create(scene(&story_id, vec![])).await.unwrap();
        assert_eq!(s1.position, 0);
        assert_eq!(s2.position, 1);

        let all = db.scenes().list_for_story(&story_id).await.unwrap();
        assert_eq!(
            all.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
            vec![s1.id, s2.id]
        );
    }

    #[tokio::test]
    async fn create_stores_participants_dialogue_and_auto_appends_logs() {
        let (db, story_id, a, b) = setup().await;
        let mut new_scene = scene(&story_id, vec![a.clone(), b.clone()]);
        new_scene.dialogue = vec![
            NewDialogueLine {
                character_id: a.clone(),
                text: "Cold tonight.".into(),
            },
            NewDialogueLine {
                character_id: b.clone(),
                text: "Aye, it is.".into(),
            },
        ];
        let s = db.scenes().create(new_scene).await.unwrap();

        let participants = db.scenes().participants(&s.id).await.unwrap();
        assert_eq!(participants, vec![a.clone(), b.clone()]);

        let dialogue = db.scenes().dialogue(&s.id).await.unwrap();
        assert_eq!(dialogue.len(), 2);
        assert_eq!(dialogue[0].text, "Cold tonight.");
        assert_eq!(dialogue[1].character_id, b);

        let log_a = db.characters().logs_for(&a).await.unwrap();
        assert_eq!(log_a.len(), 1);
        assert_eq!(log_a[0].scene_id, s.id);
        assert!(log_a[0].text.contains("5 travelers meet in a tavern"));

        let log_b = db.characters().logs_for(&b).await.unwrap();
        assert_eq!(log_b.len(), 1);
    }

    #[tokio::test]
    async fn create_deduplicates_repeated_participant_ids() {
        let (db, story_id, a, _b) = setup().await;
        let s = db
            .scenes()
            .create(scene(&story_id, vec![a.clone(), a.clone()]))
            .await
            .unwrap();

        assert_eq!(
            db.scenes().participants(&s.id).await.unwrap(),
            vec![a.clone()]
        );
        assert_eq!(db.characters().logs_for(&a).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn update_only_logs_newly_added_participants() {
        let (db, story_id, a, b) = setup().await;
        let s = db
            .scenes()
            .create(scene(&story_id, vec![a.clone()]))
            .await
            .unwrap();
        assert_eq!(db.characters().logs_for(&a).await.unwrap().len(), 1);

        db.scenes()
            .update(
                &s.id,
                SceneUpdate {
                    location_id: None,
                    narrative: "Later, a second traveler joins.".into(),
                    redline: "a stranger joins the fire".into(),
                    participant_ids: vec![a.clone(), b.clone()],
                    dialogue: vec![],
                },
            )
            .await
            .unwrap();

        // `a` was already a participant -- no second log entry for the same
        // scene re-save.
        assert_eq!(db.characters().logs_for(&a).await.unwrap().len(), 1);
        // `b` is new -- exactly one log entry.
        assert_eq!(db.characters().logs_for(&b).await.unwrap().len(), 1);

        let fetched = db.scenes().get(&s.id).await.unwrap().unwrap();
        assert_eq!(fetched.narrative, "Later, a second traveler joins.");
        assert_eq!(
            db.scenes().participants(&s.id).await.unwrap(),
            vec![a.clone(), b.clone()]
        );
    }

    #[tokio::test]
    async fn update_replaces_dialogue_wholesale() {
        let (db, story_id, a, _b) = setup().await;
        let mut new_scene = scene(&story_id, vec![a.clone()]);
        new_scene.dialogue = vec![NewDialogueLine {
            character_id: a.clone(),
            text: "old line".into(),
        }];
        let s = db.scenes().create(new_scene).await.unwrap();

        db.scenes()
            .update(
                &s.id,
                SceneUpdate {
                    location_id: None,
                    narrative: "n".into(),
                    redline: "r".into(),
                    participant_ids: vec![a.clone()],
                    dialogue: vec![NewDialogueLine {
                        character_id: a.clone(),
                        text: "new line".into(),
                    }],
                },
            )
            .await
            .unwrap();

        let dialogue = db.scenes().dialogue(&s.id).await.unwrap();
        assert_eq!(dialogue.len(), 1);
        assert_eq!(dialogue[0].text, "new line");
    }

    #[tokio::test]
    async fn delete_cascades_participants_and_dialogue() {
        let (db, story_id, a, _b) = setup().await;
        let mut new_scene = scene(&story_id, vec![a.clone()]);
        new_scene.dialogue = vec![NewDialogueLine {
            character_id: a.clone(),
            text: "line".into(),
        }];
        let s = db.scenes().create(new_scene).await.unwrap();

        db.scenes().delete(&s.id).await.unwrap();

        assert!(db.scenes().get(&s.id).await.unwrap().is_none());
        assert!(db.scenes().participants(&s.id).await.unwrap().is_empty());
        assert!(db.scenes().dialogue(&s.id).await.unwrap().is_empty());
        // The Character Log entry describing the now-gone scene goes with it.
        assert!(db.characters().logs_for(&a).await.unwrap().is_empty());
    }
}
