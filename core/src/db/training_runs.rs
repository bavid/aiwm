//! Training runs: a detached ai-toolkit process, tracked across app restarts.
//! See `0016_training_runs.sql` and design spec section 2. Unlike a `Job`, a
//! training run is not owned by the `JobEngine` — it survives independently
//! because the underlying process is launched detached and keeps running
//! (or gets interrupted) whether or not AIWM itself is up.

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{now_rfc3339, DatasetMode};
use crate::{CoreError, Result};

/// The training-run lifecycle. See design spec section 2:
/// `preparing → running → paused | interrupted → resuming → running →
/// finishing → completed | failed | cancelled`. `env_broken` is a state of
/// the trainer runtime, not of a run, and has no representation here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Preparing,
    Running,
    Paused,
    Interrupted,
    Resuming,
    Finishing,
    Completed,
    Failed,
    Cancelled,
}

impl RunState {
    pub const ALL: [RunState; 9] = [
        Self::Preparing,
        Self::Running,
        Self::Paused,
        Self::Interrupted,
        Self::Resuming,
        Self::Finishing,
        Self::Completed,
        Self::Failed,
        Self::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Interrupted => "interrupted",
            Self::Resuming => "resuming",
            Self::Finishing => "finishing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "preparing" => Some(Self::Preparing),
            "running" => Some(Self::Running),
            "paused" => Some(Self::Paused),
            "interrupted" => Some(Self::Interrupted),
            "resuming" => Some(Self::Resuming),
            "finishing" => Some(Self::Finishing),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// Terminal states never transition again.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Whether `self -> next` is a legal transition.
    pub fn can_transition_to(self, next: RunState) -> bool {
        use RunState::*;
        matches!(
            (self, next),
            (Preparing, Running)
                | (Preparing, Failed)
                | (Preparing, Cancelled)
                | (Running, Paused)
                | (Running, Interrupted)
                | (Running, Finishing)
                | (Running, Failed)
                | (Running, Cancelled)
                | (Paused, Resuming)
                | (Paused, Cancelled)
                | (Interrupted, Resuming)
                | (Interrupted, Cancelled)
                | (Resuming, Running)
                | (Resuming, Failed)
                | (Resuming, Cancelled)
                | (Finishing, Completed)
                | (Finishing, Failed)
        )
    }

    /// Validate a transition, returning an error naming both states.
    pub fn ensure_transition(self, next: RunState) -> Result<()> {
        if self.can_transition_to(next) {
            Ok(())
        } else {
            Err(CoreError::Config(format!(
                "training run: cannot go from {} to {}",
                self.as_str(),
                next.as_str()
            )))
        }
    }
}

/// Speed/quality trade-off applied on top of a `TrainingProfile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Fast,
    Balanced,
    Thorough,
}

