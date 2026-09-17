//! Spawning a real, independent terminal window.
//!
//! Deliberate deviation from `runtime::JobObject`: every other spawned
//! process in this app is assigned to a Job Object so it dies with AIWM (see
//! `super` for why a launched terminal is the opposite on purpose).
//! Windows-only — the whole project targets Windows 11.
//!
//! The child is `cmd.exe /K "title <title> && "<program>" <args...>"`,
//! spawned directly with the `CREATE_NEW_CONSOLE` flag rather than through
//! `start` — an earlier version shelled out to `cmd /C start ...` and it
//! reliably hung waiting for that shim to exit (a real, reproduced hang, not
//! a hypothetical: `start`'s own console hand-off logic does not play well
//! with a piped/non-interactive stdio setup, which is exactly what a Tauri
//! app's child processes get). Asking Windows directly for a new console via
//! `creation_flags` sidesteps `start` entirely. `/K` (not `/C`) keeps the
//! window open after `program` exits, so a failure (e.g. not found) is
//! visible instead of a window that flashes and vanishes.
//!
//! We never `.wait()` the spawned `Child` — dropping it does **not** kill
//! the process (Rust's own documented behaviour), and Windows has no
//! POSIX-style zombie-process concept requiring a parent `wait()`, so simply
//! letting the handle go out of scope is the correct "fire and forget".

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use crate::{CoreError, Result};

/// `CREATE_NEW_CONSOLE` (`wincon.h`) — give the child its own visible console
/// window instead of inheriting ours.
#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Windows creation flags (winbase.h): the child gets its own process group
/// and no console window; stdout/stderr go to `log_path` (appended).
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

fn spawn_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "launcher".into(),
        message: msg.to_string(),
    }
}

/// A detached terminal to open: `program` run with `args` in a new console
/// window titled `title`, starting in `cwd`, with `env` added on top of the
/// inherited environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub title: String,
    pub cwd: PathBuf,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// One `cmd /K` command string: `title <title> && "<program>" <arg> ...`.
/// `title` is cmd's own builtin (sets the console window's title); `&&`
/// chains it with the real command. `program`/`args` are individually
/// quoted since `cmd` re-parses this as a single line, not an argv array.
pub fn terminal_command_line(spec: &SpawnSpec) -> String {
    let mut cmd = format!(
        "title {} && {}",
        spec.title,
        quote(&spec.program.display().to_string())
    );
    for arg in &spec.args {
        cmd.push(' ');
        cmd.push_str(&quote(arg));
    }
    cmd
}

