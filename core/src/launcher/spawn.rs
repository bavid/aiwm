//! Spawning a real, independent terminal window, plus a quiet detached
//! launch used by long-running background work (training runs) that must
//! survive an app restart without popping a console.
//!
//! Deliberate deviation from `runtime::JobObject`: every other spawned
//! process in this app is assigned to a Job Object so it dies with AIWM (see
//! `super` for why a launched terminal is the opposite on purpose).
//! Windows-only — the whole project targets Windows 11.
//!
//! The visible-terminal child is `cmd.exe /K "title <title> && "<program>"
//! <args...>"`, spawned directly with the `CREATE_NEW_CONSOLE` flag rather
//! than through `start` — an earlier version shelled out to `cmd /C start
//! ...` and it reliably hung waiting for that shim to exit (a real,
//! reproduced hang, not a hypothetical: `start`'s own console hand-off
//! logic does not play well with a piped/non-interactive stdio setup, which
//! is exactly what a Tauri app's child processes get). Asking Windows
//! directly for a new console via `creation_flags` sidesteps `start`
//! entirely. `/K` (not `/C`) keeps the window open after `program` exits,
//! so a failure (e.g. not found) is visible instead of a window that
//! flashes and vanishes.
//!
//! We never `.wait()` either kind of spawned `Child` — dropping it does
//! **not** kill the process (Rust's own documented behaviour), and Windows
//! has no POSIX-style zombie-process concept requiring a parent `wait()`,
//! so simply letting the handle go out of scope is the correct "fire and
//! forget".

use std::path::PathBuf;
use std::process::Stdio;

use tokio::process::Command;

use crate::{CoreError, Result};

/// `CREATE_NEW_CONSOLE` (`wincon.h`) — give the child its own visible console
/// window instead of inheriting ours.
#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Windows creation flags (winbase.h) for the quiet detached launch: the
/// child gets its own process group and no *visible* console window.
///
/// This is deliberately `CREATE_NO_WINDOW` (a hidden console), not
/// `DETACHED_PROCESS` (no console at all). An earlier version used
/// `DETACHED_PROCESS` and it reliably broke `taskkill /PID <pid> /T /F`:
/// every process in the tree failed to terminate with `ERROR_NOT_SUPPORTED`
/// ("Dieser Vorgang wird nicht unterstützt" / "This operation is not
/// supported"), reproduced against a real training-shaped process tree.
/// `taskkill /T`'s tree walk depends on the console plumbing a normal
/// (even hidden) console process has; a `DETACHED_PROCESS` child lacks it
/// and becomes untellable to end via that path, leaving orphaned processes
/// behind. `CREATE_NO_WINDOW` still gives a fully detached, silent launch —
/// no window is ever shown — while keeping the console plumbing
/// `taskkill /T` needs.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

fn spawn_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "launcher".into(),
        message: msg.to_string(),
    }
}

/// A detached process to launch: `program` run with `args`, starting in
/// `cwd`, with `env` added on top of the inherited environment. `title`
/// names the console window for [`launch_detached`]'s visible terminal; it
/// is unused by [`launch_detached_quiet`].
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

/// Launch `spec.program` fully detached and silent: no console window, no
/// shell, `spec.args` passed as a real argument vector. Stdout and stderr
/// are both appended to `log_path` (opened once, then `stderr` gets a
/// cloned handle so both streams share one file position and interleave
/// correctly); returns the child's own PID.
///
/// `spec.program` is spawned directly — **not** wrapped in `cmd /C "<line>"`.
/// An earlier version built one `cmd /C` string (program, args and the `>>`
/// redirection all concatenated, quoted by our own rules) and handed it to
/// `Command::arg`. That collided with two independent quoting schemes at
/// once: Rust's `Command` quotes each argv value it hands to `CreateProcess`
/// assuming the target parses standard argv, but `cmd.exe`'s `/C` line does
/// not — it re-parses the *whole line* with its own idiosyncratic quote
/// rules. The two disagree the moment anything needs quoting (a path with
/// spaces, an argument containing `>` or `&`), reproduced as `cmd.exe`
/// reporting the entire re-quoted line back as an unrecognized command. It
/// also meant every argument round-tripped through `cmd.exe`'s parser at
/// all, an unnecessary `%VAR%`-expansion and `&|<>^` shell-injection
/// surface for values that were never meant to be shell code. Spawning
/// `program` directly removes both problems: there is no shell to
/// re-interpret metacharacters or disagree about quoting.
///
/// We deliberately never `.wait()` or kill this `Child` on drop: the run
/// must keep going after this app process exits, exactly like
/// [`launch_detached`] but silent instead of visible. Tokio's
/// `Command::kill_on_drop` already defaults to `false`, matching that; it is
/// set explicitly below so the intent doesn't depend on an undocumented
/// default surviving a future tokio upgrade.
#[cfg(windows)]
pub async fn launch_detached_quiet(spec: &SpawnSpec, log_path: &std::path::Path) -> Result<u32> {
    let stdout_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| {
            spawn_err(format!(
                "failed to open log file {}: {e}",
                log_path.display()
            ))
        })?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| spawn_err(format!("failed to clone log file handle: {e}")))?;

    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    cmd.current_dir(&spec.cwd);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::from(stdout_file));
    cmd.stderr(Stdio::from(stderr_file));
    cmd.kill_on_drop(false);
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);

    let child = cmd
        .spawn()
        .map_err(|e| spawn_err(format!("failed to launch a detached process: {e}")))?;
    let pid = child
        .id()
        .ok_or_else(|| spawn_err("spawned process has no PID (already reaped)"))?;
    // Deliberately not awaited -- see this fn's docs above.
    drop(child);
    Ok(pid)
}

/// This project targets Windows 11 only (see module docs); a quiet detached
/// launch relies on Windows-specific creation flags with no safe
/// equivalent worth maintaining untested, so this is a hard error rather
/// than a silently-different dev-only fallback.
#[cfg(not(windows))]
pub async fn launch_detached_quiet(_spec: &SpawnSpec, _log_path: &std::path::Path) -> Result<u32> {
    Err(spawn_err(
        "quiet detached launch is Windows-only in this build",
    ))
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

    // The real detached-and-quiet spawn (no console window, argv passed
    // through untouched, output on disk, liveness and tree-kill) is proven
    // end to end by `a_quiet_child_is_alive_until_its_tree_is_killed` in
    // `training::process`, not here -- it needs `training::process::kill_tree`
    // to clean up after itself, which would be a layering inversion here.
}
