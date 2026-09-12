//! Locating the Colibri launcher and turning a model directory + port into a
//! [`SpawnSpec`]. Kept apart from the adapter's state machine in `mod.rs`.

use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use super::RUNTIME_ID;
use crate::runtime::SpawnSpec;

/// Environment override for the Colibri launcher.
pub(super) const BIN_ENV: &str = "AIWM_COLIBRI_PATH";

const LAUNCHER: &str = if cfg!(windows) { "coli.cmd" } else { "coli" };

/// `coli.cmd` is a batch script, not a `.exe` — Windows' `CreateProcess` (what
/// `Command::new` calls) does not know how to run one directly (`%1 is not a
/// valid Win32 application`), unlike a shell, which resolves the association
/// itself. Spawning it for real needs an explicit `cmd.exe /C` wrapper; a
/// plain `coli` on other platforms is a normal executable and needs none.
pub(super) fn build_spawn_spec(
    launcher: &Path,
    model_dir: &Path,
    port: u16,
    model_id: &str,
    api_key: &str,
) -> SpawnSpec {
    let mut spec = if cfg!(windows) {
        SpawnSpec::new("cmd.exe")
            .arg("/C")
            .arg(launcher.to_string_lossy().into_owned())
    } else {
        SpawnSpec::new(launcher)
    };
    spec = spec
        .arg("serve")
        .arg("--host")
        .arg(Ipv4Addr::LOCALHOST.to_string())
        .arg("--port")
        .arg(port.to_string())
        .arg("--model-id")
        .arg(model_id);
    spec.env.push((
        "COLI_MODEL".into(),
        model_dir.to_string_lossy().into_owned(),
    ));
    spec.env.push(("COLI_API_KEY".into(), api_key.into()));
    spec
}

/// Resolve the Colibri launcher: `AIWM_COLIBRI_PATH`, then the managed install
/// under `runtimes_dir`, then `PATH`. `lookup` reads env vars (injected for
/// tests).
pub(super) fn resolve_launcher(
    runtimes_dir: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    if let Some(raw) = lookup(BIN_ENV).filter(|s| !s.is_empty()) {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return Some(path);
        }
    }

    let managed_root = runtimes_dir.join(RUNTIME_ID);
    // One version subdirectory deep: <root>/colibri/<build>/coli.cmd.
    if let Ok(entries) = std::fs::read_dir(&managed_root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let cand = entry.path().join(LAUNCHER);
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }

    if let Some(path_var) = lookup("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(LAUNCHER);
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

    #[test]
    fn build_spawn_spec_wraps_the_cmd_script_via_cmd_exe_on_windows() {
        let spec = build_spawn_spec(
            Path::new("C:\\runtimes\\colibri\\v1.10.2\\coli.cmd"),
            Path::new("D:\\models\\qwen36"),
            48300,
            "qwen3.6-colibri",
            "secret-key",
        );
        if cfg!(windows) {
            assert_eq!(spec.program, PathBuf::from("cmd.exe"));
            let joined = spec.args.join(" ");
            assert!(joined.starts_with("/C C:\\runtimes\\colibri\\v1.10.2\\coli.cmd"));
        } else {
            assert_eq!(
                spec.program,
                PathBuf::from("C:\\runtimes\\colibri\\v1.10.2\\coli.cmd")
            );
        }
    }

    #[test]
    fn build_spawn_spec_carries_the_essential_flags_and_env() {
        let spec = build_spawn_spec(
            Path::new("coli.cmd"),
            Path::new("D:\\models\\qwen36"),
            48300,
            "qwen3.6-colibri",
            "secret-key",
        );
        let joined = spec.args.join(" ");
        assert!(joined.contains("serve"));
        assert!(joined.contains("--host 127.0.0.1"));
        assert!(joined.contains("--port 48300"));
        assert!(joined.contains("--model-id qwen3.6-colibri"));
        assert!(spec
            .env
            .contains(&("COLI_MODEL".to_string(), "D:\\models\\qwen36".to_string())));
        assert!(spec
            .env
            .contains(&("COLI_API_KEY".to_string(), "secret-key".to_string())));
    }

    #[test]
    fn resolve_prefers_env_override() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("my-coli.cmd");
        std::fs::write(&exe, b"@echo off").unwrap();
        let exe_str = exe.to_string_lossy().into_owned();

        let got = resolve_launcher(Path::new("Z:\\nope"), |k| {
            (k == BIN_ENV).then(|| OsString::from(exe_str.clone()))
        });
        assert_eq!(got, Some(exe));
    }

    #[test]
    fn resolve_finds_a_managed_install() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(RUNTIME_ID).join("v1.10.2");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(LAUNCHER);
        std::fs::write(&exe, b"@echo off").unwrap();

        let got = resolve_launcher(tmp.path(), |_| None);
        assert_eq!(got, Some(exe));
    }

    #[test]
    fn resolve_is_none_when_nothing_is_installed() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(resolve_launcher(tmp.path(), |_| None), None);
    }
}
