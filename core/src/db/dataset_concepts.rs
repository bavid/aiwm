//! Concepts a dataset teaches by name (spec 3A): `name` is what the user
//! sees, `token` is the trigger inserted into captions, `description` is
//! optional extra caption text. Assignment to frames is a plain join table —
//! captions are composed at export, never rewritten here.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DatasetConcept {
    pub id: String,
    pub dataset_id: String,
    pub name: String,
    pub token: String,
    pub description: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct ConceptRow {
    id: String,
    dataset_id: String,
    name: String,
    token: String,
    description: String,
    created_at: String,
}

impl From<ConceptRow> for DatasetConcept {
    fn from(r: ConceptRow) -> Self {
        Self {
            id: r.id,
            dataset_id: r.dataset_id,
            name: r.name,
            token: r.token,
            description: r.description,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewConcept {
    pub dataset_id: String,
    pub name: String,
    pub token: String,
    pub description: String,
}

const SELECT_COLS: &str = "id, dataset_id, name, token, description, created_at";

#[derive(Debug)]
pub struct ConceptRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ConceptRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, c: NewConcept) -> Result<DatasetConcept> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO dataset_concepts (id, dataset_id, name, token, description, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(&c.dataset_id)
        .bind(c.name.trim())
        .bind(c.token.trim())
        .bind(c.description.trim())
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| crate::CoreError::Db("concept vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<DatasetConcept>> {
        let row = sqlx::query_as::<_, ConceptRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM dataset_concepts WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(DatasetConcept::from))
    }

    pub async fn list_for_dataset(&self, dataset_id: &str) -> Result<Vec<DatasetConcept>> {
        let rows = sqlx::query_as::<_, ConceptRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM dataset_concepts WHERE dataset_id = $1 ORDER BY created_at, id"
        )))
        .bind(dataset_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(DatasetConcept::from).collect())
    }

    pub async fn update(&self, id: &str, name: &str, token: &str, description: &str) -> Result<()> {
        sqlx::query(
            "UPDATE dataset_concepts SET name = $1, token = $2, description = $3 WHERE id = $4",
        )
        .bind(name.trim())
        .bind(token.trim())
        .bind(description.trim())
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM dataset_concepts WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Assign `concept_id` to every frame in `frame_ids`; already-assigned
    /// pairs are ignored (idempotent), so "Alle im Set" can be clicked twice.
    /// A frame that doesn't belong to the concept's own dataset is silently
    /// skipped (the join-table FK alone only checks the frame exists, not
    /// which dataset it's in). All frames succeed or none do.
    pub async fn assign(&self, concept_id: &str, frame_ids: &[String]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for frame_id in frame_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO frame_concepts (frame_id, concept_id) \
                 SELECT $1, $2 WHERE EXISTS ( \
                     SELECT 1 FROM dataset_frames df JOIN dataset_concepts c ON c.id = $2 \
                     WHERE df.id = $1 AND df.dataset_id = c.dataset_id \
                 )",
            )
            .bind(frame_id)
            .bind(concept_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// A 40-frame "Alle im Set" unassign is all-or-nothing too.
    pub async fn unassign(&self, concept_id: &str, frame_ids: &[String]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for frame_id in frame_ids {
            sqlx::query("DELETE FROM frame_concepts WHERE frame_id = $1 AND concept_id = $2")
                .bind(frame_id)
                .bind(concept_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// frame_id -> concept ids, for every frame of `dataset_id` that has at
    /// least one concept (frames without concepts are simply absent).
    pub async fn map_for_dataset(&self, dataset_id: &str) -> Result<HashMap<String, Vec<String>>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT fc.frame_id, fc.concept_id FROM frame_concepts fc \
             JOIN dataset_concepts c ON c.id = fc.concept_id \
             WHERE c.dataset_id = $1 ORDER BY fc.frame_id, c.created_at",
        )
        .bind(dataset_id)
        .fetch_all(self.pool)
        .await?;
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (frame_id, concept_id) in rows {
            map.entry(frame_id).or_default().push(concept_id);
        }
        Ok(map)
    }

    /// concept_id -> number of frames carrying it (0 for concepts with none).
    pub async fn counts_for_dataset(&self, dataset_id: &str) -> Result<BTreeMap<String, usize>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT c.id, COUNT(fc.frame_id) FROM dataset_concepts c \
             LEFT JOIN frame_concepts fc ON fc.concept_id = c.id \
             WHERE c.dataset_id = $1 GROUP BY c.id",
        )
        .bind(dataset_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, n)| (id, usize::try_from(n).unwrap_or(0)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, DatasetMode, NewDataset, NewDatasetFrame, NewJob};

    async fn fixture() -> (Database, String, Vec<String>) {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db.jobs().insert(NewJob::new("dataset_prep")).await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "T".into(),
                mode: DatasetMode::Frames,
                source_root: "x".into(),
                prep_job_id: Some(job.id.clone()),
            })
            .await
            .unwrap();
        let mut frame_ids = Vec::new();
        for i in 0..3 {
            let f = db
                .dataset_frames()
                .insert(NewDatasetFrame {
                    job_id: job.id.clone(),
                    dataset_id: Some(ds.id.clone()),
                    tag: "A".into(),
                    source_path: "clip.mp4".into(),
                    frame_path: format!("f{i}.png"),
                    timestamp_secs: Some(f64::from(i)),
                    rejection_reason: String::new(),
                    duration_secs: None,
                })
                .await
                .unwrap();
            frame_ids.push(f.id);
        }
        (db, ds.id, frame_ids)
    }

    #[tokio::test]
    async fn create_trims_and_rejects_duplicate_token_in_same_dataset() {
        let (db, ds_id, _) = fixture().await;
        let c = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id.clone(),
                name: " Kenji ".into(),
                token: " kenji_xy ".into(),
                description: "".into(),
            })
            .await
            .unwrap();
        assert_eq!((c.name.as_str(), c.token.as_str()), ("Kenji", "kenji_xy"));
        let dup = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id,
                name: "Other".into(),
                token: "kenji_xy".into(),
                description: "".into(),
            })
            .await;
        assert!(dup.is_err(), "UNIQUE (dataset_id, token) must reject");
    }

    #[tokio::test]
    async fn assign_is_idempotent_and_unassign_removes_only_the_pair() {
        let (db, ds_id, frames) = fixture().await;
        let c = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id.clone(),
                name: "Kenji".into(),
                token: "kenji_xy".into(),
                description: "".into(),
            })
            .await
            .unwrap();
        db.concepts().assign(&c.id, &frames[..2]).await.unwrap();
        db.concepts().assign(&c.id, &frames[..2]).await.unwrap(); // twice, no error

        let map = db.concepts().map_for_dataset(&ds_id).await.unwrap();
        assert_eq!(map.get(&frames[0]).unwrap(), &vec![c.id.clone()]);
        assert!(!map.contains_key(&frames[2]));
        assert_eq!(
            db.concepts().counts_for_dataset(&ds_id).await.unwrap()[&c.id],
            2
        );

        db.concepts().unassign(&c.id, &frames[..1]).await.unwrap();
        assert_eq!(
            db.concepts().counts_for_dataset(&ds_id).await.unwrap()[&c.id],
            1
        );
    }

    #[tokio::test]
    async fn update_rewrites_name_token_and_description_trimmed() {
        let (db, ds_id, _) = fixture().await;
        let c = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id,
                name: "A".into(),
                token: "a_xy".into(),
                description: "".into(),
            })
            .await
            .unwrap();
        db.concepts()
            .update(&c.id, " Kenji ", " kenji_xy ", " bare, visible ")
            .await
            .unwrap();
        let got = db.concepts().get(&c.id).await.unwrap().unwrap();
        assert_eq!(
            (
                got.name.as_str(),
                got.token.as_str(),
                got.description.as_str()
            ),
            ("Kenji", "kenji_xy", "bare, visible")
        );
    }

    #[tokio::test]
    async fn deleting_a_concept_cascades_its_assignments_and_counts_zero_for_unused() {
        let (db, ds_id, frames) = fixture().await;
        let used = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id.clone(),
                name: "A".into(),
                token: "a_xy".into(),
                description: "".into(),
            })
            .await
            .unwrap();
        let unused = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id.clone(),
                name: "B".into(),
                token: "b_xy".into(),
                description: "".into(),
            })
            .await
            .unwrap();
        db.concepts().assign(&used.id, &frames).await.unwrap();
        assert_eq!(
            db.concepts().counts_for_dataset(&ds_id).await.unwrap()[&unused.id],
            0
        );

        db.concepts().delete(&used.id).await.unwrap();
        assert!(db
            .concepts()
            .map_for_dataset(&ds_id)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            db.concepts().list_for_dataset(&ds_id).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn deleting_the_dataset_removes_its_concepts() {
        let (db, ds_id, frames) = fixture().await;
        let c = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id.clone(),
                name: "A".into(),
                token: "a_xy".into(),
                description: "".into(),
            })
            .await
            .unwrap();
        db.concepts().assign(&c.id, &frames).await.unwrap();
        db.datasets().delete(&ds_id).await.unwrap();
        assert!(db.concepts().get(&c.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn assign_ignores_frames_from_another_dataset() {
        let (db, ds_a, frames_a) = fixture().await;
        let job_b = db.jobs().insert(NewJob::new("dataset_prep")).await.unwrap();
        let ds_b = db
            .datasets()
            .create(NewDataset {
                name: "U".into(),
                mode: DatasetMode::Frames,
                source_root: "y".into(),
                prep_job_id: Some(job_b.id.clone()),
            })
            .await
            .unwrap();
        let frame_b = db
            .dataset_frames()
            .insert(NewDatasetFrame {
                job_id: job_b.id.clone(),
                dataset_id: Some(ds_b.id.clone()),
                tag: "B".into(),
                source_path: "clip.mp4".into(),
                frame_path: "b0.png".into(),
                timestamp_secs: Some(0.0),
                rejection_reason: String::new(),
                duration_secs: None,
            })
            .await
            .unwrap();

        let concept_a = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_a.clone(),
                name: "A".into(),
                token: "a_xy".into(),
                description: "".into(),
            })
            .await
            .unwrap();

        db.concepts()
            .assign(&concept_a.id, std::slice::from_ref(&frame_b.id))
            .await
            .unwrap();
        assert!(db
            .concepts()
            .map_for_dataset(&ds_a)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            db.concepts().counts_for_dataset(&ds_a).await.unwrap()[&concept_a.id],
            0
        );

        db.concepts()
            .assign(&concept_a.id, std::slice::from_ref(&frames_a[0]))
            .await
            .unwrap();
        assert_eq!(
            db.concepts().counts_for_dataset(&ds_a).await.unwrap()[&concept_a.id],
            1
        );
    }

    #[tokio::test]
    async fn assign_and_unassign_with_no_frames_are_noops() {
        let (db, ds_id, _) = fixture().await;
        let c = db
            .concepts()
            .create(NewConcept {
                dataset_id: ds_id.clone(),
                name: "A".into(),
                token: "a_xy".into(),
                description: "".into(),
            })
            .await
            .unwrap();

        db.concepts().assign(&c.id, &[]).await.unwrap();
        db.concepts().unassign(&c.id, &[]).await.unwrap();

        assert_eq!(
            db.concepts().counts_for_dataset(&ds_id).await.unwrap()[&c.id],
            0
        );
    }
}
