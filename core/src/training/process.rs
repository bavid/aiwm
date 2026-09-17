//! Process control for a detached training run (spec "Task 5"): liveness
//! checks and a tree-kill for the PID
//! [`crate::launcher::spawn::launch_detached_quiet`] returns, plus the
//! on-disk PID file a run's `work_dir` uses to find that process again
//! after an app restart.
//!
//! Everything here shells out to Windows console tools (`tasklist`,
//! `taskkill`) rather than a process-inspection crate: the app already
//! commits to Windows-only (see `launcher::spawn` module docs), and these
//! two tools are the standard, always-present way to query and kill an
//! arbitrary PID (including one this process never parented) by tree.
//!
//! Every liveness/kill call takes the *expected image name* (`"python.exe"`
//! for a real training run, `"ping.exe"` in this module's own test)
//! alongside the PID: Windows recycles PIDs once a process exits, so a bare
//! PID recovered from [`pid_file`] after an app restart is not a stable
//! identity. Comparing the live process's image name against the one
//! recorded at launch turns "the recovered PID now belongs to a total
//! stranger process" into a cheap, harmless no-op instead of silently
//! reporting someone else's process alive, or worse, killing it.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::training::training_err;
use crate::Result;

/// The file a run's `work_dir` uses to remember the detached process's PID
/// (and the image name it was launched as) across an app restart:
/// `<work_dir>/trainer.pid`.
pub fn pid_file(work_dir: &Path) -> PathBuf {
    work_dir.join("trainer.pid")
}

/// Write `pid`/`image` to [`pid_file`] as `"<pid> <image>"`, overwriting any
/// previous contents.
pub async fn write_pid_file(work_dir: &Path, pid: u32, image: &str) -> Result<()> {
    tokio::fs::write(pid_file(work_dir), format!("{pid} {image}"))
        .await
        .map_err(|e| training_err(format!("failed to write trainer.pid: {e}")))
}

/// Read back the `(pid, image)` [`write_pid_file`] stored under `work_dir`.
/// `Ok(None)` if the file does not exist yet (no run has been launched);
/// `Err` if it exists but isn't in the `"<pid> <image>"` shape.
pub async fn read_pid_file(work_dir: &Path) -> Result<Option<(u32, String)>> {
    let path = pid_file(work_dir);
    let contents = match tokio::fs::read_to_string(&path).await {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(training_err(format!("failed to read trainer.pid: {e}"))),
    };
    parse_pid_file(&contents)
        .map(Some)
        .ok_or_else(|| training_err(format!("trainer.pid contains garbage: {contents:?}")))
}

/// Parses the `"<pid> <image>"` shape [`write_pid_file`] writes. Pulled out
/// as a pure function so the garbage-rejection shape can be asserted
/// without touching the filesystem.
fn parse_pid_file(contents: &str) -> Option<(u32, String)> {
    let (pid_str, image) = contents.trim().split_once(' ')?;
    if image.is_empty() {
        return None;
    }
    let pid = pid_str.parse::<u32>().ok()?;
    Some((pid, image.to_string()))
}

/// Pulls the image-name column (first CSV field) out of one line of
/// `tasklist`'s `/FO CSV /NH` output, e.g.
/// `"ping.exe","1234","Console","1","4 K"` -> `Some("ping.exe")`. `None` if
/// `tasklist` printed nothing (no process matched its `/FI` filter) or a
/// line that isn't in the expected quoted-CSV shape.
fn csv_image_name(tasklist_stdout: &str) -> Option<&str> {
    let line = tasklist_stdout.lines().next()?;
    let field = line.strip_prefix('"')?;
    let end = field.find('"')?;
    Some(&field[..end])
}

