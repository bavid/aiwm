//! Job and job-event persistence. The lifecycle rules live in
//! [`crate::orchestrator::state`]; this module only stores and queries.

use serde::Serialize;
use sqlx::{AssertSqlSafe, SqlitePool};
use uuid::Uuid;

use super::now_rfc3339;
use crate::orchestrator::JobState;
use crate::{CoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLevel {
    Info,
    Warn,
    Error,
}

impl EventLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// A job as stored.
#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub job_type: String,
    pub capability: Option<String>,
    pub state: JobState,
    pub params: serde_json::Value,
    pub runtime_id: Option<String>,
    pub model_id: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error_text: Option<String>,
    pub output_path: Option<String>,
}

/// Fields a caller supplies when queueing a job.
#[derive(Debug, Clone)]
pub struct NewJob {
    pub job_type: String,
    pub capability: Option<String>,
    pub runtime_id: Option<String>,
    pub model_id: Option<String>,
    pub params: serde_json::Value,
}

impl NewJob {
    pub fn new(job_type: impl Into<String>) -> Self {
        Self {
            job_type: job_type.into(),
            capability: None,
            runtime_id: None,
            model_id: None,
            params: serde_json::json!({}),
        }
    }

    /// Target a specific runtime + model, with the scheduler's VRAM estimate.
    pub fn on(
        mut self,
        runtime_id: impl Into<String>,
        model_id: impl Into<String>,
        vram_mb: u64,
    ) -> Self {
        self.runtime_id = Some(runtime_id.into());
        self.model_id = Some(model_id.into());
        self.params["vram_needed_mb"] = vram_mb.into();
        self
    }

    /// Mark this job as owned by a long-running agent session (pins the model).
    pub fn agent_session(mut self) -> Self {
        self.params["agent_session"] = true.into();
        self
    }
}

impl Job {
    /// The scheduler's VRAM estimate for this job's model (0 if unset).
    pub fn vram_needed_mb(&self) -> u64 {
        self.params
            .get("vram_needed_mb")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    }

    pub fn is_agent_session(&self) -> bool {
        self.params
            .get("agent_session")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }
}

/// Extra columns to update alongside a state change.
#[derive(Debug, Default)]
pub struct JobPatch {
    pub error_text: Option<String>,
    pub output_path: Option<String>,
    pub set_started_at: bool,
    pub set_finished_at: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobEvent {
    pub ts: String,
    pub level: EventLevel,
    pub message: String,
}

#[derive(Debug, Default, Clone)]
pub struct JobFilter {
    /// Restrict to these states (empty = any).
    pub states: Vec<JobState>,
    pub limit: Option<u32>,
}

#[derive(Debug)]
pub struct JobRepo<'a> {
    pool: &'a SqlitePool,
}

#[derive(sqlx::FromRow)]
struct JobRow {
    id: String,
    job_type: String,
    capability: Option<String>,
    state: String,
    params_json: String,
    runtime_id: Option<String>,
    model_id: Option<String>,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    error_text: Option<String>,
    output_path: Option<String>,
}

// The `SELECT` statements below interpolate only this compile-time constant,
// `$N` bind placeholders, and integer limits — never caller data (always bound).
// `AssertSqlSafe` documents that we have checked this.
const SELECT_COLS: &str = "id, type AS job_type, capability, state, params_json, \
     runtime_id, model_id, created_at, started_at, finished_at, error_text, output_path";

impl TryFrom<JobRow> for Job {
    type Error = CoreError;

    fn try_from(r: JobRow) -> Result<Self> {
        Ok(Self {
            id: r.id,
            job_type: r.job_type,
            capability: r.capability,
            state: r.state.parse()?,
            params: serde_json::from_str(&r.params_json)
                .map_err(|e| CoreError::Db(format!("bad params_json: {e}")))?,
            runtime_id: r.runtime_id,
            model_id: r.model_id,
            created_at: r.created_at,
            started_at: r.started_at,
            finished_at: r.finished_at,
            error_text: r.error_text,
            output_path: r.output_path,
        })
    }
}

