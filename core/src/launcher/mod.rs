//! External agent launcher: open a real, visible, INDEPENDENT terminal
//! running OpenCode or Hermes against a local coding model, for the user to
//! drive directly — unlike `capability::agent`'s embedded, AIWM-supervised
//! sessions.
//!
//! Deliberate architecture deviation: every other spawned process in this
//! app — `runtime::JobObject`, `sidecar`, every `RuntimeAdapter`, both
//! `capability::agent` runtimes — is assigned to a Windows Job Object so it
//! can never outlive AIWM. A launched terminal is the opposite on purpose:
//! it's a standalone, user-driven session meant to keep running after AIWM
//! closes ([`spawn::launch_detached`], proven detached by
//! `tests/launcher_detached_spawn.rs`).
//!
//! One real limitation this does **not** paper over: the *model* backing the
//! terminal is still `llama-server`, which stays Job-Object-supervised like
//! every other runtime. Closing AIWM kills the model even though the
//! terminal window survives — the user gets a dead endpoint, not a crashed
//! terminal. There is no way around this without detaching `llama-server`
//! from AIWM's own supervision too, which would break the VRAM scheduler's
//! "one thing owns the GPU" invariant everywhere else in the app. Surfaced to
//! the user, not hidden.

pub mod config;
pub mod spawn;

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;

use crate::capability::agent::CodingRuntime;
use crate::db::{Database, Model};
use crate::select::{self, AutoPreference};
use crate::{CoreError, Result};
use spawn::SpawnSpec;

/// VRAM to assume for a coding model whose import left no estimate. Mirrors
/// `capability::agent`'s own fallback for the same situation.
const CODING_VRAM_FALLBACK_MB: u64 = 8192;

fn launcher_err(msg: impl fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "launcher".into(),
        message: msg.to_string(),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A tool [`Launcher`] can open in its own terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchTool {
    OpenCode,
    Hermes,
}

impl LaunchTool {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenCode => "opencode",
            Self::Hermes => "hermes",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "opencode" => Some(Self::OpenCode),
            "hermes" => Some(Self::Hermes),
            _ => None,
        }
    }
}

/// Where to find each tool's real, installed binary. AIWM already resolves
/// these for the embedded adapters (`agent::OpenCodeAdapter::binary`,
/// the private `agent::hermes::HermesAgentAdapter::binary`) — the launcher
/// reuses them rather than re-deriving PATH/venv lookup.
pub trait ToolBinaries: Send + Sync + fmt::Debug {
    fn binary(&self, tool: LaunchTool) -> Option<PathBuf>;
}

/// The production [`ToolBinaries`]: defers to the same embedded adapters
/// [`crate::App`] already holds for its in-app agent sessions, so "is this
/// tool installed" always agrees between the two features.
#[derive(Debug)]
pub struct AdapterBinaries {
    pub opencode: Arc<crate::agent::OpenCodeAdapter>,
    pub hermes: Arc<crate::agent::HermesAgentAdapter>,
}

impl ToolBinaries for AdapterBinaries {
    fn binary(&self, tool: LaunchTool) -> Option<PathBuf> {
        match tool {
            LaunchTool::OpenCode => self.opencode.binary(),
            LaunchTool::Hermes => self.hermes.binary(),
        }
    }
}

/// Actually opens the terminal window. A trait so orchestration tests don't
/// pop up a real console — production wiring uses [`RealTerminalLauncher`].
#[async_trait]
pub trait TerminalLauncher: Send + Sync + fmt::Debug {
    async fn launch(&self, spec: &SpawnSpec) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct RealTerminalLauncher;

#[async_trait]
impl TerminalLauncher for RealTerminalLauncher {
    async fn launch(&self, spec: &SpawnSpec) -> Result<()> {
        spawn::launch_detached(spec).await
    }
}

/// A request to open an external terminal.
#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub tool: LaunchTool,
    /// `None` = Auto, same "coding"-role pick as an agent profile without an
    /// explicit model.
    pub model_id: Option<String>,
    pub workspace: PathBuf,
}

/// What's currently pinned for an external launch, if any.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchInfo {
    pub tool: LaunchTool,
    pub model_id: String,
    pub model_name: String,
    pub base_url: String,
    pub workspace: PathBuf,
    /// Set when the picked model doesn't meet a tool's own requirements
    /// (currently: Hermes' 64K context floor). Not a hard block — the user
    /// may know better than AIWM's metadata — but surfaced so a failure to
    /// start isn't a mystery.
    pub warning: Option<String>,
}

