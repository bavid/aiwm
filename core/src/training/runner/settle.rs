//! What the runner makes of a run's on-disk state: reading the log and the
//! work directory ([`Runner::observe`]), deciding what a dead process means
//! ([`Runner::settle`]), moving the row ([`Runner::finish`]) and importing the
//! finished LoRA.
//!
//! Split out of [`super`] because nothing here decides anything about the
//! future — every function reconciles what already happened, which is exactly
//! why the poller, startup recovery and the pause/cancel paths can share it.

use std::path::{Path, PathBuf};
use std::sync::PoisonError;

use super::Runner;
use crate::db::{now_rfc3339, RunState, TrainingRun};
use crate::model::{import_model, ImportRequest};
use crate::runtime::RuntimeAdapter;
use crate::scheduler::Scheduler;
use crate::training::config::training_folder;
use crate::training::process::read_pid_file;
use crate::training::progress::{
    parse_marker, parse_progress, scan_work_dir, split_updates, tail_log, Marker,
};
use crate::training::TRAINING_MODEL_ID;
use crate::{CoreError, Result};

/// Per-run bookkeeping the poller carries between ticks: how far into the log
/// it has read, and the lifecycle markers it has seen so far. Deliberately
/// in-memory — after a restart [`Runner::recover`] rebuilds what it needs from
/// the log tail rather than persisting a parser cursor.
#[derive(Debug, Clone, Default)]
pub(super) struct PollState {
    pub(super) offset: u64,
    pub(super) completed: bool,
    pub(super) failure: Option<String>,
    pub(super) checkpoint_step: Option<u64>,
}

/// What one pass over a run's on-disk state found: the updated cursor and
/// markers, the run's state after any progress-driven promotion, and the
/// newest checkpoint it has written.
#[derive(Debug, Clone)]
pub(super) struct Observation {
    pub(super) poll: PollState,
    pub(super) state: RunState,
    pub(super) checkpoint: Option<PathBuf>,
}

/// A LoRA that reached the library. `warning` is set when the file itself
/// landed but the metadata on top of it could not be finalised — the run is
/// still linked to it, because an imported-but-unfindable LoRA is worse than
/// one with a stale family.
#[derive(Debug, Clone)]
pub(super) struct ImportedLora {
    pub(super) model_id: String,
    pub(super) warning: Option<String>,
}

impl Runner {
    pub(super) async fn poll_run(&self, run: &TrainingRun) -> Result<()> {
        // A launch still in flight. `finish` clears the PID when an attempt
        // ends, and `adopt` records the new one only once the trainer is
        // actually running, so a `preparing`/`resuming` row with no PID sits
        // between those two points. The stale PID file next to it belongs to
        // the attempt that already ended and would read as "dead" — which is
        // true, and entirely beside the point.
        if run.pid.is_none() && matches!(run.state, RunState::Preparing | RunState::Resuming) {
            return Ok(());
        }

        // Neither a recorded PID nor a PID file: there is no evidence about
        // this run at all, and "we cannot tell" is not "it died". Startup
        // recovery still settles such a row — there the absence *is* the
        // evidence, because nothing survived the restart.
        if run.pid.is_none()
            && read_pid_file(&self.work_dir(&run.id))
                .await
                .ok()
                .flatten()
                .is_none()
        {
            return Ok(());
        }

        // Liveness **before** the reads in `observe`, not after them. A
        // trainer writes its last words — the final checkpoint, the
        // completion block, an `Error running job:` — in the instant before
        // it exits, so a check made *after* the read can find the process
        // already gone while the chunk we read still ends a step or two short
        // of them. That is exactly how a finished run gets settled as
        // `interrupted` (reproduced by `tests/training_run.rs`: the log's
        // completion block landed between the read and the check). Checked
        // first, a `false` here means "everything this run will ever write is
        // already on disk", so the reads that follow are complete by
        // construction; a `true` costs nothing but one more poll interval
        // before the run settles.
        //
        // A `finishing` row is exempt: it has no process left, only an import
        // that did not get to run.
        let alive = run.state != RunState::Finishing
            && self.process_alive(run, &self.work_dir(&run.id)).await;

        let seen = self.observe(run).await?;
        if alive {
            return Ok(());
        }
        self.settle(&with_state(run, seen.state), &seen.poll, seen.checkpoint)
            .await
    }

