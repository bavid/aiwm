//! The Colibri chat capability: drive one `job_type=colibri` body.
//!
//! Colibri's real constraint is system RAM, not VRAM — the scheduler (built
//! around a VRAM budget) never sees this, since `ColibriAdapter` always
//! reports `0` VRAM used (see that adapter's own doc comment). Instead, this
//! module runs its own RAM preflight right before generating: a **warn**, not
//! a block, because Colibri's own docs are explicit that insufficient RAM
//! just means more disk streaming (slower), never a crash — same shape as
//! the existing video-job RAM preflight (Phase 4.5).
//!
//! By the time `run` executes, the engine has already put the model on
//! `ColibriAdapter`. Here we stream the answer from that resident model,
//! writing it into `jobs.result` as it grows so the UI can poll and watch it
//! appear — mirrors `capability::chat` almost exactly; the wire format is the
//! same OpenAI-compatible SSE shape.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::db::{Database, EventLevel, Model};
use crate::runtime::{ColibriAdapter, ColibriGenerationEvent as GenerationEvent};
use crate::{CoreError, Result};

/// How often the growing answer is flushed to `jobs.result`.
const FLUSH_INTERVAL: Duration = Duration::from_millis(200);
const DEFAULT_MAX_TOKENS: i32 = 512;
/// Free system RAM kept as a safety margin on top of the model's own resident
/// size before warning — OS + AIWM + everything else running normally.
const COLIBRI_RAM_SLACK_MB: u64 = 4096;

fn colibri_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "colibri".into(),
        message: msg.to_string(),
    }
}

/// What the user asked for, pulled from a job's `params`.
#[derive(Debug, Clone)]
pub struct ColibriChatRequest {
    pub prompt: String,
    pub max_tokens: i32,
}

impl ColibriChatRequest {
    pub fn from_params(params: &Value) -> Result<Self> {
        let prompt = params
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| colibri_err("colibri job has no `prompt`"))?
            .to_string();
        let max_tokens = params
            .get("max_tokens")
            .and_then(Value::as_i64)
            .filter(|n| *n > 0)
            .and_then(|n| i32::try_from(n).ok())
            .unwrap_or(DEFAULT_MAX_TOKENS);
        Ok(Self { prompt, max_tokens })
    }
}

/// Result of a finished Colibri chat body.
#[derive(Debug, Clone)]
pub struct ColibriDone {
    pub text: String,
    pub tokens: u32,
}

/// How the Colibri chat body came to rest.
#[derive(Debug, Clone)]
pub enum ColibriOutcome {
    Done(ColibriDone),
    /// The user cancelled mid-generation; `partial` is what had streamed so far.
    Cancelled {
        partial: String,
    },
}