/// Whether `pid` currently identifies a running process whose image name
/// matches `image` (case-insensitive) -- see the module docs on why the
/// image check matters. `Err` if `tasklist` itself could not be run (not
/// found, permissions denied); that is a real failure, not "the process is
/// dead", and callers must not conflate the two.
#[cfg(windows)]
pub async fn is_alive(pid: u32, image: &str) -> Result<bool> {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .await
        .map_err(|e| training_err(format!("failed to run tasklist: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(csv_image_name(&stdout).is_some_and(|found| found.eq_ignore_ascii_case(image)))
}

#[cfg(not(windows))]
pub async fn is_alive(_pid: u32, _image: &str) -> Result<bool> {
    Err(training_err(
        "process liveness checks are Windows-only in this build",
    ))
}

/// The `taskkill` argv (minus the program name itself) that kills `pid` and
/// its whole process tree: `/PID <pid> /T /F` (`/T` walks the tree,
/// leaves-first; `/F` forces termination). Pulled out as a pure function so
/// the shape can be asserted without actually spawning `taskkill`.
fn kill_tree_args(pid: u32) -> Vec<String> {
    vec!["/PID".into(), pid.to_string(), "/T".into(), "/F".into()]
}

/// `taskkill`'s own documented exit code for "no process matched" --
/// distinct from a genuine failure (access denied, bad argument, etc).
/// Checked by this numeric code rather than parsing `taskkill`'s
/// locale-dependent stderr text (which differs per Windows display
/// language, e.g. German "Dieser Vorgang wird nicht unterstützt").
const TASKKILL_NOT_FOUND_EXIT_CODE: i32 = 128;

/// Interprets a finished `taskkill /PID <pid> /T /F` run: exit code `0`
/// (killed) and [`TASKKILL_NOT_FOUND_EXIT_CODE`] (already gone) both count
/// as success; anything else maps to `Err` carrying the stderr tail.
fn taskkill_outcome(code: Option<i32>, stderr: &str) -> Result<()> {
    match code {
        Some(0) | Some(TASKKILL_NOT_FOUND_EXIT_CODE) => Ok(()),
        _ => Err(training_err(format!(
            "taskkill exited with {code:?}: {stderr}"
        ))),
    }
}

/// Kill `pid` and its entire process tree, but only while it is still
/// `image` -- see the module docs on PID reuse. If the PID is already dead,
/// or alive under a different image (recycled to an unrelated process),
/// this is a warned no-op rather than an error or an accidental kill of a
/// stranger process.
#[cfg(windows)]
pub async fn kill_tree(pid: u32, image: &str) -> Result<()> {
    if !is_alive(pid, image).await? {
        tracing::warn!(
            pid,
            image,
            "kill_tree: pid is not (or no longer) this image; skipping"
        );
        return Ok(());
    }
    let output = Command::new("taskkill")
        .args(kill_tree_args(pid))
        .output()
        .await
        .map_err(|e| training_err(format!("failed to run taskkill: {e}")))?;
    taskkill_outcome(
        output.status.code(),
        &String::from_utf8_lossy(&output.stderr),
    )
}

#[cfg(not(windows))]
pub async fn kill_tree(_pid: u32, _image: &str) -> Result<()> {
    Err(training_err(
        "process tree kill is Windows-only in this build",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::spawn::{launch_detached_quiet, SpawnSpec};
    use std::time::Duration;

    #[test]
    fn kill_tree_args_use_taskkill_with_tree_and_force() {
        assert_eq!(kill_tree_args(4242), vec!["/PID", "4242", "/T", "/F"]);
    }

    #[test]
    fn taskkill_outcome_treats_success_and_not_found_as_ok() {
        assert!(taskkill_outcome(Some(0), "").is_ok());
        assert!(taskkill_outcome(Some(TASKKILL_NOT_FOUND_EXIT_CODE), "nicht unterstützt").is_ok());
    }

    #[test]
    fn taskkill_outcome_is_err_for_any_other_exit_code() {
        let err = taskkill_outcome(Some(1), "Access is denied.").unwrap_err();
        assert!(err.to_string().contains("Access is denied"));
        assert!(taskkill_outcome(None, "killed by signal").is_err());
    }

    #[test]
    fn csv_image_name_reads_the_first_quoted_field() {
        assert_eq!(
            csv_image_name("\"ping.exe\",\"1234\",\"Console\",\"1\",\"4 K\""),
            Some("ping.exe")
        );
    }

    #[test]
    fn csv_image_name_is_none_for_empty_tasklist_output() {
        assert_eq!(csv_image_name(""), None);
    }

    #[test]
    fn parse_pid_file_round_trips_pid_and_image() {
        assert_eq!(
            parse_pid_file("4242 python.exe"),
            Some((4242, "python.exe".to_string()))
        );
    }

    #[test]
    fn parse_pid_file_rejects_garbage() {
        assert_eq!(parse_pid_file("not-a-pid"), None);
        assert_eq!(parse_pid_file("4242"), None);
        assert_eq!(parse_pid_file("4242 "), None);
    }

    #[tokio::test]
    async fn pid_file_round_trips_and_rejects_garbage() {
        let dir = tempfile::tempdir().expect("tempdir");
        let work_dir = dir.path();

        assert_eq!(read_pid_file(work_dir).await.expect("read"), None);

        write_pid_file(work_dir, 1234, "python.exe")
            .await
            .expect("write");
        assert_eq!(
            read_pid_file(work_dir).await.expect("read"),
            Some((1234, "python.exe".to_string()))
        );

        tokio::fs::write(pid_file(work_dir), "not-a-pid")
            .await
            .expect("write garbage");
        assert!(read_pid_file(work_dir).await.is_err());
    }

    /// Guards a spawned test process: kills its tree on drop so a failed
    /// assertion (which unwinds past every explicit cleanup call) never
    /// leaves a `ping` straggler running. Uses `taskkill` directly (not
    /// `kill_tree`, which is async and can't run from `Drop`).
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
        const IMAGE: &str = "ping.exe";
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("train.log");
        let spec = SpawnSpec {
            title: "AIWM: test".into(),
            cwd: dir.path().to_path_buf(),
            program: PathBuf::from(IMAGE),
            args: vec!["-n".into(), "30".into(), "127.0.0.1".into()],
            env: vec![],
        };

        let pid = launch_detached_quiet(&spec, &log_path)
            .await
            .expect("launch");
        let _guard = KillOnDrop(pid);

        let mut alive = false;
        for _ in 0..20 {
            if is_alive(pid, IMAGE).await.expect("is_alive") {
                alive = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(alive, "pid {pid} never showed up as alive");

        // Give ping a moment to actually write some output before we kill
        // it, so the log-content assertion below proves the `>>`
        // redirection carries real stdout, not just an empty file. Read as
        // raw bytes, not `read_to_string`: a console app's output
        // redirected to a file lands in the OS's ANSI/OEM codepage (e.g.
        // CP850 on a German Windows box, for words like "für"), which is
        // not valid UTF-8 -- `read_to_string` would silently look like "no
        // update yet" forever on every such byte, never on a length check.
        let mut log_has_output = false;
        for _ in 0..30 {
            if let Ok(bytes) = tokio::fs::read(&log_path).await {
                if bytes.len() > 10 {
                    log_has_output = true;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(log_has_output, "train.log never received ping's own output");

        kill_tree(pid, IMAGE).await.expect("kill_tree");

        let mut dead = false;
        for _ in 0..50 {
            if !is_alive(pid, IMAGE).await.expect("is_alive") {
                dead = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(dead, "pid {pid} still alive 5s after kill_tree");
    }
}