/// The launcher subsystem. One per [`crate::App`]. MVP: one active launch at
/// a time — a real constraint, not just simplicity, since only one GGUF
/// model can be resident on `llama-server` at once regardless.
#[derive(Debug)]
pub struct Launcher {
    db: Database,
    coding: Arc<dyn CodingRuntime>,
    binaries: Arc<dyn ToolBinaries>,
    terminal: Arc<dyn TerminalLauncher>,
    auto_preference: AutoPreference,
    active: Mutex<Option<LaunchInfo>>,
}

impl Launcher {
    pub fn new(
        db: Database,
        coding: Arc<dyn CodingRuntime>,
        binaries: Arc<dyn ToolBinaries>,
    ) -> Self {
        Self {
            db,
            coding,
            binaries,
            terminal: Arc::new(RealTerminalLauncher),
            auto_preference: AutoPreference::default(),
            active: Mutex::new(None),
        }
    }

    #[must_use]
    pub fn with_auto_preference(mut self, pref: AutoPreference) -> Self {
        self.auto_preference = pref;
        self
    }

    #[must_use]
    pub fn with_terminal_launcher(mut self, terminal: Arc<dyn TerminalLauncher>) -> Self {
        self.terminal = terminal;
        self
    }

    /// What's pinned for an external launch right now, if anything.
    pub fn status(&self) -> Option<LaunchInfo> {
        lock(&self.active).clone()
    }

    /// Resolve the model, pin it on `llama-server`, write the tool's config,
    /// and open a detached terminal running it. Rolls back the pin if the
    /// terminal fails to open.
    pub async fn launch(&self, req: LaunchRequest) -> Result<LaunchInfo> {
        if lock(&self.active).is_some() {
            return Err(launcher_err(
                "an external launch is already tracked — stop it before starting another",
            ));
        }
        let bin = self
            .binaries
            .binary(req.tool)
            .ok_or_else(|| launcher_err(format!("{} is not installed", req.tool.as_str())))?;
        if !req.workspace.is_dir() {
            return Err(launcher_err(format!(
                "{} is not a folder",
                req.workspace.display()
            )));
        }

        let model = self.resolve_model(req.model_id.as_deref()).await?;
        let vram_mb = model
            .vram_estimate_mb
            .and_then(|mb| u64::try_from(mb).ok())
            .filter(|mb| *mb > 0)
            .unwrap_or(CODING_VRAM_FALLBACK_MB);
        let base_url = self.coding.acquire(&model.id, vram_mb).await?;

        let warning = matches!(req.tool, LaunchTool::Hermes)
            .then(|| config::hermes_context_warning(&model))
            .flatten();

        let spawn_result = self.spawn_for(req.tool, &bin, &req.workspace, &base_url, &model);
        let spec = match spawn_result {
            Ok(spec) => spec,
            Err(e) => {
                self.coding.release(&model.id).await;
                return Err(e);
            }
        };

        if let Err(e) = self.terminal.launch(&spec).await {
            self.coding.release(&model.id).await;
            return Err(e);
        }

        let info = LaunchInfo {
            tool: req.tool,
            model_id: model.id.clone(),
            model_name: model.name.clone(),
            base_url,
            workspace: req.workspace,
            warning,
        };
        *lock(&self.active) = Some(info.clone());
        Ok(info)
    }

    /// Release the pinned model. Cannot and does not touch the external
    /// terminal window itself — by design (see module docs), AIWM has no
    /// handle on it. This only frees the GPU/RAM slot; the user closes their
    /// own terminal whenever they're done with it. A no-op when nothing is
    /// tracked.
    pub async fn stop(&self) -> Result<()> {
        let Some(info) = lock(&self.active).take() else {
            return Ok(());
        };
        self.coding.release(&info.model_id).await;
        Ok(())
    }

