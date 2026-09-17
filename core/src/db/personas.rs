//! Personas: named presets of name + emoji icon + system prompt. A persona can
//! be active globally (settings key `chat.active_persona_id`) and a chat session
//! can override it (`sessions.persona_mode` / `sessions.persona_id`). This is
//! storage only — the limits, the resolution rule and the self-healing live in
//! [`crate::persona`].

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{AssertSqlSafe, SqlitePool};
use uuid::Uuid;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// The settings key holding the globally active persona's id. Absent (or
/// pointing at a deleted persona) means "no global persona".
pub const ACTIVE_PERSONA_KEY: &str = "chat.active_persona_id";

/// A stored persona.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Persona {
    pub id: String,
    pub name: String,
    /// One emoji, as text.
    pub icon: String,
    /// Passed to the model verbatim; the tool applies no content filter.
    pub system_prompt: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Persona {
    /// Write `persona: { id, name, icon }` back over a job's `params` object so
    /// the chat history still shows which persona answered after it has been
    /// renamed or deleted — same write-back mechanism as
    /// [`crate::capability::image::ImageRequest::apply_to`]. The prompt text is
    /// deliberately *not* copied into the job.
    pub fn apply_to(&self, params: &mut Value) {
        let Some(obj) = params.as_object_mut() else {
            return;
        };
        obj.insert(
            "persona".into(),
            json!({ "id": self.id, "name": self.name, "icon": self.icon }),
        );
    }
}

/// How one chat session picks its persona.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PersonaMode {
    /// Use whichever persona is active globally (the default).
    #[default]
    Inherit,
    /// Explicitly no persona for this chat, whatever is active globally.
    None,
    /// Use this session's own `persona_id`.
    Persona,
}

impl PersonaMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::None => "none",
            Self::Persona => "persona",
        }
    }

    /// Lenient on purpose: an unrecognised stored value reads back as
    /// `Inherit` (the neutral default) so one odd row can never break loading a
    /// session — the same posture as the rest of the chat path.
    pub(super) fn from_db(value: &str) -> Self {
        match value {
            "none" => Self::None,
            "persona" => Self::Persona,
            _ => Self::Inherit,
        }
    }
}

#[derive(sqlx::FromRow)]
struct PersonaRow {
    id: String,
    name: String,
    icon: String,
    system_prompt: String,
    created_at: String,
    updated_at: String,
}

