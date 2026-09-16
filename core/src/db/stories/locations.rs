//! Locations belong to a Story. Like a Character's portrait, a Location's
//! reference image is just a plain `job_id` pointing at a `job_type=image`
//! job -- Phase 1 has no automatic consistency mechanism.

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::now_rfc3339;
use crate::{CoreError, Result};

#[derive(Debug, Clone, Serialize)]
pub struct Location {
    pub id: String,
    pub story_id: String,
    pub name: String,
    pub description: String,
    /// A `job_type=image` job id, or `None` until a reference is picked.
    pub reference_job_id: Option<String>,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct LocationRow {
    id: String,
    story_id: String,
    name: String,
    description: String,
    reference_job_id: Option<String>,
    created_at: String,
}

impl From<LocationRow> for Location {
    fn from(r: LocationRow) -> Self {
        Self {
            id: r.id,
            story_id: r.story_id,
            name: r.name,
            description: r.description,
            reference_job_id: r.reference_job_id,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewLocation {
    pub story_id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Default)]
pub struct LocationUpdate {
    pub name: String,
    pub description: String,
}

const SELECT_COLS: &str = "id, story_id, name, description, reference_job_id, created_at";

#[derive(Debug)]
pub struct LocationRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> LocationRepo<'a> {
    pub(crate) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, body: NewLocation) -> Result<Location> {
        let name = body.name.trim();
        if name.is_empty() {
            return Err(CoreError::Config("location name must not be empty".into()));
        }
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO locations (id, story_id, name, description, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&id)
        .bind(&body.story_id)
        .bind(name)
        .bind(&body.description)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("location vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Location>> {
        let row: Option<LocationRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM locations WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Location::from))
    }

    pub async fn list_for_story(&self, story_id: &str) -> Result<Vec<Location>> {
        let rows: Vec<LocationRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM locations WHERE story_id = $1 ORDER BY created_at"
        )))
        .bind(story_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Location::from).collect())
    }

    pub async fn update(&self, id: &str, body: LocationUpdate) -> Result<()> {
        let name = body.name.trim();
        if name.is_empty() {
            return Err(CoreError::Config("location name must not be empty".into()));
        }
        sqlx::query("UPDATE locations SET name = $1, description = $2 WHERE id = $3")
            .bind(name)
            .bind(&body.description)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM locations WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Pick (or clear, with `None`) this location's reference image.
    pub async fn set_reference(&self, id: &str, job_id: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE locations SET reference_job_id = $1 WHERE id = $2")
            .bind(job_id)
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

    use super::{LocationUpdate, NewLocation};

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

    fn new_loc(story_id: &str, name: &str) -> NewLocation {
        NewLocation {
            story_id: story_id.to_string(),
            name: name.to_string(),
            description: "a smoky dockside tavern".into(),
        }
    }

    #[tokio::test]
    async fn create_and_get_round_trip() {
        let (db, story_id) = db_with_story().await;
        let loc = db
            .locations()
            .create(new_loc(&story_id, "The Anchor"))
            .await
            .unwrap();
        assert_eq!(loc.name, "The Anchor");
        assert!(loc.reference_job_id.is_none());

        let fetched = db.locations().get(&loc.id).await.unwrap().unwrap();
        assert_eq!(fetched.description, "a smoky dockside tavern");
    }

    #[tokio::test]
    async fn create_rejects_a_blank_name() {
        let (db, story_id) = db_with_story().await;
        let err = db
            .locations()
            .create(new_loc(&story_id, " "))
            .await
            .unwrap_err();
        assert!(matches!(err, crate::CoreError::Config(_)));
    }

    #[tokio::test]
    async fn list_for_story_filters_correctly() {
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
        db.locations()
            .create(new_loc(&story_a, "Dock"))
            .await
            .unwrap();
        db.locations()
            .create(new_loc(&story_b, "Cave"))
            .await
            .unwrap();

        let locs = db.locations().list_for_story(&story_a).await.unwrap();
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].name, "Dock");
    }

    #[tokio::test]
    async fn update_and_set_reference() {
        let (db, story_id) = db_with_story().await;
        let loc = db
            .locations()
            .create(new_loc(&story_id, "Dock"))
            .await
            .unwrap();
        let job = db
            .jobs()
            .insert(crate::db::NewJob::new("image"))
            .await
            .unwrap();

        db.locations()
            .update(
                &loc.id,
                LocationUpdate {
                    name: "The Old Dock".into(),
                    description: "half-collapsed now".into(),
                },
            )
            .await
            .unwrap();
        db.locations()
            .set_reference(&loc.id, Some(&job.id))
            .await
            .unwrap();

        let fetched = db.locations().get(&loc.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "The Old Dock");
        assert_eq!(fetched.reference_job_id.as_deref(), Some(job.id.as_str()));
    }

    #[tokio::test]
    async fn deleting_the_referenced_job_clears_the_reference() {
        let (db, story_id) = db_with_story().await;
        let loc = db
            .locations()
            .create(new_loc(&story_id, "Dock"))
            .await
            .unwrap();
        let job = db
            .jobs()
            .insert(crate::db::NewJob::new("image"))
            .await
            .unwrap();
        db.locations()
            .set_reference(&loc.id, Some(&job.id))
            .await
            .unwrap();

        db.jobs().delete(&job.id).await.unwrap();

        let fetched = db.locations().get(&loc.id).await.unwrap().unwrap();
        assert!(
            fetched.reference_job_id.is_none(),
            "the location must survive, unreferenced"
        );
    }

    #[tokio::test]
    async fn delete_removes_the_location() {
        let (db, story_id) = db_with_story().await;
        let loc = db
            .locations()
            .create(new_loc(&story_id, "Dock"))
            .await
            .unwrap();
        db.locations().delete(&loc.id).await.unwrap();
        assert!(db.locations().get(&loc.id).await.unwrap().is_none());
    }
}
