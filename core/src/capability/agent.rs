//! `capability::agent` — driving long-running agent sessions (Phase 5.1c).
//!
//! Unlike chat/image/video, an agent session is **not** a job body: it lives for
//! many turns and must not sit inside the single-threaded job loop. [`AgentSessions`]
//! is its own subsystem. On `open` it:
//!
//! 1. resolves the coding model (the profile's, or `Auto` over the `coding` role),
//! 2. hands it to a [`CodingRuntime`], which places it on the GPU through the
//!    scheduler and **pins** it (never evicted while the session lives — R8 /
//!    ADR-003) and returns the `llama-server` `/v1` endpoint,
//! 3. opens a runtime session pointed at that endpoint with the forced,
//!    sandboxed config,
//! 4. drains the runtime's [`AgentEvent`] stream into `agent_session_events` and
//!    keeps `agent_sessions.state` in step.
//!
//! `stop` closes the runtime session, unpins + unloads the model, and marks the
//! session `Stopped`. MVP: **one live session at a time** (the scheduler pins one
//! agent model); multi-session polish is 5.5.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;

use crate::agent::{
    AgentAdapter, AgentEvent, AgentKind, EndpointConfig, EventStream, PermissionDecision,
    SessionSpec,
};
use crate::db::{Agent, AgentSession, AgentSessionState, Database, Model};
use crate::runtime::{LlamaCppAdapter, RuntimeAdapter, RuntimeRegistry};
use crate::scheduler::{Decision, HybridScheduler, PlanRequest, Scheduler};
use crate::{CoreError, Result};

const LLAMACPP: &str = "llamacpp";
/// VRAM to assume for a coding model whose import left no estimate.
const CODING_VRAM_FALLBACK_MB: u64 = 8192;

fn agent_err(msg: impl fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "agent".into(),
        message: msg.to_string(),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Owns the coding model's place on the GPU for the life of a session. The real
/// implementation drives the [`HybridScheduler`] + [`LlamaCppAdapter`]; tests use
/// a stub.
#[async_trait]
pub trait CodingRuntime: Send + Sync + fmt::Debug {
    /// Ensure `model_id` is served and pinned. Returns its OpenAI-compatible
    /// base URL (`http://127.0.0.1:<port>/v1`).
    async fn acquire(&self, model_id: &str, vram_mb: u64) -> Result<String>;

    /// Unpin and unload `model_id` (best effort — a stop must not fail on this).
    async fn release(&self, model_id: &str);
}

/// The production [`CodingRuntime`]: plan on the scheduler, load on `llama-server`,
/// pin so nothing evicts it.
#[derive(Debug)]
pub struct LlamaCodingRuntime {
    registry: RuntimeRegistry,
    scheduler: Arc<HybridScheduler>,
    llama: Arc<LlamaCppAdapter>,
}

impl LlamaCodingRuntime {
    pub fn new(
        registry: RuntimeRegistry,
        scheduler: Arc<HybridScheduler>,
        llama: Arc<LlamaCppAdapter>,
    ) -> Self {
        Self {
            registry,
            scheduler,
            llama,
        }
    }
}

#[async_trait]
impl CodingRuntime for LlamaCodingRuntime {
    async fn acquire(&self, model_id: &str, vram_mb: u64) -> Result<String> {
        let req = PlanRequest {
            job_id: format!("agent:{model_id}"),
            runtime_id: LLAMACPP.into(),
            model_id: model_id.into(),
            vram_needed_mb: vram_mb,
            is_agent_session: true,
        };
        match self.scheduler.plan(&req).await {
            Decision::RunNow => {}
            Decision::LoadThenRun => self.llama.load_model(model_id, vram_mb).await?,
            Decision::EvictThenLoad { victim_model } => {
                if let Some(rt) = self.registry.runtime_with_model(&victim_model) {
                    self.scheduler.unpin(&victim_model);
                    rt.unload_model(&victim_model).await?;
                }
                self.llama.load_model(model_id, vram_mb).await?;
            }
            Decision::Blocked { reason } => return Err(CoreError::SchedulerBlocked(reason)),
        }
        self.scheduler.pin(model_id);
        self.llama
            .base_url()
            .map(|base| format!("{base}/v1"))
            .ok_or_else(|| agent_err("llama-server has no endpoint after loading the model"))
    }

    async fn release(&self, model_id: &str) {
        self.scheduler.unpin(model_id);
        if let Some(rt) = self.registry.runtime_with_model(model_id) {
            let _ = rt.unload_model(model_id).await;
        }
    }
}

/// A session that is currently running: its runtime, the model it pinned, and
/// the task draining its event stream.
#[derive(Debug)]
struct Live {
    kind: AgentKind,
    adapter_session_id: String,
    model_id: String,
    drain: tokio::task::JoinHandle<()>,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.drain.abort();
    }
}

