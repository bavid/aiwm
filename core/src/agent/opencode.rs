//! The OpenCode adapter (Phase 5.1b, ADR-021 — first agent adapter).
//!
//! OpenCode ships as a single self-contained `opencode` binary. We supervise one
//! `opencode serve` process **per session** (MVP: the scheduler pins one agent
//! model at a time anyway), started with `cwd` = the workspace and its whole
//! config **forced** through the `OPENCODE_CONFIG_CONTENT` env var.
//!
//! The forced config is the MVP sandbox (5.2, ADR-010 — config-level, not
//! process-level): the endpoint is the local `llama-server` and nothing else,
//! every shell command and file edit needs approval, edits are confined to the
//! workspace, reads outside it are denied bar the profile's extra roots, and
//! every network tool is off. The child's environment is also scrubbed of cloud
//! credentials ([`super::scrubbed_env`]).
//!
//! Its `GET /event` SSE stream is translated into [`AgentEvent`]s. The exact
//! real event shapes (`message.part.updated`, `permission.asked`, idle) are from
//! the 5.0 probe; a real coding-model run will calibrate the corners (like the
//! ComfyUI `/history` output key was for video).

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

        let supervisor = RuntimeSupervisor::start(ADAPTER_ID, build_spawn_spec(&bin, port, spec))?;
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

/// OpenCode config overrides scrubbed on top of [`super::CLOUD_CREDENTIAL_ENV`]
/// so a stray one on the host cannot weaken the forced config.
const OPENCODE_ENV_OVERRIDES: &[&str] = &["OPENCODE_CONFIG", "OPENCODE_API_KEY"];

/// The `opencode serve` launch spec: loopback port, `cwd` = the workspace, the
/// forced config in the env, and every known cloud credential scrubbed.
fn build_spawn_spec(bin: &Path, port: u16, spec: &SessionSpec) -> SpawnSpec {
    let mut ss = SpawnSpec::new(bin);
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
    ss.env_remove = super::scrubbed_env(OPENCODE_ENV_OVERRIDES);
    ss
}

/// `<path>/**`, forward-slashed — an OpenCode permission glob. `None` for an
/// empty path (so the caller leaves the rule fully closed rather than matching
/// everything).
fn glob_root(path: &Path) -> Option<String> {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/');
    (!s.is_empty()).then(|| format!("{s}/**"))
}

/// The forced `opencode.json` (via `OPENCODE_CONFIG_CONTENT`, which merges and
/// wins): only the local `llama-server`, every shell command and file edit needs
/// approval, edits are confined to the workspace, reads outside it are denied,
/// and every network tool is off. The MVP sandbox is **config-level** — real
/// process/FS isolation is a later, opt-in ADR (ADR-010).
fn forced_config(spec: &SessionSpec) -> Value {
    let model = &spec.endpoint.model;

    // edit/write: deny everywhere, then `ask` inside the workspace (OpenCode
    // evaluates last-match-wins, so the catch-all goes first).
    let mut edit = serde_json::Map::new();
    edit.insert("*".into(), json!("deny"));
    if let Some(ws) = glob_root(&spec.workspace) {
        edit.insert(ws, json!("ask"));
    }
    let edit = Value::Object(edit);

    // external_directory: deny everything outside `cwd`, then allow the profile's
    // extra roots — read-only, since `edit` above still denies writing there.
    let mut external = serde_json::Map::new();
    external.insert("*".into(), json!("deny"));
    for p in &spec.allowed_paths {
        if let Some(g) = glob_root(p) {
            external.insert(g, json!("allow"));
        }
    }

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
        "permission": {
            "bash": "ask",
            "edit": edit.clone(),
            "write": edit,
            "webfetch": "deny",
            "websearch": "deny",
            "external_directory": Value::Object(external)
        },
        "tools": { "webfetch": false, "websearch": false }
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
        // Every network tool is off, both as a tool and as a permission.
        assert_eq!(c["permission"]["webfetch"], "deny");
        assert_eq!(c["permission"]["websearch"], "deny");
        assert_eq!(c["tools"]["webfetch"], false);
        assert_eq!(c["tools"]["websearch"], false);
    }

    #[test]
    fn forced_config_confines_edits_to_the_workspace_and_reads_to_the_allowlist() {
        let mut s = spec();
        s.allowed_paths = vec![PathBuf::from("E:\\shared\\lib")];
        let c = forced_config(&s);

        // edit: deny by default, ask only under the workspace.
        assert_eq!(c["permission"]["edit"]["*"], "deny");
        assert_eq!(c["permission"]["edit"]["E:/proj/**"], "ask");
        assert!(c["permission"]["edit"].get("E:/shared/lib/**").is_none());
        // write is scoped the same way.
        assert_eq!(c["permission"]["write"]["E:/proj/**"], "ask");

        // reads outside cwd are denied except the profile's extra roots.
        assert_eq!(c["permission"]["external_directory"]["*"], "deny");
        assert_eq!(
            c["permission"]["external_directory"]["E:/shared/lib/**"],
            "allow"
        );

        // OpenCode is last-match-wins, so the catch-all must serialize first.
        let edit = serde_json::to_string(&c["permission"]["edit"]).unwrap();
        assert!(
            edit.find("\"*\"").unwrap() < edit.find("E:/proj").unwrap(),
            "{edit}"
        );
    }

    #[test]
    fn spawn_spec_scrubs_cloud_credentials_and_carries_the_forced_config() {
        let ss = build_spawn_spec(Path::new("opencode.exe"), 41234, &spec());
        assert!(ss.args.contains(&"41234".to_string()));
        assert!(ss.env.iter().any(|(k, _)| k == "OPENCODE_CONFIG_CONTENT"));
        for key in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "OPENCODE_CONFIG"] {
            assert!(
                ss.env_remove.iter().any(|k| k == key),
                "{key} should be scrubbed"
            );
        }
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
