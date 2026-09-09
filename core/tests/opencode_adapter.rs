//! End-to-end for `OpenCodeAdapter` against `aiwm-fake-opencode`: the adapter
//! spawns the (fake) `opencode serve`, opens a session, sends a turn, and the
//! SSE stream is translated into `AgentEvent`s — text, a tool call, a
//! permission request; after the reply, the tool result, closing text, and
//! idle. Windows only (matches the other runtime integration tests).

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Duration;

use aiwm_core::agent::{AgentAdapter, AgentEvent, EndpointConfig, SessionSpec, ToolStatus};
use aiwm_core::OpenCodeAdapter;

fn fake_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-opencode"))
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

/// Drain events until `pred` matches one or a 5 s deadline passes.
async fn recv_until(
    rx: &mut aiwm_core::agent::EventStream,
    mut pred: impl FnMut(&AgentEvent) -> bool,
) -> Vec<AgentEvent> {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
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
    let adapter = OpenCodeAdapter::with_binary(Some(fake_bin()));
    assert!(adapter.is_installed());

    let sid = adapter.open_session(&spec()).await.unwrap();
    assert!(!sid.is_empty());
    let mut rx = adapter.events(&sid).await.unwrap();

    adapter
        .send(&sid, "read readme.txt and tell me what it says")
        .await
        .unwrap();

    // Up to the permission ask.
    let pre = recv_until(&mut rx, |e| matches!(e, AgentEvent::Permission { .. })).await;
    assert!(
        pre.iter()
            .any(|e| matches!(e, AgentEvent::Text { text } if text.contains("read"))),
        "{pre:?}"
    );
    let (perm_id, always) = pre
        .iter()
        .find_map(|e| match e {
            AgentEvent::Permission {
                id,
                kind,
                always_pattern,
                ..
            } => {
                assert_eq!(kind, "bash");
                Some((id.clone(), always_pattern.clone()))
            }
            _ => None,
        })
        .expect("a permission request");
    assert_eq!(always.as_deref(), Some("cat *"));
    assert!(
        pre.iter()
            .any(|e| matches!(e, AgentEvent::Tool { name, status, .. }
            if name == "bash" && *status == ToolStatus::Running)),
        "{pre:?}"
    );

    adapter
        .reply_permission(&sid, &perm_id, aiwm_core::PermissionDecision::AllowOnce)
        .await
        .unwrap();

    // Tool result, closing text, idle.
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

    adapter.close_session(&sid).await.unwrap();
    // The stream ends once the process is gone.
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn a_denied_permission_still_ends_the_turn() {
    let adapter = OpenCodeAdapter::with_binary(Some(fake_bin()));
    let sid = adapter.open_session(&spec()).await.unwrap();
    let mut rx = adapter.events(&sid).await.unwrap();
    adapter.send(&sid, "run something").await.unwrap();

    let pre = recv_until(&mut rx, |e| matches!(e, AgentEvent::Permission { .. })).await;
    let perm_id = pre
        .iter()
        .find_map(|e| match e {
            AgentEvent::Permission { id, .. } => Some(id.clone()),
            _ => None,
        })
        .unwrap();

    adapter
        .reply_permission(&sid, &perm_id, aiwm_core::PermissionDecision::Deny)
        .await
        .unwrap();
    let post = recv_until(&mut rx, |e| matches!(e, AgentEvent::Idle)).await;
    assert!(post.iter().any(|e| matches!(e, AgentEvent::Idle)));

    adapter.close_session(&sid).await.unwrap();
}

#[tokio::test]
async fn interrupt_is_accepted_while_a_turn_is_in_flight() {
    let adapter = OpenCodeAdapter::with_binary(Some(fake_bin()));
    let sid = adapter.open_session(&spec()).await.unwrap();
    let mut rx = adapter.events(&sid).await.unwrap();
    adapter.send(&sid, "do a long thing").await.unwrap();

    recv_until(&mut rx, |e| matches!(e, AgentEvent::Permission { .. })).await;
    // Abort mid-turn — best-effort, must not error.
    adapter.interrupt(&sid).await.unwrap();

    adapter.close_session(&sid).await.unwrap();
}
