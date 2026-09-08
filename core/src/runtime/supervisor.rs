//! Owns a runtime's OS process: spawns it into a [`JobObject`], watches it, and
//! restarts it with capped exponential backoff if it exits unexpectedly.

use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::{JobObject, SpawnSpec};
use crate::{CoreError, Result};

/// After this many unexpected exits the supervisor stops trying.
const MAX_RESTARTS: u32 = 10;
const BACKOFF_BASE: Duration = Duration::from_millis(200);
const BACKOFF_CAP: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorState {
    Running,
    Restarting,
    Stopped,
    GaveUp,
}

#[derive(Debug)]
struct Shared {
    pid: Mutex<Option<u32>>,
    restarts: AtomicU32,
    state: Mutex<SupervisorState>,
}

impl Shared {
    fn set_state(&self, s: SupervisorState) {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = s;
    }
    fn set_pid(&self, pid: Option<u32>) {
        *self.pid.lock().unwrap_or_else(PoisonError::into_inner) = pid;
    }
}

/// Handle to a supervised runtime process. Dropping it stops the process.
#[derive(Debug)]
pub struct RuntimeSupervisor {
    id: String,
    stop_tx: watch::Sender<bool>,
    shared: Arc<Shared>,
    monitor: JoinHandle<()>,
}

impl RuntimeSupervisor {
    /// Spawn `spec` and start watching it. Requires a Tokio runtime.
    pub fn start(id: impl Into<String>, spec: SpawnSpec) -> Result<Self> {
        let id = id.into();
        let job = JobObject::new()?;
        let child = spawn(&spec, &job)?;

        let shared = Arc::new(Shared {
            pid: Mutex::new(child.id()),
            restarts: AtomicU32::new(0),
            state: Mutex::new(SupervisorState::Running),
        });
        let (stop_tx, stop_rx) = watch::channel(false);
        let monitor = tokio::spawn(monitor_loop(
            id.clone(),
            spec,
            job,
            child,
            shared.clone(),
            stop_rx,
        ));

        Ok(Self {
            id,
            stop_tx,
            shared,
            monitor,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn pid(&self) -> Option<u32> {
        *self
            .shared
            .pid
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn state(&self) -> SupervisorState {
        *self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn restart_count(&self) -> u32 {
        self.shared.restarts.load(Ordering::SeqCst)
    }

    /// Ask the process to stop and wait for the monitor to finish.
    pub async fn stop(&mut self) -> Result<()> {
        let _ = self.stop_tx.send(true);
        (&mut self.monitor).await.ok();
        Ok(())
    }
}

impl Drop for RuntimeSupervisor {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(true);
        // The monitor owns the child + job object; aborting it drops both, and
        // KILL_ON_JOB_CLOSE (plus kill_on_drop) terminates the process tree.
        self.monitor.abort();
    }
}

fn spawn(spec: &SpawnSpec, job: &JobObject) -> Result<tokio::process::Child> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args).stdin(Stdio::null()).kill_on_drop(true);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }

    let child = cmd.spawn().map_err(|e| CoreError::Runtime {
        runtime: "supervisor".into(),
        message: format!("spawn {}: {e}", spec.program.display()),
    })?;
    job.assign_child(&child)?;
    Ok(child)
}

fn backoff_for(restarts: u32) -> Duration {
    let shift = restarts.saturating_sub(1).min(8);
    BACKOFF_BASE.saturating_mul(1u32 << shift).min(BACKOFF_CAP)
}

async fn stop_signalled(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow() {
            return;
        }
        if rx.changed().await.is_err() {
            return; // sender gone => treat as stop
        }
    }
}

async fn monitor_loop(
    id: String,
    spec: SpawnSpec,
    job: JobObject,
    first_child: tokio::process::Child,
    shared: Arc<Shared>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let mut child = first_child;
    let mut restarts: u32 = 0;

    loop {
        tokio::select! {
            biased;
            () = stop_signalled(&mut stop_rx) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                shared.set_pid(None);
                shared.set_state(SupervisorState::Stopped);
                return;
            }
            exit = child.wait() => {
                shared.set_pid(None);
                if *stop_rx.borrow() {
                    shared.set_state(SupervisorState::Stopped);
                    return;
                }
                restarts += 1;
                shared.restarts.store(restarts, Ordering::SeqCst);
                tracing::warn!(runtime = %id, ?exit, restarts, "runtime exited unexpectedly");

                if restarts > MAX_RESTARTS {
                    tracing::error!(runtime = %id, "too many restarts; giving up");
                    shared.set_state(SupervisorState::GaveUp);
                    return;
                }
                shared.set_state(SupervisorState::Restarting);

                let backoff = backoff_for(restarts);
                tokio::select! {
                    () = stop_signalled(&mut stop_rx) => {
                        shared.set_state(SupervisorState::Stopped);
                        return;
                    }
                    () = tokio::time::sleep(backoff) => {}
                }

                match spawn(&spec, &job) {
                    Ok(next) => {
                        shared.set_pid(next.id());
                        shared.set_state(SupervisorState::Running);
                        child = next;
                    }
                    Err(e) => {
                        tracing::error!(runtime = %id, error = %e, "respawn failed; giving up");
                        shared.set_state(SupervisorState::GaveUp);
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::time::Duration;

    use super::*;

    fn pid_alive(pid: u32) -> bool {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .expect("run tasklist");
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    }

    fn long_running() -> SpawnSpec {
        SpawnSpec::new("ping")
            .arg("-n")
            .arg("3600")
            .arg("127.0.0.1")
    }

    #[tokio::test]
    async fn start_then_stop_terminates_the_process() {
        let mut sup = RuntimeSupervisor::start("t", long_running()).unwrap();
        let pid = sup.pid().expect("pid");
        assert!(pid_alive(pid));
        assert_eq!(sup.state(), SupervisorState::Running);

        sup.stop().await.unwrap();

        assert_eq!(sup.state(), SupervisorState::Stopped);
        assert!(!pid_alive(pid), "process should be gone after stop()");
        assert_eq!(sup.restart_count(), 0);
    }

    #[tokio::test]
    async fn dropping_the_supervisor_kills_the_process() {
        let sup = RuntimeSupervisor::start("t", long_running()).unwrap();
        let pid = sup.pid().expect("pid");
        assert!(pid_alive(pid));

        drop(sup);

        for _ in 0..60 {
            if !pid_alive(pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("process {pid} survived supervisor drop");
    }

    #[tokio::test]
    async fn crashed_process_is_restarted_with_backoff() {
        // `cmd /c exit 1` exits immediately => the monitor keeps respawning it.
        let spec = SpawnSpec::new("cmd").arg("/c").arg("exit").arg("1");
        let mut sup = RuntimeSupervisor::start("flaky", spec).unwrap();

        // base 200ms, then 400ms, 800ms ... so ~1.5s buys at least two restarts.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let seen = sup.restart_count();
        assert!(seen >= 2, "expected >=2 restarts, saw {seen}");
        assert!(matches!(
            sup.state(),
            SupervisorState::Running | SupervisorState::Restarting
        ));

        sup.stop().await.unwrap();
        assert_eq!(sup.state(), SupervisorState::Stopped);
    }
}