/// Stream the answer into `jobs.result`. Returns when generation finishes, the
/// stream errors, or `cancel` flips to `true` (the in-flight HTTP request is
/// dropped, which stops the server too).
pub async fn run(
    db: &Database,
    colibri: &Arc<ColibriAdapter>,
    job_id: &str,
    session_id: Option<&str>,
    model: &Model,
    req: ColibriChatRequest,
    mut cancel: watch::Receiver<bool>,
) -> Result<ColibriOutcome> {
    if let Some(short_by) = ram_shortfall_mb(model) {
        db.jobs()
            .append_event(
                job_id,
                EventLevel::Warn,
                &format!(
                    "system RAM is tight (~{short_by} MB short of {}'s resident size) \u{2014} \
                     colibri streams more from disk when RAM is short, so this may be slow. \
                     Close other apps to speed it up.",
                    model.name
                ),
            )
            .await?;
    }

    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!(
                "prompt sent ({} chars, up to {} tokens)",
                req.prompt.chars().count(),
                req.max_tokens
            ),
        )
        .await?;

    // Colibri speaks the same OpenAI-compatible `messages` array llama.cpp
    // does, so a persona works here exactly as it does in `capability::chat`.
    //
    // No `is_assistant` guard is needed on this path (unlike `capability::chat`,
    // which must keep Prompt Assistant completions persona-free): the Prompt
    // Assistant only ever submits `job_type: "chat"`, on `llamacpp` or on Auto,
    // so an `assistant_for` job can never reach this body.
    let system = crate::persona::prepare_for_job(db, job_id, session_id).await?;

    let (tx, mut rx) = mpsc::channel::<GenerationEvent>(64);
    let stream = tokio::spawn({
        let colibri = Arc::clone(colibri);
        let prompt = req.prompt.clone();
        let max_tokens = req.max_tokens;
        async move {
            colibri
                .stream_completion_with(&prompt, max_tokens, system.as_deref(), tx)
                .await
        }
    });

    let mut answer = String::new();
    let mut tokens = 0u32;
    let mut last_flush = Instant::now();

    loop {
        tokio::select! {
            biased;
            () = wait_until_set(&mut cancel) => {
                stream.abort();
                let _ = db.jobs().set_result(job_id, &answer).await;
                return Ok(ColibriOutcome::Cancelled { partial: answer });
            }
            event = rx.recv() => match event {
                Some(GenerationEvent::Token(chunk)) => {
                    answer.push_str(&chunk);
                    if last_flush.elapsed() >= FLUSH_INTERVAL {
                        db.jobs().set_result(job_id, &answer).await?;
                        last_flush = Instant::now();
                    }
                }
                Some(GenerationEvent::Done { tokens: t }) => {
                    tokens = t;
                }
                None => break,
            },
        }
    }

    // The stream task's Result carries any HTTP / parse error.
    stream
        .await
        .map_err(|e| colibri_err(format!("stream task panicked: {e}")))??;

    db.jobs().set_result(job_id, &answer).await?;
    Ok(ColibriOutcome::Done(ColibriDone {
        text: answer,
        tokens,
    }))
}

/// Resolve once `rx` holds `true`. If the sender is dropped without setting it
/// (should not happen — the engine keeps it alive), never resolve, so `select!`
/// falls through to the stream instead of a false cancel.
async fn wait_until_set(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// How many MB the model's recorded RAM need exceeds free system RAM by, or
/// `None` when there is enough headroom (or the model has no RAM estimate to
/// judge against — nothing to warn about). Reads live RAM via `sysinfo`.
fn ram_shortfall_mb(model: &Model) -> Option<u64> {
    let model_mb = u64::try_from(model.ram_estimate_mb?).ok()?;
    ram_shortfall(model_mb, available_ram_mb())
}

fn available_ram_mb() -> u64 {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    sys.available_memory() / (1024 * 1024)
}

/// `model_mb + COLIBRI_RAM_SLACK_MB` minus `available_mb`, when positive.
fn ram_shortfall(model_mb: u64, available_mb: u64) -> Option<u64> {
    model_mb
        .saturating_add(COLIBRI_RAM_SLACK_MB)
        .checked_sub(available_mb)
        .filter(|short| *short > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colibri_chat_request_from_params() {
        let r = ColibriChatRequest::from_params(&serde_json::json!({
            "prompt": "  Hello there  ",
            "max_tokens": 64
        }))
        .unwrap();
        assert_eq!(r.prompt, "Hello there");
        assert_eq!(r.max_tokens, 64);

        let d = ColibriChatRequest::from_params(&serde_json::json!({ "prompt": "hi" })).unwrap();
        assert_eq!(d.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn colibri_chat_request_rejects_a_missing_or_blank_prompt() {
        assert!(ColibriChatRequest::from_params(&serde_json::json!({})).is_err());
        assert!(ColibriChatRequest::from_params(&serde_json::json!({ "prompt": "   " })).is_err());
    }

    #[test]
    fn ram_shortfall_flags_a_tight_budget_but_not_a_comfortable_one() {
        // Qwen3.6 wants ~24 GB; 64 GB free is comfortable.
        assert_eq!(ram_shortfall(24_576, 65_536), None);
        // The same model against this project's actual 32 GB machine (a few
        // GB already used at idle) is genuinely tight.
        assert_eq!(
            ram_shortfall(24_576, 28_000),
            Some(24_576 + COLIBRI_RAM_SLACK_MB - 28_000)
        );
    }
}
