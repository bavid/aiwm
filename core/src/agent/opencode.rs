//! The OpenCode adapter (Phase 5.1b, ADR-021 — first agent adapter).
//!
//! OpenCode ships as a single self-contained `opencode` binary. We supervise one
//! `opencode serve` process **per session** (MVP: the scheduler pins one agent
//! model at a time anyway), started with `cwd` = the workspace and its whole
//! config **forced** through the `OPENCODE_CONFIG_CONTENT` env var — the endpoint
//! is the local `llama-server` and nothing else, `bash`/`edit` need approval,
//! `webfetch` is denied.
//!
//! Its `GET /event` SSE stream is translated into [`AgentEvent`]s. The exact
//! real event shapes (`message.part.updated`, `permission.asked`, idle) are from
//! the 5.0 probe; a real coding-model run in 5.1c will calibrate the corners
//! (like the ComfyUI `/history` output key was for video).

mod sse;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{AgentAdapter, AgentKind, EventStream, PermissionDecision, SessionSpec};
use crate::runtime::{free_loopback_port, Health, RuntimeSupervisor, SpawnSpec, SupervisorState};
use crate::{CoreError, Result};

pub(super) const ADAPTER_ID: &str = "opencode";
/// Env override for the `opencode` executable (tests point it at the fixture).
const BIN_ENV: &str = "AIWM_OPENCODE_PATH";
const SERVER_EXE: &str = if cfg!(windows) {
    "opencode.exe"
} else {
    "opencode"
};
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_POLL: Duration = Duration::from_millis(250);

fn err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: ADAPTER_ID.into(),
        message: msg.to_string(),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug)]
enum LaunchSource {
    /// Exactly this path, or nothing (tests / an override).
    Fixed(Option<PathBuf>),
    /// Re-scan `<runtimes_dir>/opencode/` + `AIWM_OPENCODE_PATH` on every open.
    Scan(PathBuf),
}

struct Session {
    /// Owns the `opencode serve` child — dropping it kills the process.
    _supervisor: RuntimeSupervisor,
    base: String,
    /// Taken once by [`AgentAdapter::events`].
    pending_rx: Option<EventStream>,
    reader: JoinHandle<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

#[derive(Debug)]
pub struct OpenCodeAdapter {
    launch: LaunchSource,
    http: reqwest::Client,
    /// adapter session id → its process + stream.
    sessions: Mutex<HashMap<String, Session>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("base", &self.base).finish()
    }
}

impl OpenCodeAdapter {
    /// Re-scans the managed install dir + `AIWM_OPENCODE_PATH` on demand.
    pub fn discover(runtimes_dir: &Path) -> Self {
        Self::new(LaunchSource::Scan(runtimes_dir.to_path_buf()))
    }

    /// Fixed launch path (tests point this at `aiwm-fake-opencode`).
    pub fn with_binary(bin: Option<PathBuf>) -> Self {
        Self::new(LaunchSource::Fixed(bin))
    }

    fn new(launch: LaunchSource) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            launch,
            http,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn resolve_bin(&self) -> Option<PathBuf> {
        match &self.launch {
            LaunchSource::Fixed(p) => p.clone().filter(|p| p.is_file()),
            LaunchSource::Scan(dir) => resolve_bin(dir, |k| std::env::var_os(k)),
        }
    }

    pub fn is_installed(&self) -> bool {
        self.resolve_bin().is_some()
    }

    fn session_base(&self, session: &str) -> Result<String> {
        lock(&self.sessions)
            .get(session)
            .map(|s| s.base.clone())
            .ok_or_else(|| err(format!("no such agent session {session}")))
    }

    async fn await_config(&self, base: &str, supervisor: &RuntimeSupervisor) -> Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if supervisor.state() == SupervisorState::GaveUp {
                return Err(err("opencode serve keeps crashing on startup"));
            }
            if self
                .http
                .get(format!("{base}/config"))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(err("opencode serve did not become ready in time"));
            }
            tokio::time::sleep(HEALTH_POLL).await;
        }
    }
}

