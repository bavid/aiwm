//! Client for the Python sidecar.
//!
//! Transport: JSON-RPC 2.0 over the child's stdio, one JSON object per line. The
//! sidecar is spawned into a [`JobObject`] so it can never outlive the core.
//! Phase-1 methods: `handshake`, `ping`. `inspect_model_file` is in the contract
//! but the sidecar reports it as not-implemented until Phase 2.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::runtime::JobObject;
use crate::{CoreError, Result};

/// JSON-RPC contract version. Must match `aiwm_sidecar.PROTOCOL_VERSION`.
pub const PROTOCOL_VERSION: u32 = 1;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

fn sidecar_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Sidecar(msg.to_string())
}

/// How to launch the sidecar process.
#[derive(Debug, Clone)]
pub struct SidecarSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// What the sidecar tells us on connect.
#[derive(Debug, Clone, Deserialize)]
pub struct Handshake {
    pub sidecar_version: String,
    pub protocol_version: u32,
    pub capabilities: Vec<String>,
}

struct Io {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    child: Child,
}

/// A live sidecar. Dropping it terminates the process.
#[derive(Debug)]
pub struct SidecarClient {
    io: Mutex<Io>,
    next_id: AtomicU64,
    handshake: Handshake,
    pid: Option<u32>,
    _job: JobObject,
}

impl std::fmt::Debug for Io {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Io").finish_non_exhaustive()
    }
}

impl SidecarClient {
    /// Launch the dev sidecar via `uv run` against `<repo>/sidecar`.
    pub async fn for_dev() -> Result<Self> {
        let sidecar_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("sidecar");
        let uv = resolve_uv().ok_or_else(|| sidecar_err("`uv` was not found on PATH"))?;
        Self::spawn(SidecarSpec {
            program: uv,
            args: vec![
                "run".into(),
                "--directory".into(),
                sidecar_dir.to_string_lossy().into_owned(),
                "aiwm-sidecar".into(),
            ],
            env: vec![("PYTHONUNBUFFERED".into(), "1".into())],
        })
        .await
    }

    /// Spawn a sidecar from an explicit spec and complete the handshake.
    pub async fn spawn(spec: SidecarSpec) -> Result<Self> {
        let job = JobObject::new()?;

        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| sidecar_err(format!("spawn {}: {e}", spec.program.display())))?;
        job.assign_child(&child)?;
        let pid = child.id();

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| sidecar_err("sidecar stdin missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| sidecar_err("sidecar stdout missing"))?;
        let mut io = Io {
            stdin,
            stdout: BufReader::new(stdout),
            child,
        };

        let handshake = tokio::time::timeout(HANDSHAKE_TIMEOUT, do_handshake(&mut io))
            .await
            .map_err(|_| sidecar_err("handshake timed out"))??;

        Ok(Self {
            io: Mutex::new(io),
            next_id: AtomicU64::new(1),
            handshake,
            pid,
            _job: job,
        })
    }

    pub fn handshake(&self) -> &Handshake {
        &self.handshake
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Round-trip a JSON-RPC call.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut io = self.io.lock().await;
        raw_call(&mut io, id, method, params).await
    }

    pub async fn ping(&self) -> Result<()> {
        match self.call("ping", Value::Null).await? {
            Value::String(s) if s == "pong" => Ok(()),
            other => Err(sidecar_err(format!("unexpected ping reply: {other}"))),
        }
    }

    /// Ask the sidecar to exit and wait for it.
    pub async fn shutdown(self) -> Result<()> {
        let mut io = self.io.lock().await;
        let _ = io.child.start_kill();
        let _ = io.child.wait().await;
        Ok(())
    }
}

async fn do_handshake(io: &mut Io) -> Result<Handshake> {
    let result = raw_call(
        io,
        0,
        "handshake",
        json!({ "protocol_version": PROTOCOL_VERSION }),
    )
    .await?;
    let hs: Handshake =
        serde_json::from_value(result).map_err(|e| sidecar_err(format!("bad handshake: {e}")))?;
    if hs.protocol_version != PROTOCOL_VERSION {
        return Err(sidecar_err(format!(
            "protocol mismatch: core {PROTOCOL_VERSION}, sidecar {}",
            hs.protocol_version
        )));
    }
    Ok(hs)
}

async fn raw_call(io: &mut Io, id: u64, method: &str, params: Value) -> Result<Value> {
    let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    io.stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(|e| sidecar_err(format!("write: {e}")))?;
    io.stdin
        .flush()
        .await
        .map_err(|e| sidecar_err(format!("flush: {e}")))?;

    loop {
        let mut line = String::new();
        let n = io
            .stdout
            .read_line(&mut line)
            .await
            .map_err(|e| sidecar_err(format!("read: {e}")))?;
        if n == 0 {
            return Err(sidecar_err("sidecar closed the connection"));
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let resp: Value =
            serde_json::from_str(line).map_err(|e| sidecar_err(format!("bad response: {e}")))?;
        if resp.get("id").and_then(Value::as_u64) != Some(id) {
            continue; // not our reply
        }
        if let Some(err) = resp.get("error") {
            return Err(sidecar_err(format!("rpc error: {err}")));
        }
        return Ok(resp.get("result").cloned().unwrap_or(Value::Null));
    }
}

/// Locate the `uv` executable: PATH, then the common install locations.
pub fn resolve_uv() -> Option<PathBuf> {
    let names = ["uv.exe", "uv"];

    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in names {
                let cand = dir.join(name);
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        for name in names {
            let cand = home.join(".local").join("bin").join(name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }

    // WinGet: %LOCALAPPDATA%\Microsoft\WinGet\Packages\astral-sh.uv_*\uv.exe
    if let Some(local) = dirs::data_local_dir() {
        let packages = local.join("Microsoft").join("WinGet").join("Packages");
        if let Ok(entries) = std::fs::read_dir(&packages) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("astral-sh.uv")
                {
                    let cand = entry.path().join("uv.exe");
                    if cand.is_file() {
                        return Some(cand);
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Some CI has no `uv`; skip (loudly) rather than fail there.
    macro_rules! sidecar_or_skip {
        () => {
            match SidecarClient::for_dev().await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("skipping sidecar test: {e}");
                    return;
                }
            }
        };
    }

    #[tokio::test]
    async fn handshake_and_ping_roundtrip() {
        let client = sidecar_or_skip!();

        assert_eq!(client.handshake().protocol_version, PROTOCOL_VERSION);
        assert_eq!(client.handshake().sidecar_version, "0.0.1");

        client.ping().await.unwrap();
        client.ping().await.unwrap();

        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn unknown_method_is_an_error() {
        let client = sidecar_or_skip!();
        let err = client
            .call("no_such_method", Value::Null)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::Sidecar(_)));
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn planned_method_is_not_implemented_yet() {
        let client = sidecar_or_skip!();
        let err = client
            .call("inspect_model_file", json!({ "path": "x.gguf" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not implemented"));
        client.shutdown().await.unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn sidecar_dies_when_client_is_dropped() {
        let client = sidecar_or_skip!();
        let pid = client.pid().expect("pid");

        let alive = |pid: u32| {
            let out = std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .output()
                .expect("tasklist");
            String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
        };
        assert!(alive(pid));

        drop(client);

        for _ in 0..60 {
            if !alive(pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("sidecar {pid} survived the client being dropped");
    }
}
