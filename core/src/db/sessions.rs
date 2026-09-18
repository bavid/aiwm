//! Named, switchable job groupings for Chat/Image/Video (`sessions`) — a
//! lightweight "project" concept, e.g. one session for anime-style prompts,
//! another for realism. Deliberately thin: a session is just an id, a
//! capability, and a name. "Remembers what you last used" is derived by the
//! caller from the session's most recent job, not stored here.

use serde::Serialize;
use sqlx::{AssertSqlSafe, SqlitePool};
use uuid::Uuid;

use super::now_rfc3339;
use super::personas::PersonaMode;
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
    /// How this chat picks its persona. `Inherit` (the default for every
    /// session, including every one that predates personas) means "whatever is
    /// active globally".
    pub persona_mode: PersonaMode,
    /// Only meaningful with [`PersonaMode::Persona`]. May name a persona that
    /// has since been deleted — [`crate::persona::resolve`] heals that.
    pub persona_id: Option<String>,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    capability: String,
    name: String,
    created_at: String,
    archived_at: Option<String>,
    persona_mode: String,
    persona_id: Option<String>,
}

impl From<SessionRow> for Session {
    fn from(r: SessionRow) -> Self {
        Self {
            id: r.id,
            capability: r.capability,
            name: r.name,
            created_at: r.created_at,
            archived_at: r.archived_at,
            persona_mode: PersonaMode::from_db(&r.persona_mode),
            persona_id: r.persona_id,
        }
    }
}

