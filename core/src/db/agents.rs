//! Agent profile + session persistence (Phase 5). The lifecycle is driven by
//! `core::agent` + `capability::agent`; this module only stores and queries.
//! Session events are append-only, like `job_events`.

use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::now_rfc3339;
use crate::{CoreError, Result};

/// The state a session is in. Mirrors `agent_sessions.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionState {
    /// The runtime is coming up / the session is being created.
    Starting,
    /// Ready for a user turn.
    Idle,
    /// The agent is working on a turn (thinking, running tools).
    Working,
    /// A tool wants permission — the UI must answer before it continues.
    AwaitingApproval,
    /// Ended cleanly.
    Stopped,
    /// Ended with an error (`error_text`).
    Failed,
}

impl AgentSessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Idle => "idle",
            Self::Working => "working",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "starting" => Self::Starting,
            "idle" => Self::Idle,
            "working" => Self::Working,
            "awaiting_approval" => Self::AwaitingApproval,
            "stopped" => Self::Stopped,
            "failed" => Self::Failed,
            other => return Err(CoreError::Db(format!("bad agent session state {other:?}"))),
        })
    }

    /// Whether the session has finished (no more turns).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }
}

/// A stored agent profile.
#[derive(Debug, Clone, Serialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// `"opencode"` | `"hermes"`.
    pub adapter: String,
    /// Explicit model, or `None` for `Auto` over the `coding` role.
    pub model_id: Option<String>,
    pub workspace_path: String,
    /// Extra roots the agent may read (beyond the workspace).
    pub allowed_paths: Vec<String>,
    /// `None` = the adapter's default toolset.
    pub toolset: Option<Vec<String>>,
    pub created_at: String,
}

/// Fields a caller supplies to create a profile.
#[derive(Debug, Clone)]
pub struct NewAgent {
    pub name: String,
    pub adapter: String,
    pub model_id: Option<String>,
    pub workspace_path: String,
    pub allowed_paths: Vec<String>,
    pub toolset: Option<Vec<String>>,
}

/// A stored session.
#[derive(Debug, Clone, Serialize)]
pub struct AgentSession {
    pub id: String,
    pub agent_id: String,
    pub adapter_session_id: Option<String>,
    pub state: AgentSessionState,
    pub error_text: Option<String>,
    pub checkpoint_path: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
}

/// One transcript entry. `payload` shape depends on `kind` (see `core::agent`).
#[derive(Debug, Clone, Serialize)]
pub struct AgentSessionEvent {
    pub ts: String,
    pub kind: String,
    pub payload: Value,
}

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: String,
    name: String,
    adapter: String,
    model_id: Option<String>,
    workspace_path: String,
    allowed_paths_json: String,
    toolset_json: Option<String>,
    created_at: String,
}

