//! A scripted [`AgentAdapter`] for tests: `send` replays a fixed list of events
//! on the session's stream, `reply_permission` records the decision. No process,
//! no HTTP.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{AgentAdapter, AgentEvent, AgentKind, EventStream, PermissionDecision, SessionSpec};
use crate::runtime::Health;
use crate::{CoreError, Result};

/// Poison is irrelevant for a test double — recover the guard.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Default)]
pub struct FakeAgentAdapter {
    /// Events each `send` replays, in order.
    script: Mutex<Vec<AgentEvent>>,
    /// Per-session event senders, keyed by the fake session id.
    streams: Mutex<HashMap<String, mpsc::UnboundedSender<AgentEvent>>>,
    /// `(session, request_id, decision)` from every `reply_permission`.
    replies: Mutex<Vec<(String, String, PermissionDecision)>>,
    /// The last `SessionSpec` `open_session` was given.
    last_spec: Mutex<Option<SessionSpec>>,
    next_id: Mutex<u32>,
}

impl FakeAgentAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the events a `send` will replay.
    pub fn with_script(self, events: Vec<AgentEvent>) -> Self {
        *lock(&self.script) = events;
        self
    }

    /// Every recorded permission reply.
    pub fn replies(&self) -> Vec<(String, String, PermissionDecision)> {
        lock(&self.replies).clone()
    }

    /// The spec the last `open_session` received.
    pub fn last_spec(&self) -> Option<SessionSpec> {
        lock(&self.last_spec).clone()
    }
}

#[async_trait]
impl AgentAdapter for FakeAgentAdapter {
    fn id(&self) -> &str {
        "fake-agent"
    }

    fn kind(&self) -> AgentKind {
        AgentKind::Fake
    }

    async fn health(&self) -> Health {
        Health::Healthy
    }

    async fn open_session(&self, spec: &SessionSpec) -> Result<String> {
        *lock(&self.last_spec) = Some(spec.clone());
        let mut n = lock(&self.next_id);
        *n += 1;
        Ok(format!("fake-ses-{n}"))
    }

    async fn send(&self, session: &str, _text: &str) -> Result<()> {
        let tx = lock(&self.streams)
            .get(session)
            .cloned()
            .ok_or_else(|| CoreError::Other(anyhow::anyhow!("no event stream for {session}")))?;
        for ev in lock(&self.script).iter().cloned() {
            let _ = tx.send(ev);
        }
        Ok(())
    }

    async fn events(&self, session: &str) -> Result<EventStream> {
        let (tx, rx) = mpsc::unbounded_channel();
        lock(&self.streams).insert(session.to_string(), tx);
        Ok(rx)
    }

    async fn reply_permission(
        &self,
        session: &str,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<()> {
        lock(&self.replies).push((session.to_string(), request_id.to_string(), decision));
        Ok(())
    }

    async fn interrupt(&self, _session: &str) -> Result<()> {
        Ok(())
    }

    async fn close_session(&self, session: &str) -> Result<()> {
        lock(&self.streams).remove(session);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{EndpointConfig, ToolStatus};
    use std::path::PathBuf;

    fn spec() -> SessionSpec {
        SessionSpec {
            workspace: PathBuf::from("E:\\proj"),
            allowed_paths: vec![],
            toolset: None,
            endpoint: EndpointConfig {
                base_url: "http://127.0.0.1:1/v1".into(),
                model: "coder".into(),
            },
        }
    }

    #[tokio::test]
    async fn send_replays_the_script_and_records_replies() {
        let fake = FakeAgentAdapter::new().with_script(vec![
            AgentEvent::Text { text: "hi".into() },
            AgentEvent::Permission {
                id: "per_1".into(),
                kind: "bash".into(),
                summary: "ls".into(),
                always_pattern: None,
            },
            AgentEvent::Tool {
                id: "t1".into(),
                name: "bash".into(),
                status: ToolStatus::Done,
                input: serde_json::json!({ "command": "ls" }),
                output: Some("a.rs\n".into()),
            },
            AgentEvent::Idle,
        ]);

        let sid = fake.open_session(&spec()).await.unwrap();
        assert_eq!(sid, "fake-ses-1");
        assert_eq!(
            fake.last_spec().unwrap().workspace,
            PathBuf::from("E:\\proj")
        );

        let mut rx = fake.events(&sid).await.unwrap();
        fake.send(&sid, "run ls").await.unwrap();
        fake.reply_permission(&sid, "per_1", PermissionDecision::AllowOnce)
            .await
            .unwrap();

        let mut got = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            got.push(ev.kind());
        }
        assert_eq!(got, ["text", "permission", "tool", "idle"]);
        assert_eq!(
            fake.replies(),
            [(
                "fake-ses-1".to_string(),
                "per_1".to_string(),
                PermissionDecision::AllowOnce
            )]
        );
    }

    #[tokio::test]
    async fn send_without_a_subscriber_errors() {
        let fake = FakeAgentAdapter::new();
        let sid = fake.open_session(&spec()).await.unwrap();
        assert!(fake.send(&sid, "x").await.is_err());
    }
}