#[async_trait]
impl AgentAdapter for OpenCodeAdapter {
    fn id(&self) -> &str {
        ADAPTER_ID
    }

    fn kind(&self) -> AgentKind {
        AgentKind::OpenCode
    }

    async fn health(&self) -> Health {
        let base = lock(&self.sessions).values().next().map(|s| s.base.clone());
        let Some(base) = base else {
            return Health::Unknown;
        };
        match self.http.get(format!("{base}/config")).send().await {
            Ok(r) if r.status().is_success() => Health::Healthy,
            _ => Health::Unhealthy,
        }
    }

    async fn open_session(&self, spec: &SessionSpec) -> Result<String> {
        let bin = self
            .resolve_bin()
            .ok_or_else(|| err("OpenCode is not installed — run agent setup first"))?;
        let port = free_loopback_port()?;
        let base = format!("http://127.0.0.1:{port}");

        let mut ss = SpawnSpec::new(&bin);
        ss.args = vec![
            "serve".into(),
            "--port".into(),
            port.to_string(),
            "--hostname".into(),
            "127.0.0.1".into(),
            "--print-logs".into(),
            "--log-level".into(),
            "WARN".into(),
        ];
        ss.cwd = Some(spec.workspace.clone());
        ss.env = vec![(
            "OPENCODE_CONFIG_CONTENT".into(),
            forced_config(spec).to_string(),
        )];

        let supervisor = RuntimeSupervisor::start(ADAPTER_ID, ss)?;
        self.await_config(&base, &supervisor).await?;

        let created: Value = self
            .http
            .post(format!("{base}/session"))
            .json(&json!({ "title": "aiwm agent session" }))
            .send()
            .await
            .map_err(|e| err(format!("create session: {e}")))?
            .error_for_status()
            .map_err(|e| err(format!("opencode rejected session create: {e}")))?
            .json()
            .await
            .map_err(|e| err(format!("bad session response: {e}")))?;
        let session_id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| err("session response had no id"))?
            .to_string();

        let (tx, rx) = mpsc::unbounded_channel();
        let reader = tokio::spawn(sse::run(
            self.http.clone(),
            base.clone(),
            session_id.clone(),
            tx,
        ));

        lock(&self.sessions).insert(
            session_id.clone(),
            Session {
                _supervisor: supervisor,
                base,
                pending_rx: Some(rx),
                reader,
            },
        );
        Ok(session_id)
    }

    async fn send(&self, session: &str, text: &str) -> Result<()> {
        let base = self.session_base(session)?;
        self.http
            .post(format!("{base}/session/{session}/prompt_async"))
            .json(&json!({ "parts": [{ "type": "text", "text": text }] }))
            .send()
            .await
            .map_err(|e| err(format!("send prompt: {e}")))?
            .error_for_status()
            .map_err(|e| err(format!("opencode rejected the prompt: {e}")))?;
        Ok(())
    }

    async fn events(&self, session: &str) -> Result<EventStream> {
        lock(&self.sessions)
            .get_mut(session)
            .and_then(|s| s.pending_rx.take())
            .ok_or_else(|| err(format!("no event stream for {session} (already taken?)")))
    }

    async fn reply_permission(
        &self,
        session: &str,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<()> {
        let base = self.session_base(session)?;
        let response = match decision {
            PermissionDecision::AllowOnce => "once",
            PermissionDecision::AllowAlways => "always",
            PermissionDecision::Deny => "reject",
        };
        self.http
            .post(format!("{base}/permission/{request_id}/reply"))
            .json(&json!({ "response": response }))
            .send()
            .await
            .map_err(|e| err(format!("permission reply: {e}")))?
            .error_for_status()
            .map_err(|e| err(format!("opencode rejected the permission reply: {e}")))?;
        Ok(())
    }

    async fn interrupt(&self, session: &str) -> Result<()> {
        let base = self.session_base(session)?;
        let _ = self
            .http
            .post(format!("{base}/session/{session}/abort"))
            .send()
            .await;
        Ok(())
    }

    async fn close_session(&self, session: &str) -> Result<()> {
        let base = self.session_base(session).ok();
        if let Some(base) = &base {
            let _ = self
                .http
                .delete(format!("{base}/session/{session}"))
                .send()
                .await;
        }
        // Drop the Session → aborts the reader and kills the child process.
        lock(&self.sessions).remove(session);
        Ok(())
    }
}

