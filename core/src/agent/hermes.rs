//! The Hermes Agent adapter (Phase 5.4, ADR-021 — the second agent runtime).
//!
//! Hermes (`hermes-agent`, Nous Research) is a Python agent installed via `uv`
//! (5.4b). We run one `hermes gateway` HTTP server **per session** in a managed,
//! per-session `HERMES_HOME` — its `config.yaml` and `.env` are **forced** (5.2
//! sandbox, ADR-010): the model is the local `llama-server` and nothing else,
//! `redact_secrets` / `redact_pii` on, `cwd` = the workspace. The child's
//! environment is scrubbed of cloud credentials ([`super::scrubbed_env`]).
//!
//! Each turn is `POST /api/sessions/{id}/chat/stream` → an SSE stream translated
//! into [`AgentEvent`]s. Approvals resume the run via `POST /v1/runs/{id}/approval`.
//!
//! The exact event names and the `config.yaml` sandbox keys are **researched,
//! not verified** (the docs are incomplete). The SSE mapper accepts several
//! plausible spellings; a real `hermes` run will calibrate the corners — like
//! the ComfyUI `/history` output key was for video.

mod sse;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::{AgentAdapter, AgentEvent, AgentKind, EventStream, PermissionDecision, SessionSpec};
use crate::runtime::{free_loopback_port, Health, RuntimeSupervisor, SpawnSpec, SupervisorState};
use crate::{CoreError, Result};

pub(super) const ADAPTER_ID: &str = "hermes";
/// Env override for the `hermes` executable (tests point it at the fixture).
const BIN_ENV: &str = "AIWM_HERMES_PATH";
const SERVER_EXE: &str = if cfg!(windows) {
    "hermes.exe"
} else {
    "hermes"
};
const READY_TIMEOUT: Duration = Duration::from_secs(45);
const HEALTH_POLL: Duration = Duration::from_millis(250);
/// Local endpoints stream slowly; give a turn room before the read times out.
const STREAM_READ_TIMEOUT_SECS: u32 = 1800;

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
    /// Re-scan `<runtimes_dir>/hermes/` + `AIWM_HERMES_PATH` on every open.
    Scan(PathBuf),
}

struct Session {
    /// Owns the `hermes gateway` child — dropping it kills the process.
    _supervisor: RuntimeSupervisor,
    base: String,
    /// Bearer token the gateway expects on every request.
    key: String,
    /// Managed `HERMES_HOME` for this session — removed on close.
    home: PathBuf,
    tx: mpsc::UnboundedSender<AgentEvent>,
    /// Taken once by [`AgentAdapter::events`].
    pending_rx: Option<EventStream>,
    /// The id of the run the turn stream is on — for approval / stop.
    run: Arc<Mutex<Option<String>>>,
    /// The in-flight turn's SSE forwarder.
    turn: Option<JoinHandle<()>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(t) = self.turn.take() {
            t.abort();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("base", &self.base).finish()
    }
}

#[derive(Debug)]
pub struct HermesAgentAdapter {
    launch: LaunchSource,
    /// Root for the per-session `HERMES_HOME` directories.
    home_root: PathBuf,
    http: reqwest::Client,
    sessions: Mutex<HashMap<String, Session>>,
}

impl HermesAgentAdapter {
    /// Re-scans the managed install dir + `AIWM_HERMES_PATH` on demand; keeps
    /// per-session homes under `<runtimes_dir>/hermes/homes/`.
    pub fn discover(runtimes_dir: &Path) -> Self {
        let dir = runtimes_dir.join(ADAPTER_ID);
        Self::new(LaunchSource::Scan(dir.clone()), dir.join("homes"))
    }

    /// Fixed launch path + home root (tests point this at `aiwm-fake-hermes`).
    pub fn with_binary(bin: Option<PathBuf>, home_root: PathBuf) -> Self {
        Self::new(LaunchSource::Fixed(bin), home_root)
    }

    fn new(launch: LaunchSource, home_root: PathBuf) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            launch,
            home_root,
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