    fn spawn_for(
        &self,
        tool: LaunchTool,
        bin: &std::path::Path,
        workspace: &std::path::Path,
        base_url: &str,
        model: &Model,
    ) -> Result<SpawnSpec> {
        match tool {
            LaunchTool::OpenCode => Ok(SpawnSpec {
                title: format!("AIWM: OpenCode - {}", model.name),
                cwd: workspace.to_path_buf(),
                program: bin.to_path_buf(),
                args: vec![],
                env: vec![(
                    "OPENCODE_CONFIG_CONTENT".into(),
                    config::opencode_config_content(base_url, model),
                )],
            }),
            LaunchTool::Hermes => {
                let home = std::env::temp_dir()
                    .join("aiwm-launcher-hermes")
                    .join(uuid::Uuid::now_v7().to_string());
                std::fs::create_dir_all(&home).map_err(|e| {
                    launcher_err(format!("could not create a Hermes home dir: {e}"))
                })?;
                std::fs::write(
                    home.join("config.yaml"),
                    config::hermes_config_yaml(base_url, model),
                )
                .map_err(|e| launcher_err(format!("could not write Hermes' config.yaml: {e}")))?;
                Ok(SpawnSpec {
                    title: format!("AIWM: Hermes - {}", model.name),
                    cwd: workspace.to_path_buf(),
                    program: bin.to_path_buf(),
                    args: vec![],
                    env: vec![("HERMES_HOME".into(), home.to_string_lossy().into_owned())],
                })
            }
        }
    }

