//! User configuration: `%APPDATA%\AIWorkstationManager\config.toml`.
//!
//! Precedence: environment overrides (`AIWM_*`) > `config.toml` > built-in
//! defaults. A missing file is created with the defaults on first load.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::AppPaths;
use crate::{CoreError, Result};

/// Default model store. Overridable via `config.toml` or `AIWM_STORE_PATH`
/// (ADR-012: `E:\AI\models`, ~1.5 TB free).
pub const DEFAULT_STORE_PATH: &str = r"E:\AI\models";
/// Loopback port for the core HTTP/WS API (WP-6). Bound to `127.0.0.1` only.
pub const DEFAULT_API_PORT: u16 = 48160;
pub const DEFAULT_LOG_FILTER: &str = "info,aiwm_core=debug,aiwm_cored=info";
/// Ports below this are rejected (privileged / collision-prone).
const MIN_API_PORT: u16 = 1024;
/// Smallest non-auto `vram_budget_mb` the scheduler is allowed to plan against.
const MIN_VRAM_BUDGET_MB: u64 = 1024;
/// Bounds for `[llama] ctx_size` when it is not `0` (auto).
const MIN_CTX_SIZE: u32 = 512;
/// Bounds for `[llama] load_timeout_secs`.
const MIN_LOAD_TIMEOUT_SECS: u64 = 10;
const MAX_LOAD_TIMEOUT_SECS: u64 = 3600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Canonical model store directory.
    pub store_path: PathBuf,
    /// Port for the loopback core API.
    pub core_api_port: u16,
    /// When true, no feature may reach the network (Phase-wide contract, ADR-009).
    pub offline_mode: bool,
    /// `tracing` env-filter directive string.
    pub log_filter: String,
    /// VRAM budget (MB) for the scheduler. `0` = auto-detect from the GPU
    /// (falling back to [`FALLBACK_VRAM_BUDGET_MB`] when no GPU is present).
    pub vram_budget_mb: u64,
    /// `llama-server` launch options (see [`crate::runtime::LlamaServerOptions`]).
    pub llama: LlamaConfig,
    /// ComfyUI server launch options (see [`crate::runtime::ComfyOptions`]).
    pub comfyui: ComfyConfig,
}

/// Upper bound for `[comfyui].reserve_vram_mb` — reserving more than this on a
/// 16 GB card leaves too little for the model.
const MAX_RESERVE_VRAM_MB: u64 = 8192;

/// The `[comfyui]` table — ComfyUI server options the Settings UI exposes.
/// Applied at startup; a change needs a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ComfyConfig {
    /// VRAM mode: `auto` (let ComfyUI decide) | `highvram` | `normalvram` |
    /// `lowvram` | `novram`.
    pub vram_mode: String,
    /// `--reserve-vram <GB>` — VRAM ComfyUI keeps free for the OS / other apps.
    /// `0` omits the flag. Given in MB here; the flag takes GB.
    pub reserve_vram_mb: u64,
    /// Extra raw arguments appended to the ComfyUI command line, split on
    /// whitespace (power users — e.g. `--fast --use-sage-attention`).
    pub extra_args: String,
}

impl Default for ComfyConfig {
    fn default() -> Self {
        Self {
            vram_mode: "auto".to_string(),
            reserve_vram_mb: 0,
            extra_args: String::new(),
        }
    }
}

impl ComfyConfig {
    /// Build the runtime options the adapter launches with.
    pub fn to_options(&self) -> crate::runtime::ComfyOptions {
        let mut extra_args: Vec<String> = Vec::new();
        if self.reserve_vram_mb > 0 {
            extra_args.push("--reserve-vram".to_string());
            extra_args.push(format!("{:.2}", self.reserve_vram_mb as f64 / 1024.0));
        }
        extra_args.extend(self.extra_args.split_whitespace().map(str::to_string));
        crate::runtime::ComfyOptions {
            vram_mode: crate::runtime::VramMode::parse(&self.vram_mode).unwrap_or_default(),
            extra_args,
        }
    }