    /// `(base_url, bearer_key)` for a live session.
    fn endpoint(&self, session: &str) -> Result<(String, String)> {
        lock(&self.sessions)
            .get(session)
            .map(|s| (s.base.clone(), s.key.clone()))
            .ok_or_else(|| err(format!("no such agent session {session}")))
    }

    async fn await_ready(&self, base: &str, key: &str, sup: &RuntimeSupervisor) -> Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if sup.state() == SupervisorState::GaveUp {
                return Err(err("hermes gateway keeps crashing on startup"));
            }
            if self
                .http
                .get(format!("{base}/health"))
                .bearer_auth(key)
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(err("hermes gateway did not become ready in time"));
            }
            tokio::time::sleep(HEALTH_POLL).await;
        }
    }
}

#[async_trait]
impl AgentAdapter for HermesAgentAdapter {
    fn id(&self) -> &str {
        ADAPTER_ID
    }

    fn kind(&self) -> AgentKind {
        AgentKind::Hermes
    }

    async fn health(&self) -> Health {
        let ep = lock(&self.sessions)
            .values()
            .next()
            .map(|s| (s.base.clone(), s.key.clone()));
        let Some((base, key)) = ep else {
            return Health::Unknown;
        };
        match self
            .http
            .get(format!("{base}/health"))
            .bearer_auth(key)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => Health::Healthy,
            _ => Health::Unhealthy,
        }
    }

    async fn open_session(&self, spec: &SessionSpec) -> Result<String> {
        let bin = self
            .resolve_bin()
            .ok_or_else(|| err("Hermes is not installed — run agent setup first"))?;
        let port = free_loopback_port()?;
        let base = format!("http://127.0.0.1:{port}");
        let key = Uuid::now_v7().to_string();

        let home = self.home_root.join(Uuid::now_v7().to_string());
        write_home(&home, spec).map_err(|e| err(format!("write HERMES_HOME: {e}")))?;

        let spawn = spawn_spec(&bin, &home, &key, port, spec);
        // Any failure from here on drops `supervisor` (kills the gateway) and
        // must also take the managed home dir with it.
        let started = async {
            let supervisor = RuntimeSupervisor::start(ADAPTER_ID, spawn)?;
            self.await_ready(&base, &key, &supervisor).await?;
            let created: Value = self
                .http
                .post(format!("{base}/api/sessions"))
                .bearer_auth(&key)
                .json(&json!({}))
                .send()
                .await
                .map_err(|e| err(format!("create session: {e}")))?
                .error_for_status()
                .map_err(|e| err(format!("hermes rejected session create: {e}")))?
                .json()
                .await
                .map_err(|e| err(format!("bad session response: {e}")))?;
            let id = created
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| err("session response had no id"))?
                .to_string();
            Ok::<_, CoreError>((supervisor, id))
        }
        .await;
        let (supervisor, session_id) = match started {
            Ok(v) => v,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&home);
                return Err(e);
            }
        };

        let (tx, rx) = mpsc::unbounded_channel();
        lock(&self.sessions).insert(
            session_id.clone(),
            Session {
                _supervisor: supervisor,
                base,
                key,
                home,
                tx,
                pending_rx: Some(rx),
                run: Arc::new(Mutex::new(None)),
                turn: None,
            },
        );
        Ok(session_id)
    }

    async fn send(&self, session: &str, text: &str) -> Result<()> {
        let (base, key) = self.endpoint(session)?;
        let (tx, run) = {
            let mut g = lock(&self.sessions);
            let s = g
                .get_mut(session)
                .ok_or_else(|| err(format!("no such agent session {session}")))?;
            if let Some(t) = s.turn.take() {
                t.abort();
            }
            (s.tx.clone(), Arc::clone(&s.run))
        };

        let resp = self
            .http
            .post(format!("{base}/api/sessions/{session}/chat/stream"))
            .bearer_auth(&key)
            .json(&json!({ "input": text }))
            .send()
            .await
            .map_err(|e| err(format!("send prompt: {e}")))?
            .error_for_status()
            .map_err(|e| err(format!("hermes rejected the prompt: {e}")))?;

        let handle = tokio::spawn(sse::run(resp, tx, run));
        if let Some(s) = lock(&self.sessions).get_mut(session) {
            s.turn = Some(handle);
        }
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
        let (base, key) = self.endpoint(session)?;
        self.http
            .post(format!("{base}/v1/runs/{request_id}/approval"))
            .bearer_auth(&key)
            .json(&json!({ "approved": !matches!(decision, PermissionDecision::Deny) }))
            .send()
            .await
            .map_err(|e| err(format!("approval: {e}")))?
            .error_for_status()
            .map_err(|e| err(format!("hermes rejected the approval: {e}")))?;
        Ok(())
    }

    async fn interrupt(&self, session: &str) -> Result<()> {
        let (base, key) = self.endpoint(session)?;
        let run = lock(&self.sessions)
            .get(session)
            .and_then(|s| lock(&s.run).clone());
        if let Some(run) = run {
            let _ = self
                .http
                .post(format!("{base}/v1/runs/{run}/stop"))
                .bearer_auth(&key)
                .send()
                .await;
        }
        Ok(())
    }

    async fn close_session(&self, session: &str) -> Result<()> {
        if let Ok((base, key)) = self.endpoint(session) {
            let _ = self
                .http
                .delete(format!("{base}/api/sessions/{session}"))
                .bearer_auth(&key)
                .send()
                .await;
        }
        // Drop the Session → aborts the turn, kills the gateway, removes HERMES_HOME.
        lock(&self.sessions).remove(session);
        Ok(())
    }
}