impl From<PersonaRow> for Persona {
    fn from(r: PersonaRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            icon: r.icon,
            system_prompt: r.system_prompt,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

// The `SELECT` statements below interpolate only this compile-time constant
// and `$N` bind placeholders — never caller data (always bound). `AssertSqlSafe`
// documents that we have checked this.
const SELECT_COLS: &str = "id, name, icon, system_prompt, created_at, updated_at";

#[derive(Debug)]
pub struct PersonaRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> PersonaRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Stores what it is given; the caller has already run
    /// [`crate::persona::validate`].
    pub async fn create(&self, name: &str, icon: &str, system_prompt: &str) -> Result<Persona> {
        let id = Uuid::now_v7().to_string();
        let now = now_rfc3339();
        sqlx::query(
            "INSERT INTO personas (id, name, icon, system_prompt, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $5)",
        )
        .bind(&id)
        .bind(name)
        .bind(icon)
        .bind(system_prompt)
        .bind(&now)
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("persona vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Persona>> {
        let row = sqlx::query_as::<_, PersonaRow>(AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM personas WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Persona::from))
    }

    /// Every persona, by name — the order the manage dialog and the picker show.
    pub async fn list(&self) -> Result<Vec<Persona>> {
        let rows = sqlx::query_as::<_, PersonaRow>(AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM personas ORDER BY name COLLATE NOCASE, created_at"
        )))
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Persona::from).collect())
    }

    /// `None` when there is no such persona (the API turns that into a 404).
    pub async fn update(
        &self,
        id: &str,
        name: &str,
        icon: &str,
        system_prompt: &str,
    ) -> Result<Option<Persona>> {
        let affected = sqlx::query(
            "UPDATE personas SET name = $1, icon = $2, system_prompt = $3, updated_at = $4 \
             WHERE id = $5",
        )
        .bind(name)
        .bind(icon)
        .bind(system_prompt)
        .bind(now_rfc3339())
        .bind(id)
        .execute(self.pool)
        .await?
        .rows_affected();
        if affected == 0 {
            return Ok(None);
        }
        self.get(id).await
    }

    /// Deletes the persona and, in the same transaction, drops every reference
    /// to it: sessions pointing at it fall back to `inherit`, and the global
    /// active key is cleared when it names this persona. Nothing else can then
    /// observe a dangling id. Returns whether a row was actually removed —
    /// deleting an unknown id is not an error (same posture as sessions and
    /// documents).
    pub async fn delete(&self, id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let removed = sqlx::query("DELETE FROM personas WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        sqlx::query(
            "UPDATE sessions SET persona_mode = 'inherit', persona_id = NULL WHERE persona_id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM settings WHERE key = $1 AND value = $2")
            .bind(ACTIVE_PERSONA_KEY)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(removed > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn db() -> Database {
        Database::connect_in_memory().await.unwrap()
    }

    /// Migration 0018 is additive: a new `personas` table plus two columns on
    /// the *already `STRICT`* `sessions` table. SQLite accepts `ADD COLUMN` on a
    /// `STRICT` table as long as the column has a declared type and, when
    /// `NOT NULL`, a constant default — this pins that it really applied.
    #[tokio::test]
    async fn migration_0018_adds_personas_and_extends_the_strict_sessions_table() {
        let db = db().await;
        assert!(
            db.table_names()
                .await
                .unwrap()
                .contains(&"personas".to_string()),
            "migration 0018 must create the personas table"
        );

        let (sessions_sql,): (String,) =
            sqlx::query_as("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?")
                .bind("sessions")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(
            sessions_sql.to_uppercase().contains("STRICT"),
            "sessions is expected to be STRICT: {sessions_sql}"
        );

        // The two new columns exist and carry the documented defaults for a row
        // inserted by the existing repo code, which never mentions them.
        let session = db.sessions().create("chat", "Defaults").await.unwrap();
        let (mode, persona_id): (String, Option<String>) =
            sqlx::query_as("SELECT persona_mode, persona_id FROM sessions WHERE id = $1")
                .bind(&session.id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(mode, "inherit");
        assert_eq!(persona_id, None);
    }

    #[tokio::test]
    async fn create_and_get_round_trip() {
        let db = db().await;
        let p = db
            .personas()
            .create("Blunt", "🪓", "Answer in at most three sentences.")
            .await
            .unwrap();
        assert_eq!(p.name, "Blunt");
        assert_eq!(p.icon, "🪓");
        assert_eq!(p.system_prompt, "Answer in at most three sentences.");
        assert_eq!(p.created_at, p.updated_at, "fresh row: same timestamps");

        let fetched = db.personas().get(&p.id).await.unwrap().unwrap();
        assert_eq!(fetched, p);
    }

    #[tokio::test]
    async fn get_returns_none_for_an_unknown_id() {
        let db = db().await;
        assert!(db.personas().get("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_orders_by_name_case_insensitively() {
        let db = db().await;
        for name in ["zebra", "Apple", "mango"] {
            db.personas().create(name, "🙂", "be nice").await.unwrap();
        }
        let names: Vec<String> = db
            .personas()
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["Apple", "mango", "zebra"]);
    }

    #[tokio::test]
    async fn update_rewrites_every_field_and_touches_updated_at() {
        let db = db().await;
        let p = db
            .personas()
            .create("Old", "🙂", "old prompt")
            .await
            .unwrap();

        let updated = db
            .personas()
            .update(&p.id, "New", "🧑‍🏫", "new prompt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.id, p.id);
        assert_eq!(updated.name, "New");
        assert_eq!(updated.icon, "🧑‍🏫");
        assert_eq!(updated.system_prompt, "new prompt");
        assert_eq!(updated.created_at, p.created_at, "created_at is immutable");
    }

    #[tokio::test]
    async fn update_returns_none_for_an_unknown_id() {
        let db = db().await;
        assert!(db
            .personas()
            .update("nope", "N", "🙂", "p")
            .await
            .unwrap()
            .is_none());
    }

    /// The whole point of doing the delete in one transaction: afterwards
    /// nothing anywhere still names the deleted persona.
    #[tokio::test]
    async fn delete_clears_the_global_key_and_resets_pointing_sessions() {
        let db = db().await;
        let doomed = db.personas().create("Doomed", "💀", "p").await.unwrap();
        let kept = db.personas().create("Kept", "🙂", "p").await.unwrap();

        let pointing = db.sessions().create("chat", "Pointing").await.unwrap();
        let other = db.sessions().create("chat", "Other").await.unwrap();
        db.sessions()
            .set_persona(&pointing.id, PersonaMode::Persona, Some(&doomed.id))
            .await
            .unwrap();
        db.sessions()
            .set_persona(&other.id, PersonaMode::Persona, Some(&kept.id))
            .await
            .unwrap();
        db.settings()
            .set(ACTIVE_PERSONA_KEY, &doomed.id)
            .await
            .unwrap();

        assert!(db.personas().delete(&doomed.id).await.unwrap());

        assert!(db.personas().get(&doomed.id).await.unwrap().is_none());
        let healed = db.sessions().get(&pointing.id).await.unwrap().unwrap();
        assert_eq!(healed.persona_mode, PersonaMode::Inherit);
        assert_eq!(healed.persona_id, None);
        assert_eq!(
            db.settings().get(ACTIVE_PERSONA_KEY).await.unwrap(),
            None,
            "the global key named the deleted persona"
        );

        // An unrelated session and the other persona are untouched.
        let untouched = db.sessions().get(&other.id).await.unwrap().unwrap();
        assert_eq!(untouched.persona_mode, PersonaMode::Persona);
        assert_eq!(untouched.persona_id.as_deref(), Some(kept.id.as_str()));
        assert!(db.personas().get(&kept.id).await.unwrap().is_some());
    }

    /// Deleting persona A must not clear a global key that names persona B.
    #[tokio::test]
    async fn delete_leaves_a_global_key_naming_another_persona_alone() {
        let db = db().await;
        let a = db.personas().create("A", "🙂", "p").await.unwrap();
        let b = db.personas().create("B", "🙂", "p").await.unwrap();
        db.settings().set(ACTIVE_PERSONA_KEY, &b.id).await.unwrap();

        db.personas().delete(&a.id).await.unwrap();

        assert_eq!(
            db.settings()
                .get(ACTIVE_PERSONA_KEY)
                .await
                .unwrap()
                .as_deref(),
            Some(b.id.as_str())
        );
    }

    #[tokio::test]
    async fn delete_of_an_unknown_id_is_a_noop() {
        let db = db().await;
        assert!(!db.personas().delete("nope").await.unwrap());
    }

    #[test]
    fn mode_round_trips_through_its_stored_string_and_is_lenient_on_read() {
        for mode in [
            PersonaMode::Inherit,
            PersonaMode::None,
            PersonaMode::Persona,
        ] {
            assert_eq!(PersonaMode::from_db(mode.as_str()), mode);
        }
        assert_eq!(PersonaMode::from_db("gibberish"), PersonaMode::Inherit);
        assert_eq!(PersonaMode::default(), PersonaMode::Inherit);
    }

    #[test]
    fn mode_serialises_lowercase() {
        assert_eq!(
            serde_json::to_string(&PersonaMode::Persona).unwrap(),
            r#""persona""#
        );
        assert_eq!(
            serde_json::from_str::<PersonaMode>(r#""none""#).unwrap(),
            PersonaMode::None
        );
    }

    /// The job keeps the persona's identity, never its prompt text.
    #[test]
    fn apply_to_writes_id_name_icon_and_keeps_the_prompt_out() {
        let persona = Persona {
            id: "p1".into(),
            name: "Blunt".into(),
            icon: "🪓".into(),
            system_prompt: "secret instructions".into(),
            created_at: "t".into(),
            updated_at: "t".into(),
        };
        let mut params = json!({ "prompt": "hi", "max_tokens": 64 });
        persona.apply_to(&mut params);

        assert_eq!(
            params["persona"],
            json!({ "id": "p1", "name": "Blunt", "icon": "🪓" })
        );
        assert_eq!(params["prompt"], "hi", "existing params survive");
        assert!(
            !params.to_string().contains("secret instructions"),
            "the system prompt must not be copied into the job: {params}"
        );
    }

    #[test]
    fn apply_to_ignores_non_object_params() {
        let mut params = json!("not an object");
        Persona {
            id: "p1".into(),
            name: "N".into(),
            icon: "🙂".into(),
            system_prompt: "p".into(),
            created_at: "t".into(),
            updated_at: "t".into(),
        }
        .apply_to(&mut params);
        assert_eq!(params, json!("not an object"));
    }
}