/// The agent subsystem. One per [`App`](crate::App).
#[derive(Debug)]
pub struct AgentSessions {
    db: Database,
    coding: Arc<dyn CodingRuntime>,
    adapters: HashMap<AgentKind, Arc<dyn AgentAdapter>>,
    live: Mutex<HashMap<String, Live>>,
}

impl AgentSessions {
    pub fn new(db: Database, coding: Arc<dyn CodingRuntime>) -> Self {
        Self {
            db,
            coding,
            adapters: HashMap::new(),
            live: Mutex::new(HashMap::new()),
        }
    }

    /// Register the adapter for one runtime kind (OpenCode now; Hermes in 5.4).
    #[must_use]
    pub fn with_adapter(mut self, adapter: Arc<dyn AgentAdapter>) -> Self {
        self.adapters.insert(adapter.kind(), adapter);
        self
    }

    fn adapter(&self, kind: AgentKind) -> Result<Arc<dyn AgentAdapter>> {
        self.adapters
            .get(&kind)
            .cloned()
            .ok_or_else(|| agent_err(format!("no {} adapter is configured", kind.as_str())))
    }

    /// Open a session for profile `agent_id`. `first_message`, if given, is sent
    /// as the opening turn once the session is live.
    pub async fn open(&self, agent_id: &str, first_message: Option<&str>) -> Result<AgentSession> {
        if !lock(&self.live).is_empty() {
            return Err(CoreError::Config(
                "an agent session is already running — stop it before starting another".into(),
            ));
        }

        let profile = self
            .db
            .agents()
            .get(agent_id)
            .await?
            .ok_or_else(|| agent_err(format!("no such agent profile {agent_id}")))?;
        let kind = AgentKind::from_adapter(&profile.adapter)
            .ok_or_else(|| agent_err(format!("unknown adapter {:?}", profile.adapter)))?;
        // Fail before we touch the GPU if the adapter isn't wired up.
        let adapter = self.adapter(kind)?;

        let model = self.resolve_model(profile.model_id.as_deref()).await?;
        let vram_mb = model
            .vram_estimate_mb
            .and_then(|mb| u64::try_from(mb).ok())
            .filter(|mb| *mb > 0)
            .unwrap_or(CODING_VRAM_FALLBACK_MB);

        let base_url = self.coding.acquire(&model.id, vram_mb).await?;

        let session = self.db.agents().create_session(agent_id).await?;
        if let Err(e) = self
            .start_runtime(
                &adapter,
                &profile,
                &model,
                &base_url,
                &session.id,
                first_message,
            )
            .await
        {
            // Roll everything back so a failed open leaves nothing live or pinned.
            drop(lock(&self.live).remove(&session.id));
            self.coding.release(&model.id).await;
            let _ = self
                .db
                .agents()
                .set_session_state(&session.id, AgentSessionState::Failed, Some(&e.to_string()))
                .await;
            return Err(e);
        }

        self.db
            .agents()
            .session(&session.id)
            .await?
            .ok_or_else(|| agent_err("session vanished right after open"))
    }

    /// Send a user turn to a live session.
    pub async fn message(&self, session_id: &str, text: &str) -> Result<()> {
        let (kind, adapter_session_id) = self.route(session_id)?;
        self.adapter(kind)?.send(&adapter_session_id, text).await?;
        self.db
            .agents()
            .set_session_state(session_id, AgentSessionState::Working, None)
            .await
    }

