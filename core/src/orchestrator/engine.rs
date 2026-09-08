//! The job engine: pull a runnable job, ask the scheduler where its model goes,
//! act on that decision, and drive the job through the state machine — recording
//! every transition and event.
//!
//! WP-5 executes jobs as no-ops (the fake runtime does the "load"); real work
//! lands with the capability work packages.

use std::sync::Arc;

use serde::Serialize;

use super::JobState;
use crate::db::{Job, JobPatch, NewJob};
use crate::runtime::RuntimeRegistry;
use crate::scheduler::{Decision, PlanRequest, Scheduler};
use crate::{CoreError, Database, Result};

/// How a job came to rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum JobOutcome {
    Completed { job_id: String },
    Blocked { job_id: String, reason: String },
    Failed { job_id: String, error: String },
}

#[derive(Debug)]
pub struct JobEngine {
    db: Database,
    registry: RuntimeRegistry,
    scheduler: Arc<dyn Scheduler>,
}

impl JobEngine {
    pub fn new(db: Database, registry: RuntimeRegistry, scheduler: Arc<dyn Scheduler>) -> Self {
        Self {
            db,
            registry,
            scheduler,
        }
    }

    /// Queue a job.
    pub async fn submit(&self, new: NewJob) -> Result<Job> {
        self.db.jobs().insert(new).await
    }

    /// Mark crash-interrupted jobs as failed. Call once at startup.
    pub async fn recover(&self) -> Result<u64> {
        let n = self.db.jobs().recover_interrupted().await?;
        if n > 0 {
            tracing::warn!(count = n, "recovered interrupted jobs as failed");
        }
        Ok(n)
    }

    /// Process the next runnable job to a resting state. `None` when idle.
    pub async fn run_next(&self) -> Result<Option<JobOutcome>> {
        let Some(job) = self.db.jobs().next_runnable().await? else {
            return Ok(None);
        };
        Ok(Some(self.drive(job).await))
    }

    async fn drive(&self, job: Job) -> JobOutcome {
        let job_id = job.id.clone();
        match self.try_drive(job).await {
            Ok(outcome) => outcome,
            Err(err) => {
                let error = err.to_string();
                let _ = self
                    .db
                    .jobs()
                    .set_state(
                        &job_id,
                        JobState::Failed,
                        JobPatch {
                            error_text: Some(error.clone()),
                            set_finished_at: true,
                            ..Default::default()
                        },
                    )
                    .await;
                JobOutcome::Failed { job_id, error }
            }
        }
    }

    async fn try_drive(&self, mut job: Job) -> Result<JobOutcome> {
        let runtime_id = job.runtime_id.clone().ok_or_else(|| CoreError::Runtime {
            runtime: "?".into(),
            message: format!("job {} has no runtime", job.id),
        })?;
        let model_id = job.model_id.clone().ok_or_else(|| CoreError::Runtime {
            runtime: runtime_id.clone(),
            message: format!("job {} has no model", job.id),
        })?;

        self.to(&mut job, JobState::Scheduled, JobPatch::default())
            .await?;

        let request = PlanRequest {
            job_id: job.id.clone(),
            runtime_id: runtime_id.clone(),
            model_id: model_id.clone(),
            vram_needed_mb: job.vram_needed_mb(),
            is_agent_session: job.is_agent_session(),
        };

        match self.scheduler.plan(&request).await {
            Decision::Blocked { reason } => {
                self.to(
                    &mut job,
                    JobState::Blocked,
                    JobPatch {
                        error_text: Some(reason.clone()),
                        ..Default::default()
                    },
                )
                .await?;
                return Ok(JobOutcome::Blocked {
                    job_id: job.id,
                    reason,
                });
            }
            Decision::RunNow => {
                self.to(&mut job, JobState::Preparing, JobPatch::default())
                    .await?;
            }
            Decision::LoadThenRun => {
                self.to(&mut job, JobState::Preparing, JobPatch::default())
                    .await?;
                self.load(&runtime_id, &model_id, request.vram_needed_mb)
                    .await?;
            }
            Decision::EvictThenLoad { victim_model } => {
                self.to(&mut job, JobState::Preparing, JobPatch::default())
                    .await?;
                self.evict(&victim_model).await?;
                self.load(&runtime_id, &model_id, request.vram_needed_mb)
                    .await?;
            }
        }

        if request.is_agent_session {
            self.scheduler.pin(&model_id);
        }

        self.to(
            &mut job,
            JobState::Running,
            JobPatch {
                set_started_at: true,
                ..Default::default()
            },
        )
        .await?;

        // --- job body: no-op in WP-5 ---

        self.to(&mut job, JobState::Post, JobPatch::default())
            .await?;
        self.to(
            &mut job,
            JobState::Completed,
            JobPatch {
                set_finished_at: true,
                ..Default::default()
            },
        )
        .await?;

        Ok(JobOutcome::Completed { job_id: job.id })
    }

