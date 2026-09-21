//! Download-queue persistence (Phase 6.4). The transfer + verify + import is
//! driven by [`crate::download`]; this module only stores and queries.

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// Where a download is in its lifecycle. Mirrors `downloads.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    /// Waiting for the worker's single slot.
    Queued,
    /// Transferring.
    Running,
    /// The user paused it; `resume` puts it back to `Queued`.
    Paused,
    /// Fully transferred, hashing before the import.
    Verifying,
    /// Imported into the store (`model_id` is set).
    Done,
    /// Gave up (`error_text`), or the user cancelled.
    Failed,
}

impl DownloadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Verifying => "verifying",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "verifying" => Self::Verifying,
            "done" => Self::Done,
            "failed" => Self::Failed,
            other => return Err(CoreError::Db(format!("bad download state {other:?}"))),
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

/// A download as stored.
#[derive(Debug, Clone, Serialize)]
pub struct Download {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub dest_path: String,
    pub model_type: Option<String>,
    pub sha256: Option<String>,
    pub size_bytes: Option<i64>,
    pub bytes_done: i64,
    pub retries: i64,
    pub state: DownloadState,
    pub error_text: Option<String>,
    pub model_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Roles to stamp on the model once imported (e.g. `["chat", "coding"]`
    /// for an agent pick) — empty for a plain download.
    pub roles: Vec<String>,
    /// `civitai:<model>/<version>` | `hf:<repo>@<rev>` — recorded on the
    /// model as its `source` at import (migration 0024).
    pub origin: Option<String>,
    /// Registry id of the base family the source's label maps to.
    pub base_family: Option<String>,
    /// `civitai` | `hf` — how `base_family` was decided.
    pub family_source: Option<String>,
}

/// Fields a caller supplies to queue a download. `dest_path` is filled in from
/// the generated id.
#[derive(Debug, Clone, Default)]
pub struct NewDownload {
    pub url: String,
    pub filename: String,
    pub model_type: Option<String>,
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
    pub roles: Vec<String>,
    pub origin: Option<String>,
    pub base_family: Option<String>,
    pub family_source: Option<String>,
}

/// `["chat", "coding"]` <-> `"chat,coding"` — plenty for a handful of short,
/// comma-free role names; no need for a join table like `model_roles`.
fn join_roles(roles: &[String]) -> String {
    roles.join(",")
}

fn split_roles(joined: &str) -> Vec<String> {
    joined
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[derive(Debug)]
pub struct DownloadRepo<'a> {
    pool: &'a SqlitePool,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    url: String,
    filename: String,
    dest_path: String,
    model_type: Option<String>,
    sha256: Option<String>,
    size_bytes: Option<i64>,
    bytes_done: i64,
    retries: i64,
    state: String,
    error_text: Option<String>,
    model_id: Option<String>,
    created_at: String,
    updated_at: String,
    roles: String,
    origin: Option<String>,
    base_family: Option<String>,
    family_source: Option<String>,
}

impl Row {
    fn into_download(self) -> Result<Download> {
        Ok(Download {
            state: DownloadState::parse(&self.state)?,
            id: self.id,
            url: self.url,
            filename: self.filename,
            dest_path: self.dest_path,
            model_type: self.model_type,
            sha256: self.sha256,
            size_bytes: self.size_bytes,
            bytes_done: self.bytes_done,
            retries: self.retries,
            error_text: self.error_text,
            model_id: self.model_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
            roles: split_roles(&self.roles),
            origin: self.origin,
            base_family: self.base_family,
            family_source: self.family_source,
        })
    }
}

const COLS: &str = "id, url, filename, dest_path, model_type, sha256, size_bytes, bytes_done, \
     retries, state, error_text, model_id, created_at, updated_at, roles, origin, base_family,      family_source";

impl<'a> DownloadRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Queue a download. `staging_root` is `AppPaths::downloads_dir()`; the file
    /// lands at `<staging_root>/<id>/<filename>`.
    pub async fn create(
        &self,
        new: NewDownload,
        staging_root: &std::path::Path,
    ) -> Result<Download> {
        if new.url.trim().is_empty() || new.filename.trim().is_empty() {
            return Err(CoreError::Db("download needs a url and a filename".into()));
        }
        let id = Uuid::now_v7().to_string();
        let now = now_rfc3339();
        let dest = staging_root
            .join(&id)
            .join(&new.filename)
            .to_string_lossy()
            .into_owned();

        sqlx::query(
            "INSERT INTO downloads (id, url, filename, dest_path, model_type, sha256, size_bytes,
                 state, created_at, updated_at, roles, origin, base_family, family_source)
             VALUES ($1,$2,$3,$4,$5,$6,$7,'queued',$8,$8,$9,$10,$11,$12)",
        )
        .bind(&id)
        .bind(new.url.trim())
        .bind(new.filename.trim())
        .bind(&dest)
        .bind(&new.model_type)
        .bind(new.sha256.map(|s| s.to_lowercase()))
        .bind(new.size_bytes.and_then(|n| i64::try_from(n).ok()))
        .bind(&now)
        .bind(join_roles(&new.roles))
        .bind(&new.origin)
        .bind(&new.base_family)
        .bind(&new.family_source)
        .execute(self.pool)
        .await?;

        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("download vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Download>> {
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM downloads WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        row.map(Row::into_download).transpose()
    }

