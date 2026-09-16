//! Frame persistence for a `dataset_prep` job (see
//! [`crate::capability::dataset`]). Cascade-deletes with its job — see the
//! `0014_dataset_frames.sql` migration's own doc comment.

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::Result;

/// One kept frame, ready for curation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DatasetFrame {
    pub id: String,
    pub job_id: String,
    pub tag: String,
    pub source_path: String,
    pub frame_path: String,
    pub timestamp_secs: Option<f64>,
    pub caption: String,
    /// `"florence2"` | `"qwen2.5-vl"` | `""` (not captioned yet).
    pub caption_engine: String,
    pub excluded: bool,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct DatasetFrameRow {
    id: String,
    job_id: String,
    tag: String,
    source_path: String,
    frame_path: String,
    timestamp_secs: Option<f64>,
    caption: String,
    caption_engine: String,
    excluded: i64,
    created_at: String,
}

impl From<DatasetFrameRow> for DatasetFrame {
    fn from(r: DatasetFrameRow) -> Self {
        Self {
            id: r.id,
            job_id: r.job_id,
            tag: r.tag,
            source_path: r.source_path,
            frame_path: r.frame_path,
            timestamp_secs: r.timestamp_secs,
            caption: r.caption,
            caption_engine: r.caption_engine,
            excluded: r.excluded != 0,
            created_at: r.created_at,
        }
    }
}

/// Body for [`DatasetFrameRepo::insert`] — everything decided at extraction/
/// filtering time, before a caption exists.
#[derive(Debug, Clone)]
pub struct NewDatasetFrame {
    pub job_id: String,
    pub tag: String,
    pub source_path: String,
    pub frame_path: String,
    pub timestamp_secs: Option<f64>,
}

const SELECT_COLS: &str = "id, job_id, tag, source_path, frame_path, timestamp_secs, \
     caption, caption_engine, excluded, created_at";

#[derive(Debug)]
pub struct DatasetFrameRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> DatasetFrameRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, f: NewDatasetFrame) -> Result<DatasetFrame> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO dataset_frames \
             (id, job_id, tag, source_path, frame_path, timestamp_secs, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&id)
        .bind(&f.job_id)
        .bind(&f.tag)
        .bind(&f.source_path)
        .bind(&f.frame_path)
        .bind(f.timestamp_secs)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;

        self.get(&id)
            .await?
            .ok_or_else(|| crate::CoreError::Db("dataset frame vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<DatasetFrame>> {
        let row = sqlx::query_as::<_, DatasetFrameRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM dataset_frames WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(DatasetFrame::from))
    }

    /// Every frame produced by `job_id`, insertion order (matches extraction/
    /// filtering order, which is itself tag-then-source-then-time order).
    pub async fn list_for_job(&self, job_id: &str) -> Result<Vec<DatasetFrame>> {
        let rows = sqlx::query_as::<_, DatasetFrameRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM dataset_frames WHERE job_id = $1 ORDER BY created_at, id"
        )))
        .bind(job_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(DatasetFrame::from).collect())
    }

    /// Record a caption (auto-generated, or a curator's edit) and which
    /// engine produced it. Passing `engine = ""` is how a curator's manual
    /// edit is recorded — same column, empty engine meaning "hand-written".
    pub async fn set_caption(&self, id: &str, caption: &str, engine: &str) -> Result<()> {
        sqlx::query("UPDATE dataset_frames SET caption = $1, caption_engine = $2 WHERE id = $3")
            .bind(caption)
            .bind(engine)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Curator include/exclude toggle — an excluded frame is skipped by
    /// [`super::super::capability::dataset::export_dataset`] but stays in the
    /// review grid.
    pub async fn set_excluded(&self, id: &str, excluded: bool) -> Result<()> {
        sqlx::query("UPDATE dataset_frames SET excluded = $1 WHERE id = $2")
            .bind(excluded)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewJob};

    async fn db_with_job() -> (Database, String) {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db.jobs().insert(NewJob::new("dataset_prep")).await.unwrap();
        (db, job.id)
    }

    fn new_frame(job_id: &str, tag: &str) -> NewDatasetFrame {
        NewDatasetFrame {
            job_id: job_id.to_string(),
            tag: tag.to_string(),
            source_path: "E:\\Data\\Ghibli\\clip.mp4".into(),
            frame_path: "C:\\datasets\\job-1\\0001.png".into(),
            timestamp_secs: Some(1.5),
        }
    }

    #[tokio::test]
    async fn insert_stores_defaults_for_caption_and_excluded() {
        let (db, job_id) = db_with_job().await;
        let frame = db
            .dataset_frames()
            .insert(new_frame(&job_id, "Ghibli"))
            .await
            .unwrap();
        assert_eq!(frame.tag, "Ghibli");
        assert_eq!(frame.caption, "");
        assert_eq!(frame.caption_engine, "");
        assert!(!frame.excluded);
        assert_eq!(frame.timestamp_secs, Some(1.5));
    }

    #[tokio::test]
    async fn list_for_job_only_returns_that_jobs_frames_in_order() {
        let (db, job_a) = db_with_job().await;
        let job_b = db
            .jobs()
            .insert(NewJob::new("dataset_prep"))
            .await
            .unwrap()
            .id;

        db.dataset_frames()
            .insert(new_frame(&job_a, "Ghibli"))
            .await
            .unwrap();
        db.dataset_frames()
            .insert(new_frame(&job_a, "Ghibli"))
            .await
            .unwrap();
        db.dataset_frames()
            .insert(new_frame(&job_b, "Other"))
            .await
            .unwrap();

        let frames = db.dataset_frames().list_for_job(&job_a).await.unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|f| f.job_id == job_a));
    }

    #[tokio::test]
    async fn set_caption_updates_text_and_engine() {
        let (db, job_id) = db_with_job().await;
        let frame = db
            .dataset_frames()
            .insert(new_frame(&job_id, "Ghibli"))
            .await
            .unwrap();

        db.dataset_frames()
            .set_caption(&frame.id, "a lush forest clearing", "florence2")
            .await
            .unwrap();

        let updated = db.dataset_frames().get(&frame.id).await.unwrap().unwrap();
        assert_eq!(updated.caption, "a lush forest clearing");
        assert_eq!(updated.caption_engine, "florence2");
    }

    #[tokio::test]
    async fn set_excluded_toggles_the_flag() {
        let (db, job_id) = db_with_job().await;
        let frame = db
            .dataset_frames()
            .insert(new_frame(&job_id, "Ghibli"))
            .await
            .unwrap();

        db.dataset_frames()
            .set_excluded(&frame.id, true)
            .await
            .unwrap();
        assert!(
            db.dataset_frames()
                .get(&frame.id)
                .await
                .unwrap()
                .unwrap()
                .excluded
        );

        db.dataset_frames()
            .set_excluded(&frame.id, false)
            .await
            .unwrap();
        assert!(
            !db.dataset_frames()
                .get(&frame.id)
                .await
                .unwrap()
                .unwrap()
                .excluded
        );
    }

    #[tokio::test]
    async fn deleting_the_job_cascades_to_its_frames() {
        let (db, job_id) = db_with_job().await;
        let frame = db
            .dataset_frames()
            .insert(new_frame(&job_id, "Ghibli"))
            .await
            .unwrap();

        db.jobs().delete(&job_id).await.unwrap();

        assert!(db.dataset_frames().get(&frame.id).await.unwrap().is_none());
    }
}