    /// Read everything new off a run's disk state: the log tail (progress
    /// bars and lifecycle markers) and the work dir (the newest checkpoint).
    /// Shared by the poller, startup recovery and the pause/cancel paths, so
    /// all three judge a dead process from exactly the same evidence.
    pub(super) async fn observe(&self, run: &TrainingRun) -> Result<Observation> {
        let work_dir = self.work_dir(&run.id);
        let mut poll = self.poll_state(&run.id);
        let mut state = run.state;

        let (chunk, offset) = tail_log(&self.log_path(&run.id), poll.offset).await?;
        poll.offset = offset;

        let mut latest = None;
        for update in split_updates(&chunk) {
            if let Some(progress) = parse_progress(update) {
                latest = Some(progress);
                continue;
            }
            match parse_marker(update) {
                Some(Marker::Resuming(path)) => {
                    tracing::info!(run = %run.id, %path, "the trainer is resuming from a checkpoint");
                }
                Some(Marker::FoundStep(step)) => {
                    tracing::info!(run = %run.id, step, "the trainer picked up its previous step");
                }
                Some(Marker::Oom { attempt }) => {
                    tracing::warn!(
                        run = %run.id,
                        attempt,
                        "the trainer ran out of video memory and skipped a batch"
                    );
                }
                Some(Marker::OomAbort) => {
                    poll.failure = Some(
                        "ran out of video memory three times in a row — try a smaller \
                         resolution or rank"
                            .to_string(),
                    );
                }
                Some(Marker::Error(msg)) => poll.failure = Some(msg),
                Some(Marker::Completed) => poll.completed = true,
                Some(Marker::Stopped) => {
                    tracing::info!(run = %run.id, "the trainer reported a clean stop");
                }
                Some(Marker::SavedCheckpoint(path)) => {
                    tracing::info!(run = %run.id, %path, "the trainer saved a checkpoint");
                }
                None => {}
            }
        }

        if let Some(progress) = &latest {
            self.db
                .training_runs()
                .set_progress(
                    &run.id,
                    i64::try_from(progress.step).unwrap_or(i64::MAX),
                    i64::try_from(progress.total).unwrap_or(i64::MAX),
                    progress.loss.or(run.last_loss),
                )
                .await?;
            // A relaunched run is only really back once it prints a step.
            if state == RunState::Resuming
                && self
                    .transition(&run.id, RunState::Resuming, RunState::Running)
                    .await?
            {
                state = RunState::Running;
            }
        }

        let scanned = scan_work_dir(&training_folder(&work_dir).join(&run.name), &run.name)?;
        let checkpoint = scanned.latest_checkpoint;
        if let Some((step, _)) = &checkpoint {
            if poll.checkpoint_step != Some(*step) {
                poll.checkpoint_step = Some(*step);
                self.db
                    .training_runs()
                    .set_checkpoint_at(&run.id, &now_rfc3339())
                    .await?;
            }
        }
        self.set_poll_state(&run.id, poll.clone());

        Ok(Observation {
            poll,
            state,
            checkpoint: checkpoint.map(|(_, path)| path),
        })
    }

    /// What a dead process means, read off the log and the work dir.
    pub(super) async fn reconcile_dead(&self, run: &TrainingRun) -> Result<()> {
        let seen = self.observe(run).await?;
        self.settle(&with_state(run, seen.state), &seen.poll, seen.checkpoint)
            .await
    }