    /// Newest first.
    pub async fn list(&self) -> Result<Vec<Download>> {
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM downloads ORDER BY created_at DESC"
        )))
        .fetch_all(self.pool)
        .await?;
        rows.into_iter().map(Row::into_download).collect()
    }

    /// The one download the worker should act on: the oldest `queued`, or a
    /// `running` one left by a crash. `None` = nothing to do.
    pub async fn next_actionable(&self) -> Result<Option<Download>> {
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM downloads
             WHERE state IN ('queued', 'running')
             ORDER BY (state = 'running') DESC, created_at ASC
             LIMIT 1"
        )))
        .fetch_optional(self.pool)
        .await?;
        row.map(Row::into_download).transpose()
    }

    pub async fn set_state(
        &self,
        id: &str,
        state: DownloadState,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE downloads SET state = $1, error_text = $2, updated_at = $3 WHERE id = $4",
        )
        .bind(state.as_str())
        .bind(error)
        .bind(now_rfc3339())
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_progress(
        &self,
        id: &str,
        bytes_done: u64,
        size_bytes: Option<u64>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE downloads
             SET bytes_done = $1,
                 size_bytes = COALESCE($2, size_bytes),
                 updated_at = $3
             WHERE id = $4",
        )
        .bind(i64::try_from(bytes_done).unwrap_or(i64::MAX))
        .bind(size_bytes.and_then(|n| i64::try_from(n).ok()))
        .bind(now_rfc3339())
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Bump the transport-retry counter; returns the new value.
    pub async fn bump_retries(&self, id: &str) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "UPDATE downloads SET retries = retries + 1, updated_at = $1 WHERE id = $2
             RETURNING retries",
        )
        .bind(now_rfc3339())
        .bind(id)
        .fetch_one(self.pool)
        .await?;
        Ok(row.0)
    }

    /// Replace the roles stamped on the model once this download is imported.
    pub async fn set_roles(&self, id: &str, roles: &[String]) -> Result<()> {
        sqlx::query("UPDATE downloads SET roles = $1, updated_at = $2 WHERE id = $3")
            .bind(join_roles(roles))
            .bind(now_rfc3339())
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_model_id(&self, id: &str, model_id: &str) -> Result<()> {
        sqlx::query("UPDATE downloads SET model_id = $1, updated_at = $2 WHERE id = $3")
            .bind(model_id)
            .bind(now_rfc3339())
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM downloads WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Startup recovery: a `running` / `verifying` download from a previous
    /// process goes back to `queued` (it resumes from the partial file);
    /// `paused` stays. Returns how many were re-queued.
    pub async fn recover_interrupted(&self) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE downloads SET state = 'queued', updated_at = $1
             WHERE state IN ('running', 'verifying')",
        )
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn new_dl() -> NewDownload {
        NewDownload {
            url: "https://example.test/model.gguf".into(),
            filename: "model.gguf".into(),
            model_type: Some("chat".into()),
            sha256: Some("ABCDEF".into()),
            size_bytes: Some(1024),
            roles: vec![],
            ..NewDownload::default()
        }
    }

    #[tokio::test]
    async fn create_get_list_round_trip() {
        let db = Database::connect_in_memory().await.unwrap();
        let staging = std::path::Path::new("/tmp/dl");
        let d = db.downloads().create(new_dl(), staging).await.unwrap();

        assert_eq!(d.state, DownloadState::Queued);
        assert_eq!(d.sha256.as_deref(), Some("abcdef"), "hash is lower-cased");
        assert_eq!(d.bytes_done, 0);
        assert!(d
            .dest_path
            .replace('\\', "/")
            .ends_with(&format!("{}/model.gguf", d.id)));

        let got = db.downloads().get(&d.id).await.unwrap().unwrap();
        assert_eq!(got.id, d.id);
        assert_eq!(db.downloads().list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn next_actionable_prefers_a_recovered_running_row() {
        let db = Database::connect_in_memory().await.unwrap();
        let staging = std::path::Path::new("/tmp/dl");
        let a = db.downloads().create(new_dl(), staging).await.unwrap();
        let b = db.downloads().create(new_dl(), staging).await.unwrap();
        db.downloads()
            .set_state(&b.id, DownloadState::Running, None)
            .await
            .unwrap();

        assert_eq!(
            db.downloads().next_actionable().await.unwrap().unwrap().id,
            b.id
        );
        let _ = a;
    }

    #[tokio::test]
    async fn recover_interrupted_requeues_running_and_verifying_only() {
        let db = Database::connect_in_memory().await.unwrap();
        let staging = std::path::Path::new("/tmp/dl");
        let run = db.downloads().create(new_dl(), staging).await.unwrap();
        let ver = db.downloads().create(new_dl(), staging).await.unwrap();
        let pause = db.downloads().create(new_dl(), staging).await.unwrap();
        db.downloads()
            .set_state(&run.id, DownloadState::Running, None)
            .await
            .unwrap();
        db.downloads()
            .set_state(&ver.id, DownloadState::Verifying, None)
            .await
            .unwrap();
        db.downloads()
            .set_state(&pause.id, DownloadState::Paused, None)
            .await
            .unwrap();

        assert_eq!(db.downloads().recover_interrupted().await.unwrap(), 2);
        assert_eq!(
            db.downloads().get(&run.id).await.unwrap().unwrap().state,
            DownloadState::Queued
        );
        assert_eq!(
            db.downloads().get(&pause.id).await.unwrap().unwrap().state,
            DownloadState::Paused
        );
    }

    #[tokio::test]
    async fn progress_and_retries_and_model_id_stick() {
        let db = Database::connect_in_memory().await.unwrap();
        let d = db
            .downloads()
            .create(
                NewDownload {
                    size_bytes: None,
                    ..new_dl()
                },
                std::path::Path::new("/tmp/dl"),
            )
            .await
            .unwrap();

        db.downloads()
            .set_progress(&d.id, 512, Some(2048))
            .await
            .unwrap();
        assert_eq!(db.downloads().bump_retries(&d.id).await.unwrap(), 1);
        db.downloads().set_model_id(&d.id, "m-123").await.unwrap();

        let got = db.downloads().get(&d.id).await.unwrap().unwrap();
        assert_eq!(got.bytes_done, 512);
        assert_eq!(got.size_bytes, Some(2048));
        assert_eq!(got.retries, 1);
        assert_eq!(got.model_id.as_deref(), Some("m-123"));
    }

    #[tokio::test]
    async fn roles_round_trip_and_default_to_empty() {
        let db = Database::connect_in_memory().await.unwrap();
        let staging = std::path::Path::new("/tmp/dl");

        let plain = db.downloads().create(new_dl(), staging).await.unwrap();
        assert_eq!(plain.roles, Vec::<String>::new());

        let coding = db
            .downloads()
            .create(
                NewDownload {
                    roles: vec!["chat".into(), "coding".into()],
                    ..new_dl()
                },
                staging,
            )
            .await
            .unwrap();
        assert_eq!(coding.roles, vec!["chat".to_string(), "coding".to_string()]);

        // Survives a re-read from disk, not just the insert's own echo.
        let reread = db.downloads().get(&coding.id).await.unwrap().unwrap();
        assert_eq!(reread.roles, vec!["chat".to_string(), "coding".to_string()]);
    }
}