/// The `hermes gateway` launch spec: the managed `HERMES_HOME`, the API-server
/// settings, `cwd` = the workspace, cloud credentials scrubbed. Env vars beat
/// config files in Hermes, so setting the API-server config here is what makes
/// it authoritative.
fn spawn_spec(bin: &Path, home: &Path, key: &str, port: u16, spec: &SessionSpec) -> SpawnSpec {
    let mut ss = SpawnSpec::new(bin);
    ss.args = vec!["gateway".into()];
    ss.cwd = Some(spec.workspace.clone());
    ss.env = vec![
        ("HERMES_HOME".into(), home.to_string_lossy().into_owned()),
        (
            "HERMES_STREAM_READ_TIMEOUT".into(),
            STREAM_READ_TIMEOUT_SECS.to_string(),
        ),
        ("API_SERVER_ENABLED".into(), "true".into()),
        ("API_SERVER_HOST".into(), "127.0.0.1".into()),
        ("API_SERVER_PORT".into(), port.to_string()),
        ("API_SERVER_KEY".into(), key.to_string()),
    ];
    ss.env_remove = super::scrubbed_env(&["HERMES_CONFIG"]);
    ss
}

/// Write the forced `config.yaml` into `home`.
fn write_home(home: &Path, spec: &SessionSpec) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    std::fs::write(home.join("config.yaml"), forced_config_yaml(spec))
}

/// The forced `config.yaml`: only the local `llama-server`, redaction on. The
/// sandbox keys here are a best guess (docs incomplete) — 5.4b's real run
/// calibrates them.
fn forced_config_yaml(spec: &SessionSpec) -> String {
    let model = yaml_scalar(&spec.endpoint.model);
    let base = yaml_scalar(&spec.endpoint.base_url);
    format!(
        "model:\n\
        \x20 default: {model}\n\
        \x20 provider: custom\n\
        \x20 base_url: {base}\n\
        \x20 context_length: 32000\n\
        providers:\n\
        \x20 aiwm-local:\n\
        \x20   api: {base}\n\
        \x20   api_key: aiwm-local\n\
        \x20   default_model: {model}\n\
        security:\n\
        \x20 redact_secrets: true\n\
        privacy:\n\
        \x20 redact_pii: true\n\
        permissions:\n\
        \x20 mode: ask\n\
        tools:\n\
        \x20 web_search: false\n\
        \x20 browser: false\n"
    )
}