    /// The single place a dead run's outcome is decided: completion (with a
    /// checkpoint to import) wins, then an explicit failure marker, then
    /// `interrupted` — never `failed` on a silent disappearance.
    pub(super) async fn settle(
        &self,
        run: &TrainingRun,
        poll: &PollState,
        checkpoint: Option<PathBuf>,
    ) -> Result<()> {
        // A `finishing` row has already been judged complete — the verdict is
        // in the database, not in this tick's markers (which, after a
        // restart, no longer include the completion block). All that is left
        // is the import, which the app may have died in the middle of.
        if run.state == RunState::Finishing {
            return self.complete(run, checkpoint).await;
        }

        if poll.completed {
            if let Some(path) = checkpoint {
                if !self
                    .transition(&run.id, run.state, RunState::Finishing)
                    .await?
                {
                    // Someone else claimed this run between our read and now.
                    return Ok(());
                }
                return self
                    .complete(&with_state(run, RunState::Finishing), Some(path))
                    .await;
            }
            tracing::warn!(
                run = %run.id,
                "the trainer reported completion but left no checkpoint behind"
            );
        }
        match &poll.failure {
            Some(reason) => self
                .finish(run, RunState::Failed, Some(reason.clone()))
                .await
                .map(|_| ()),
            None => self
                .finish(run, RunState::Interrupted, None)
                .await
                .map(|_| ()),
        }
    }

    /// Turn a `finishing` run into a `completed` one by importing its
    /// checkpoint. Safe to re-enter: `import_model` deduplicates by SHA-256,
    /// so a retry after a crash mid-import finds the same library row rather
    /// than making a second copy.
    async fn complete(&self, run: &TrainingRun, checkpoint: Option<PathBuf>) -> Result<()> {
        let Some(path) = checkpoint else {
            return self
                .finish(
                    run,
                    RunState::Failed,
                    Some("the run reported success but left no checkpoint behind".to_string()),
                )
                .await
                .map(|_| ());
        };
        match self.import_result(run, &path).await {
            Ok(imported) => {
                self.db
                    .training_runs()
                    .set_result(&run.id, &imported.model_id)
                    .await?;
                self.finish(run, RunState::Completed, imported.warning)
                    .await
                    .map(|_| ())
            }
            Err(e) => self
                .finish(
                    run,
                    RunState::Failed,
                    Some(format!("the finished LoRA could not be imported: {e}")),
                )
                .await
                .map(|_| ()),
        }
    }

    /// Land a run in `next`, record `error` if there is one, and hand the GPU
    /// back.
    pub(super) async fn finish(
        &self,
        run: &TrainingRun,
        next: RunState,
        error: Option<String>,
    ) -> Result<bool> {
        if !self.transition(&run.id, run.state, next).await? {
            // Another writer settled this run first. Its own `finish` already
            // released the GPU; do not stamp our verdict over theirs.
            self.release(&run.id).await;
            return Ok(false);
        }
        if let Some(text) = &error {
            self.db.training_runs().set_error(&run.id, text).await?;
        }
        self.db.training_runs().set_pid(&run.id, None).await?;
        self.release(&run.id).await;
        Ok(true)
    }