impl TryFrom<AgentRow> for Agent {
    type Error = CoreError;
    fn try_from(r: AgentRow) -> Result<Self> {
        let allowed_paths = serde_json::from_str(&r.allowed_paths_json)
            .map_err(|e| CoreError::Db(format!("bad allowed_paths_json: {e}")))?;
        let toolset = match r.toolset_json {
            Some(j) => Some(
                serde_json::from_str(&j)
                    .map_err(|e| CoreError::Db(format!("bad toolset_json: {e}")))?,
            ),
            None => None,
        };
        Ok(Self {
            id: r.id,
            name: r.name,
            adapter: r.adapter,
            model_id: r.model_id,
            workspace_path: r.workspace_path,
            allowed_paths,
            toolset,
            created_at: r.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    agent_id: String,
    adapter_session_id: Option<String>,
    state: String,
    error_text: Option<String>,
    checkpoint_path: Option<String>,
    started_at: String,
    ended_at: Option<String>,
}

impl TryFrom<SessionRow> for AgentSession {
    type Error = CoreError;
    fn try_from(r: SessionRow) -> Result<Self> {
        Ok(Self {
            id: r.id,
            agent_id: r.agent_id,
            adapter_session_id: r.adapter_session_id,
            state: AgentSessionState::parse(&r.state)?,
            error_text: r.error_text,
            checkpoint_path: r.checkpoint_path,
            started_at: r.started_at,
            ended_at: r.ended_at,
        })
    }
}

#[derive(Debug)]
pub struct AgentRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> AgentRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    // --- profiles ---

    pub async fn create(&self, new: NewAgent) -> Result<Agent> {
        let id = Uuid::now_v7().to_string();
        let now = now_rfc3339();
        let allowed = serde_json::to_string(&new.allowed_paths).unwrap_or_else(|_| "[]".into());
        let toolset = new
            .toolset
            .as_ref()
            .map(|t| serde_json::to_string(t).unwrap_or_else(|_| "[]".into()));
        sqlx::query(
            "INSERT INTO agents
                 (id, name, adapter, model_id, workspace_path, allowed_paths_json, toolset_json, created_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(&id)
        .bind(&new.name)
        .bind(&new.adapter)
        .bind(&new.model_id)
        .bind(&new.workspace_path)
        .bind(&allowed)
        .bind(&toolset)
        .bind(&now)
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("agent vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Agent>> {
        let row = sqlx::query_as::<_, AgentRow>(
            "SELECT id, name, adapter, model_id, workspace_path, allowed_paths_json, toolset_json, \
             created_at FROM agents WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        row.map(Agent::try_from).transpose()
    }

    pub async fn list(&self) -> Result<Vec<Agent>> {
        let rows = sqlx::query_as::<_, AgentRow>(
            "SELECT id, name, adapter, model_id, workspace_path, allowed_paths_json, toolset_json, \
             created_at FROM agents ORDER BY created_at",
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter().map(Agent::try_from).collect()
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM agents WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    // --- sessions ---

    pub async fn create_session(&self, agent_id: &str) -> Result<AgentSession> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO agent_sessions (id, agent_id, state, started_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&id)
        .bind(agent_id)
        .bind(AgentSessionState::Starting.as_str())
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.session(&id)
            .await?
            .ok_or_else(|| CoreError::Db("session vanished right after insert".into()))
    }

    pub async fn session(&self, id: &str) -> Result<Option<AgentSession>> {
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT id, agent_id, adapter_session_id, state, error_text, checkpoint_path, \
             started_at, ended_at FROM agent_sessions WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        row.map(AgentSession::try_from).transpose()
    }

    pub async fn sessions_for(&self, agent_id: &str) -> Result<Vec<AgentSession>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT id, agent_id, adapter_session_id, state, error_text, checkpoint_path, \
             started_at, ended_at FROM agent_sessions WHERE agent_id = $1 ORDER BY started_at DESC",
        )
        .bind(agent_id)
        .fetch_all(self.pool)
        .await?;
        rows.into_iter().map(AgentSession::try_from).collect()
    }

    /// The sessions that have not finished — for crash recovery (slice 5.5).
    pub async fn live_sessions(&self) -> Result<Vec<AgentSession>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT id, agent_id, adapter_session_id, state, error_text, checkpoint_path, \
             started_at, ended_at FROM agent_sessions \
             WHERE state NOT IN ('stopped', 'failed') ORDER BY started_at",
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter().map(AgentSession::try_from).collect()
    }

    /// On startup: any session still `starting|idle|working|awaiting_approval`
    /// is orphaned — its runtime process died with the previous app. Mark them
    /// `failed` so the UI shows the truth (5.5). Returns how many were closed.
    pub async fn recover_orphaned(&self) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE agent_sessions
             SET state = 'failed',
                 error_text = COALESCE(error_text, 'the agent runtime did not survive an app restart'),
                 ended_at = COALESCE(ended_at, $1)
             WHERE state NOT IN ('stopped', 'failed')",
        )
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    pub async fn bind_adapter_session(&self, id: &str, adapter_session_id: &str) -> Result<()> {
        sqlx::query("UPDATE agent_sessions SET adapter_session_id = $1 WHERE id = $2")
            .bind(adapter_session_id)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Set the session state. Terminal states also stamp `ended_at`; `error`
    /// is stored only when moving to `Failed`.
    pub async fn set_session_state(
        &self,
        id: &str,
        state: AgentSessionState,
        error: Option<&str>,
    ) -> Result<()> {
        let ended = state.is_terminal().then(now_rfc3339);
        sqlx::query(
            "UPDATE agent_sessions SET
                 state = $1,
                 error_text = COALESCE($2, error_text),
                 ended_at = COALESCE($3, ended_at)
             WHERE id = $4",
        )
        .bind(state.as_str())
        .bind(error)
        .bind(&ended)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_checkpoint(&self, id: &str, path: &str) -> Result<()> {
        sqlx::query("UPDATE agent_sessions SET checkpoint_path = $1 WHERE id = $2")
            .bind(path)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    // --- transcript ---

    pub async fn append_event(&self, session_id: &str, kind: &str, payload: &Value) -> Result<()> {
        sqlx::query(
            "INSERT INTO agent_session_events (session_id, ts, kind, payload_json)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(session_id)
        .bind(now_rfc3339())
        .bind(kind)
        .bind(payload.to_string())
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn session_events(&self, session_id: &str) -> Result<Vec<AgentSessionEvent>> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT ts, kind, payload_json FROM agent_session_events
             WHERE session_id = $1 ORDER BY ts, rowid",
        )
        .bind(session_id)
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(|(ts, kind, payload_json)| {
                Ok(AgentSessionEvent {
                    ts,
                    kind,
                    payload: serde_json::from_str(&payload_json)
                        .map_err(|e| CoreError::Db(format!("bad event payload: {e}")))?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn new_agent() -> NewAgent {
        NewAgent {
            name: "Coder".into(),
            adapter: "opencode".into(),
            model_id: None,
            workspace_path: "E:\\proj\\demo".into(),
            allowed_paths: vec!["E:\\proj\\shared".into()],
            toolset: Some(vec!["bash".into(), "edit".into()]),
        }
    }

    #[tokio::test]
    async fn profile_round_trips_including_json_columns() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = db.agents().create(new_agent()).await.unwrap();
        assert_eq!(a.adapter, "opencode");
        assert_eq!(a.model_id, None);
        assert_eq!(a.allowed_paths, ["E:\\proj\\shared"]);
        assert_eq!(
            a.toolset.as_deref(),
            Some(&["bash".to_string(), "edit".to_string()][..])
        );

        let got = db.agents().get(&a.id).await.unwrap().unwrap();
        assert_eq!(got.id, a.id);
        assert_eq!(db.agents().list().await.unwrap().len(), 1);

        db.agents().delete(&a.id).await.unwrap();
        assert!(db.agents().get(&a.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn session_lifecycle_and_transcript() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = db.agents().create(new_agent()).await.unwrap();
        let s = db.agents().create_session(&a.id).await.unwrap();
        assert_eq!(s.state, AgentSessionState::Starting);
        assert!(s.ended_at.is_none());

        db.agents()
            .bind_adapter_session(&s.id, "ses_abc")
            .await
            .unwrap();
        db.agents()
            .set_session_state(&s.id, AgentSessionState::Working, None)
            .await
            .unwrap();
        db.agents()
            .append_event(
                &s.id,
                "text",
                &serde_json::json!({ "delta": "reading files" }),
            )
            .await
            .unwrap();
        db.agents()
            .append_event(
                &s.id,
                "permission",
                &serde_json::json!({ "id": "per_1", "command": "cargo test" }),
            )
            .await
            .unwrap();

        let events = db.agents().session_events(&s.id).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "text");
        assert_eq!(events[1].payload["command"], "cargo test");

        let mid = db.agents().session(&s.id).await.unwrap().unwrap();
        assert_eq!(mid.adapter_session_id.as_deref(), Some("ses_abc"));
        assert_eq!(mid.state, AgentSessionState::Working);

        db.agents()
            .set_session_state(&s.id, AgentSessionState::Failed, Some("llama-server died"))
            .await
            .unwrap();
        let done = db.agents().session(&s.id).await.unwrap().unwrap();
        assert_eq!(done.state, AgentSessionState::Failed);
        assert_eq!(done.error_text.as_deref(), Some("llama-server died"));
        assert!(done.ended_at.is_some());
        assert!(db.agents().live_sessions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deleting_a_profile_cascades_to_sessions_and_events() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = db.agents().create(new_agent()).await.unwrap();
        let s = db.agents().create_session(&a.id).await.unwrap();
        db.agents()
            .append_event(&s.id, "idle", &serde_json::json!({}))
            .await
            .unwrap();

        db.agents().delete(&a.id).await.unwrap();
        assert!(db.agents().session(&s.id).await.unwrap().is_none());
        assert!(db.agents().session_events(&s.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn recover_orphaned_fails_the_unfinished_and_leaves_the_rest() {
        let db = Database::connect_in_memory().await.unwrap();
        let a = db.agents().create(new_agent()).await.unwrap();
        let working = db.agents().create_session(&a.id).await.unwrap();
        db.agents()
            .set_session_state(&working.id, AgentSessionState::Working, None)
            .await
            .unwrap();
        let stopped = db.agents().create_session(&a.id).await.unwrap();
        db.agents()
            .set_session_state(&stopped.id, AgentSessionState::Stopped, None)
            .await
            .unwrap();

        assert_eq!(db.agents().recover_orphaned().await.unwrap(), 1);

        let w = db.agents().session(&working.id).await.unwrap().unwrap();
        assert_eq!(w.state, AgentSessionState::Failed);
        assert!(w.error_text.unwrap().contains("app restart"));
        assert!(w.ended_at.is_some());
        assert_eq!(
            db.agents()
                .session(&stopped.id)
                .await
                .unwrap()
                .unwrap()
                .state,
            AgentSessionState::Stopped
        );
        // A second sweep is a no-op.
        assert_eq!(db.agents().recover_orphaned().await.unwrap(), 0);
    }
}
