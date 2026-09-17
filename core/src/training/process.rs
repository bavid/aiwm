//! Process control for a detached training run (spec "Task 5"): liveness
//! checks and a tree-kill for the `cmd /C` wrapper PID that
//! [`crate::launcher::spawn::launch_detached_quiet`] returns, plus the
//! on-disk PID file a run's `work_dir` uses to find that process again
//! after an app restart.
//!
//! Everything here shells out to Windows console tools (`tasklist`,
//! `taskkill`) rather than a process-inspection crate: the app already
//! commits to Windows-only (see `launcher::spawn` module docs), and these
//! two tools are the standard, always-present way to query and kill an
//! arbitrary PID (including one this process never parented) by tree.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::training::training_err;
use crate::Result;

/// The file a run's `work_dir` uses to remember the detached wrapper's PID
/// across an app restart: `<work_dir>/trainer.pid`.
pub fn pid_file(work_dir: &Path) -> PathBuf {
    work_dir.join("trainer.pid")
}

/// Write `pid` to [`pid_file`], overwriting any previous contents.
pub async fn write_pid_file(work_dir: &Path, pid: u32) -> Result<()> {
    tokio::fs::write(pid_file(work_dir), pid.to_string())
        .await
        .map_err(|e| training_err(format!("failed to write trainer.pid: {e}")))
}

/// Read back the PID [`write_pid_file`] stored under `work_dir`. `Ok(None)`
/// if the file does not exist yet (no run has been launched); `Err` if it
/// exists but its contents aren't a valid PID.
pub async fn read_pid_file(work_dir: &Path) -> Result<Option<u32>> {
    let path = pid_file(work_dir);
    let contents = match tokio::fs::read_to_string(&path).await {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(training_err(format!("failed to read trainer.pid: {e}"))),
    };
    contents
        .trim()
        .parse::<u32>()
        .map(Some)
        .map_err(|_| training_err(format!("trainer.pid contains garbage: {contents:?}")))
}

/// Whether `pid` currently identifies a running process.
#[cfg(windows)]
pub async fn is_alive(pid: u32) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .await;
    let Ok(output) = output else {
        return false;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.contains(&format!("\"{pid}\""))
}

#[cfg(not(windows))]
pub async fn is_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The `taskkill` argv (minus the program name itself) that kills `pid` and
/// its whole process tree: `/PID <pid> /T /F` (`/T` walks the tree,
/// leaves-first; `/F` forces termination). Pulled out as a pure function so
/// the shape can be asserted without actually spawning `taskkill`.
#[cfg(windows)]
fn kill_tree_args(pid: u32) -> Vec<String> {
    vec!["/PID".into(), pid.to_string(), "/T".into(), "/F".into()]
}

/// Kill `pid` and its entire process tree. Succeeds (as a no-op) if the
/// process is already gone -- callers use this for cleanup on paths where
/// "already dead" is not an error.
#[cfg(windows)]
pub async fn kill_tree(pid: u32) -> Result<()> {
    let output = Command::new("taskkill")
        .args(kill_tree_args(pid))
        .output()
        .await
        .map_err(|e| training_err(format!("failed to run taskkill: {e}")))?;
    if output.status.success() {
        return Ok(());
    }
    // taskkill exits non-zero both for "already gone" (which we treat as
    // success) and for genuine failures -- distinguish by message text.
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("not found") {
        return Ok(());
    }
    Err(training_err(format!(
        "taskkill failed for pid {pid}: {stderr}"
    )))
}

#[cfg(not(windows))]
pub async fn kill_tree(pid: u32) -> Result<()> {
    let status = Command::new("kill")
        .args(["-9", &format!("-{pid}")])
        .status()
        .await
        .map_err(|e| training_err(format!("failed to run kill: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(training_err(format!("kill failed for pid {pid}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::spawn::{launch_detached_quiet, SpawnSpec};
    use std::time::Duration;

    #[cfg(windows)]
    #[test]
    fn kill_tree_args_use_taskkill_with_tree_and_force() {
        assert_eq!(kill_tree_args(4242), vec!["/PID", "4242", "/T", "/F"]);
    }

    #[tokio::test]
    async fn pid_file_round_trips_and_rejects_garbage() {
        let dir = tempfile::tempdir().expect("tempdir");
        let work_dir = dir.path();

        assert_eq!(read_pid_file(work_dir).await.expect("read"), None);

        write_pid_file(work_dir, 1234).await.expect("write");
        assert_eq!(read_pid_file(work_dir).await.expect("read"), Some(1234));

        tokio::fs::write(pid_file(work_dir), "not-a-pid")
            .await
            .expect("write garbage");
        assert!(read_pid_file(work_dir).await.is_err());
    }

    /// Guards a spawned test process: kills its tree on drop so a failed
    /// assertion (which unwinds past every explicit cleanup call) never
    /// leaves a `ping` straggler running.
    #[cfg(windows)]
    struct KillOnDrop(u32);

    #[cfg(windows)]
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let pid = self.0;
            // Best-effort, synchronous: Drop can't await. A blocking spawn
            // is fine here -- this only runs in the test process, once, on
            // an already-slow-path (panic) exit.
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_quiet_child_is_alive_until_its_tree_is_killed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("train.log");
        let spec = SpawnSpec {
            title: "AIWM: test".into(),
            cwd: dir.path().to_path_buf(),
            program: PathBuf::from("cmd"),
            args: vec!["/C".into(), "ping -n 30 127.0.0.1 >NUL".into()],
            env: vec![],
        };

        let pid = launch_detached_quiet(&spec, &log_path)
            .await
            .expect("launch");
        let _guard = KillOnDrop(pid);

        let mut alive = false;
        for _ in 0..20 {
            if is_alive(pid).await {
                alive = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(alive, "pid {pid} never showed up as alive");

        let mut log_exists = false;
        for _ in 0..20 {
            if tokio::fs::try_exists(&log_path).await.unwrap_or(false) {
                log_exists = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(log_exists, "log file was never created at {log_path:?}");

        kill_tree(pid).await.expect("kill_tree");

        let mut dead = false;
        for _ in 0..50 {
            if !is_alive(pid).await {
                dead = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(dead, "pid {pid} still alive 5s after kill_tree");
    }
}