    async fn resolve_model(&self, explicit: Option<&str>) -> Result<Model> {
        match explicit {
            Some(id) => self
                .db
                .models()
                .get(id)
                .await?
                .ok_or_else(|| CoreError::Config(format!("model {id} is not in the library"))),
            None => select::pick_for_role(
                &self.db,
                "coding",
                self.coding.budget_mb(),
                self.auto_preference,
            )
            .await?
            .ok_or_else(|| {
                CoreError::Config(
                    "no model carries the 'coding' role — import a coding GGUF and mark it 'coding'"
                        .into(),
                )
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewModel;

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

    #[derive(Debug)]
    struct StubBinaries(Option<PathBuf>);
    impl ToolBinaries for StubBinaries {
        fn binary(&self, _tool: LaunchTool) -> Option<PathBuf> {
            self.0.clone()
        }
    }

    #[derive(Debug, Default)]
    struct StubTerminal {
        launched: Mutex<Vec<SpawnSpec>>,
        fail: bool,
    }

    #[async_trait]
    impl TerminalLauncher for StubTerminal {
        async fn launch(&self, spec: &SpawnSpec) -> Result<()> {
            if self.fail {
                return Err(launcher_err("could not open a terminal"));
            }
            lock(&self.launched).push(spec.clone());
            Ok(())
        }
    }

    async fn coding_model(db: &Database, ctx_max: Option<i64>) -> Model {
        let id = db
            .models()
            .insert(NewModel {
                name: "Qwen2.5 Coder".into(),
                format: "gguf".into(),
                file_path: "E:\\models\\coder.gguf".into(),
                size_bytes: 4096,
                source: "manual".into(),
                vram_estimate_mb: Some(6000),
                ctx_max,
                roles: vec!["coding".into()],
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id;
        db.models().get(&id).await.unwrap().unwrap()
    }

    fn launcher(db: &Database, coding: Arc<StubCoding>, bin: Option<PathBuf>) -> Launcher {
        Launcher::new(db.clone(), coding, Arc::new(StubBinaries(bin)))
            .with_terminal_launcher(Arc::new(StubTerminal::default()))
    }

    fn req(tool: LaunchTool, workspace: PathBuf) -> LaunchRequest {
        LaunchRequest {
            tool,
            model_id: None,
            workspace,
        }
    }

    #[tokio::test]
    async fn launch_needs_the_tool_installed() {
        let db = Database::connect_in_memory().await.unwrap();
        let l = launcher(&db, Arc::default(), None);
        let err = l
            .launch(req(LaunchTool::OpenCode, std::env::temp_dir()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not installed"), "{err}");
    }

    #[tokio::test]
    async fn launch_without_a_coding_model_is_a_clear_error() {
        let db = Database::connect_in_memory().await.unwrap();
        let l = launcher(&db, Arc::default(), Some(PathBuf::from("opencode.exe")));
        let err = l
            .launch(req(LaunchTool::OpenCode, std::env::temp_dir()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("coding"), "{err}");
    }

    #[tokio::test]
    async fn launch_needs_a_real_workspace_folder() {
        let db = Database::connect_in_memory().await.unwrap();
        let _m = coding_model(&db, Some(128_000)).await;
        let l = launcher(&db, Arc::default(), Some(PathBuf::from("opencode.exe")));
        let err = l
            .launch(req(LaunchTool::OpenCode, PathBuf::from("Z:\\nope\\nope")))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("folder"), "{err}");
    }

    #[tokio::test]
    async fn a_blocked_scheduler_pins_nothing_and_surfaces() {
        let db = Database::connect_in_memory().await.unwrap();
        let _m = coding_model(&db, Some(128_000)).await;
        let coding = Arc::new(StubCoding {
            fail: true,
            ..Default::default()
        });
        let l = launcher(&db, coding, Some(PathBuf::from("opencode.exe")));
        let err = l
            .launch(req(LaunchTool::OpenCode, std::env::temp_dir()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("VRAM"), "{err}");
        assert!(l.status().is_none());
    }

    #[tokio::test]
    async fn a_successful_launch_is_tracked_and_stop_releases_the_model() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = coding_model(&db, Some(128_000)).await;
        let coding = Arc::<StubCoding>::default();
        let l = launcher(&db, coding.clone(), Some(PathBuf::from("opencode.exe")));

        let info = l
            .launch(req(LaunchTool::OpenCode, std::env::temp_dir()))
            .await
            .unwrap();
        assert_eq!(info.model_id, m.id);
        assert_eq!(info.warning, None);
        assert_eq!(
            lock(&coding.acquired).as_slice(),
            std::slice::from_ref(&m.id)
        );
        assert_eq!(l.status().map(|s| s.model_id), Some(m.id.clone()));

        l.stop().await.unwrap();
        assert_eq!(
            lock(&coding.released).as_slice(),
            std::slice::from_ref(&m.id)
        );
        assert!(l.status().is_none());
    }

    #[tokio::test]
    async fn stop_without_an_active_launch_is_a_harmless_no_op() {
        let db = Database::connect_in_memory().await.unwrap();
        let l = launcher(&db, Arc::default(), None);
        l.stop().await.unwrap();
    }

    #[tokio::test]
    async fn only_one_launch_runs_at_a_time() {
        let db = Database::connect_in_memory().await.unwrap();
        let _m = coding_model(&db, Some(128_000)).await;
        let l = launcher(&db, Arc::default(), Some(PathBuf::from("opencode.exe")));

        l.launch(req(LaunchTool::OpenCode, std::env::temp_dir()))
            .await
            .unwrap();
        let err = l
            .launch(req(LaunchTool::OpenCode, std::env::temp_dir()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already"), "{err}");
    }

    #[tokio::test]
    async fn a_failed_terminal_spawn_rolls_back_the_pin() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = coding_model(&db, Some(128_000)).await;
        let coding = Arc::<StubCoding>::default();
        let l = Launcher::new(
            db.clone(),
            coding.clone(),
            Arc::new(StubBinaries(Some(PathBuf::from("opencode.exe")))),
        )
        .with_terminal_launcher(Arc::new(StubTerminal {
            fail: true,
            ..Default::default()
        }));

        let err = l
            .launch(req(LaunchTool::OpenCode, std::env::temp_dir()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("terminal"), "{err}");
        assert_eq!(lock(&coding.released).as_slice(), [m.id]);
        assert!(l.status().is_none());
    }

    #[tokio::test]
    async fn hermes_below_the_context_floor_still_launches_but_warns() {
        let db = Database::connect_in_memory().await.unwrap();
        let _m = coding_model(&db, Some(32_000)).await;
        let l = launcher(&db, Arc::default(), Some(PathBuf::from("hermes.exe")));

        let info = l
            .launch(req(LaunchTool::Hermes, std::env::temp_dir()))
            .await
            .unwrap();
        assert!(info.warning.is_some());
    }

    #[tokio::test]
    async fn opencode_never_warns_about_the_hermes_context_floor() {
        let db = Database::connect_in_memory().await.unwrap();
        let _m = coding_model(&db, Some(32_000)).await;
        let l = launcher(&db, Arc::default(), Some(PathBuf::from("opencode.exe")));

        let info = l
            .launch(req(LaunchTool::OpenCode, std::env::temp_dir()))
            .await
            .unwrap();
        assert_eq!(info.warning, None);
    }
}