    fn validate(&self) -> Result<()> {
        if crate::runtime::VramMode::parse(&self.vram_mode).is_none() {
            return Err(CoreError::Config(format!(
                "comfyui.vram_mode {:?} must be auto / highvram / normalvram / lowvram / novram",
                self.vram_mode
            )));
        }
        if self.reserve_vram_mb > MAX_RESERVE_VRAM_MB {
            return Err(CoreError::Config(format!(
                "comfyui.reserve_vram_mb {} is above the {MAX_RESERVE_VRAM_MB} cap",
                self.reserve_vram_mb
            )));
        }
        Ok(())
    }
}

/// The subset of `LlamaServerOptions` a user configures via `[llama]` in
/// `config.toml` / the Settings UI. Applied at startup; a change needs a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LlamaConfig {
    /// `-ngl` — layers offloaded to the GPU. `999` = all.
    pub gpu_layers: u32,
    /// `-c` — context window. `0` = let the adapter cap the model's trained
    /// context ([`crate::compat::effective_ctx`]).
    pub ctx_size: u32,
    /// Pass `--flash-attn on`.
    pub flash_attention: bool,
    /// Pass `--jinja` (the GGUF's embedded chat template). Needed for reliable
    /// tool-call parsing — agents (5.1 / ADR-021) — and correct for plain chat.
    /// Default on; turn off + set `chat_template` if a GGUF's embedded template
    /// misbehaves.
    pub jinja: bool,
    /// `--chat-template <name>`; empty = use the GGUF's own.
    pub chat_template: String,
    /// Seconds a freshly started server has to answer `/health`.
    pub load_timeout_secs: u64,
}

impl Default for LlamaConfig {
    fn default() -> Self {
        Self {
            gpu_layers: 999,
            ctx_size: 0,
            flash_attention: true,
            jinja: true,
            chat_template: String::new(),
            load_timeout_secs: 180,
        }
    }
}

impl LlamaConfig {
    /// Build the runtime options the adapter actually launches with.
    pub fn to_options(&self) -> crate::runtime::LlamaServerOptions {
        let chat_template = self.chat_template.trim();
        crate::runtime::LlamaServerOptions {
            gpu_layers: self.gpu_layers,
            ctx_size: (self.ctx_size > 0).then_some(self.ctx_size),
            flash_attention: self.flash_attention,
            jinja: self.jinja,
            chat_template: (!chat_template.is_empty()).then(|| chat_template.to_string()),
            load_timeout: std::time::Duration::from_secs(self.load_timeout_secs),
            extra_args: Vec::new(),
        }
    }
}

/// Used when `vram_budget_mb` is `0` and no NVIDIA GPU is detected.
pub const FALLBACK_VRAM_BUDGET_MB: u64 = 8192;

impl Default for Config {
    fn default() -> Self {
        Self {
            store_path: PathBuf::from(DEFAULT_STORE_PATH),
            core_api_port: DEFAULT_API_PORT,
            offline_mode: false,
            log_filter: DEFAULT_LOG_FILTER.to_string(),
            vram_budget_mb: 0,
            llama: LlamaConfig::default(),
            comfyui: ComfyConfig::default(),
        }
    }
}

impl Config {
    /// Load config for the given layout, creating a default file if absent, then
    /// apply `AIWM_*` environment overrides and validate.
    pub fn load(paths: &AppPaths) -> Result<Self> {
        let mut cfg = Self::read_or_create(&paths.config_file())?;
        cfg.apply_overrides(|key| std::env::var(key).ok())?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Read `config.toml` as it sits on disk — no `AIWM_*` overlay — creating it
    /// with defaults if absent. The Settings UI shows and rewrites *this*, so an
    /// env override never silently swallows a user's saved value.
    pub fn read_from(paths: &AppPaths) -> Result<Self> {
        Self::read_or_create(&paths.config_file())
    }

    /// Validate, then write `config.toml`. Startup config — the caller tells the
    /// user a restart is needed for most fields to take effect.
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        self.validate()?;
        self.write(&paths.config_file())
    }

