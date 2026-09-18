//! End-to-end: a `job_type=chat` job through the real `JobEngine` — scheduler
//! decision, `LlamaCppAdapter` spawns the (fake) server, the chat body streams
//! the answer into `jobs.result`, the job reaches `Completed`.
//!
//! Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use aiwm_core::db::{Database, NewJob, NewModel};
use aiwm_core::orchestrator::{JobEngine, JobOutcome, JobState};
use aiwm_core::runtime::{
    ComfyDirs, ComfyUiAdapter, LlamaCppAdapter, LlamaServerOptions, RuntimeRegistry,
};
use aiwm_core::scheduler::HybridScheduler;

fn fake_llama_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-llama"))
}

fn chat_job(prompt: &str) -> NewJob {
    let mut job = NewJob::new("chat");
    job.params = serde_json::json!({ "prompt": prompt, "max_tokens": 64 });
    job
}

struct Harness {
    db: Database,
    engine: Arc<JobEngine>,
    llama: Arc<LlamaCppAdapter>,
    _tmp: tempfile::TempDir,
}

impl Harness {
    /// The body of the last `/v1/chat/completions` the fixture served —
    /// fake-llama's test-only `GET /__test/last_request`.
    async fn last_request(&self) -> serde_json::Value {
        let base = self.llama.base_url().expect("a loaded fake server");
        reqwest::get(format!("{base}/__test/last_request"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

async fn harness(with_model: bool) -> Harness {
    harness_with(with_model, &[]).await
}

/// Engine wired to the fixture binary. `with_model` registers a `chat`-role
/// model; `extra_args` are appended to the fake server's command line.
async fn harness_with(with_model: bool, extra_args: &[&str]) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();

    if with_model {
        let gguf = tmp.path().join("smol.gguf");
        std::fs::write(&gguf, b"GGUF\0fixture").unwrap();
        db.models()
            .insert(NewModel {
                name: "Smol Chat".into(),
                format: "gguf".into(),
                file_path: gguf.to_string_lossy().into_owned(),
                size_bytes: 4096,
                vram_estimate_mb: Some(1500),
                source: "manual".into(),
                roles: vec!["chat".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
    }

    let registry = RuntimeRegistry::new();
    let llama = Arc::new(
        LlamaCppAdapter::with_binary(db.clone(), Some(fake_llama_bin())).with_options(
            LlamaServerOptions {
                flash_attention: false,
                extra_args: extra_args.iter().map(|s| s.to_string()).collect(),
                ..LlamaServerOptions::default()
            },
        ),
    );
    let probe = llama.clone();
    registry.register(llama.clone());
    let comfyui = Arc::new(ComfyUiAdapter::with_launch(
        db.clone(),
        None,
        ComfyDirs {
            base: tmp.path().join("comfyui-data"),
            output: tmp.path().join("outputs"),
            models_store: tmp.path().join("store"),
        },
    ));
    registry.register(comfyui.clone());
    let scheduler = Arc::new(HybridScheduler::new(registry.clone(), 16_384));
    let engine = Arc::new(JobEngine::new(
        db.clone(),
        registry,
        scheduler,
        llama,
        comfyui,
        tmp.path().join("outputs"),
    ));

    Harness {
        db,
        engine,
        llama: probe,
        _tmp: tmp,
    }
}

/// The chat messages the fixture last received, as `(role, content)` pairs.
fn messages(body: &serde_json::Value) -> Vec<(String, String)> {
    body["messages"]
        .as_array()
        .expect("a messages array")
        .iter()
        .map(|m| {
            (
                m["role"].as_str().unwrap_or_default().to_string(),
                m["content"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

async fn persona(h: &Harness, name: &str, icon: &str, prompt: &str) -> aiwm_core::db::Persona {
    aiwm_core::persona::create(&h.db, name, icon, prompt)
        .await
        .unwrap()
}

/// A chat with no persona anywhere must send exactly the one user message it
/// always sent — the guard for "nothing changes until you opt in".
#[tokio::test]
async fn without_a_persona_the_request_carries_only_the_user_message() {
    let h = harness(true).await;
    let job = h.engine.submit(chat_job("Hello there")).await.unwrap();
    h.engine.run_next().await.unwrap().unwrap();

    assert_eq!(
        messages(&h.last_request().await),
        [("user".to_string(), "Hello there".to_string())]
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(stored.params.get("persona").is_none(), "{}", stored.params);
}

#[tokio::test]
async fn a_global_persona_prepends_a_system_message_and_lands_in_the_job_params() {
    let h = harness(true).await;
    let p = persona(&h, "Blunt", "🪓", "Answer in at most three sentences.").await;
    aiwm_core::persona::set_active(&h.db, Some(&p.id))
        .await
        .unwrap();

    let job = h.engine.submit(chat_job("Hello there")).await.unwrap();
    h.engine.run_next().await.unwrap().unwrap();

    let got = messages(&h.last_request().await);
    assert_eq!(
        got,
        [
            (
                "system".to_string(),
                "Answer in at most three sentences.".to_string()
            ),
            ("user".to_string(), "Hello there".to_string()),
        ],
        "the system message must come first"
    );

    // The job remembers which persona answered — id, name and icon, never the
    // prompt text.
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.params["persona"]["id"], p.id);
    assert_eq!(stored.params["persona"]["name"], "Blunt");
    assert_eq!(stored.params["persona"]["icon"], "🪓");
    assert!(
        !stored.params.to_string().contains("three sentences"),
        "{}",
        stored.params
    );

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(
        events.iter().any(|m| m == "persona: 🪓 Blunt"),
        "{events:?}"
    );
}

/// A Prompt Assistant completion (Image/Video/Voice's "talk through what you
/// want") is a `chat` job too, but its answer is parsed for `PROMPT:` /
/// `NEGATIVE:` markers and it is submitted from tabs the persona was never
/// chosen from. It must stay persona-free even with a global persona active.
#[tokio::test]
async fn a_prompt_assistant_job_stays_persona_free() {
    let h = harness(true).await;
    let p = persona(&h, "Blunt", "🪓", "Answer in at most three sentences.").await;
    aiwm_core::persona::set_active(&h.db, Some(&p.id))
        .await
        .unwrap();

    let mut job = chat_job("a moody lighthouse portrait");
    job.params = serde_json::json!({
        "prompt": "a moody lighthouse portrait",
        "max_tokens": 64,
        "assistant_for": "image"
    });
    let job = h.engine.submit(job).await.unwrap();
    h.engine.run_next().await.unwrap().unwrap();

    assert_eq!(
        messages(&h.last_request().await),
        [(
            "user".to_string(),
            "a moody lighthouse portrait".to_string()
        )],
        "no system message may reach a Prompt Assistant completion"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(stored.params.get("persona").is_none(), "{}", stored.params);

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(
        !events.iter().any(|m| m.starts_with("persona:")),
        "{events:?}"
    );
}

/// A session that opted out sends the plain single-message request even with a
/// global persona active.
#[tokio::test]
async fn a_session_override_of_none_sends_a_single_user_message() {
    let h = harness(true).await;
    let p = persona(&h, "Blunt", "🪓", "Answer in at most three sentences.").await;
    aiwm_core::persona::set_active(&h.db, Some(&p.id))
        .await
        .unwrap();
    let session = h.db.sessions().create("chat", "Plain").await.unwrap();
    aiwm_core::persona::set_session_persona(
        &h.db,
        &session.id,
        aiwm_core::db::PersonaMode::None,
        None,
    )
    .await
    .unwrap();

    let mut job = chat_job("Hello there");
    job.session_id = Some(session.id.clone());
    let job = h.engine.submit(job).await.unwrap();
    h.engine.run_next().await.unwrap().unwrap();

    assert_eq!(
        messages(&h.last_request().await),
        [("user".to_string(), "Hello there".to_string())]
    );
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(stored.params.get("persona").is_none(), "{}", stored.params);
}

/// A session may pick its own persona over the global one.
#[tokio::test]
async fn a_session_persona_wins_over_the_global_one() {
    let h = harness(true).await;
    let global = persona(&h, "Global", "🌍", "global prompt").await;
    let own = persona(&h, "Own", "🎯", "own prompt").await;
    aiwm_core::persona::set_active(&h.db, Some(&global.id))
        .await
        .unwrap();
    let session = h.db.sessions().create("chat", "Own").await.unwrap();
    aiwm_core::persona::set_session_persona(
        &h.db,
        &session.id,
        aiwm_core::db::PersonaMode::Persona,
        Some(&own.id),
    )
    .await
    .unwrap();

    let mut job = chat_job("Hi");
    job.session_id = Some(session.id.clone());
    h.engine.submit(job).await.unwrap();
    h.engine.run_next().await.unwrap().unwrap();

    assert_eq!(
        messages(&h.last_request().await)[0],
        ("system".to_string(), "own prompt".to_string())
    );
}

/// The two features stack without interfering: RAG grounding stays inside the
/// user message exactly as before, and the persona rides in front of it as its
/// own system message.
#[tokio::test]
async fn an_attached_document_grounds_the_user_message_while_the_persona_stays_a_system_message() {
    use aiwm_core::db::NewDocument;

    let h = harness(true).await;
    let p = persona(&h, "Blunt", "🪓", "Answer in at most three sentences.").await;
    aiwm_core::persona::set_active(&h.db, Some(&p.id))
        .await
        .unwrap();
    let session = h.db.sessions().create("chat", "Docs").await.unwrap();
    h.db.documents()
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

    let mut job = chat_job("what is the refund window");
    job.session_id = Some(session.id.clone());
    let job = h.engine.submit(job).await.unwrap();
    h.engine.run_next().await.unwrap().unwrap();

    let got = messages(&h.last_request().await);
    assert_eq!(got.len(), 2, "exactly a system and a user message: {got:?}");
    assert_eq!(
        got[0],
        (
            "system".to_string(),
            "Answer in at most three sentences.".to_string()
        )
    );
    assert_eq!(got[1].0, "user");
    // The grounding is in the user message, where it has always been — and the
    // persona's prompt is not duplicated into it.
    assert!(got[1].1.contains("thirty days"), "{:?}", got[1].1);
    assert!(
        got[1].1.contains("what is the refund window"),
        "{:?}",
        got[1].1
    );
    assert!(
        !got[1].1.contains("three sentences"),
        "the persona prompt must not leak into the user message: {:?}",
        got[1].1
    );

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(
        events.iter().any(|m| m == "persona: 🪓 Blunt"),
        "{events:?}"
    );
    assert!(
        events.iter().any(|m| m.contains("grounded in")),
        "{events:?}"
    );
}

/// The self-healing case that matters most: deleting the persona between two
/// messages must leave the second chat working, with no persona, rather than
/// failing on a stale id.
#[tokio::test]
async fn a_persona_deleted_between_two_chats_leaves_the_second_one_working() {
    let h = harness(true).await;
    let p = persona(&h, "Doomed", "💀", "be doomed").await;
    let session = h.db.sessions().create("chat", "Chat").await.unwrap();
    aiwm_core::persona::set_session_persona(
        &h.db,
        &session.id,
        aiwm_core::db::PersonaMode::Persona,
        Some(&p.id),
    )
    .await
    .unwrap();
    aiwm_core::persona::set_active(&h.db, Some(&p.id))
        .await
        .unwrap();

    let mut first = chat_job("First");
    first.session_id = Some(session.id.clone());
    let first = h.engine.submit(first).await.unwrap();
    h.engine.run_next().await.unwrap().unwrap();
    assert_eq!(
        messages(&h.last_request().await)[0],
        ("system".to_string(), "be doomed".to_string())
    );

    h.db.personas().delete(&p.id).await.unwrap();

    let mut second = chat_job("Second");
    second.session_id = Some(session.id.clone());
    let second = h.engine.submit(second).await.unwrap();
    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == second.id),
        "the chat must still complete: {outcome:?}"
    );

    assert_eq!(
        messages(&h.last_request().await),
        [("user".to_string(), "Second".to_string())],
        "no persona left to apply"
    );
    let stored = h.db.jobs().get(&second.id).await.unwrap().unwrap();
    assert!(stored.params.get("persona").is_none(), "{}", stored.params);
    // The first job keeps its record of who answered, even though the persona
    // is gone.
    let first = h.db.jobs().get(&first.id).await.unwrap().unwrap();
    assert_eq!(first.params["persona"]["name"], "Doomed");
}

#[tokio::test]
async fn auto_chat_job_streams_an_answer_to_completion() {
    let h = harness(true).await;
    let job = h.engine.submit(chat_job("Hello there")).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Completed { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Completed);
    assert!(stored.model_id.is_some(), "Auto should have bound a model");
    assert!(stored.finished_at.is_some());

    let answer = stored.result.unwrap_or_default();
    assert!(answer.contains("fake-llama"), "answer: {answer:?}");
    assert!(answer.contains("Hello there"), "answer: {answer:?}");

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(
        events.iter().any(|m| m.contains("auto-selected")),
        "{events:?}"
    );
    assert!(events.iter().any(|m| m.contains("answered")), "{events:?}");

    assert_eq!(h.db.models().list().await.unwrap()[0].use_count, 1);
}

#[tokio::test]
async fn explicit_model_chat_job_completes() {
    let h = harness(true).await;
    let model_id = h.db.models().list().await.unwrap()[0].id.clone();

    let mut job = chat_job("Ping");
    job = job.on("llamacpp", &model_id, 1500);
    let job = h.engine.submit(job).await.unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(outcome, JobOutcome::Completed { .. }),
        "got {outcome:?}"
    );
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(stored.result.unwrap_or_default().contains("Ping"));
}

#[tokio::test]
async fn chat_job_without_a_prompt_fails() {
    let h = harness(true).await;
    let model_id = h.db.models().list().await.unwrap()[0].id.clone();
    let job = h
        .engine
        .submit(NewJob::new("chat").on("llamacpp", &model_id, 1500))
        .await
        .unwrap();

    let outcome = h.engine.run_next().await.unwrap().unwrap();
    assert!(
        matches!(outcome, JobOutcome::Failed { .. }),
        "got {outcome:?}"
    );
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Failed);
    assert!(stored.error_text.unwrap_or_default().contains("prompt"));
}

#[tokio::test]
async fn auto_chat_job_fails_cleanly_without_a_chat_model() {
    let h = harness(false).await;
    let outcome = h.engine.submit(chat_job("hi")).await.unwrap();
    let _ = outcome;

    match h.engine.run_next().await.unwrap().unwrap() {
        JobOutcome::Failed { error, .. } => assert!(error.contains("no chat model"), "{error}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn cancel_stops_a_running_chat_job_mid_stream() {
    // 40 tokens at 100 ms each = 4 s of streaming — lots of room to cancel.
    let h = harness_with(true, &["--fake-token-ms", "100", "--fake-tokens", "40"]).await;
    let job = h
        .engine
        .submit(chat_job("Tell me a long story"))
        .await
        .unwrap();

    let engine = h.engine.clone();
    let run = tokio::spawn(async move { engine.run_next().await.unwrap().unwrap() });

    // Wait past the model "load" and a good number of streamed tokens, then cancel.
    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
    assert!(
        h.engine.cancel(&job.id).await.unwrap(),
        "cancel should apply"
    );

    let outcome = run.await.unwrap();
    assert!(
        matches!(&outcome, JobOutcome::Cancelled { job_id } if *job_id == job.id),
        "got {outcome:?}"
    );

    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Cancelled);
    assert!(stored.finished_at.is_some());
    let partial = stored.result.unwrap_or_default();
    assert!(
        !partial.is_empty(),
        "some text should have streamed before cancel"
    );
    assert!(
        partial.len() < 500,
        "cancel should have cut it short, got {} chars",
        partial.len()
    );

    let events: Vec<String> =
        h.db.jobs()
            .events(&job.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.message)
            .collect();
    assert!(
        events.iter().any(|m| m.contains("cancelled after")),
        "{events:?}"
    );
}

#[tokio::test]
async fn cancel_of_a_queued_job_marks_it_cancelled() {
    let h = harness(true).await;
    let job = h.engine.submit(chat_job("hi")).await.unwrap();

    assert!(h.engine.cancel(&job.id).await.unwrap());
    let stored = h.db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.state, JobState::Cancelled);

    // A cancelled job is not runnable.
    assert!(h.engine.run_next().await.unwrap().is_none());
}

#[tokio::test]
async fn cancel_of_a_finished_job_is_a_noop() {
    let h = harness(true).await;
    let job = h.engine.submit(chat_job("hi")).await.unwrap();
    h.engine.run_next().await.unwrap();
    assert_eq!(
        h.db.jobs().get(&job.id).await.unwrap().unwrap().state,
        JobState::Completed
    );
    assert!(!h.engine.cancel(&job.id).await.unwrap());
}