/// Quote a value if it needs it — enough for URLs and model names.
fn yaml_scalar(s: &str) -> String {
    if s.is_empty() || s.contains([':', '#', '\n', '"', '\'']) {
        format!("{s:?}")
    } else {
        s.to_string()
    }
}

/// Resolve the `hermes` binary: `AIWM_HERMES_PATH`, then a managed venv under
/// `<dir>/[<ver>/][bin|Scripts/]hermes`, then `PATH`.
fn resolve_bin(dir: &Path, lookup: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(raw) = lookup(BIN_ENV).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(raw);
        if p.is_file() {
            return Some(p);
        }
    }
    let sub = ["", "bin", "Scripts"];
    for base in [dir.to_path_buf()] {
        for s in sub {
            let cand = if s.is_empty() {
                base.join(SERVER_EXE)
            } else {
                base.join(s).join(SERVER_EXE)
            };
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                for s in sub {
                    let cand = if s.is_empty() {
                        entry.path().join(SERVER_EXE)
                    } else {
                        entry.path().join(s).join(SERVER_EXE)
                    };
                    if cand.is_file() {
                        return Some(cand);
                    }
                }
            }
        }
    }
    if let Some(path_var) = lookup("PATH") {
        for d in std::env::split_paths(&path_var) {
            let cand = d.join(SERVER_EXE);
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
    fn forced_config_pins_the_local_endpoint_and_redaction() {
        let y = forced_config_yaml(&spec());
        assert!(y.contains("provider: custom"));
        assert!(y.contains("base_url: \"http://127.0.0.1:48999/v1\""));
        assert!(y.contains("default: qwen2.5-coder"));
        assert!(y.contains("redact_secrets: true"));
        assert!(y.contains("redact_pii: true"));
        assert!(y.contains("web_search: false"));
    }

    #[test]
    fn spawn_spec_uses_the_managed_home_and_scrubs_credentials() {
        let ss = spawn_spec(
            Path::new("hermes"),
            Path::new("Z:\\home"),
            "sekret",
            41234,
            &spec(),
        );
        assert_eq!(ss.args, vec!["gateway".to_string()]);
        assert_eq!(ss.cwd.as_deref(), Some(Path::new("E:\\proj")));
        assert!(ss
            .env
            .iter()
            .any(|(k, v)| k == "HERMES_HOME" && v == "Z:\\home"));
        assert!(ss
            .env
            .iter()
            .any(|(k, v)| k == "API_SERVER_KEY" && v == "sekret"));
        assert!(ss
            .env
            .iter()
            .any(|(k, v)| k == "API_SERVER_PORT" && v == "41234"));
        assert!(ss.env_remove.iter().any(|k| k == "ANTHROPIC_API_KEY"));
    }

    #[test]
    fn write_home_lays_down_the_forced_config() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("h1");
        write_home(&home, &spec()).unwrap();
        let cfg = std::fs::read_to_string(home.join("config.yaml")).unwrap();
        assert!(cfg.contains("provider: custom"));
    }

    #[tokio::test]
    async fn not_installed_is_a_clear_error() {
        let tmp = tempfile::tempdir().unwrap();
        let a = HermesAgentAdapter::with_binary(None, tmp.path().to_path_buf());
        assert!(!a.is_installed());
        assert_eq!(a.health().await, Health::Unknown);
        let e = a.open_session(&spec()).await.unwrap_err();
        assert!(e.to_string().contains("not installed"), "{e}");
    }

    #[test]
    fn resolve_bin_prefers_the_env_override() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join(SERVER_EXE);
        std::fs::write(&exe, b"x").unwrap();
        let s = exe.to_string_lossy().into_owned();
        let got = resolve_bin(Path::new("Z:\\nope"), |k| {
            (k == BIN_ENV).then(|| OsString::from(s.clone()))
        });
        assert_eq!(got, Some(exe));
    }
}