fn quote(s: &str) -> String {
    if s.is_empty() || s.contains([' ', '&', '|', '<', '>', '^']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Open `spec` as a brand-new, independent console window: no Job Object, no
/// tracked child handle. Only the initial `CreateProcess` call can fail
/// (e.g. `cmd.exe` itself missing, which never happens on a real Windows
/// install) — once spawned, the window is already fully independent.
pub async fn launch_detached(spec: &SpawnSpec) -> Result<()> {
    let mut cmd = Command::new("cmd");
    cmd.arg("/K").arg(terminal_command_line(spec));
    cmd.current_dir(&spec.cwd);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NEW_CONSOLE);

    let child = cmd
        .spawn()
        .map_err(|e| spawn_err(format!("failed to open a terminal: {e}")))?;
    // Deliberately not awaited/killed -- see module docs. Drop the handle
    // now so nothing on our side references the process any further.
    drop(child);
    Ok(())
}

/// One `cmd /C` command string that redirects the child's stdout/stderr to
/// `log_path` (appended, both streams merged): `"<program>" <arg> ... >>
/// "<log>" 2>&1`. A single string because `cmd` re-parses it as a line, not
/// an argv array -- same reasoning as [`terminal_command_line`], plus the
/// trailing redirection cmd itself understands.
///
/// Used for training runs: they must survive an app restart (so they are
/// launched detached, not tracked as a child of this process) but must
/// never pop a visible console window, and their output must land on disk
/// even if the wrapped program crashes before it opens its own log (e.g.
/// ai-toolkit's own `run.py -l <log>` starts teeing its output only once
/// Python itself is up).
pub fn quiet_command_line(spec: &SpawnSpec, log_path: &Path) -> String {
    let mut cmd = quote(&spec.program.display().to_string());
    for arg in &spec.args {
        cmd.push(' ');
        cmd.push_str(&quote(arg));
    }
    cmd.push_str(" >> ");
    cmd.push_str(&quote(&log_path.display().to_string()));
    cmd.push_str(" 2>&1");
    cmd
}

/// Launch `spec` fully detached and silent: no console window, stdout/stderr
/// appended to `log_path`. Returns the PID of the `cmd /C` wrapper process --
/// kill it with [`crate::training::process::kill_tree`] so the real child
/// (e.g. the Python process `cmd` execs into a shell for) dies too.
///
/// We deliberately never `.wait()` or kill this `Child` on drop: the run
/// must keep going after this app process exits, exactly like
/// [`launch_detached`] but silent instead of visible. Tokio's
/// `Command::kill_on_drop` already defaults to `false`, matching that; it is
/// set explicitly below so the intent doesn't depend on an undocumented
/// default surviving a future tokio upgrade.
pub async fn launch_detached_quiet(spec: &SpawnSpec, log_path: &Path) -> Result<u32> {
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(quiet_command_line(spec, log_path));
        cmd
    };
    // Non-Windows fallback (this project targets Windows 11, but the shape
    // is kept correct rather than `unimplemented!`): `sh -c` plus a new
    // process group in place of `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`.
    #[cfg(not(windows))]
    let mut cmd = {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(quiet_command_line(spec, log_path));
        cmd
    };
    cmd.current_dir(&spec.cwd);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    cmd.kill_on_drop(false);
    #[cfg(windows)]
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    #[cfg(not(windows))]
    cmd.process_group(0);

    let child = cmd
        .spawn()
        .map_err(|e| spawn_err(format!("failed to launch a detached process: {e}")))?;
    let pid = child
        .id()
        .ok_or_else(|| spawn_err("spawned process has no PID (already reaped)"))?;
    // Deliberately not awaited -- see module docs and this fn's docs above.
    drop(child);
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn spec() -> SpawnSpec {
        SpawnSpec {
            title: "AIWM: OpenCode".into(),
            cwd: Path::new("E:\\projects\\demo").to_path_buf(),
            program: Path::new("C:\\tools\\opencode.exe").to_path_buf(),
            args: vec![],
            env: vec![("OPENCODE_CONFIG_CONTENT".into(), "{}".into())],
        }
    }

    #[test]
    fn sets_the_title_then_runs_the_program() {
        let line = terminal_command_line(&spec());
        assert_eq!(line, "title AIWM: OpenCode && C:\\tools\\opencode.exe");
    }

    #[test]
    fn quotes_a_program_path_containing_spaces() {
        let mut s = spec();
        s.program = Path::new("C:\\Program Files\\opencode\\opencode.exe").to_path_buf();
        let line = terminal_command_line(&s);
        assert!(
            line.ends_with("\"C:\\Program Files\\opencode\\opencode.exe\""),
            "{line}"
        );
    }

    #[test]
    fn quotes_args_that_need_it_and_leaves_simple_ones_bare() {
        let mut s = spec();
        s.args = vec!["--flag".into(), "a value with spaces".into()];
        let line = terminal_command_line(&s);
        assert!(line.ends_with("--flag \"a value with spaces\""), "{line}");
    }

    #[test]
    fn escapes_embedded_quotes_in_an_argument() {
        let mut s = spec();
        s.args = vec!["say \"hi\"".into()];
        let line = terminal_command_line(&s);
        assert!(line.ends_with("\"say \"\"hi\"\"\""), "{line}");
    }

    // The real spawn (a genuinely new, independent console window) is proven
    // end to end by `tests/launcher_detached_spawn.rs`, not here -- popping a
    // real window isn't something a unit test should do.

    #[test]
    fn quiet_command_line_redirects_output_to_the_log_file() {
        let line = quiet_command_line(&spec(), Path::new("E:\\runs\\r1\\train.log"));
        assert_eq!(
            line,
            "C:\\tools\\opencode.exe >> E:\\runs\\r1\\train.log 2>&1"
        );
    }

    #[test]
    fn quiet_command_line_quotes_program_args_and_log_path_that_need_it() {
        let mut s = spec();
        s.program = Path::new("C:\\Program Files\\opencode\\opencode.exe").to_path_buf();
        s.args = vec!["--flag".into(), "a value with spaces".into()];
        let line = quiet_command_line(&s, Path::new("E:\\Training Runs\\r1\\train.log"));
        assert_eq!(
            line,
            "\"C:\\Program Files\\opencode\\opencode.exe\" --flag \"a value with spaces\" >> \"E:\\Training Runs\\r1\\train.log\" 2>&1"
        );
    }

    // The real detached-and-quiet spawn (no console window, output on disk,
    // liveness and tree-kill) is proven end to end by
    // `a_quiet_child_is_alive_until_its_tree_is_killed` in
    // `training::process`, not here.
}