    async fn load(&self, runtime_id: &str, model_id: &str, vram_mb: u64) -> Result<()> {
        let rt = self
            .registry
            .get(runtime_id)
            .ok_or_else(|| CoreError::Runtime {
                runtime: runtime_id.to_string(),
                message: "runtime not registered".into(),
            })?;
        rt.load_model(model_id, vram_mb).await
    }

    async fn evict(&self, model_id: &str) -> Result<()> {
        if let Some(rt) = self.registry.runtime_with_model(model_id) {
            self.scheduler.unpin(model_id);
            rt.unload_model(model_id).await?;
        }
        Ok(())
    }

    async fn to(&self, job: &mut Job, next: JobState, patch: JobPatch) -> Result<()> {
        self.db.jobs().set_state(&job.id, next, patch).await?;
        job.state = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::runtime::{FakeRuntimeAdapter, RuntimeAdapter};
    use crate::scheduler::HybridScheduler;

    struct Fixture {
        engine: JobEngine,
        db: Database,
        rt: Arc<FakeRuntimeAdapter>,
        scheduler: Arc<HybridScheduler>,
    }

    async fn fixture(budget_mb: u64) -> Fixture {
        let db = Database::connect_in_memory().await.unwrap();
        let registry = RuntimeRegistry::new();
        let rt = Arc::new(FakeRuntimeAdapter::healthy("llamacpp"));
        registry.register(rt.clone());
        let scheduler = Arc::new(HybridScheduler::new(registry.clone(), budget_mb));
        let engine = JobEngine::new(db.clone(), registry, scheduler.clone());
        Fixture {
            engine,
            db,
            rt,
            scheduler,
        }
    }

    #[tokio::test]
    async fn happy_path_runs_a_job_to_completion() {
        let fx = fixture(16_384).await;
        let job = fx
            .engine
            .submit(NewJob::new("chat").on("llamacpp", "qwen-14b", 9_000))
            .await
            .unwrap();

        let outcome = fx.engine.run_next().await.unwrap().unwrap();
        assert_eq!(
            outcome,
            JobOutcome::Completed {
                job_id: job.id.clone()
            }
        );

        let stored = fx.db.jobs().get(&job.id).await.unwrap().unwrap();
        assert_eq!(stored.state, JobState::Completed);
        assert!(stored.started_at.is_some() && stored.finished_at.is_some());
        assert_eq!(fx.rt.vram_used_mb(), 9_000);

        // Every transition left an event trail.
        let events = fx.db.jobs().events(&job.id).await.unwrap();
        let messages: Vec<_> = events.iter().map(|e| e.message.as_str()).collect();
        assert!(messages
            .iter()
            .any(|m| m.contains("scheduled -> preparing")));
        assert!(messages.iter().any(|m| m.contains("post -> completed")));
    }

    #[tokio::test]
    async fn run_next_is_none_when_idle() {
        let fx = fixture(16_384).await;
        assert!(fx.engine.run_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn job_without_runtime_fails_cleanly() {
        let fx = fixture(16_384).await;
        let job = fx.engine.submit(NewJob::new("chat")).await.unwrap();

        let outcome = fx.engine.run_next().await.unwrap().unwrap();
        assert!(matches!(outcome, JobOutcome::Failed { .. }));
        let stored = fx.db.jobs().get(&job.id).await.unwrap().unwrap();
        assert_eq!(stored.state, JobState::Failed);
        assert!(stored.error_text.unwrap().contains("no runtime"));
    }

    #[tokio::test]
    async fn simulated_load_failure_fails_the_job() {
        let db = Database::connect_in_memory().await.unwrap();
        let registry = RuntimeRegistry::new();
        let rt = Arc::new(FakeRuntimeAdapter::new(crate::runtime::FakeConfig {
            fail_load_for: vec!["broken".to_string()],
            ..Default::default()
        }));
        registry.register(rt.clone());
        let scheduler = Arc::new(HybridScheduler::new(registry.clone(), 16_384));
        let engine = JobEngine::new(db.clone(), registry, scheduler);

        let job = engine
            .submit(NewJob::new("chat").on("fake", "broken", 100))
            .await
            .unwrap();
        let outcome = engine.run_next().await.unwrap().unwrap();

        assert!(matches!(outcome, JobOutcome::Failed { .. }));
        assert_eq!(
            db.jobs().get(&job.id).await.unwrap().unwrap().state,
            JobState::Failed
        );
    }

    #[tokio::test]
    async fn blocked_job_stays_blocked_and_is_picked_up_again() {
        // Tiny budget: 2000 usable after margins. A 1500 MB job fits; then a
        // pinned session holds it and a second 1500 MB job is blocked.
        let fx = fixture(2_000 + 1024 + 512).await;

        let a = fx
            .engine
            .submit(
                NewJob::new("agent")
                    .on("llamacpp", "agent-model", 1_500)
                    .agent_session(),
            )
            .await
            .unwrap();
        assert!(matches!(
            fx.engine.run_next().await.unwrap().unwrap(),
            JobOutcome::Completed { .. }
        ));
        assert!(fx.scheduler.is_pinned("agent-model"));

        let b = fx
            .engine
            .submit(NewJob::new("chat").on("llamacpp", "other-model", 1_500))
            .await
            .unwrap();
        let outcome = fx.engine.run_next().await.unwrap().unwrap();
        match outcome {
            JobOutcome::Blocked { job_id, reason } => {
                assert_eq!(job_id, b.id);
                assert!(reason.contains("agent-model"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
        assert_eq!(
            fx.db.jobs().get(&b.id).await.unwrap().unwrap().state,
            JobState::Blocked
        );

        // Session ends: unpin, unload, and the blocked job now goes through.
        fx.scheduler.unpin("agent-model");
        fx.rt.unload_model("agent-model").await.unwrap();
        assert!(matches!(
            fx.engine.run_next().await.unwrap().unwrap(),
            JobOutcome::Completed { job_id } if job_id == b.id
        ));
        let _ = a;
    }

    #[tokio::test]
    async fn evict_then_load_swaps_models() {
        // 14848 usable. Load a 10 GB model, then a 6 GB job forces its eviction.
        let fx = fixture(16_384).await;

        fx.engine
            .submit(NewJob::new("chat").on("llamacpp", "big", 10_000))
            .await
            .unwrap();
        fx.engine.run_next().await.unwrap();
        assert_eq!(fx.rt.vram_used_mb(), 10_000);

        fx.engine
            .submit(NewJob::new("chat").on("llamacpp", "small", 6_000))
            .await
            .unwrap();
        assert!(matches!(
            fx.engine.run_next().await.unwrap().unwrap(),
            JobOutcome::Completed { .. }
        ));

        let loaded: Vec<_> = fx
            .rt
            .loaded_models()
            .into_iter()
            .map(|m| m.model_id)
            .collect();
        assert_eq!(loaded, ["small"]);
    }

    #[tokio::test]
    async fn recover_marks_interrupted_jobs_failed() {
        let fx = fixture(16_384).await;
        let job = fx
            .engine
            .submit(NewJob::new("chat").on("llamacpp", "m", 1_000))
            .await
            .unwrap();
        // Simulate a crash mid-run.
        fx.db
            .jobs()
            .set_state(&job.id, JobState::Scheduled, JobPatch::default())
            .await
            .unwrap();
        fx.db
            .jobs()
            .set_state(&job.id, JobState::Preparing, JobPatch::default())
            .await
            .unwrap();
        fx.db
            .jobs()
            .set_state(&job.id, JobState::Running, JobPatch::default())
            .await
            .unwrap();

        assert_eq!(fx.engine.recover().await.unwrap(), 1);
        assert_eq!(
            fx.db.jobs().get(&job.id).await.unwrap().unwrap().state,
            JobState::Failed
        );
    }
}