impl Preset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Thorough => "thorough",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fast" => Some(Self::Fast),
            "balanced" => Some(Self::Balanced),
            "thorough" => Some(Self::Thorough),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrainingRun {
    pub id: String,
    pub name: String,
    pub profile_family: String,
    pub target_model_id: Option<String>,
    pub dataset_id: Option<String>,
    pub data_kind: DatasetMode,
    pub trigger_word: String,
    pub preset: Preset,
    pub hyperparams_json: String,
    pub sample_prompts_json: String,
    pub state: RunState,
    pub step: i64,
    pub total_steps: i64,
    pub last_loss: Option<f64>,
    pub last_checkpoint_at: Option<String>,
    pub pid: Option<i64>,
    pub work_dir: String,
    pub result_model_id: Option<String>,
    pub error_text: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewTrainingRun {
    pub name: String,
    pub profile_family: String,
    pub target_model_id: Option<String>,
    pub dataset_id: Option<String>,
    pub data_kind: DatasetMode,
    pub trigger_word: String,
    pub preset: Preset,
    pub hyperparams_json: String,
    pub sample_prompts_json: String,
    pub work_dir: String,
}

#[derive(sqlx::FromRow)]
struct TrainingRunRow {
    id: String,
    name: String,
    profile_family: String,
    target_model_id: Option<String>,
    dataset_id: Option<String>,
    data_kind: String,
    trigger_word: String,
    preset: String,
    hyperparams_json: String,
    sample_prompts_json: String,
    state: String,
    step: i64,
    total_steps: i64,
    last_loss: Option<f64>,
    last_checkpoint_at: Option<String>,
    pid: Option<i64>,
    work_dir: String,
    result_model_id: Option<String>,
    error_text: Option<String>,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}

impl TryFrom<TrainingRunRow> for TrainingRun {
    type Error = CoreError;

    fn try_from(r: TrainingRunRow) -> Result<Self> {
        let data_kind = DatasetMode::parse(&r.data_kind).ok_or_else(|| {
            CoreError::Db(format!("bad training run data_kind {:?}", r.data_kind))
        })?;
        let preset = Preset::parse(&r.preset)
            .ok_or_else(|| CoreError::Db(format!("bad training run preset {:?}", r.preset)))?;
        let state = RunState::parse(&r.state)
            .ok_or_else(|| CoreError::Db(format!("bad training run state {:?}", r.state)))?;
        Ok(Self {
            id: r.id,
            name: r.name,
            profile_family: r.profile_family,
            target_model_id: r.target_model_id,
            dataset_id: r.dataset_id,
            data_kind,
            trigger_word: r.trigger_word,
            preset,
            hyperparams_json: r.hyperparams_json,
            sample_prompts_json: r.sample_prompts_json,
            state,
            step: r.step,
            total_steps: r.total_steps,
            last_loss: r.last_loss,
            last_checkpoint_at: r.last_checkpoint_at,
            pid: r.pid,
            work_dir: r.work_dir,
            result_model_id: r.result_model_id,
            error_text: r.error_text,
            created_at: r.created_at,
            started_at: r.started_at,
            finished_at: r.finished_at,
        })
    }
}

const SELECT_COLS: &str = "id, name, profile_family, target_model_id, dataset_id, data_kind, \
     trigger_word, preset, hyperparams_json, sample_prompts_json, state, step, total_steps, \
     last_loss, last_checkpoint_at, pid, work_dir, result_model_id, error_text, created_at, \
     started_at, finished_at";

#[derive(Debug)]
pub struct TrainingRunRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> TrainingRunRepo<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, run: NewTrainingRun) -> Result<TrainingRun> {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO training_runs
                 (id, name, profile_family, target_model_id, dataset_id, data_kind,
                  trigger_word, preset, hyperparams_json, sample_prompts_json, state,
                  work_dir, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(&id)
        .bind(&run.name)
        .bind(&run.profile_family)
        .bind(&run.target_model_id)
        .bind(&run.dataset_id)
        .bind(run.data_kind.as_str())
        .bind(&run.trigger_word)
        .bind(run.preset.as_str())
        .bind(&run.hyperparams_json)
        .bind(&run.sample_prompts_json)
        .bind(RunState::Preparing.as_str())
        .bind(&run.work_dir)
        .bind(now_rfc3339())
        .execute(self.pool)
        .await?;
        self.get(&id)
            .await?
            .ok_or_else(|| CoreError::Db("training run vanished right after insert".into()))
    }

    pub async fn get(&self, id: &str) -> Result<Option<TrainingRun>> {
        let row = sqlx::query_as::<_, TrainingRunRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM training_runs WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        row.map(TrainingRun::try_from).transpose()
    }

