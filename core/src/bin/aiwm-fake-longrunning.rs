//! Test fixture: writes its own PID to the file named in argv[1], then waits
//! until the file named in argv[2] exists before exiting.
//!
//! Used by `tests/launcher_detached_spawn.rs` to prove a
//! `launcher::spawn::launch_detached` process survives even after everything
//! on the AIWM side is dropped -- the property the whole feature rests on.

use std::env;
use std::fs;
use std::path::Path;
use std::process;
use std::thread::sleep;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    let pid_file = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing pid file path (argv[1])"))?;
    let stop_file = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing stop file path (argv[2])"))?;

    fs::write(&pid_file, process::id().to_string())?;

    // 120s safety cap so a failed test cleanup doesn't leave this running
    // forever.
    for _ in 0..1200 {
        if Path::new(&stop_file).exists() {
            return Ok(());
        }
        sleep(Duration::from_millis(100));
    }
    Ok(())
}