    fn read_or_create(file: &Path) -> Result<Self> {
        if file.exists() {
            let text = std::fs::read_to_string(file)
                .map_err(|e| CoreError::Config(format!("reading {}: {e}", file.display())))?;
            toml::from_str(&text)
                .map_err(|e| CoreError::Config(format!("parsing {}: {e}", file.display())))
        } else {
            let cfg = Self::default();
            cfg.write(file)?;
            Ok(cfg)
        }
    }

    fn write(&self, file: &Path) -> Result<()> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Config(format!("creating {}: {e}", parent.display())))?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| CoreError::Config(format!("serializing config: {e}")))?;
        std::fs::write(file, text)
            .map_err(|e| CoreError::Config(format!("writing {}: {e}", file.display())))
    }

    /// Apply overrides from a key -> value lookup. Injected so tests need not
    /// touch process-global environment. An explicitly set but unparseable
    /// value is a hard error, not a silent fallback.
    fn apply_overrides(&mut self, lookup: impl Fn(&str) -> Option<String>) -> Result<()> {
        if let Some(v) = lookup("AIWM_STORE_PATH").filter(|s| !s.is_empty()) {
            self.store_path = PathBuf::from(v);
        }
        if let Some(v) = lookup("AIWM_CORE_API_PORT").filter(|s| !s.is_empty()) {
            self.core_api_port = v.parse().map_err(|_| {
                CoreError::Config(format!(
                    "AIWM_CORE_API_PORT must be a port number, got {v:?}"
                ))
            })?;
        }
        if let Some(v) = lookup("AIWM_OFFLINE").filter(|s| !s.is_empty()) {
            self.offline_mode = parse_bool(&v).ok_or_else(|| {
                CoreError::Config(format!("AIWM_OFFLINE must be a boolean, got {v:?}"))
            })?;
        }
        if let Some(v) = lookup("AIWM_LOG").filter(|s| !s.trim().is_empty()) {
            self.log_filter = v;
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if self.core_api_port < MIN_API_PORT {
            return Err(CoreError::Config(format!(
                "core_api_port {} is below the minimum {MIN_API_PORT}",
                self.core_api_port
            )));
        }
        if self.store_path.as_os_str().is_empty() {
            return Err(CoreError::Config("store_path must not be empty".into()));
        }
        if self.log_filter.trim().is_empty() {
            return Err(CoreError::Config("log_filter must not be empty".into()));
        }
        if self.vram_budget_mb != 0 && self.vram_budget_mb < MIN_VRAM_BUDGET_MB {
            return Err(CoreError::Config(format!(
                "vram_budget_mb must be 0 (auto) or at least {MIN_VRAM_BUDGET_MB}"
            )));
        }
        self.llama.validate()?;
        self.comfyui.validate()?;
        Ok(())
    }
}

impl LlamaConfig {
    fn validate(&self) -> Result<()> {
        if self.ctx_size != 0 && self.ctx_size < MIN_CTX_SIZE {
            return Err(CoreError::Config(format!(
                "llama.ctx_size must be 0 (auto) or at least {MIN_CTX_SIZE}"
            )));
        }
        if !(MIN_LOAD_TIMEOUT_SECS..=MAX_LOAD_TIMEOUT_SECS).contains(&self.load_timeout_secs) {
            return Err(CoreError::Config(format!(
                "llama.load_timeout_secs must be between {MIN_LOAD_TIMEOUT_SECS} and {MAX_LOAD_TIMEOUT_SECS}"
            )));
        }
        Ok(())
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn one(key: &'static str, val: &'static str) -> impl Fn(&str) -> Option<String> {
        move |k| (k == key).then(|| val.to_string())
    }

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn missing_file_is_created_with_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());

        let cfg = Config::load(&paths).unwrap();