    /// Newest first — the Training tab lists the most recent run on top.
    pub async fn list(&self) -> Result<Vec<TrainingRun>> {
        let rows = sqlx::query_as::<_, TrainingRunRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM training_runs ORDER BY created_at DESC, id DESC"
        )))
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(TrainingRun::try_from)
            .collect::<Result<Vec<_>>>()
    }

    /// Runs that still occupy (or are about to reoccupy) the GPU reservation:
    /// `running` and `resuming`. Used on app start to reattach the poller.
    pub async fn list_alive(&self) -> Result<Vec<TrainingRun>> {
        let rows = sqlx::query_as::<_, TrainingRunRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SELECT_COLS} FROM training_runs
             WHERE state IN ('running', 'resuming')
             ORDER BY created_at DESC, id DESC"
        )))
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(TrainingRun::try_from)
            .collect::<Result<Vec<_>>>()
    }

    /// Move a run to `next`, validating the transition first. Sets
    /// `started_at` the first time a run reaches `running`, and
    /// `finished_at` when it lands in a terminal state.
    pub async fn set_state(&self, id: &str, next: RunState) -> Result<()> {
        let current = self
            .get(id)
            .await?
            .ok_or_else(|| CoreError::Db(format!("no training run {id}")))?;
        current.state.ensure_transition(next)?;
        self.set_state_from(id, current.state, next).await
    }

    /// The compare-and-swap underlying [`Self::set_state`]: the `UPDATE`
    /// only applies if the row is still in `expected` state, so a writer
    /// that read a stale state (e.g. the poller observing a process death
    /// concurrently with a user's Cancel) can never clobber a state change
    /// that landed in between the read and the write.
    pub(crate) async fn set_state_from(
        &self,
        id: &str,
        expected: RunState,
        next: RunState,
    ) -> Result<()> {
        let now = now_rfc3339();
        let started = (next == RunState::Running).then(|| now.clone());
        let finished = next.is_terminal().then_some(now);

        let result = sqlx::query(
            "UPDATE training_runs SET
                 state = $1,
                 started_at = COALESCE(started_at, $2),
                 finished_at = COALESCE(finished_at, $3)
             WHERE id = $4 AND state = $5",
        )
        .bind(next.as_str())
        .bind(&started)
        .bind(&finished)
        .bind(id)
        .bind(expected.as_str())
        .execute(self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(CoreError::Config(format!(
                "training run {id}: state changed concurrently (expected {})",
                expected.as_str()
            )));
        }
        Ok(())
    }

    pub async fn set_progress(
        &self,
        id: &str,
        step: i64,
        total_steps: i64,
        last_loss: Option<f64>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE training_runs SET step = $1, total_steps = $2, last_loss = $3 WHERE id = $4",
        )
        .bind(step)
        .bind(total_steps)
        .bind(last_loss)
        .bind(id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_pid(&self, id: &str, pid: Option<i64>) -> Result<()> {
        sqlx::query("UPDATE training_runs SET pid = $1 WHERE id = $2")
            .bind(pid)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_checkpoint_at(&self, id: &str, when: &str) -> Result<()> {
        sqlx::query("UPDATE training_runs SET last_checkpoint_at = $1 WHERE id = $2")
            .bind(when)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_result(&self, id: &str, model_id: &str) -> Result<()> {
        sqlx::query("UPDATE training_runs SET result_model_id = $1 WHERE id = $2")
            .bind(model_id)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_error(&self, id: &str, error_text: &str) -> Result<()> {
        sqlx::query("UPDATE training_runs SET error_text = $1 WHERE id = $2")
            .bind(error_text)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM training_runs WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewDataset, NewModel};

    fn sample_run() -> NewTrainingRun {
        NewTrainingRun {
            name: "Anime style v1".into(),
            profile_family: "flux2_klein_4b".into(),
            target_model_id: None,
            dataset_id: None,
            data_kind: DatasetMode::Frames,
            trigger_word: "ghibli_xy".into(),
            preset: Preset::Fast,
            hyperparams_json: "{}".into(),
            sample_prompts_json: "[\"ghibli_xy portrait\"]".into(),
            work_dir: "E:\\Data\\training\\run-1".into(),
        }
    }

    #[test]
    fn run_state_round_trips_and_rejects_unknown() {
        for st in RunState::ALL {
            assert_eq!(RunState::parse(st.as_str()), Some(st));
        }
        assert_eq!(RunState::parse("not_a_state"), None);
    }

    #[test]
    fn only_allowed_transitions_pass() {
        use RunState::*;

        let allowed: &[(RunState, &[RunState])] = &[
            (Preparing, &[Running, Failed, Cancelled]),
            (
                Running,
                &[Paused, Interrupted, Finishing, Failed, Cancelled],
            ),
            (Paused, &[Resuming, Cancelled]),
            (Interrupted, &[Resuming, Cancelled]),
            (Resuming, &[Running, Failed, Cancelled]),
            (Finishing, &[Completed, Failed]),
        ];

        for (from, tos) in allowed {
            for to in RunState::ALL {
                let expected = tos.contains(&to);
                assert_eq!(
                    from.can_transition_to(to),
                    expected,
                    "{from:?} -> {to:?} expected {expected}"
                );
                if expected {
                    from.ensure_transition(to).unwrap();
                } else {
                    let err = from.ensure_transition(to).unwrap_err();
                    assert!(matches!(err, CoreError::Config(_)));
                }
            }
        }

        for term in [Completed, Failed, Cancelled] {
            assert!(term.is_terminal());
            for any in RunState::ALL {
                assert!(!term.can_transition_to(any));
                assert!(term.ensure_transition(any).is_err());
            }
        }
    }

    #[tokio::test]
    async fn create_get_list_and_progress_updates() {
        let db = Database::connect_in_memory().await.unwrap();
        let run = db.training_runs().create(sample_run()).await.unwrap();
        assert_eq!(run.state, RunState::Preparing);
        assert_eq!(run.step, 0);
        assert_eq!(run.total_steps, 0);
        assert_eq!(run.started_at, None);
        assert_eq!(run.finished_at, None);

        db.training_runs()
            .set_state(&run.id, RunState::Running)
            .await
            .unwrap();
        let running = db.training_runs().get(&run.id).await.unwrap().unwrap();
        assert_eq!(running.state, RunState::Running);
        assert!(running.started_at.is_some());
        assert_eq!(running.finished_at, None);

        db.training_runs()
            .set_progress(&run.id, 10, 100, Some(0.42))
            .await
            .unwrap();
        db.training_runs()
            .set_pid(&run.id, Some(4242))
            .await
            .unwrap();
        let progressed = db.training_runs().get(&run.id).await.unwrap().unwrap();
        assert_eq!(progressed.step, 10);
        assert_eq!(progressed.total_steps, 100);
        assert_eq!(progressed.last_loss, Some(0.42));
        assert_eq!(progressed.pid, Some(4242));

        // A second run so list() ordering and list_alive() filtering are meaningful.
        let other = db.training_runs().create(sample_run()).await.unwrap();
        db.training_runs()
            .set_state(&other.id, RunState::Running)
            .await
            .unwrap();
        db.training_runs()
            .set_state(&other.id, RunState::Interrupted)
            .await
            .unwrap();
        db.training_runs()
            .set_state(&other.id, RunState::Resuming)
            .await
            .unwrap();

        let all = db.training_runs().list().await.unwrap();
        assert_eq!(
            all.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec![other.id.as_str(), run.id.as_str()]
        );

        // `run` is still `running` and `other` is `resuming` at this point,
        // so both count as alive.
        let alive = db.training_runs().list_alive().await.unwrap();
        assert_eq!(
            alive.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec![other.id.as_str(), run.id.as_str()]
        );

        let lora = db
            .models()
            .insert(NewModel {
                name: "Anime style v1".into(),
                format: "safetensors".into(),
                file_path: "E:\\Models\\loras\\anime-style-v1.safetensors".into(),
                size_bytes: 128,
                source: "training:run".into(),
                roles: vec![],
                ..NewModel::default()
            })
            .await
            .unwrap();
        db.training_runs()
            .set_result(&run.id, &lora.id)
            .await
            .unwrap();
        let with_result = db.training_runs().get(&run.id).await.unwrap().unwrap();
        assert_eq!(
            with_result.result_model_id.as_deref(),
            Some(lora.id.as_str())
        );

        db.training_runs()
            .set_checkpoint_at(&run.id, "2026-09-17T00:00:00Z")
            .await
            .unwrap();
        let with_checkpoint = db.training_runs().get(&run.id).await.unwrap().unwrap();
        assert_eq!(
            with_checkpoint.last_checkpoint_at.as_deref(),
            Some("2026-09-17T00:00:00Z")
        );

        db.training_runs()
            .set_error(&run.id, "OOM during training step 3 times in a row")
            .await
            .unwrap();
        db.training_runs()
            .set_state(&run.id, RunState::Failed)
            .await
            .unwrap();
        let failed = db.training_runs().get(&run.id).await.unwrap().unwrap();
        assert_eq!(failed.state, RunState::Failed);
        assert!(failed.finished_at.is_some());
        assert_eq!(
            failed.error_text.as_deref(),
            Some("OOM during training step 3 times in a row")
        );
    }

    #[tokio::test]
    async fn deleting_a_dataset_keeps_the_run_with_a_null_dataset_id() {
        let db = Database::connect_in_memory().await.unwrap();
        let ds = db
            .datasets()
            .create(NewDataset {
                name: "Anime".into(),
                mode: DatasetMode::Frames,
                source_root: "E:\\Data\\Anime".into(),
                prep_job_id: None,
            })
            .await
            .unwrap();

        let mut new_run = sample_run();
        new_run.dataset_id = Some(ds.id.clone());
        let run = db.training_runs().create(new_run).await.unwrap();
        assert_eq!(run.dataset_id.as_deref(), Some(ds.id.as_str()));

        db.datasets().delete(&ds.id).await.unwrap();

        let got = db.training_runs().get(&run.id).await.unwrap().unwrap();
        assert_eq!(got.dataset_id, None);
    }

    #[tokio::test]
    async fn set_state_rejects_an_invalid_transition() {
        let db = Database::connect_in_memory().await.unwrap();
        let run = db.training_runs().create(sample_run()).await.unwrap();

        let err = db
            .training_runs()
            .set_state(&run.id, RunState::Completed)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::Config(_)));

        let unchanged = db.training_runs().get(&run.id).await.unwrap().unwrap();
        assert_eq!(unchanged.state, RunState::Preparing);
    }

    #[tokio::test]
    async fn list_alive_returns_only_running_and_resuming() {
        let db = Database::connect_in_memory().await.unwrap();

        let preparing = db.training_runs().create(sample_run()).await.unwrap();

        let running = db.training_runs().create(sample_run()).await.unwrap();
        db.training_runs()
            .set_state(&running.id, RunState::Running)
            .await
            .unwrap();

        let resuming = db.training_runs().create(sample_run()).await.unwrap();
        db.training_runs()
            .set_state(&resuming.id, RunState::Running)
            .await
            .unwrap();
        db.training_runs()
            .set_state(&resuming.id, RunState::Interrupted)
            .await
            .unwrap();
        db.training_runs()
            .set_state(&resuming.id, RunState::Resuming)
            .await
            .unwrap();

        let cancelled = db.training_runs().create(sample_run()).await.unwrap();
        db.training_runs()
            .set_state(&cancelled.id, RunState::Cancelled)
            .await
            .unwrap();

        let alive = db.training_runs().list_alive().await.unwrap();
        let alive_ids: Vec<&str> = alive.iter().map(|r| r.id.as_str()).collect();
        assert!(alive_ids.contains(&running.id.as_str()));
        assert!(alive_ids.contains(&resuming.id.as_str()));
        assert!(!alive_ids.contains(&preparing.id.as_str()));
        assert!(!alive_ids.contains(&cancelled.id.as_str()));
    }

    #[tokio::test]
    async fn set_state_refuses_a_concurrent_change() {
        let db = Database::connect_in_memory().await.unwrap();
        let run = db.training_runs().create(sample_run()).await.unwrap();
        db.training_runs()
            .set_state(&run.id, RunState::Running)
            .await
            .unwrap();

        // Simulate another writer (e.g. the poller noticing the process died)
        // moving the run to `cancelled` without going through this repo.
        sqlx::query("UPDATE training_runs SET state = 'cancelled' WHERE id = $1")
            .bind(&run.id)
            .execute(&db.pool)
            .await
            .unwrap();

        // A caller that read the run while it was still `running` (stale by
        // now) tries to move it to `paused`. The compare-and-swap must catch
        // that the row no longer matches the state it was validated against.
        let err = db
            .training_runs()
            .set_state_from(&run.id, RunState::Running, RunState::Paused)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::Config(_)));
        assert!(err.to_string().contains("changed concurrently"));

        let unchanged = db.training_runs().get(&run.id).await.unwrap().unwrap();
        assert_eq!(unchanged.state, RunState::Cancelled);
    }

    #[tokio::test]
    async fn deleting_the_result_model_nulls_result_model_id() {
        let db = Database::connect_in_memory().await.unwrap();
        let run = db.training_runs().create(sample_run()).await.unwrap();

        let lora = db
            .models()
            .insert(NewModel {
                name: "Anime style v1".into(),
                format: "safetensors".into(),
                file_path: "E:\\Models\\loras\\anime-style-v1.safetensors".into(),
                size_bytes: 128,
                source: "training:run".into(),
                roles: vec![],
                ..NewModel::default()
            })
            .await
            .unwrap();
        db.training_runs()
            .set_result(&run.id, &lora.id)
            .await
            .unwrap();
        assert_eq!(
            db.training_runs()
                .get(&run.id)
                .await
                .unwrap()
                .unwrap()
                .result_model_id
                .as_deref(),
            Some(lora.id.as_str())
        );

        db.models().delete(&lora.id).await.unwrap();

        let got = db.training_runs().get(&run.id).await.unwrap().unwrap();
        assert_eq!(got.result_model_id, None);
    }
}
