//! Named, switchable job groupings for Chat/Image/Video (`sessions`) — a
//! lightweight "project" concept, e.g. one session for anime-style prompts,
//! another for realism. Deliberately thin: a session is just an id, a
//! capability, and a name. "Remembers what you last used" is derived by the
//! caller from the session's most recent job, not stored here.

use serde::Serialize;
use sqlx::{AssertSqlSafe, SqlitePool};
use uuid::Uuid;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// A stored session.
#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub id: String,
    /// `"chat"` | `"image"` | `"video"`.
    pub capability: String,
    pub name: String,
    pub created_at: String,
    /// `None` = active (shown in the switcher); `Some` = archived (hidden by
    /// default, jobs untouched).
    pub archived_at: Option<String>,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    capability: String,
    name: String,
    created_at: String,
    archived_at: Option<String>,
}

impl From<SessionRow> for Session {
    fn from(r: SessionRow) -> Self {
        Self {
            id: r.id,
            capability: r.capability,
            name: r.name,
            created_at: r.created_at,
            archived_at: r.archived_at,
        }
    }
}

// The `SELECT` statements below interpolate only this compile-time constant
// and `$N` bind placeholders — never caller data (always bound). `AssertSqlSafe`
// documents that we have checked this.
const SELECT_COLS: &str = "id, capability, name, created_at, archived_at";

#[derive(Debug)]
pub struct SessionRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SessionRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, capability: &str, name: &str) -> Result<Session> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO sessions (id, capability, name, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(&id)
        .bind(capability)
        .bind(name)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("session vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Session>> {
        let row = sqlx::query_as::<_, SessionRow>(AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM sessions WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Session::from))
    }

    /// Every session for `capability`, active ones first (by recency), then
    /// archived ones (also by recency) — so the switcher can just show the
    /// first block and fold the rest under "Archived".
    pub async fn list_for(&self, capability: &str) -> Result<Vec<Session>> {
        let rows = sqlx::query_as::<_, SessionRow>(AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM sessions WHERE capability = $1 \
             ORDER BY (archived_at IS NOT NULL), created_at DESC"
        )))
        .bind(capability)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Session::from).collect())
    }

    pub async fn rename(&self, id: &str, name: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET name = $1 WHERE id = $2")
            .bind(name)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_archived(&self, id: &str, archived: bool) -> Result<()> {
        let value = archived.then(now_rfc3339);
        sqlx::query("UPDATE sessions SET archived_at = $1 WHERE id = $2")
            .bind(value)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Deletes the session. Its jobs are **not** deleted — `jobs.session_id`
    /// is a real FK with `ON DELETE SET NULL`, so they fall back to
    /// "ungrouped" automatically; nothing in the Model Library / gallery /
    /// chat history is ever lost.
    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{Database, NewJob};

    async fn db() -> Database {
        Database::connect_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn create_and_get_round_trip() {
        let db = db().await;
        let s = db
            .sessions()
            .create("chat", "Debugging help")
            .await
            .unwrap();
        assert_eq!(s.capability, "chat");
        assert_eq!(s.name, "Debugging help");
        assert!(s.archived_at.is_none());

        let fetched = db.sessions().get(&s.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, s.id);
    }

    #[tokio::test]
    async fn list_for_filters_by_capability_and_puts_active_first() {
        let db = db().await;
        let chat_a = db.sessions().create("chat", "A").await.unwrap();
        let chat_b = db.sessions().create("chat", "B").await.unwrap();
        db.sessions().create("image", "Anime pics").await.unwrap();

        db.sessions().set_archived(&chat_a.id, true).await.unwrap();

        let chats = db.sessions().list_for("chat").await.unwrap();
        assert_eq!(chats.len(), 2);
        // Active (B) before archived (A), even though A was created first.
        assert_eq!(chats[0].id, chat_b.id);
        assert_eq!(chats[1].id, chat_a.id);
        assert!(chats[1].archived_at.is_some());

        let images = db.sessions().list_for("image").await.unwrap();
        assert_eq!(images.len(), 1);
    }

    #[tokio::test]
    async fn rename_updates_the_name_only() {
        let db = db().await;
        let s = db.sessions().create("video", "Untitled").await.unwrap();
        db.sessions().rename(&s.id, "B-roll clips").await.unwrap();
        let fetched = db.sessions().get(&s.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "B-roll clips");
    }

    #[tokio::test]
    async fn deleting_a_session_orphans_its_jobs_instead_of_deleting_them() {
        let db = db().await;
        let s = db.sessions().create("image", "Anime pics").await.unwrap();
        let mut new = NewJob::new("image");
        new.session_id = Some(s.id.clone());
        let job = db.jobs().insert(new).await.unwrap();
        assert_eq!(job.session_id.as_deref(), Some(s.id.as_str()));

        db.sessions().delete(&s.id).await.unwrap();

        assert!(db.sessions().get(&s.id).await.unwrap().is_none());
        let survived = db.jobs().get(&job.id).await.unwrap().unwrap();
        assert_eq!(survived.session_id, None, "the job must survive, ungrouped");
    }
}