        assert_eq!(cfg, Config::default());
        assert!(paths.config_file().is_file());
        // Re-loading reads the same values back.
        assert_eq!(Config::load(&paths).unwrap(), cfg);
    }

    #[test]
    fn existing_file_is_parsed() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        std::fs::write(
            paths.config_file(),
            "store_path = 'D:\\\\models'\ncore_api_port = 5555\noffline_mode = true\nlog_filter = 'warn'\n",
        )
        .unwrap();

        let cfg = Config::load(&paths).unwrap();

        assert_eq!(cfg.store_path, PathBuf::from("D:\\models"));
        assert_eq!(cfg.core_api_port, 5555);
        assert!(cfg.offline_mode);
        assert_eq!(cfg.log_filter, "warn");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        std::fs::write(paths.config_file(), "core_api_port = 5000\nbogus = 1\n").unwrap();

        let err = Config::load(&paths).unwrap_err();
        assert!(matches!(err, CoreError::Config(_)));
    }

    #[test]
    fn invalid_toml_is_a_config_error() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        std::fs::write(paths.config_file(), "core_api_port = = =").unwrap();

        assert!(matches!(
            Config::load(&paths).unwrap_err(),
            CoreError::Config(_)
        ));
    }

    #[test]
    fn env_overrides_win_over_file() {
        let mut cfg = Config::default();
        let env = HashMap::from([
            ("AIWM_STORE_PATH".to_string(), "F:\\ai".to_string()),
            ("AIWM_CORE_API_PORT".to_string(), "40001".to_string()),
            ("AIWM_OFFLINE".to_string(), "TRUE".to_string()),
            ("AIWM_LOG".to_string(), "trace".to_string()),
        ]);

        cfg.apply_overrides(|k| env.get(k).cloned()).unwrap();

        assert_eq!(cfg.store_path, PathBuf::from("F:\\ai"));
        assert_eq!(cfg.core_api_port, 40001);
        assert!(cfg.offline_mode);
        assert_eq!(cfg.log_filter, "trace");
    }

    #[test]
    fn non_numeric_port_override_is_rejected() {
        let mut cfg = Config::default();
        let err = cfg
            .apply_overrides(one("AIWM_CORE_API_PORT", "abc"))
            .unwrap_err();
        assert!(matches!(err, CoreError::Config(_)));
    }

    #[test]
    fn non_boolean_offline_override_is_rejected() {
        let mut cfg = Config::default();
        assert!(cfg.apply_overrides(one("AIWM_OFFLINE", "maybe")).is_err());
    }

    #[test]
    fn offline_override_accepts_word_and_number_forms() {
        for truthy in ["1", "true", "TRUE", "yes", "on"] {
            let mut cfg = Config::default();
            cfg.apply_overrides(one("AIWM_OFFLINE", truthy)).unwrap();
            assert!(cfg.offline_mode, "{truthy} should be true");
        }
        let mut cfg = Config {
            offline_mode: true,
            ..Config::default()
        };
        cfg.apply_overrides(one("AIWM_OFFLINE", "off")).unwrap();
        assert!(!cfg.offline_mode);
    }

    #[test]
    fn empty_env_values_do_not_clobber() {
        let mut cfg = Config::default();
        cfg.apply_overrides(|k| {
            matches!(
                k,
                "AIWM_STORE_PATH" | "AIWM_LOG" | "AIWM_CORE_API_PORT" | "AIWM_OFFLINE"
            )
            .then(String::new)
        })
        .unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn low_port_is_rejected() {
        let mut cfg = Config::default();
        cfg.apply_overrides(no_env).unwrap();
        cfg.core_api_port = 80;
        assert!(matches!(cfg.validate().unwrap_err(), CoreError::Config(_)));
    }

    #[test]
    fn blank_log_filter_is_rejected() {
        let cfg = Config {
            log_filter: "   ".into(),
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn llama_defaults_map_to_runtime_options() {
        let opts = LlamaConfig::default().to_options();
        assert_eq!(opts.gpu_layers, 999);
        assert_eq!(opts.ctx_size, None); // 0 => auto
        assert!(opts.flash_attention);
        assert!(opts.jinja, "jinja on by default (tool calls)");
        assert_eq!(opts.chat_template, None);
        assert_eq!(opts.load_timeout.as_secs(), 180);

        let opts = LlamaConfig {
            ctx_size: 4096,
            jinja: false,
            chat_template: "  qwen2.5-coder  ".into(),
            ..LlamaConfig::default()
        }
        .to_options();
        assert_eq!(opts.ctx_size, Some(4096));
        assert!(!opts.jinja);
        assert_eq!(opts.chat_template.as_deref(), Some("qwen2.5-coder"));
    }

    #[test]
    fn save_then_read_from_round_trips_including_llama() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        let cfg = Config {
            vram_budget_mb: 12_000,
            llama: LlamaConfig {
                gpu_layers: 20,
                ctx_size: 16_384,
                flash_attention: false,
                load_timeout_secs: 90,
                ..LlamaConfig::default()
            },
            ..Config::default()
        };

        cfg.save(&paths).unwrap();
        assert_eq!(Config::read_from(&paths).unwrap(), cfg);
    }

    #[test]
    fn config_without_a_llama_table_still_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        std::fs::write(paths.config_file(), "vram_budget_mb = 8000\n").unwrap();

        let cfg = Config::load(&paths).unwrap();
        assert_eq!(cfg.vram_budget_mb, 8000);
        assert_eq!(cfg.llama, LlamaConfig::default());
    }

    #[test]
    fn comfyui_vram_mode_round_trips_and_is_validated() {
        let opts = ComfyConfig::default().to_options();
        assert_eq!(opts.vram_mode, crate::runtime::VramMode::Auto);
        assert!(opts.extra_args.is_empty());

        let low = ComfyConfig {
            vram_mode: "lowvram".into(),
            ..ComfyConfig::default()
        };
        assert_eq!(
            low.to_options().vram_mode,
            crate::runtime::VramMode::LowVram
        );
        assert!(low.validate().is_ok());

        assert!(ComfyConfig {
            vram_mode: "turbo".into(),
            ..ComfyConfig::default()
        }
        .validate()
        .is_err());

        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        let cfg = Config {
            comfyui: low,
            ..Config::default()
        };
        cfg.save(&paths).unwrap();
        assert_eq!(
            Config::read_from(&paths).unwrap().comfyui.vram_mode,
            "lowvram"
        );
    }

    #[test]
    fn comfyui_reserve_vram_and_extra_args_become_command_line_flags() {
        let cfg = ComfyConfig {
            reserve_vram_mb: 1536,
            extra_args: "  --fast   --use-sage-attention ".into(),
            ..ComfyConfig::default()
        };
        assert_eq!(
            cfg.to_options().extra_args,
            vec!["--reserve-vram", "1.50", "--fast", "--use-sage-attention"]
        );
        assert!(cfg.validate().is_ok());

        assert!(ComfyConfig {
            reserve_vram_mb: 99_999,
            ..ComfyConfig::default()
        }
        .validate()
        .is_err());

        // 0 → no reserve flag.
        assert!(!ComfyConfig::default()
            .to_options()
            .extra_args
            .iter()
            .any(|a| a == "--reserve-vram"));
    }

    #[test]
    fn config_without_a_comfyui_table_still_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted(tmp.path());
        std::fs::write(paths.config_file(), "vram_budget_mb = 8000\n").unwrap();

        let cfg = Config::load(&paths).unwrap();
        assert_eq!(cfg.comfyui, ComfyConfig::default());
    }

    #[test]
    fn invalid_llama_and_budget_values_are_rejected() {
        let bad_ctx = Config {
            llama: LlamaConfig {
                ctx_size: 100,
                ..LlamaConfig::default()
            },
            ..Config::default()
        };
        assert!(bad_ctx.validate().is_err());

        let bad_timeout = Config {
            llama: LlamaConfig {
                load_timeout_secs: 5,
                ..LlamaConfig::default()
            },
            ..Config::default()
        };
        assert!(bad_timeout.validate().is_err());

        let bad_budget = Config {
            vram_budget_mb: 200,
            ..Config::default()
        };
        assert!(bad_budget.validate().is_err());
    }
}