impl<'a> JobRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewJob) -> Result<Job> {
        let id = Uuid::now_v7().to_string();
        let now = now_rfc3339();
        let params_json = serde_json::to_string(&new.params)
            .map_err(|e| CoreError::Db(format!("serialize params: {e}")))?;

        sqlx::query(
            "INSERT INTO jobs (id, type, capability, state, params_json, runtime_id, model_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&id)
        .bind(&new.job_type)
        .bind(&new.capability)
        .bind(JobState::Queued.as_str())
        .bind(&params_json)
        .bind(&new.runtime_id)
        .bind(&new.model_id)
        .bind(&now)
        .execute(self.pool)
        .await?;

        self.append_event(&id, EventLevel::Info, "job queued")
            .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("job vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<Job>> {
        let row: Option<JobRow> = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM jobs WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        row.map(Job::try_from).transpose()
    }

    pub async fn list(&self, filter: &JobFilter) -> Result<Vec<Job>> {
        let mut sql = format!("SELECT {SELECT_COLS} FROM jobs");
        if !filter.states.is_empty() {
            let placeholders = (1..=filter.states.len())
                .map(|i| format!("${i}"))
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(" WHERE state IN ({placeholders})"));
        }
        sql.push_str(" ORDER BY created_at DESC");
        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut query = sqlx::query_as::<_, JobRow>(AssertSqlSafe(sql));
        for st in &filter.states {
            query = query.bind(st.as_str());
        }
        query
            .fetch_all(self.pool)
            .await?
            .into_iter()
            .map(Job::try_from)
            .collect()
    }

    /// The oldest job the engine may act on (queued or blocked).
    pub async fn next_runnable(&self) -> Result<Option<Job>> {
        let row: Option<JobRow> = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM jobs
             WHERE state IN ('queued', 'blocked')
             ORDER BY created_at ASC LIMIT 1"
        )))
        .fetch_optional(self.pool)
        .await?;
        row.map(Job::try_from).transpose()
    }

    /// Move a job to `next`, validating the transition first, and record an event.
    pub async fn set_state(&self, id: &str, next: JobState, patch: JobPatch) -> Result<()> {
        let current = self
            .get(id)
            .await?
            .ok_or_else(|| CoreError::Db(format!("no job {id}")))?;
        current.state.ensure_transition(next)?;

        let now = now_rfc3339();
        let started = patch.set_started_at.then(|| now.clone());
        let finished = patch.set_finished_at.then(|| now.clone());

        sqlx::query(
            "UPDATE jobs SET
                 state = $1,
                 error_text = COALESCE($2, error_text),
                 output_path = COALESCE($3, output_path),
                 started_at = COALESCE($4, started_at),
                 finished_at = COALESCE($5, finished_at)
             WHERE id = $6",
        )
        .bind(next.as_str())
        .bind(&patch.error_text)
        .bind(&patch.output_path)
        .bind(&started)
        .bind(&finished)
        .bind(id)
        .execute(self.pool)
        .await?;

        let level = if next == JobState::Failed {
            EventLevel::Error
        } else {
            EventLevel::Info
        };
        let msg = match &patch.error_text {
            Some(text) => format!("{} -> {}: {text}", current.state.as_str(), next.as_str()),
            None => format!("{} -> {}", current.state.as_str(), next.as_str()),
        };
        self.append_event(id, level, &msg).await
    }

    pub async fn append_event(&self, id: &str, level: EventLevel, message: &str) -> Result<()> {
        sqlx::query("INSERT INTO job_events (job_id, ts, level, message) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(now_rfc3339())
            .bind(level.as_str())
            .bind(message)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn events(&self, id: &str) -> Result<Vec<JobEvent>> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT ts, level, message FROM job_events WHERE job_id = $1 ORDER BY ts ASC, rowid ASC",
        )
        .bind(id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(ts, level, message)| JobEvent {
                ts,
                level: match level.as_str() {
                    "warn" => EventLevel::Warn,
                    "error" => EventLevel::Error,
                    _ => EventLevel::Info,
                },
                message,
            })
            .collect())
    }

    /// Crash recovery: any job left mid-flight by a previous run is marked
    /// failed. Returns how many were recovered.
    pub async fn recover_interrupted(&self) -> Result<u64> {
        let stuck: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM jobs WHERE state IN ('scheduled', 'preparing', 'running', 'post')",
        )
        .fetch_all(self.pool)
        .await?;

        for (id,) in &stuck {
            sqlx::query(
                "UPDATE jobs SET state = 'failed',
                     error_text = 'interrupted by a previous shutdown',
                     finished_at = $1
                 WHERE id = $2",
            )
            .bind(now_rfc3339())
            .bind(id)
            .execute(self.pool)
            .await?;
            self.append_event(id, EventLevel::Error, "recovered as failed after shutdown")
                .await?;
        }
        Ok(stuck.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;
    use crate::db::{EventLevel, JobFilter, JobPatch, NewJob};
    use crate::orchestrator::JobState;

    async fn db() -> Database {
        Database::connect_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn insert_starts_queued_with_an_event() {
        let db = db().await;
        let job = db.jobs().insert(NewJob::new("chat")).await.unwrap();

        assert_eq!(job.state, JobState::Queued);
        let events = db.jobs().events(&job.id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].message, "job queued");
    }

    #[tokio::test]
    async fn set_state_validates_the_transition() {
        let db = db().await;
        let job = db.jobs().insert(NewJob::new("chat")).await.unwrap();

        let bad = db
            .jobs()
            .set_state(&job.id, JobState::Running, JobPatch::default())
            .await
            .unwrap_err();
        assert!(matches!(bad, crate::CoreError::InvalidJobTransition { .. }));

        db.jobs()
            .set_state(&job.id, JobState::Scheduled, JobPatch::default())
            .await
            .unwrap();
        assert_eq!(
            db.jobs().get(&job.id).await.unwrap().unwrap().state,
            JobState::Scheduled
        );
    }

    #[tokio::test]
    async fn patch_sets_timestamps_and_error() {
        let db = db().await;
        let job = db.jobs().insert(NewJob::new("chat")).await.unwrap();
        let r = db.jobs();

        r.set_state(&job.id, JobState::Scheduled, JobPatch::default())
            .await
            .unwrap();
        r.set_state(&job.id, JobState::Preparing, JobPatch::default())
            .await
            .unwrap();
        r.set_state(
            &job.id,
            JobState::Running,
            JobPatch {
                set_started_at: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        r.set_state(
            &job.id,
            JobState::Failed,
            JobPatch {
                error_text: Some("boom".into()),
                set_finished_at: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let got = r.get(&job.id).await.unwrap().unwrap();
        assert_eq!(got.state, JobState::Failed);
        assert!(got.started_at.is_some());
        assert!(got.finished_at.is_some());
        assert_eq!(got.error_text.as_deref(), Some("boom"));

        let events = r.events(&job.id).await.unwrap();
        assert!(events
            .iter()
            .any(|e| e.level == EventLevel::Error && e.message.contains("boom")));
    }

    #[tokio::test]
    async fn next_runnable_is_fifo_over_queued_and_blocked() {
        let db = db().await;
        let a = db.jobs().insert(NewJob::new("a")).await.unwrap();
        let b = db.jobs().insert(NewJob::new("b")).await.unwrap();

        assert_eq!(db.jobs().next_runnable().await.unwrap().unwrap().id, a.id);

        // Push `a` to a non-runnable state; `b` becomes next.
        db.jobs()
            .set_state(&a.id, JobState::Scheduled, JobPatch::default())
            .await
            .unwrap();
        assert_eq!(db.jobs().next_runnable().await.unwrap().unwrap().id, b.id);
    }

    #[tokio::test]
    async fn list_filters_by_state() {
        let db = db().await;
        let a = db.jobs().insert(NewJob::new("a")).await.unwrap();
        let _b = db.jobs().insert(NewJob::new("b")).await.unwrap();
        db.jobs()
            .set_state(&a.id, JobState::Cancelled, JobPatch::default())
            .await
            .unwrap();

        let cancelled = db
            .jobs()
            .list(&JobFilter {
                states: vec![JobState::Cancelled],
                limit: None,
            })
            .await
            .unwrap();
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].id, a.id);
    }

    #[tokio::test]
    async fn recover_interrupted_fails_mid_flight_jobs_only() {
        let db = db().await;
        let r = db.jobs();
        let running = r.insert(NewJob::new("r")).await.unwrap();
        let queued = r.insert(NewJob::new("q")).await.unwrap();
        let done = r.insert(NewJob::new("d")).await.unwrap();

        r.set_state(&running.id, JobState::Scheduled, JobPatch::default())
            .await
            .unwrap();
        r.set_state(&running.id, JobState::Preparing, JobPatch::default())
            .await
            .unwrap();
        r.set_state(&running.id, JobState::Running, JobPatch::default())
            .await
            .unwrap();
        r.set_state(&done.id, JobState::Scheduled, JobPatch::default())
            .await
            .unwrap();
        r.set_state(&done.id, JobState::Cancelled, JobPatch::default())
            .await
            .unwrap();

        let recovered = r.recover_interrupted().await.unwrap();
        assert_eq!(recovered, 1);

        assert_eq!(
            r.get(&running.id).await.unwrap().unwrap().state,
            JobState::Failed
        );
        assert_eq!(
            r.get(&queued.id).await.unwrap().unwrap().state,
            JobState::Queued
        );
        assert_eq!(
            r.get(&done.id).await.unwrap().unwrap().state,
            JobState::Cancelled
        );
    }
}
