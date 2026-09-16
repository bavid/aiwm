//! Frame persistence for a dataset (see [`crate::capability::dataset`]). A
//! frame's lifecycle is governed by its dataset, not the job that produced
//! it: `job_id` is nulled out (never cascaded) when the producing
//! `dataset_prep` job is deleted, while the frame itself cascade-deletes
//! only with its dataset — see the `0015_datasets.sql` migration's own doc
//! comment.

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::Result;

/// One kept frame, ready for curation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DatasetFrame {
    pub id: String,
    /// The `dataset_prep` job that produced this frame; `None` once that job
    /// has been deleted (`ON DELETE SET NULL`) — the frame lives on with its
    /// dataset.
    pub job_id: Option<String>,
    pub dataset_id: Option<String>,
    pub tag: String,
    pub source_path: String,
    pub frame_path: String,
    pub timestamp_secs: Option<f64>,
    pub caption: String,
    /// `"florence2"` | `"wd-eva02-tagger-v3"` | `"qwen2.5-vl"` | `""` (not captioned / hand-edited).
    pub caption_engine: String,
    pub excluded: bool,
    /// `""` = kept; otherwise one of the pipeline's rejection reasons
    /// (`black` | `transition` | `blur` | `duplicate` | `cap` | `unusable`).
    pub rejection_reason: String,
    /// Clip mode: the clip's length. `None` for a still frame.
    pub duration_secs: Option<f64>,
    pub clip_start_secs: Option<f64>,
    pub clip_end_secs: Option<f64>,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct DatasetFrameRow {
    id: String,
    job_id: Option<String>,
    dataset_id: Option<String>,
    tag: String,
    source_path: String,
    frame_path: String,
    timestamp_secs: Option<f64>,
    caption: String,
    caption_engine: String,
    excluded: i64,
    rejection_reason: String,
    duration_secs: Option<f64>,
    clip_start_secs: Option<f64>,
    clip_end_secs: Option<f64>,
    created_at: String,
}

impl From<DatasetFrameRow> for DatasetFrame {
    fn from(r: DatasetFrameRow) -> Self {
        Self {
            id: r.id,
            job_id: r.job_id,
            dataset_id: r.dataset_id,
            tag: r.tag,
            source_path: r.source_path,
            frame_path: r.frame_path,
            timestamp_secs: r.timestamp_secs,
            caption: r.caption,
            caption_engine: r.caption_engine,
            excluded: r.excluded != 0,
            rejection_reason: r.rejection_reason,
            duration_secs: r.duration_secs,
            clip_start_secs: r.clip_start_secs,
            clip_end_secs: r.clip_end_secs,
            created_at: r.created_at,
        }
    }
}

/// Body for [`DatasetFrameRepo::insert`] — everything decided at extraction/
/// filtering time, before a caption exists.
#[derive(Debug, Clone)]
pub struct NewDatasetFrame {
    pub job_id: String,
    pub dataset_id: Option<String>,
    pub tag: String,
    pub source_path: String,
    pub frame_path: String,
    pub timestamp_secs: Option<f64>,
    pub rejection_reason: String,
    pub duration_secs: Option<f64>,
}

