//! The chat / text-completion capability: drive one `job_type=chat` body.
//!
//! By the time this runs, the engine has already put the model on the GPU
//! (scheduler → `LlamaCppAdapter::load_model`). Here we stream the answer from
//! that resident model, writing it into `jobs.result` as it grows so the UI can
//! poll and watch it appear.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::db::{Database, EventLevel};
use crate::runtime::{GenerationEvent, GenerationOptions, LlamaCppAdapter};
use crate::{CoreError, Result};

/// How often the growing answer is flushed to `jobs.result`.
const FLUSH_INTERVAL: Duration = Duration::from_millis(200);
const DEFAULT_MAX_TOKENS: i32 = 512;

fn chat_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "llamacpp".into(),
        message: msg.to_string(),
    }
}

/// What the user asked for, pulled from a job's `params`.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub prompt: String,
    pub max_tokens: i32,
}

impl ChatRequest {
    pub fn from_params(params: &Value) -> Result<Self> {
        let prompt = params
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| chat_err("chat job has no `prompt`"))?
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

/// Result of a finished chat body.
#[derive(Debug, Clone)]
pub struct ChatDone {
    pub text: String,
    pub tokens: u32,
    pub tokens_per_second: f64,
}

/// How the chat body came to rest.
#[derive(Debug, Clone)]
pub enum ChatOutcome {
    Done(ChatDone),
    /// The user cancelled mid-generation; `partial` is what had streamed so far.
    Cancelled {
        partial: String,
    },
}

/// Stream the answer into `jobs.result`. Returns when generation finishes, the
/// stream errors, or `cancel` flips to `true` (the in-flight HTTP request is
/// dropped, which stops the server too). `session_id` — when the session has
/// documents attached — grounds the prompt in the most relevant chunks
/// (lexical search, [`crate::rag`]) before it reaches the model.
pub async fn run(
    db: &Database,
    llama: &Arc<LlamaCppAdapter>,
    job_id: &str,
    session_id: Option<&str>,
    req: ChatRequest,
    mut cancel: watch::Receiver<bool>,
) -> Result<ChatOutcome> {
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

    let system = crate::persona::prepare_for_job(db, job_id, session_id).await?;
    let prompt = ground_prompt(db, job_id, session_id, &req.prompt).await?;

    let (tx, mut rx) = mpsc::channel::<GenerationEvent>(64);
    let stream = tokio::spawn({
        let llama = Arc::clone(llama);
        let max_tokens = req.max_tokens;
        // No persona → `GenerationOptions::default()`, which serialises to
        // exactly the request body this path has always sent.
        let opts = match &system {
            Some(system) => GenerationOptions::with_system(system),
            None => GenerationOptions::default(),
        };
        async move {
            llama
                .stream_completion_with(&prompt, max_tokens, &opts, tx)
                .await
        }
    });

    let mut answer = String::new();
    let mut tokens = 0u32;
    let mut tokens_per_second = 0.0;
    let mut last_flush = Instant::now();

    loop {
        tokio::select! {
            biased;
            () = wait_until_set(&mut cancel) => {
                stream.abort();
                let _ = db.jobs().set_result(job_id, &answer).await;
                return Ok(ChatOutcome::Cancelled { partial: answer });
            }
            event = rx.recv() => match event {
                Some(GenerationEvent::Token(chunk)) => {
                    answer.push_str(&chunk);
                    if last_flush.elapsed() >= FLUSH_INTERVAL {
                        db.jobs().set_result(job_id, &answer).await?;
                        last_flush = Instant::now();
                    }
                }
                Some(GenerationEvent::Done { tokens: t, tokens_per_second: tps, .. }) => {
                    tokens = t;
                    tokens_per_second = tps;
                }
                None => break,
            },
        }
    }

    // The stream task's Result carries any HTTP / parse error.
    stream
        .await
        .map_err(|e| chat_err(format!("stream task panicked: {e}")))??;

    db.jobs().set_result(job_id, &answer).await?;
    Ok(ChatOutcome::Done(ChatDone {
        text: answer,
        tokens,
        tokens_per_second,
    }))
}

/// When `session_id` has documents attached, retrieve the most relevant
/// chunks (lexical search over every chunk in the session) and wrap the
/// prompt with them; otherwise the prompt passes through unchanged. A session
/// with no attached documents costs one empty query, not a full scan.
async fn ground_prompt(
    db: &Database,
    job_id: &str,
    session_id: Option<&str>,
    prompt: &str,
) -> Result<String> {
    let Some(session_id) = session_id else {
        return Ok(prompt.to_string());
    };
    let chunks = db.documents().chunks_for_session(session_id).await?;
    if chunks.is_empty() {
        return Ok(prompt.to_string());
    }

    let texts: Vec<String> = chunks.into_iter().map(|c| c.text).collect();
    let top = crate::rag::top_k_by_relevance(&texts, prompt, crate::rag::DEFAULT_TOP_K);
    if top.is_empty() {
        return Ok(prompt.to_string());
    }

    let retrieved: Vec<&str> = top.iter().map(|&i| texts[i].as_str()).collect();
    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!("grounded in {} attached-document chunk(s)", retrieved.len()),
        )
        .await?;
    Ok(crate::rag::build_grounded_prompt(prompt, &retrieved))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_from_params() {
        let r = ChatRequest::from_params(&serde_json::json!({
            "prompt": "  Hello there  ",
            "max_tokens": 64
        }))
        .unwrap();
        assert_eq!(r.prompt, "Hello there");
        assert_eq!(r.max_tokens, 64);

        let d = ChatRequest::from_params(&serde_json::json!({ "prompt": "hi" })).unwrap();
        assert_eq!(d.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn chat_request_rejects_a_missing_or_blank_prompt() {
        assert!(ChatRequest::from_params(&serde_json::json!({})).is_err());
        assert!(ChatRequest::from_params(&serde_json::json!({ "prompt": "   " })).is_err());
    }

    #[tokio::test]
    async fn ground_prompt_passes_through_without_a_session() {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db
            .jobs()
            .insert(crate::db::NewJob::new("chat"))
            .await
            .unwrap();
        let out = ground_prompt(&db, &job.id, None, "hello").await.unwrap();
        assert_eq!(out, "hello");
    }

    #[tokio::test]
    async fn ground_prompt_passes_through_when_the_session_has_no_documents() {
        let db = Database::connect_in_memory().await.unwrap();
        let job = db
            .jobs()
            .insert(crate::db::NewJob::new("chat"))
            .await
            .unwrap();
        let session = db.sessions().create("chat", "Empty").await.unwrap();
        let out = ground_prompt(&db, &job.id, Some(&session.id), "hello")
            .await
            .unwrap();
        assert_eq!(out, "hello");
    }

    #[tokio::test]
    async fn ground_prompt_wraps_the_prompt_with_the_matching_chunk() {
        use crate::db::NewDocument;

        let db = Database::connect_in_memory().await.unwrap();
        let job = db
            .jobs()
            .insert(crate::db::NewJob::new("chat"))
            .await
            .unwrap();
        let session = db.sessions().create("chat", "Docs").await.unwrap();
        db.documents()
            .insert(
                NewDocument {
                    session_id: session.id.clone(),
                    name: "policy.md".into(),
                    source_path: "policy.md".into(),
                    format: "md".into(),
                },
                &[
                    "The refund window is thirty days from purchase.".to_string(),
                    "Shipping normally takes three to five business days.".to_string(),
                ],
            )
            .await
            .unwrap();

        let out = ground_prompt(&db, &job.id, Some(&session.id), "what is the refund window")
            .await
            .unwrap();
        assert!(out.contains("thirty days"));
        assert!(out.contains("what is the refund window"));
        assert!(!out.contains("Shipping normally"));

        let events = db.jobs().events(&job.id).await.unwrap();
        assert!(events.iter().any(|e| e.message.contains("grounded in")));
    }
}
