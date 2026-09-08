//! `aiwm-cored` boots and then shuts down cleanly on an OS signal.
//!
//! Windows only: uses `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT)`, which is the
//! one console signal that can target a specific child process group.

#![cfg(windows)]
// Sending a console control event requires one FFI call into kernel32.
#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CTRL_BREAK_EVENT: u32 = 1;

extern "system" {
    fn GenerateConsoleCtrlEvent(dw_ctrl_event: u32, dw_process_group_id: u32) -> i32;
}

#[test]
fn boots_and_shuts_down_cleanly_on_ctrl_break() {
    let data_dir = tempfile::tempdir().unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_aiwm-cored"))
        .env("AIWM_DATA_DIR", data_dir.path())
        .env("AIWM_LOG", "info")
        .creation_flags(CREATE_NEW_PROCESS_GROUP)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn aiwm-cored");

    // Collect stdout on a worker; signal the main thread once the ready banner lands.
    let stdout = child.stdout.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut collected = String::new();
        let mut announced = false;
        for line in BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if !announced && line.contains("Press Ctrl-C to stop") {
                let _ = ready_tx.send(());
                announced = true;
            }
            collected.push_str(&line);
            collected.push('\n');
        }
        collected
    });

    ready_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("daemon should print its ready banner");

    // Signal handlers are installed before the banner, so this cannot race.
    // SAFETY: plain FFI call to kernel32; both args are integers and the child
    // (spawned with CREATE_NEW_PROCESS_GROUP) shares this process's console.
    let sent = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) };
    assert_ne!(sent, 0, "GenerateConsoleCtrlEvent failed");

    let status = wait_with_timeout(&mut child, Duration::from_secs(15));
    let output = reader.join().unwrap();

    assert!(
        status.success(),
        "expected clean exit, got {status:?}\n---\n{output}"
    );
    assert!(
        output.contains("shutdown complete (ctrl-break)"),
        "missing graceful-shutdown line:\n{output}"
    );
    assert!(data_dir.path().join("config.toml").is_file());
    assert!(data_dir.path().join("logs").is_dir());
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("aiwm-cored did not exit within {timeout:?} of the signal");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