const SELECT_COLS: &str = "id, job_id, dataset_id, tag, source_path, frame_path, timestamp_secs, \
     caption, caption_engine, excluded, rejection_reason, duration_secs, clip_start_secs, \
     clip_end_secs, created_at";

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
             (id, job_id, dataset_id, tag, source_path, frame_path, timestamp_secs, \
              rejection_reason, duration_secs, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&id)
        .bind(&f.job_id)
        .bind(&f.dataset_id)
        .bind(&f.tag)
        .bind(&f.source_path)
        .bind(&f.frame_path)
        .bind(f.timestamp_secs)
        .bind(&f.rejection_reason)
        .bind(f.duration_secs)
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

    /// Every frame of a dataset, insertion order — rejected ones included
    /// (`rejection_reason != ""`); callers filter for the curation grid.
    pub async fn list_for_dataset(&self, dataset_id: &str) -> Result<Vec<DatasetFrame>> {
        let rows = sqlx::query_as::<_, DatasetFrameRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM dataset_frames WHERE dataset_id = $1 ORDER BY created_at, id"
        )))
        .bind(dataset_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(DatasetFrame::from).collect())
    }

    /// `""` restores a rejected frame into the kept set.
    pub async fn set_rejection_reason(&self, id: &str, reason: &str) -> Result<()> {
        sqlx::query("UPDATE dataset_frames SET rejection_reason = $1 WHERE id = $2")
            .bind(reason)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_clip_range(
        &self,
        id: &str,
        start: Option<f64>,
        end: Option<f64>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE dataset_frames SET clip_start_secs = $1, clip_end_secs = $2 WHERE id = $3",
        )
        .bind(start)
        .bind(end)
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
            dataset_id: None,
            tag: tag.to_string(),
            source_path: "E:\\Data\\Ghibli\\clip.mp4".into(),
            frame_path: "C:\\datasets\\job-1\\0001.png".into(),
            timestamp_secs: Some(1.5),
            rejection_reason: String::new(),
            duration_secs: None,
        }
    }

    async fn db_with_dataset() -> (Database, String, String) {
        let (db, job_id) = db_with_job().await;
        let ds = db
            .datasets()
            .create(crate::db::NewDataset {
                name: "T".into(),
                mode: crate::db::DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: Some(job_id.clone()),
            })
            .await
            .unwrap();
        (db, job_id, ds.id)
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
        assert!(frames
            .iter()
            .all(|f| f.job_id.as_deref() == Some(job_a.as_str())));
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

    // 0015 changed `job_id`'s FK action from CASCADE to SET NULL: a frame with
    // no dataset must still survive its producing job's deletion (it becomes
    // unreachable by job_id, not gone — its row lives on with `job_id` NULL).
    #[tokio::test]
    async fn deleting_the_job_makes_the_frame_unreachable_by_job_id() {
        let (db, job_id) = db_with_job().await;
        let frame = db
            .dataset_frames()
            .insert(new_frame(&job_id, "Ghibli"))
            .await
            .unwrap();

        db.jobs().delete(&job_id).await.unwrap();

        assert!(db
            .dataset_frames()
            .list_for_job(&job_id)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            db.dataset_frames()
                .get(&frame.id)
                .await
                .unwrap()
                .unwrap()
                .job_id,
            None
        );
    }

    #[tokio::test]
    async fn insert_records_dataset_rejection_and_clip_fields() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut f = new_frame(&job_id, "Ghibli");
        f.dataset_id = Some(ds_id.clone());
        f.rejection_reason = "black".into();
        f.duration_secs = Some(12.5);
        let frame = db.dataset_frames().insert(f).await.unwrap();
        assert_eq!(frame.job_id.as_deref(), Some(job_id.as_str()));
        assert_eq!(frame.dataset_id.as_deref(), Some(ds_id.as_str()));
        assert_eq!(frame.rejection_reason, "black");
        assert_eq!(frame.duration_secs, Some(12.5));
        assert_eq!(frame.clip_start_secs, None);
    }

    #[tokio::test]
    async fn list_for_dataset_returns_only_that_datasets_frames() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut in_ds = new_frame(&job_id, "A");
        in_ds.dataset_id = Some(ds_id.clone());
        db.dataset_frames().insert(in_ds).await.unwrap();
        db.dataset_frames()
            .insert(new_frame(&job_id, "B"))
            .await
            .unwrap(); // no dataset

        let frames = db.dataset_frames().list_for_dataset(&ds_id).await.unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].tag, "A");
    }

    #[tokio::test]
    async fn set_rejection_reason_restores_a_rejected_frame() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut f = new_frame(&job_id, "A");
        f.dataset_id = Some(ds_id);
        f.rejection_reason = "blur".into();
        let frame = db.dataset_frames().insert(f).await.unwrap();
        db.dataset_frames()
            .set_rejection_reason(&frame.id, "")
            .await
            .unwrap();
        assert_eq!(
            db.dataset_frames()
                .get(&frame.id)
                .await
                .unwrap()
                .unwrap()
                .rejection_reason,
            ""
        );
    }

    #[tokio::test]
    async fn set_clip_range_persists_start_and_end() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut f = new_frame(&job_id, "A");
        f.dataset_id = Some(ds_id);
        let frame = db.dataset_frames().insert(f).await.unwrap();
        db.dataset_frames()
            .set_clip_range(&frame.id, Some(1.0), Some(4.5))
            .await
            .unwrap();
        let got = db.dataset_frames().get(&frame.id).await.unwrap().unwrap();
        assert_eq!(
            (got.clip_start_secs, got.clip_end_secs),
            (Some(1.0), Some(4.5))
        );
    }

    #[tokio::test]
    async fn deleting_the_dataset_cascades_to_its_frames() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut f = new_frame(&job_id, "A");
        f.dataset_id = Some(ds_id.clone());
        let frame = db.dataset_frames().insert(f).await.unwrap();
        db.datasets().delete(&ds_id).await.unwrap();
        assert!(db.dataset_frames().get(&frame.id).await.unwrap().is_none());
    }

    /// Regression guard for the silent NULL -> "" decode a reviewer found
    /// while `job_id` was still a plain `String`: after the producing job is
    /// deleted the row must survive AND report `job_id == None`, never `""`.
    #[tokio::test]
    async fn deleting_the_job_keeps_a_dataset_frame_with_job_id_none() {
        let (db, job_id, ds_id) = db_with_dataset().await;
        let mut f = new_frame(&job_id, "A");
        f.dataset_id = Some(ds_id);
        let frame = db.dataset_frames().insert(f).await.unwrap();
        db.jobs().delete(&job_id).await.unwrap();
        let got = db.dataset_frames().get(&frame.id).await.unwrap().unwrap();
        assert_eq!(got.job_id, None);
        assert!(db
            .dataset_frames()
            .list_for_job(&job_id)
            .await
            .unwrap()
            .is_empty());
    }
}
