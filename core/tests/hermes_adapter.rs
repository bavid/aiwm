//! End-to-end for `HermesAgentAdapter` against `aiwm-fake-hermes`: the adapter
//! spawns the (fake) `hermes gateway`, opens a session, sends a turn, and the
//! per-turn SSE stream is translated into `AgentEvent`s — a text delta, a tool
//! call, an approval request; after the approval, the tool result, a closing
//! delta and `run.completed`. Windows only (matches the other runtime tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Duration;

use aiwm_core::agent::{AgentAdapter, AgentEvent, EndpointConfig, SessionSpec, ToolStatus};
use aiwm_core::{HermesAgentAdapter, PermissionDecision};

fn fake_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-hermes"))
}

fn adapter(home_root: &std::path::Path) -> HermesAgentAdapter {
    HermesAgentAdapter::with_binary(Some(fake_bin()), home_root.to_path_buf())
}

fn spec() -> SessionSpec {
    SessionSpec {
        workspace: std::env::temp_dir(),
        allowed_paths: vec![],
        toolset: None,
        endpoint: EndpointConfig {
            base_url: "http://127.0.0.1:1/v1".into(),
            model: "qwen2.5-coder".into(),
        },
    }
}

async fn recv_until(
    rx: &mut aiwm_core::agent::EventStream,
    mut pred: impl FnMut(&AgentEvent) -> bool,
) -> Vec<AgentEvent> {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let ev = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("timed out waiting for an agent event")
            .expect("the event stream closed early");
        let done = pred(&ev);
        seen.push(ev);
        if done {
            return seen;
        }
    }
}

#[tokio::test]
async fn a_full_turn_open_send_approve_idle() {
    let home = tempfile::tempdir().unwrap();
    let a = adapter(home.path());
    assert!(a.is_installed());

    let sid = a.open_session(&spec()).await.unwrap();
    assert!(!sid.is_empty());
    let mut rx = a.events(&sid).await.unwrap();

    a.send(&sid, "read readme.txt").await.unwrap();

    let pre = recv_until(&mut rx, |e| matches!(e, AgentEvent::Permission { .. })).await;
    assert!(
        pre.iter()
            .any(|e| matches!(e, AgentEvent::Text { text } if text.contains("read"))),
        "{pre:?}"
    );
    assert!(
        pre.iter()
            .any(|e| matches!(e, AgentEvent::Tool { name, status, .. }
            if name == "terminal" && *status == ToolStatus::Running)),
        "{pre:?}"
    );
    let run_id = pre
        .iter()
        .find_map(|e| match e {
            AgentEvent::Permission { id, kind, .. } => {
                assert_eq!(kind, "terminal");
                Some(id.clone())
            }
            _ => None,
        })
        .expect("an approval request");

    a.reply_permission(&sid, &run_id, PermissionDecision::AllowOnce)
        .await
        .unwrap();

    let post = recv_until(&mut rx, |e| matches!(e, AgentEvent::Idle)).await;
    assert!(
        post.iter().any(|e| matches!(e, AgentEvent::Tool { status, output, .. }
            if *status == ToolStatus::Done && output.as_deref() == Some("hello from the readme\n"))),
        "{post:?}"
    );
    assert!(
        post.iter()
            .any(|e| matches!(e, AgentEvent::Text { text } if text.contains("hello"))),
        "{post:?}"
    );

    a.close_session(&sid).await.unwrap();
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn a_denied_turn_still_reaches_idle() {
    let home = tempfile::tempdir().unwrap();
    let a = adapter(home.path());
    let sid = a.open_session(&spec()).await.unwrap();
    let mut rx = a.events(&sid).await.unwrap();
    a.send(&sid, "run something").await.unwrap();

    let pre = recv_until(&mut rx, |e| matches!(e, AgentEvent::Permission { .. })).await;
    let run_id = pre
        .iter()
        .find_map(|e| match e {
            AgentEvent::Permission { id, .. } => Some(id.clone()),
            _ => None,
        })
        .unwrap();

    a.reply_permission(&sid, &run_id, PermissionDecision::Deny)
        .await
        .unwrap();
    let post = recv_until(&mut rx, |e| matches!(e, AgentEvent::Idle)).await;
    assert!(post.iter().any(|e| matches!(e, AgentEvent::Idle)));

    a.close_session(&sid).await.unwrap();
}

#[tokio::test]
async fn not_installed_is_a_clear_error() {
    let home = tempfile::tempdir().unwrap();
    let a = HermesAgentAdapter::with_binary(None, home.path().to_path_buf());
    let err = a.open_session(&spec()).await.unwrap_err();
    assert!(err.to_string().contains("not installed"), "{err}");
}
