//! Datasets: the durable, reusable object a training run points at. See
//! `0015_datasets.sql`.

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::{CoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatasetMode {
    Frames,
    Clips,
}

impl DatasetMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Frames => "frames",
            Self::Clips => "clips",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "frames" => Some(Self::Frames),
            "clips" => Some(Self::Clips),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Dataset {
    pub id: String,
    pub name: String,
    pub mode: DatasetMode,
    pub source_root: String,
    pub trigger_word: String,
    pub prep_job_id: Option<String>,
    pub export_dir: Option<String>,
    /// The absolute work folder (`<data_dir>/<prep_job_id>`) the prep run
    /// wrote into; `None` on rows from before migration 0019, whose folder
    /// is derived from the datasets root.
    pub work_dir: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct NewDataset {
    pub name: String,
    pub mode: DatasetMode,
    pub source_root: String,
    pub prep_job_id: Option<String>,
    /// See [`Dataset::work_dir`].
    pub work_dir: Option<String>,
}

#[derive(sqlx::FromRow)]
struct DatasetRow {
    id: String,
    name: String,
    mode: String,
    source_root: String,
    trigger_word: String,
    prep_job_id: Option<String>,
    export_dir: Option<String>,
    work_dir: Option<String>,
    created_at: String,
}

impl TryFrom<DatasetRow> for Dataset {
    type Error = CoreError;

    fn try_from(r: DatasetRow) -> Result<Self> {
        let mode = DatasetMode::parse(&r.mode)
            .ok_or_else(|| CoreError::Db(format!("bad dataset mode {:?}", r.mode)))?;
        Ok(Self {
            id: r.id,
            name: r.name,
            mode,
            source_root: r.source_root,
            trigger_word: r.trigger_word,
            prep_job_id: r.prep_job_id,
            export_dir: r.export_dir,
            work_dir: r.work_dir,
            created_at: r.created_at,
        })
    }
}

const SELECT_COLS: &str =
    "id, name, mode, source_root, trigger_word, prep_job_id, export_dir, work_dir, created_at";

#[derive(Debug)]
pub struct DatasetRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> DatasetRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, d: NewDataset) -> Result<Dataset> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO datasets \
             (id, name, mode, source_root, prep_job_id, work_dir, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&id)
        .bind(&d.name)
        .bind(d.mode.as_str())
        .bind(&d.source_root)
        .bind(&d.prep_job_id)
        .bind(&d.work_dir)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| crate::CoreError::Db("dataset vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Dataset>> {
        let row = sqlx::query_as::<_, DatasetRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM datasets WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        row.map(Dataset::try_from).transpose()
    }

    /// Newest first — the Dataset tab lists the most recent run on top.
    pub async fn list(&self) -> Result<Vec<Dataset>> {
        let rows = sqlx::query_as::<_, DatasetRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM datasets ORDER BY created_at DESC, id DESC"
        )))
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(Dataset::try_from)
            .collect::<Result<Vec<_>>>()
    }

    pub async fn set_trigger_word(&self, id: &str, trigger_word: &str) -> Result<()> {
        sqlx::query("UPDATE datasets SET trigger_word = $1 WHERE id = $2")
            .bind(trigger_word.trim())
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_export_dir(&self, id: &str, export_dir: &str) -> Result<()> {
        sqlx::query("UPDATE datasets SET export_dir = $1 WHERE id = $2")
            .bind(export_dir)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM datasets WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewDatasetFrame, NewJob};

    #[tokio::test]
    async fn create_then_get_round_trips_and_defaults_trigger_to_empty() {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db.jobs().insert(NewJob::new("dataset_prep")).await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "Anime style".into(),
                mode: DatasetMode::Frames,
                source_root: "E:\\Data\\Anime".into(),
                prep_job_id: Some(job.id.clone()),
                work_dir: None,
            })
            .await
            .unwrap();
        assert_eq!(ds.mode, DatasetMode::Frames);
        assert_eq!(ds.trigger_word, "");
        assert_eq!(ds.prep_job_id.as_deref(), Some(job.id.as_str()));
        let got = db.datasets().get(&ds.id).await.unwrap().unwrap();
        assert_eq!(got, ds);
    }

    #[tokio::test]
    async fn list_is_newest_first_and_set_trigger_word_persists() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = db
            .datasets()
            .create(NewDataset {
                name: "A".into(),
                mode: DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: None,
                work_dir: None,
            })
            .await
            .unwrap();
        let b = db
            .datasets()
            .create(NewDataset {
                name: "B".into(),
                mode: DatasetMode::Clips,
                source_root: "y".into(),
                prep_job_id: None,
                work_dir: None,
            })
            .await
            .unwrap();
        db.datasets()
            .set_trigger_word(&a.id, " ghibli_xy ")
            .await
            .unwrap();

        let all = db.datasets().list().await.unwrap();
        assert_eq!(
            all.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            vec![b.id.as_str(), a.id.as_str()]
        );
        assert_eq!(
            db.datasets()
                .get(&a.id)
                .await
                .unwrap()
                .unwrap()
                .trigger_word,
            "ghibli_xy"
        );
    }

    #[tokio::test]
    async fn deleting_the_prep_job_keeps_the_dataset_but_nulls_the_link() {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db.jobs().insert(NewJob::new("dataset_prep")).await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "A".into(),
                mode: DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: Some(job.id.clone()),
                work_dir: None,
            })
            .await
            .unwrap();
        db.jobs().delete(&job.id).await.unwrap();
        let got = db.datasets().get(&ds.id).await.unwrap().unwrap();
        assert_eq!(got.prep_job_id, None);
    }

    #[tokio::test]
    async fn set_export_dir_persists() {
        let db = Database::connect_in_memory().await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "A".into(),
                mode: DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: None,
                work_dir: None,
            })
            .await
            .unwrap();
        db.datasets()
            .set_export_dir(&ds.id, "E:\\Exports\\anime-lora")
            .await
            .unwrap();
        assert_eq!(
            db.datasets().get(&ds.id).await.unwrap().unwrap().export_dir,
            Some("E:\\Exports\\anime-lora".to_string())
        );
    }

    #[tokio::test]
    async fn deleting_the_prep_job_keeps_frames_that_belong_to_a_dataset() {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db.jobs().insert(NewJob::new("dataset_prep")).await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "A".into(),
                mode: DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: Some(job.id.clone()),
                work_dir: None,
            })
            .await
            .unwrap();
        let frame = db
            .dataset_frames()
            .insert(NewDatasetFrame {
                job_id: job.id.clone(),
                dataset_id: Some(ds.id.clone()),
                tag: "Ghibli".into(),
                source_path: "E:\\Data\\Ghibli\\clip.mp4".into(),
                frame_path: "C:\\datasets\\job-1\\0001.png".into(),
                timestamp_secs: Some(1.5),
                rejection_reason: String::new(),
                duration_secs: None,
            })
            .await
            .unwrap();

        db.jobs().delete(&job.id).await.unwrap();

        let got_ds = db.datasets().get(&ds.id).await.unwrap().unwrap();
        assert_eq!(got_ds.prep_job_id, None);
        let got = db.dataset_frames().get(&frame.id).await.unwrap().unwrap();
        assert_eq!(got.job_id, None);

        db.datasets().delete(&ds.id).await.unwrap();
        assert!(db.dataset_frames().get(&frame.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_removes_the_dataset() {
        let db = Database::connect_in_memory().await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "A".into(),
                mode: DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: None,
                work_dir: None,
            })
            .await
            .unwrap();
        db.datasets().delete(&ds.id).await.unwrap();
        assert!(db.datasets().get(&ds.id).await.unwrap().is_none());
    }
}
