//! The agent subsystem (Phase 5): a trait over the two managed agent runtimes
//! (OpenCode — ADR-021 — first, Hermes second) and the value types that flow
//! between them, the core, and the UI.
//!
//! We do **not** build an agent. An adapter supervises the runtime's own
//! process, forces its config (endpoint = the local `llama-server`, path
//! allowlist, command approval), and translates its event stream into
//! [`AgentEvent`]s. `capability::agent` (slice 5.1c) drives one session: it
//! drains the event stream into `agent_session_events`, drives the session
//! state, and proxies permission replies.
//!
//! - 5.1a: the trait, the value types, the DB layer ([`crate::db::AgentRepo`])
//!   and a [`FakeAgentAdapter`] for tests.
//! - 5.1b: [`OpenCodeAdapter`] — the first real adapter (see `opencode`).
//! - 5.1c: `capability::agent` + scheduler pinning + the API/UI wiring.
//! - 5.2: the config-level sandbox (forced config + [`scrubbed_env`]).
//! - 5.4: [`HermesAgentAdapter`] — the second adapter (see `hermes`).

mod fake;
mod hermes;
mod opencode;

use std::fmt;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

pub use fake::FakeAgentAdapter;
pub use hermes::HermesAgentAdapter;
pub use opencode::OpenCodeAdapter;

use crate::runtime::Health;
use crate::Result;

/// Cloud-provider credentials stripped from every managed agent runtime's child
/// process (ADR-010, ADR-009) — the agent must reach only the local endpoint,
/// and a stray key on the host must never leak into it.
pub(crate) const CLOUD_CREDENTIAL_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENROUTER_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GENERATIVE_AI_API_KEY",
    "GROQ_API_KEY",
    "XAI_API_KEY",
    "MISTRAL_API_KEY",
    "DEEPSEEK_API_KEY",
    "PERPLEXITY_API_KEY",
    "TOGETHER_API_KEY",
    "FIREWORKS_API_KEY",
    "CEREBRAS_API_KEY",
    "COHERE_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "HF_TOKEN",
    "HUGGING_FACE_HUB_TOKEN",
];

/// [`CLOUD_CREDENTIAL_ENV`] plus `extra` adapter-specific config overrides, as
/// owned strings for [`crate::runtime::SpawnSpec::env_remove`].
pub(crate) fn scrubbed_env(extra: &[&str]) -> Vec<String> {
    CLOUD_CREDENTIAL_ENV
        .iter()
        .chain(extra)
        .map(|s| (*s).to_string())
        .collect()
}

/// Which managed agent runtime an adapter wraps. Matches the `agents.adapter`
/// column and the serde name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    OpenCode,
    Hermes,
    Fake,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenCode => "opencode",
            Self::Hermes => "hermes",
            Self::Fake => "fake",
        }
    }

    /// Parse the `agents.adapter` column value.
    pub fn from_adapter(s: &str) -> Option<Self> {
        match s {
            "opencode" => Some(Self::OpenCode),
            "hermes" => Some(Self::Hermes),
            "fake" => Some(Self::Fake),
            _ => None,
        }
    }
}

/// The local model endpoint the agent must use — nothing else (ADR-009,
/// ADR-021). An OpenAI-compatible `llama-server` with `--jinja`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointConfig {
    /// e.g. `http://127.0.0.1:48213/v1`.
    pub base_url: String,
    /// The model name the endpoint serves it under.
    pub model: String,
}

/// Everything an adapter needs to open a session. The workspace is the
/// path-allowlist root; `allowed_paths` are extra readable roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSpec {
    pub workspace: PathBuf,
    pub allowed_paths: Vec<PathBuf>,
    /// `None` = the adapter's default toolset.
    pub toolset: Option<Vec<String>>,
    pub endpoint: EndpointConfig,
}

/// Progress of one tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Pending,
    Running,
    Done,
    Error,
}

/// One event from a running session. Serialized whole into
/// `agent_session_events.payload_json`; [`kind`](Self::kind) is the coarse
/// column value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A chunk of the assistant's text reply.
    Text { text: String },
    /// A tool invocation update.
    Tool {
        id: String,
        name: String,
        status: ToolStatus,
        #[serde(default)]
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<String>,
    },
    /// A tool needs approval before it runs — the session is now
    /// `AwaitingApproval` until [`AgentAdapter::reply_permission`].
    Permission {
        id: String,
        /// The tool kind, e.g. `"bash"` / `"edit"`.
        kind: String,
        /// A human-readable summary, e.g. the command.
        summary: String,
        /// A pattern the runtime offers as "always allow", if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        always_pattern: Option<String>,
    },
    /// The turn finished; the session is idle again.
    Idle,
    /// The session hit an error. `terminal` = the session cannot continue.
    Error { message: String, terminal: bool },
}

impl AgentEvent {
    /// The `agent_session_events.kind` column value.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Tool { .. } => "tool",
            Self::Permission { .. } => "permission",
            Self::Idle => "idle",
            Self::Error { .. } => "error",
        }
    }
}

/// A user's answer to an [`AgentEvent::Permission`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Run it this once.
    AllowOnce,
    /// Run it and remember the pattern (the runtime's "always").
    AllowAlways,
    /// Refuse — the tool call returns an error to the agent.
    Deny,
}

/// One session's event receiver — the capability drains it into the transcript.
pub type EventStream = mpsc::UnboundedReceiver<AgentEvent>;

/// A managed agent runtime. The core drives every adapter through this trait
/// only; the adapter owns the runtime process + its HTTP client.
#[async_trait]
pub trait AgentAdapter: Send + Sync + fmt::Debug {
    /// Stable id, e.g. `"opencode"`.
    fn id(&self) -> &str;

    fn kind(&self) -> AgentKind;

    /// Probe the (shared) runtime process.
    async fn health(&self) -> Health;

    /// Ensure the runtime is up, then open a fresh session in `spec.workspace`
    /// with the forced config. Returns the runtime-assigned session id.
    async fn open_session(&self, spec: &SessionSpec) -> Result<String>;

    /// Queue a user turn. Returns immediately; output arrives on [`events`].
    async fn send(&self, session: &str, text: &str) -> Result<()>;

    /// Subscribe to `session`'s events. Called once per session by the
    /// capability; dropping the receiver ends the subscription.
    async fn events(&self, session: &str) -> Result<EventStream>;

    /// Answer a pending [`AgentEvent::Permission`].
    async fn reply_permission(
        &self,
        session: &str,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<()>;

    /// Interrupt the running turn. The session stays open.
    async fn interrupt(&self, session: &str) -> Result<()>;

    /// Close one session. Does **not** stop the shared runtime process.
    async fn close_session(&self, session: &str) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_event_serializes_with_a_type_tag_and_kind() {
        let e = AgentEvent::Permission {
            id: "per_1".into(),
            kind: "bash".into(),
            summary: "cargo test".into(),
            always_pattern: Some("cargo *".into()),
        };
        assert_eq!(e.kind(), "permission");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "permission");
        assert_eq!(v["summary"], "cargo test");
        let back: AgentEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);

        // Tool output is omitted when absent.
        let t = AgentEvent::Tool {
            id: "t1".into(),
            name: "edit".into(),
            status: ToolStatus::Running,
            input: serde_json::json!({ "path": "a.rs" }),
            output: None,
        };
        assert!(serde_json::to_value(&t).unwrap().get("output").is_none());
    }
}
