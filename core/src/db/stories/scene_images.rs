//! A Scene can have multiple generated images (regenerate / pick
//! alternates); exactly one is canonical at a time. Uniqueness of the
//! canonical flag is enforced by a partial unique index in the migration,
//! not application discipline alone -- these methods just keep the flag
//! consistent on the way there.

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::now_rfc3339;
use crate::{CoreError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct SceneImage {
    pub id: String,
    pub scene_id: String,
    /// A `job_type=image` job id.
    pub job_id: String,
    pub is_canonical: bool,
    pub created_at: String,
}

const SELECT_COLS: &str = "id, scene_id, job_id, is_canonical, created_at";

#[derive(Debug)]
pub struct SceneImageRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SceneImageRepo<'a> {
    pub(crate) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Attaches a finished (or in-flight) `image` job to a scene. The first
    /// image for a scene becomes canonical automatically; later ones start
    /// out as alternates until [`Self::set_canonical`] is called.
    pub async fn add(&self, scene_id: &str, job_id: &str) -> Result<SceneImage> {
        let is_first: bool = {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM scene_images WHERE scene_id = $1")
                    .bind(scene_id)
                    .fetch_one(self.pool)
                    .await?;
            count == 0
        };
        let id = Uuid::now_v7().to_string();
        let now = now_rfc3339();
        sqlx::query(
            "INSERT INTO scene_images (id, scene_id, job_id, is_canonical, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&id)
        .bind(scene_id)
        .bind(job_id)
        .bind(is_first)
        .bind(&now)
        .execute(self.pool)
        .await?;
        Ok(SceneImage {
            id,
            scene_id: scene_id.to_string(),
            job_id: job_id.to_string(),
            is_canonical: is_first,
            created_at: now,
        })
    }

    /// Every image for a scene, oldest first (the canonical one may be
    /// anywhere in the list -- check `is_canonical`, not position).
    pub async fn list_for_scene(&self, scene_id: &str) -> Result<Vec<SceneImage>> {
        let rows = sqlx::query_as::<_, SceneImage>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM scene_images WHERE scene_id = $1 ORDER BY created_at"
        )))
        .bind(scene_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    /// Makes `id` the scene's one canonical image, demoting whichever one
    /// held that spot before.
    pub async fn set_canonical(&self, id: &str) -> Result<()> {
        let img = self
            .get(id)
            .await?
            .ok_or_else(|| CoreError::Db(format!("no scene image {id}")))?;
        sqlx::query("UPDATE scene_images SET is_canonical = 0 WHERE scene_id = $1")
            .bind(&img.scene_id)
            .execute(self.pool)
            .await?;
        sqlx::query("UPDATE scene_images SET is_canonical = 1 WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Removes an image. If it was the canonical one and other images
    /// remain, the most recently added of those becomes canonical -- a
    /// scene with any images always has exactly one canonical.
    pub async fn delete(&self, id: &str) -> Result<()> {
        let img = self.get(id).await?;
        sqlx::query("DELETE FROM scene_images WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        let Some(img) = img else { return Ok(()) };
        if !img.is_canonical {
            return Ok(());
        }
        let next: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM scene_images WHERE scene_id = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&img.scene_id)
        .fetch_optional(self.pool)
        .await?;
        if let Some((next_id,)) = next {
            self.set_canonical(&next_id).await?;
        }
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<SceneImage>> {
        let row = sqlx::query_as::<_, SceneImage>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM scene_images WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::stories::{NewScene, StoryUpdate};
    use crate::db::{Database, NewJob};

    async fn setup() -> (Database, String) {
        let db = Database::connect_in_memory().await.unwrap();
        let story = db
            .stories()
            .create(StoryUpdate {
                name: "Test Story".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let scene = db
            .scenes()
            .create(NewScene {
                story_id: story.id.clone(),
                location_id: None,
                narrative: String::new(),
                redline: String::new(),
                participant_ids: vec![],
                dialogue: vec![],
            })
            .await
            .unwrap();
        (db, scene.id)
    }

    async fn new_job(db: &Database) -> String {
        db.jobs().insert(NewJob::new("image")).await.unwrap().id
    }

    #[tokio::test]
    async fn first_image_is_canonical_automatically() {
        let (db, scene_id) = setup().await;
        let job = new_job(&db).await;
        let img = db.scene_images().add(&scene_id, &job).await.unwrap();
        assert!(img.is_canonical);
    }

    #[tokio::test]
    async fn later_images_start_as_alternates() {
        let (db, scene_id) = setup().await;
        let job1 = new_job(&db).await;
        let job2 = new_job(&db).await;
        db.scene_images().add(&scene_id, &job1).await.unwrap();
        let second = db.scene_images().add(&scene_id, &job2).await.unwrap();
        assert!(!second.is_canonical);

        let all = db.scene_images().list_for_scene(&scene_id).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn set_canonical_demotes_the_previous_one() {
        let (db, scene_id) = setup().await;
        let job1 = new_job(&db).await;
        let job2 = new_job(&db).await;
        let first = db.scene_images().add(&scene_id, &job1).await.unwrap();
        let second = db.scene_images().add(&scene_id, &job2).await.unwrap();

        db.scene_images().set_canonical(&second.id).await.unwrap();

        let all = db.scene_images().list_for_scene(&scene_id).await.unwrap();
        let by_id = |id: &str| all.iter().find(|i| i.id == id).unwrap();
        assert!(!by_id(&first.id).is_canonical);
        assert!(by_id(&second.id).is_canonical);
    }

    #[tokio::test]
    async fn deleting_the_canonical_image_promotes_the_newest_remaining_one() {
        let (db, scene_id) = setup().await;
        let job1 = new_job(&db).await;
        let job2 = new_job(&db).await;
        let first = db.scene_images().add(&scene_id, &job1).await.unwrap();
        let second = db.scene_images().add(&scene_id, &job2).await.unwrap();

        db.scene_images().delete(&first.id).await.unwrap();

        let all = db.scene_images().list_for_scene(&scene_id).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, second.id);
        assert!(all[0].is_canonical);
    }

    #[tokio::test]
    async fn deleting_the_last_image_leaves_the_scene_with_none() {
        let (db, scene_id) = setup().await;
        let job1 = new_job(&db).await;
        let only = db.scene_images().add(&scene_id, &job1).await.unwrap();

        db.scene_images().delete(&only.id).await.unwrap();

        assert!(db
            .scene_images()
            .list_for_scene(&scene_id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn deleting_the_underlying_job_cascades_the_scene_image() {
        let (db, scene_id) = setup().await;
        let job1 = new_job(&db).await;
        db.scene_images().add(&scene_id, &job1).await.unwrap();

        db.jobs().delete(&job1).await.unwrap();

        assert!(db
            .scene_images()
            .list_for_scene(&scene_id)
            .await
            .unwrap()
            .is_empty());
    }
}
