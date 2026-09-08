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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            store_path: PathBuf::from(DEFAULT_STORE_PATH),
            core_api_port: DEFAULT_API_PORT,
            offline_mode: false,
            log_filter: DEFAULT_LOG_FILTER.to_string(),
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
}