// The `SELECT` statements below interpolate only this compile-time constant
// and `$N` bind placeholders — never caller data (always bound). `AssertSqlSafe`
// documents that we have checked this.
const SELECT_COLS: &str = "id, capability, name, created_at, archived_at, persona_mode, persona_id";

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

    /// Point this session's persona override at `mode` / `persona_id`. The id is
    /// only stored for [`PersonaMode::Persona`]; the other two modes always
    /// clear it, so a stale id can never linger behind a mode that ignores it.
    /// Returns whether the session exists.
    pub async fn set_persona(
        &self,
        id: &str,
        mode: PersonaMode,
        persona_id: Option<&str>,
    ) -> Result<bool> {
        let persona_id = match mode {
            PersonaMode::Persona => persona_id,
            PersonaMode::Inherit | PersonaMode::None => None,
        };
        let affected =
            sqlx::query("UPDATE sessions SET persona_mode = $1, persona_id = $2 WHERE id = $3")
                .bind(mode.as_str())
                .bind(persona_id)
                .bind(id)
                .execute(self.pool)
                .await?
                .rows_affected();
        Ok(affected > 0)
    }

    /// Compare-and-heal: reset this session to `inherit` only while it is still
    /// in `persona` mode *and* still points at `stale`. `stale = None` targets
    /// the corrupt `mode = 'persona'` with a NULL id (`persona_id IS NULL`
    /// rather than `=`, which never matches NULL).
    ///
    /// A reader that finds a dangling override must use this rather than
    /// [`set_persona`](Self::set_persona) — between the read and the write the
    /// user may have picked a valid persona for this chat, and an unconditional
    /// reset would silently undo that. Returns whether a row was healed.
    pub async fn heal_persona(&self, id: &str, stale: Option<&str>) -> Result<bool> {
        let affected = sqlx::query(
            "UPDATE sessions SET persona_mode = 'inherit', persona_id = NULL \
             WHERE id = $1 AND persona_mode = 'persona' AND persona_id IS $2",
        )
        .bind(id)
        .bind(stale)
        .execute(self.pool)
        .await?
        .rows_affected();
        Ok(affected > 0)
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
    use crate::db::{Database, NewJob, PersonaMode};

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
        assert_eq!(s.persona_mode, PersonaMode::Inherit, "personas are opt-in");
        assert_eq!(s.persona_id, None);

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
    async fn set_persona_round_trips_and_clears_the_id_for_the_idless_modes() {
        let db = db().await;
        let s = db.sessions().create("chat", "Override").await.unwrap();
        let persona = db
            .personas()
            .create("Blunt", "🪓", "be brief")
            .await
            .unwrap();

        assert!(db
            .sessions()
            .set_persona(&s.id, PersonaMode::Persona, Some(&persona.id))
            .await
            .unwrap());
        let fetched = db.sessions().get(&s.id).await.unwrap().unwrap();
        assert_eq!(fetched.persona_mode, PersonaMode::Persona);
        assert_eq!(fetched.persona_id.as_deref(), Some(persona.id.as_str()));

        // Switching to `none` (or back to `inherit`) drops the id, so no stale
        // pointer survives behind a mode that ignores it.
        db.sessions()
            .set_persona(&s.id, PersonaMode::None, Some(&persona.id))
            .await
            .unwrap();
        let fetched = db.sessions().get(&s.id).await.unwrap().unwrap();
        assert_eq!(fetched.persona_mode, PersonaMode::None);
        assert_eq!(fetched.persona_id, None);
    }

    /// The compare-and-heal a self-healing reader needs: reset the override only
    /// while it still points at the stale id it saw. A session re-pointed at a
    /// valid persona in between must survive untouched.
    #[tokio::test]
    async fn heal_persona_only_resets_a_session_still_pointing_at_the_stale_id() {
        let db = db().await;
        let stale = db.personas().create("Stale", "💀", "p").await.unwrap();
        let fresh = db.personas().create("Fresh", "🙂", "p").await.unwrap();
        let s = db.sessions().create("chat", "Chat").await.unwrap();
        db.sessions()
            .set_persona(&s.id, PersonaMode::Persona, Some(&fresh.id))
            .await
            .unwrap();

        // A heal aimed at the *old* id must not touch the new pointer.
        assert!(!db
            .sessions()
            .heal_persona(&s.id, Some(&stale.id))
            .await
            .unwrap());
        let kept = db.sessions().get(&s.id).await.unwrap().unwrap();
        assert_eq!(kept.persona_mode, PersonaMode::Persona);
        assert_eq!(kept.persona_id.as_deref(), Some(fresh.id.as_str()));

        // Aimed at the id it really holds, it resets.
        assert!(db
            .sessions()
            .heal_persona(&s.id, Some(&fresh.id))
            .await
            .unwrap());
        let healed = db.sessions().get(&s.id).await.unwrap().unwrap();
        assert_eq!(healed.persona_mode, PersonaMode::Inherit);
        assert_eq!(healed.persona_id, None);
    }

    /// `stale = None` targets the corrupt `mode = 'persona'` + `persona_id NULL`
    /// row, which only a direct SQL write can produce.
    #[tokio::test]
    async fn heal_persona_resets_mode_persona_with_a_null_id() {
        let db = db().await;
        let s = db.sessions().create("chat", "Chat").await.unwrap();
        sqlx::query(
            "UPDATE sessions SET persona_mode = 'persona', persona_id = NULL WHERE id = $1",
        )
        .bind(&s.id)
        .execute(db.pool())
        .await
        .unwrap();

        assert!(db.sessions().heal_persona(&s.id, None).await.unwrap());
        let healed = db.sessions().get(&s.id).await.unwrap().unwrap();
        assert_eq!(healed.persona_mode, PersonaMode::Inherit);
    }

    /// A session that is not in `persona` mode at all is never rewritten.
    #[tokio::test]
    async fn heal_persona_leaves_the_other_modes_alone() {
        let db = db().await;
        let s = db.sessions().create("chat", "Chat").await.unwrap();
        db.sessions()
            .set_persona(&s.id, PersonaMode::None, None)
            .await
            .unwrap();

        assert!(!db.sessions().heal_persona(&s.id, None).await.unwrap());
        assert_eq!(
            db.sessions()
                .get(&s.id)
                .await
                .unwrap()
                .unwrap()
                .persona_mode,
            PersonaMode::None
        );
    }

    #[tokio::test]
    async fn set_persona_reports_an_unknown_session() {
        let db = db().await;
        assert!(!db
            .sessions()
            .set_persona("nope", PersonaMode::Inherit, None)
            .await
            .unwrap());
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
