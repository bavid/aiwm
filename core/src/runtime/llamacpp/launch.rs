//! Locating the `llama-server` binary and turning a model + options into a
//! [`SpawnSpec`]. Kept apart from the adapter's state machine in `mod.rs`.

use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use super::{LlamaServerOptions, RUNTIME_ID};
use crate::runtime::SpawnSpec;

/// Environment override for the `llama-server` executable.
pub(super) const BIN_ENV: &str = "AIWM_LLAMACPP_PATH";

const SERVER_EXE: &str = if cfg!(windows) {
    "llama-server.exe"
} else {
    "llama-server"
};

pub(super) fn build_spawn_spec(
    bin: &Path,
    model_path: &Path,
    port: u16,
    opts: &LlamaServerOptions,
    ctx: u32,
) -> SpawnSpec {
    let mut spec = SpawnSpec::new(bin)
        .arg("-m")
        .arg(model_path.to_string_lossy().into_owned())
        .arg("--host")
        .arg(Ipv4Addr::LOCALHOST.to_string())
        .arg("--port")
        .arg(port.to_string())
        .arg("--no-webui")
        .arg("-ngl")
        .arg(opts.gpu_layers.to_string())
        .arg("-c")
        .arg(ctx.to_string());
    if opts.flash_attention {
        spec = spec.arg("--flash-attn").arg("on");
    }
    for extra in &opts.extra_args {
        spec = spec.arg(extra.clone());
    }
    spec
}

/// Resolve the `llama-server` binary: `AIWM_LLAMACPP_PATH`, then the managed
/// install under `runtimes_dir`, then `PATH`. `lookup` reads env vars (injected
/// for tests).
pub(super) fn resolve_server_bin(
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
    let direct = [
        managed_root.join(SERVER_EXE),
        managed_root.join("bin").join(SERVER_EXE),
    ];
    if let Some(hit) = direct.into_iter().find(|p| p.is_file()) {
        return Some(hit);
    }
    // One version subdirectory deep: <root>/llamacpp/<build>/[bin/]llama-server.
    if let Ok(entries) = std::fs::read_dir(&managed_root) {
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

    fn opts() -> LlamaServerOptions {
        LlamaServerOptions::default()
    }

    #[test]
    fn build_spawn_spec_carries_the_essential_flags() {
        let spec = build_spawn_spec(
            Path::new("C:\\bin\\llama-server.exe"),
            Path::new("E:\\AI\\models\\llm\\q\\q.gguf"),
            48213,
            &opts(),
            8192,
        );
        assert_eq!(spec.program, PathBuf::from("C:\\bin\\llama-server.exe"));
        let joined = spec.args.join(" ");
        assert!(joined.contains("-m E:\\AI\\models\\llm\\q\\q.gguf"));
        assert!(joined.contains("--host 127.0.0.1"));
        assert!(joined.contains("--port 48213"));
        assert!(joined.contains("--no-webui"));
        assert!(joined.contains("-ngl 999"));
        assert!(joined.contains("-c 8192"));
        assert!(joined.contains("--flash-attn on"));
    }

    #[test]
    fn build_spawn_spec_uses_the_given_ctx_and_can_drop_flash_attn() {
        let spec = build_spawn_spec(
            Path::new("s"),
            Path::new("m.gguf"),
            1,
            &LlamaServerOptions {
                flash_attention: false,
                ..opts()
            },
            4096,
        );
        let joined = spec.args.join(" ");
        assert!(joined.contains("-c 4096"));
        assert!(!joined.contains("--flash-attn"));
    }

    #[test]
    fn resolve_prefers_env_override() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("my-llama-server.exe");
        std::fs::write(&exe, b"x").unwrap();
        let exe_str = exe.to_string_lossy().into_owned();

        let got = resolve_server_bin(Path::new("Z:\\nope"), |k| {
            (k == BIN_ENV).then(|| OsString::from(exe_str.clone()))
        });
        assert_eq!(got, Some(exe));
    }

    #[test]
    fn resolve_finds_a_managed_install() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(RUNTIME_ID).join("b10855");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(SERVER_EXE);
        std::fs::write(&exe, b"x").unwrap();

        let got = resolve_server_bin(tmp.path(), |_| None);
        assert_eq!(got, Some(exe));
    }

    #[test]
    fn resolve_is_none_when_nothing_is_installed() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(resolve_server_bin(tmp.path(), |_| None), None);
    }
}
