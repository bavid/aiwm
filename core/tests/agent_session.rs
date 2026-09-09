//! Phase 5.1c end-to-end for the agent subsystem: a real [`AgentSessions`] +
//! [`LlamaCodingRuntime`] drives a coding model onto the (fake) `llama-server`,
//! pins it, opens a (fake) `opencode` session pointed at that endpoint, and the
//! SSE turn — text, a tool call, a permission ask, then the result and idle —
//! lands in `agent_session_events` while `agent_sessions.state` tracks it.
//! `stop` unpins + unloads the model.
//!
//! Windows only (matches the other runtime integration tests). Uses the
//! `aiwm-fake-llama` and `aiwm-fake-opencode` fixtures — no real binaries, no
//! coding model.

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aiwm_core::capability::agent::{AgentSessions, LlamaCodingRuntime};
use aiwm_core::db::{AgentSessionState, Database, NewAgent, NewModel};
use aiwm_core::runtime::{LlamaCppAdapter, RuntimeAdapter, RuntimeRegistry};
use aiwm_core::scheduler::{HybridScheduler, Scheduler};
use aiwm_core::{OpenCodeAdapter, PermissionDecision};

fn fake_llama_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-llama"))
}

fn fake_opencode_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-opencode"))
}

struct Harness {
    db: Database,
    sessions: AgentSessions,
    llama: Arc<LlamaCppAdapter>,
    scheduler: Arc<HybridScheduler>,
    _tmp: tempfile::TempDir,
}

async fn harness() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();

    let registry = RuntimeRegistry::new();
    let llama = Arc::new(LlamaCppAdapter::with_binary(
        db.clone(),
        Some(fake_llama_bin()),
    ));
    registry.register(llama.clone());
    let scheduler = Arc::new(HybridScheduler::new(registry.clone(), 16_384));
    let coding = Arc::new(LlamaCodingRuntime::new(
        registry,
        scheduler.clone(),
        llama.clone(),
    ));

    let opencode = Arc::new(OpenCodeAdapter::with_binary(Some(fake_opencode_bin())));
    let sessions = AgentSessions::new(db.clone(), coding).with_adapter(opencode);

    Harness {
        db,
        sessions,
        llama,
        scheduler,
        _tmp: tmp,
    }
}

impl Harness {
    async fn import_coding_model(&self) -> String {
        let gguf = self._tmp.path().join("coder.gguf");
        std::fs::write(&gguf, b"GGUF\0fixture").unwrap();
        self.db
            .models()
            .insert(NewModel {
                name: "Qwen2.5 Coder 7B".into(),
                format: "gguf".into(),
                file_path: gguf.to_string_lossy().into_owned(),
                size_bytes: 4096,
                source: "manual".into(),
                vram_estimate_mb: Some(6000),
                roles: vec!["coding".into()],
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id
    }

    async fn profile(&self) -> String {
        self.db
            .agents()
            .create(NewAgent {
                name: "Coder".into(),
                adapter: "opencode".into(),
                model_id: None, // Auto over the `coding` role
                workspace_path: self._tmp.path().to_string_lossy().into_owned(),
                allowed_paths: vec![],
                toolset: None,
            })
            .await
            .unwrap()
            .id
    }

    /// Poll the transcript until an event of `kind` appears (or time out).
    async fn wait_for_event(&self, session_id: &str, kind: &str) -> serde_json::Value {
        for _ in 0..250 {
            let events = self.db.agents().session_events(session_id).await.unwrap();
            if let Some(e) = events.into_iter().find(|e| e.kind == kind) {
                return e.payload;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("no {kind:?} event on session {session_id}");
    }

    async fn state(&self, session_id: &str) -> AgentSessionState {
        self.db
            .agents()
            .session(session_id)
            .await
            .unwrap()
            .unwrap()
            .state
    }
}

#[tokio::test]
async fn open_send_approve_idle_then_stop_releases_the_model() {
    let h = harness().await;
    let model_id = h.import_coding_model().await;
    let agent_id = h.profile().await;

    // Open with an opening turn.
    let session = h
        .sessions
        .open(&agent_id, Some("read readme.txt and tell me what it says"))
        .await
        .unwrap();

    // The coding model is resident on the (fake) llama-server and pinned.
    assert_eq!(
        h.llama
            .loaded_models()
            .into_iter()
            .map(|m| m.model_id)
            .collect::<Vec<_>>(),
        vec![model_id.clone()],
    );
    assert!(h.scheduler.is_pinned(&model_id));
    assert!(session.adapter_session_id.is_some());

    // The turn streams in: a permission ask parks the session.
    let perm = h.wait_for_event(&session.id, "permission").await;
    let request_id = perm["id"].as_str().unwrap().to_string();
    assert_eq!(perm["kind"], "bash");
    assert_eq!(
        h.state(&session.id).await,
        AgentSessionState::AwaitingApproval
    );

    // Text before the ask made it into the transcript.
    let text = h.wait_for_event(&session.id, "text").await;
    assert!(text["text"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("read"));

    // Approve → the tool result and closing text arrive, then idle.
    h.sessions
        .reply(&session.id, &request_id, PermissionDecision::AllowOnce)
        .await
        .unwrap();

    for _ in 0..250 {
        if h.state(&session.id).await == AgentSessionState::Idle {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(h.state(&session.id).await, AgentSessionState::Idle);

    let kinds: Vec<String> =
        h.db.agents()
            .session_events(&session.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
    assert!(kinds.iter().any(|k| k == "tool"), "{kinds:?}");
    assert!(
        kinds.iter().filter(|k| *k == "text").count() >= 2,
        "{kinds:?}"
    );

    // Stop → runtime session closed, model unpinned + unloaded, state terminal.
    h.sessions.stop(&session.id).await.unwrap();
    assert!(!h.scheduler.is_pinned(&model_id));
    assert!(h.llama.loaded_models().is_empty());
    assert!(!h.sessions.is_live(&session.id));
    assert_eq!(h.state(&session.id).await, AgentSessionState::Stopped);
}

#[tokio::test]
async fn a_denied_permission_still_lets_the_session_go_idle() {
    let h = harness().await;
    let _model = h.import_coding_model().await;
    let agent_id = h.profile().await;

    let session = h
        .sessions
        .open(&agent_id, Some("run something"))
        .await
        .unwrap();
    let perm = h.wait_for_event(&session.id, "permission").await;
    let request_id = perm["id"].as_str().unwrap().to_string();

    h.sessions
        .reply(&session.id, &request_id, PermissionDecision::Deny)
        .await
        .unwrap();

    for _ in 0..250 {
        if h.state(&session.id).await == AgentSessionState::Idle {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(h.state(&session.id).await, AgentSessionState::Idle);

    h.sessions.stop(&session.id).await.unwrap();
    assert_eq!(h.state(&session.id).await, AgentSessionState::Stopped);
}
