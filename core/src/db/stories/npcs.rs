//! NPCs belong to a Story. Deliberately lightweight -- NOT the full
//! Character shape (Phase 1 scope cut, see docs/TODO.md "Story Studio").

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::now_rfc3339;
use crate::{CoreError, Result};

#[derive(Debug, Clone, Serialize)]
pub struct Npc {
    pub id: String,
    pub story_id: String,
    pub name: String,
    pub role: String,
    pub location_id: Option<String>,
    pub description: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct NpcRow {
    id: String,
    story_id: String,
    name: String,
    role: String,
    location_id: Option<String>,
    description: String,
    created_at: String,
}

impl From<NpcRow> for Npc {
    fn from(r: NpcRow) -> Self {
        Self {
            id: r.id,
            story_id: r.story_id,
            name: r.name,
            role: r.role,
            location_id: r.location_id,
            description: r.description,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewNpc {
    pub story_id: String,
    pub name: String,
    pub role: String,
    pub location_id: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, Default)]
pub struct NpcUpdate {
    pub name: String,
    pub role: String,
    pub location_id: Option<String>,
    pub description: String,
}

const SELECT_COLS: &str = "id, story_id, name, role, location_id, description, created_at";

#[derive(Debug)]
pub struct NpcRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> NpcRepo<'a> {
    pub(crate) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, body: NewNpc) -> Result<Npc> {
        let name = body.name.trim();
        if name.is_empty() {
            return Err(CoreError::Config("npc name must not be empty".into()));
        }
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO npcs (id, story_id, name, role, location_id, description, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&id)
        .bind(&body.story_id)
        .bind(name)
        .bind(&body.role)
        .bind(&body.location_id)
        .bind(&body.description)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("npc vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Npc>> {
        let row: Option<NpcRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM npcs WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Npc::from))
    }

    pub async fn list_for_story(&self, story_id: &str) -> Result<Vec<Npc>> {
        let rows: Vec<NpcRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM npcs WHERE story_id = $1 ORDER BY created_at"
        )))
        .bind(story_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Npc::from).collect())
    }

    pub async fn update(&self, id: &str, body: NpcUpdate) -> Result<()> {
        let name = body.name.trim();
        if name.is_empty() {
            return Err(CoreError::Config("npc name must not be empty".into()));
        }
        sqlx::query(
            "UPDATE npcs SET name = $1, role = $2, location_id = $3, description = $4 \
             WHERE id = $5",
        )
        .bind(name)
        .bind(&body.role)
        .bind(&body.location_id)
        .bind(&body.description)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM npcs WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::stories::StoryUpdate;
    use crate::db::Database;

    use super::{NewNpc, NpcUpdate};

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

    fn new_npc(story_id: &str, name: &str) -> NewNpc {
        NewNpc {
            story_id: story_id.to_string(),
            name: name.to_string(),
            role: "barkeep".into(),
            location_id: None,
            description: "knows everyone's business".into(),
        }
    }

    #[tokio::test]
    async fn create_and_get_round_trip() {
        let (db, story_id) = db_with_story().await;
        let npc = db
            .npcs()
            .create(new_npc(&story_id, "Old Tom"))
            .await
            .unwrap();
        assert_eq!(npc.name, "Old Tom");
        assert_eq!(npc.role, "barkeep");
        assert!(npc.location_id.is_none());
    }

    #[tokio::test]
    async fn create_rejects_a_blank_name() {
        let (db, story_id) = db_with_story().await;
        let err = db.npcs().create(new_npc(&story_id, "")).await.unwrap_err();
        assert!(matches!(err, crate::CoreError::Config(_)));
    }

    #[tokio::test]
    async fn location_reference_survives_the_locations_own_deletion_as_null() {
        let (db, story_id) = db_with_story().await;
        let loc = db
            .locations()
            .create(crate::db::stories::NewLocation {
                story_id: story_id.clone(),
                name: "The Anchor".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        let mut body = new_npc(&story_id, "Old Tom");
        body.location_id = Some(loc.id.clone());
        let npc = db.npcs().create(body).await.unwrap();
        assert_eq!(npc.location_id.as_deref(), Some(loc.id.as_str()));

        db.locations().delete(&loc.id).await.unwrap();

        let fetched = db.npcs().get(&npc.id).await.unwrap().unwrap();
        assert!(fetched.location_id.is_none(), "NPC must survive, unlocated");
    }

    #[tokio::test]
    async fn update_and_delete() {
        let (db, story_id) = db_with_story().await;
        let npc = db
            .npcs()
            .create(new_npc(&story_id, "Old Tom"))
            .await
            .unwrap();

        db.npcs()
            .update(
                &npc.id,
                NpcUpdate {
                    name: "Old Tom Reyes".into(),
                    role: "smuggler".into(),
                    location_id: None,
                    description: "not actually a barkeep".into(),
                },
            )
            .await
            .unwrap();
        let fetched = db.npcs().get(&npc.id).await.unwrap().unwrap();
        assert_eq!(fetched.role, "smuggler");

        db.npcs().delete(&npc.id).await.unwrap();
        assert!(db.npcs().get(&npc.id).await.unwrap().is_none());
    }
}