    /// A state move that also knows the one detour the state machine needs:
    /// `resuming` has edges to `running`/`failed`/`cancelled` only, so a
    /// relaunched run that produced no progress line before dying reaches
    /// `interrupted` (or `finishing`) through `running` — it did run, it just
    /// never said so.
    async fn transition(&self, run_id: &str, from: RunState, next: RunState) -> Result<bool> {
        if from == next {
            return Ok(true);
        }
        let runs = self.db.training_runs();
        let moved = if from.can_transition_to(next) {
            runs.set_state_from(run_id, from, next).await
        } else if from == RunState::Resuming && RunState::Running.can_transition_to(next) {
            match runs
                .set_state_from(run_id, RunState::Resuming, RunState::Running)
                .await
            {
                Ok(()) => runs.set_state_from(run_id, RunState::Running, next).await,
                Err(e) => Err(e),
            }
        } else {
            return from.ensure_transition(next).map(|()| true);
        };

        match moved {
            Ok(()) => Ok(true),
            Err(e) if is_lost_race(&e) => {
                tracing::info!(
                    run = %run_id,
                    from = from.as_str(),
                    to = next.as_str(),
                    "a training-run state change was overtaken by another writer"
                );
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// Hand the GPU back and forget the run's poll cursor.
    pub(super) async fn release(&self, run_id: &str) {
        if self.adapter.alive_run().as_deref() == Some(run_id) {
            if let Err(e) = self.adapter.unload_model(TRAINING_MODEL_ID).await {
                tracing::warn!(run = %run_id, error = %e, "could not release the training reservation");
            }
            self.adapter.clear_alive();
        }
        self.scheduler.unpin(TRAINING_MODEL_ID);
        self.polls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(run_id);
    }

    /// Put the finished adapter in the library as a LoRA the Image tab can
    /// actually pick: `import_model` does the hashing, the store placement and
    /// the ComfyUI link, then the two fields it cannot know — the inference
    /// family and the `training:<run_id>` provenance — are set on the row.
    pub(super) async fn import_result(
        &self,
        run: &TrainingRun,
        checkpoint: &Path,
    ) -> Result<ImportedLora> {
        let outcome = import_model(
            &self.db,
            &self.store_root,
            ImportRequest {
                source_path: checkpoint.to_path_buf(),
                roles: Vec::new(),
                // The work dir stays intact: its checkpoints are the user's,
                // and a later resume needs them back.
                keep_original: true,
                model_type: Some("lora".to_string()),
            },
        )
        .await?;
        let model_id = outcome.model.id;

        // The file is in the store and the library row exists; only the
        // cosmetics are outstanding. Losing the *run* over those would leave
        // a LoRA nothing points at, so a second failure becomes a warning on
        // a completed run rather than an error that discards it.
        let warning = match self.finalise_metadata(&model_id, run).await {
            Ok(()) => None,
            Err(first) => {
                tracing::warn!(run = %run.id, error = %first, "retrying the LoRA's library entry");
                match self.finalise_metadata(&model_id, run).await {
                    Ok(()) => None,
                    Err(second) => Some(format!(
                        "the LoRA was imported but its library entry could not be \
                         finalised ({second}) — rename it and set its family by hand"
                    )),
                }
            }
        };

        tracing::info!(run = %run.id, model = %model_id, "imported a trained LoRA");
        Ok(ImportedLora { model_id, warning })
    }

    /// The two fields `import_model` cannot infer, plus the run's own name.
    async fn finalise_metadata(&self, model_id: &str, run: &TrainingRun) -> Result<()> {
        self.db
            .models()
            .set_family_and_source(
                model_id,
                Some(library_family(&run.profile_family)),
                &format!("training:{}", run.id),
            )
            .await?;
        self.db.models().rename(model_id, &run.name).await?;
        Ok(())
    }

    pub(super) fn poll_state(&self, run_id: &str) -> PollState {
        self.polls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .cloned()
            .unwrap_or_default()
    }

    /// PIDs [`Self::launch`]'s rollback has killed, oldest first.
    #[cfg(test)]
    pub(super) fn rolled_back(&self) -> Vec<u32> {
        self.rolled_back
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn set_poll_state(&self, run_id: &str, state: PollState) {
        self.polls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run_id.to_string(), state);
    }
}

/// The compare-and-swap in `TrainingRunRepo::set_state_from` reports a lost
/// race as exactly this error. Losing it is ordinary — the poller settling a
/// process that died while the user was clicking Cancel — not a failure, so
/// the callers that can lose it treat this as "no change" instead of
/// surfacing a scary message the user can do nothing about.
pub(super) fn is_lost_race(e: &CoreError) -> bool {
    matches!(e, CoreError::Config(msg) if msg.contains("state changed concurrently"))
}

/// The `models.family` a trained LoRA must carry for the Image tab's filter to
/// offer it next to its base model. The training profiles split FLUX.2 [klein]
/// by size (`flux2-klein-4b`/`-9b`) because the two train differently; the
/// library does not — both infer as `flux2`.
pub(super) fn library_family(profile_family: &str) -> &str {
    if profile_family.starts_with("flux2-klein") || profile_family.starts_with("flux2_klein") {
        "flux2"
    } else {
        profile_family
    }
}

/// A copy of `run` as the caller now knows it to be — the poller learns a
/// run's real state mid-tick and the helpers below take it by value.
pub(super) fn with_state(run: &TrainingRun, state: RunState) -> TrainingRun {
    TrainingRun {
        state,
        ..run.clone()
    }
}
