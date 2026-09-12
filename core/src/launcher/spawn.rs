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

use std::path::PathBuf;

use tokio::process::Command;

use crate::{CoreError, Result};

/// `CREATE_NEW_CONSOLE` (`wincon.h`) — give the child its own visible console
/// window instead of inheriting ours.
#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

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
}