    /// Answer a pending permission request.
    pub async fn reply(
        &self,
        session_id: &str,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<()> {
        let (kind, adapter_session_id) = self.route(session_id)?;
        self.adapter(kind)?
            .reply_permission(&adapter_session_id, request_id, decision)
            .await?;
        self.db
            .agents()
            .set_session_state(session_id, AgentSessionState::Working, None)
            .await
    }

    /// Interrupt the in-flight turn without ending the session.
    pub async fn interrupt(&self, session_id: &str) -> Result<()> {
        let (kind, adapter_session_id) = self.route(session_id)?;
        self.adapter(kind)?.interrupt(&adapter_session_id).await
    }

    /// Close a session: stop the runtime, unpin + unload the model, mark it
    /// `Stopped`. When the session isn't live in this process (already stopped,
    /// or orphaned by a restart) it just marks any non-terminal row `Stopped`.
    pub async fn stop(&self, session_id: &str) -> Result<()> {
        let Some(live) = lock(&self.live).remove(session_id) else {
            if let Some(s) = self.db.agents().session(session_id).await? {
                if !s.state.is_terminal() {
                    self.db
                        .agents()
                        .set_session_state(session_id, AgentSessionState::Stopped, None)
                        .await?;
                }
            }
            return Ok(());
        };
        if let Ok(adapter) = self.adapter(live.kind) {
            let _ = adapter.close_session(&live.adapter_session_id).await;
        }
        live.drain.abort();
        self.coding.release(&live.model_id).await;
        self.db
            .agents()
            .set_session_state(session_id, AgentSessionState::Stopped, None)
            .await
    }

    /// Is a session running in this process right now?
    pub fn is_live(&self, session_id: &str) -> bool {
        lock(&self.live).contains_key(session_id)
    }

    // --- internals ---

    async fn resolve_model(&self, explicit: Option<&str>) -> Result<Model> {
        match explicit {
            Some(id) => self.db.models().get(id).await?.ok_or_else(|| {
                CoreError::Config(format!("the profile's model {id} is not in the library"))
            }),
            None => self
                .db
                .models()
                .pick_for_role("coding")
                .await?
                .ok_or_else(|| {
                    CoreError::Config(
                    "no model carries the 'coding' role — import a coding GGUF and mark it 'coding'"
                        .into(),
                )
                }),
        }
    }

    /// Open the runtime session, wire the event drain, send the opening turn.
    async fn start_runtime(
        &self,
        adapter: &Arc<dyn AgentAdapter>,
        profile: &Agent,
        model: &Model,
        base_url: &str,
        session_id: &str,
        first_message: Option<&str>,
    ) -> Result<()> {
        let spec = SessionSpec {
            workspace: PathBuf::from(&profile.workspace_path),
            allowed_paths: profile.allowed_paths.iter().map(PathBuf::from).collect(),
            toolset: profile.toolset.clone(),
            endpoint: EndpointConfig {
                base_url: base_url.to_string(),
                model: model.name.clone(),
            },
        };

        let adapter_session_id = adapter.open_session(&spec).await?;
        self.db
            .agents()
            .bind_adapter_session(session_id, &adapter_session_id)
            .await?;

        let rx = adapter.events(&adapter_session_id).await?;
        let drain = tokio::spawn(drain_events(self.db.clone(), session_id.to_string(), rx));
        lock(&self.live).insert(
            session_id.to_string(),
            Live {
                kind: adapter.kind(),
                adapter_session_id: adapter_session_id.clone(),
                model_id: model.id.clone(),
                drain,
            },
        );

        // From here a failure is cleaned up by `open` (which removes the live
        // entry — its Drop aborts the drain — and releases the model).
        let opening = match first_message {
            Some(text) => {
                adapter.send(&adapter_session_id, text).await?;
                AgentSessionState::Working
            }
            None => AgentSessionState::Idle,
        };
        self.db
            .agents()
            .set_session_state(session_id, opening, None)
            .await
    }

    fn route(&self, session_id: &str) -> Result<(AgentKind, String)> {
        lock(&self.live)
            .get(session_id)
            .map(|l| (l.kind, l.adapter_session_id.clone()))
            .ok_or_else(|| {
                CoreError::Config(format!(
                    "session {session_id} is not running — open a new one"
                ))
            })
    }
}

/// Append every event to the transcript and move the session state with it.
/// Ends when the stream closes or a terminal error arrives.
async fn drain_events(db: Database, session_id: String, mut rx: EventStream) {
    while let Some(ev) = rx.recv().await {
        if let Ok(payload) = serde_json::to_value(&ev) {
            let _ = db
                .agents()
                .append_event(&session_id, ev.kind(), &payload)
                .await;
        }
        if let Some((state, error)) = next_state(&ev) {
            let _ = db
                .agents()
                .set_session_state(&session_id, state, error.as_deref())
                .await;
            if state == AgentSessionState::Failed {
                return;
            }
        }
    }
}

/// The session state an event implies, if any.
fn next_state(ev: &AgentEvent) -> Option<(AgentSessionState, Option<String>)> {
    match ev {
        AgentEvent::Permission { .. } => Some((AgentSessionState::AwaitingApproval, None)),
        AgentEvent::Idle => Some((AgentSessionState::Idle, None)),
        AgentEvent::Text { .. } | AgentEvent::Tool { .. } => {
            Some((AgentSessionState::Working, None))
        }
        AgentEvent::Error { message, terminal } => {
            terminal.then(|| (AgentSessionState::Failed, Some(message.clone())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{FakeAgentAdapter, ToolStatus};
    use crate::db::{NewAgent, NewModel};

    /// A [`CodingRuntime`] stub: records acquire/release and hands back a URL.
    #[derive(Debug, Default)]
    struct StubCoding {
        acquired: Mutex<Vec<String>>,
        released: Mutex<Vec<String>>,
        fail: bool,
    }

    #[async_trait]
    impl CodingRuntime for StubCoding {
        async fn acquire(&self, model_id: &str, _vram_mb: u64) -> Result<String> {
            if self.fail {
                return Err(CoreError::SchedulerBlocked("no VRAM".into()));
            }
            lock(&self.acquired).push(model_id.to_string());
            Ok("http://127.0.0.1:9/v1".into())
        }
        async fn release(&self, model_id: &str) {
            lock(&self.released).push(model_id.to_string());
        }
    }

    async fn coding_model(db: &Database, role: &str) -> String {
        db.models()
            .insert(NewModel {
                name: "Qwen2.5 Coder".into(),
                format: "gguf".into(),
                file_path: "E:\\models\\coder.gguf".into(),
                size_bytes: 4096,
                source: "manual".into(),
                vram_estimate_mb: Some(6000),
                roles: vec![role.into()],
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id
    }

    async fn profile(db: &Database, adapter: &str) -> String {
        db.agents()
            .create(NewAgent {
                name: "Coder".into(),
                adapter: adapter.into(),
                model_id: None,
                workspace_path: "E:\\proj".into(),
                allowed_paths: vec![],
                toolset: None,
            })
            .await
            .unwrap()
            .id
    }

    fn sessions(db: &Database, coding: Arc<StubCoding>, a: Arc<dyn AgentAdapter>) -> AgentSessions {
        AgentSessions::new(db.clone(), coding).with_adapter(a)
    }

    #[test]
    fn next_state_maps_each_event() {
        assert_eq!(
            next_state(&AgentEvent::Idle),
            Some((AgentSessionState::Idle, None))
        );
        assert_eq!(
            next_state(&AgentEvent::Text { text: "x".into() }),
            Some((AgentSessionState::Working, None))
        );
        assert!(matches!(
            next_state(&AgentEvent::Permission {
                id: "p".into(),
                kind: "bash".into(),
                summary: "s".into(),
                always_pattern: None,
            }),
            Some((AgentSessionState::AwaitingApproval, None))
        ));
        assert!(next_state(&AgentEvent::Error {
            message: "transient".into(),
            terminal: false,
        })
        .is_none());
        assert!(matches!(
            next_state(&AgentEvent::Error {
                message: "fatal".into(),
                terminal: true,
            }),
            Some((AgentSessionState::Failed, Some(_)))
        ));
    }

    #[tokio::test]
    async fn open_without_a_coding_model_is_a_clear_error() {
        let db = Database::connect_in_memory().await.unwrap();
        let id = profile(&db, "fake").await;
        let s = sessions(&db, Arc::default(), Arc::new(FakeAgentAdapter::new()));
        let err = s.open(&id, None).await.unwrap_err();
        assert!(err.to_string().contains("coding"), "{err}");
    }

    #[tokio::test]
    async fn open_needs_the_matching_adapter_and_pins_nothing_on_the_way_out() {
        let db = Database::connect_in_memory().await.unwrap();
        let _m = coding_model(&db, "coding").await;
        let id = profile(&db, "opencode").await; // wants opencode…
        let coding = Arc::<StubCoding>::default();
        let s = sessions(&db, coding.clone(), Arc::new(FakeAgentAdapter::new())); // …only fake wired

        let err = s.open(&id, None).await.unwrap_err();
        assert!(err.to_string().contains("opencode adapter"), "{err}");
        assert!(lock(&coding.acquired).is_empty(), "GPU untouched");
    }

    #[tokio::test]
    async fn a_blocked_scheduler_surfaces_and_creates_no_session() {
        let db = Database::connect_in_memory().await.unwrap();
        let _m = coding_model(&db, "coding").await;
        let agent_id = profile(&db, "fake").await;
        let coding = Arc::new(StubCoding {
            fail: true,
            ..StubCoding::default()
        });
        let s = sessions(&db, coding, Arc::new(FakeAgentAdapter::new()));

        let err = s.open(&agent_id, None).await.unwrap_err();
        assert!(err.to_string().contains("VRAM"), "{err}");
        assert!(db
            .agents()
            .sessions_for(&agent_id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_full_session_drains_events_then_stop_releases_the_model() {
        let db = Database::connect_in_memory().await.unwrap();
        let model_id = coding_model(&db, "coding").await;
        let agent_id = profile(&db, "fake").await;

        let fake = Arc::new(FakeAgentAdapter::new().with_script(vec![
            AgentEvent::Text {
                text: "on it".into(),
            },
            AgentEvent::Tool {
                id: "t1".into(),
                name: "bash".into(),
                status: ToolStatus::Running,
                input: serde_json::json!({ "command": "ls" }),
                output: None,
            },
            AgentEvent::Permission {
                id: "per_1".into(),
                kind: "bash".into(),
                summary: "ls".into(),
                always_pattern: None,
            },
        ]));
        let coding = Arc::<StubCoding>::default();
        let s = sessions(&db, coding.clone(), fake.clone());

        let session = s.open(&agent_id, Some("list files")).await.unwrap();
        assert_eq!(
            lock(&coding.acquired).as_slice(),
            std::slice::from_ref(&model_id)
        );
        assert!(s.is_live(&session.id));

        let state = wait_for_state(&db, &session.id, AgentSessionState::AwaitingApproval).await;
        assert_eq!(state, AgentSessionState::AwaitingApproval);
        let kinds: Vec<String> = db
            .agents()
            .session_events(&session.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, ["text", "tool", "permission"]);

        s.reply(&session.id, "per_1", PermissionDecision::AllowOnce)
            .await
            .unwrap();
        let adapter_sid = session
            .adapter_session_id
            .clone()
            .or_else(|| Some(fake.replies().first()?.0.clone()))
            .unwrap();
        assert_eq!(
            fake.replies(),
            [(
                adapter_sid,
                "per_1".to_string(),
                PermissionDecision::AllowOnce
            )]
        );

        s.stop(&session.id).await.unwrap();
        assert_eq!(
            lock(&coding.released).as_slice(),
            std::slice::from_ref(&model_id)
        );
        assert!(!s.is_live(&session.id));
        assert_eq!(
            db.agents()
                .session(&session.id)
                .await
                .unwrap()
                .unwrap()
                .state,
            AgentSessionState::Stopped
        );
    }

    #[tokio::test]
    async fn only_one_session_runs_at_a_time() {
        let db = Database::connect_in_memory().await.unwrap();
        let _m = coding_model(&db, "coding").await;
        let agent_id = profile(&db, "fake").await;
        let s = sessions(&db, Arc::default(), Arc::new(FakeAgentAdapter::new()));

        let _first = s.open(&agent_id, None).await.unwrap();
        let err = s.open(&agent_id, None).await.unwrap_err();
        assert!(err.to_string().contains("already running"), "{err}");
    }

    async fn wait_for_state(
        db: &Database,
        session_id: &str,
        want: AgentSessionState,
    ) -> AgentSessionState {
        for _ in 0..50 {
            let s = db
                .agents()
                .session(session_id)
                .await
                .unwrap()
                .unwrap()
                .state;
            if s == want {
                return s;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("session never reached {want:?}");
    }
}