/// The forced `opencode.json` (via `OPENCODE_CONFIG_CONTENT`): only the local
/// endpoint, approval on `bash`/`edit`, no network tools.
fn forced_config(spec: &SessionSpec) -> Value {
    let model = &spec.endpoint.model;
    json!({
        "$schema": "https://opencode.ai/config.json",
        "provider": {
            "local": {
                "npm": "@ai-sdk/openai-compatible",
                "name": "AIWM local",
                "options": { "baseURL": spec.endpoint.base_url, "apiKey": "aiwm-local" },
                "models": { model: { "name": model, "tool_call": true } }
            }
        },
        "model": format!("local/{model}"),
        "enabled_providers": ["local"],
        "disabled_providers": [
            "opencode", "anthropic", "openai", "openrouter", "google",
            "github-copilot", "xai", "azure", "amazon-bedrock", "google-vertex"
        ],
        "permission": { "bash": "ask", "edit": "ask", "write": "ask", "webfetch": "deny" },
        "tools": { "webfetch": false }
    })
}

/// Resolve the `opencode` binary: `AIWM_OPENCODE_PATH`, then a managed install
/// under `<runtimes_dir>/opencode/[<version>/][bin/]opencode`, then `PATH`.
fn resolve_bin(runtimes_dir: &Path, lookup: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(raw) = lookup(BIN_ENV).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(raw);
        if p.is_file() {
            return Some(p);
        }
    }
    let root = runtimes_dir.join(ADAPTER_ID);
    let direct = [root.join(SERVER_EXE), root.join("bin").join(SERVER_EXE)];
    if let Some(hit) = direct.into_iter().find(|p| p.is_file()) {
        return Some(hit);
    }
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let sub = [
                    entry.path().join(SERVER_EXE),
                    entry.path().join("bin").join(SERVER_EXE),
                ];
                if let Some(hit) = sub.into_iter().find(|p| p.is_file()) {
                    return Some(hit);
                }
            }
        }
    }
    if let Some(path_var) = lookup("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(SERVER_EXE);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::EndpointConfig;

    fn spec() -> SessionSpec {
        SessionSpec {
            workspace: PathBuf::from("E:\\proj"),
            allowed_paths: vec![],
            toolset: None,
            endpoint: EndpointConfig {
                base_url: "http://127.0.0.1:48999/v1".into(),
                model: "qwen2.5-coder".into(),
            },
        }
    }

    #[test]
    fn forced_config_locks_the_provider_and_permissions() {
        let c = forced_config(&spec());
        assert_eq!(c["model"], "local/qwen2.5-coder");
        assert_eq!(
            c["provider"]["local"]["options"]["baseURL"],
            "http://127.0.0.1:48999/v1"
        );
        assert_eq!(c["enabled_providers"], json!(["local"]));
        assert!(c["disabled_providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "opencode"));
        assert_eq!(c["permission"]["bash"], "ask");
        assert_eq!(c["permission"]["webfetch"], "deny");
    }

    #[tokio::test]
    async fn not_installed_is_a_clear_error() {
        let a = OpenCodeAdapter::with_binary(None);
        assert!(!a.is_installed());
        assert_eq!(a.health().await, Health::Unknown);
        let e = a.open_session(&spec()).await.unwrap_err();
        assert!(e.to_string().contains("not installed"), "{e}");
    }

    #[test]
    fn resolve_bin_prefers_the_env_override() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("opencode.exe");
        std::fs::write(&exe, b"x").unwrap();
        let s = exe.to_string_lossy().into_owned();
        let got = resolve_bin(Path::new("Z:\\nope"), |k| {
            (k == BIN_ENV).then(|| OsString::from(s.clone()))
        });
        assert_eq!(got, Some(exe));
    }
}
