//! Proves `launcher::spawn::launch_detached` does what its docs claim: the
//! target process is NOT a supervised child. It survives independent of
//! anything AIWM-side, unlike every other spawned process in this app
//! (`runtime::JobObject`). Windows-only, like the feature itself.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use aiwm_core::launcher::spawn::{launch_detached, SpawnSpec};

fn fixture_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-longrunning"))
}

fn pid_alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .expect("run tasklist");
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

#[tokio::test]
async fn a_launched_process_outlives_everything_aiwm_side() {
    let tmp = tempfile::tempdir().unwrap();
    let pid_file = tmp.path().join("pid.txt");
    let stop_file = tmp.path().join("stop");

    let spec = SpawnSpec {
        title: "aiwm-launcher-test".into(),
        cwd: tmp.path().to_path_buf(),
        program: fixture_bin(),
        args: vec![
            pid_file.display().to_string(),
            stop_file.display().to_string(),
        ],
        env: vec![],
    };

    // `launch_detached` only awaits the short-lived `cmd /C start` shim --
    // by the time this returns, that shim (and everything AIWM-side) is
    // already gone. If the fixture were still a child of it, or assigned to
    // a Job Object like every other spawned process in this app, it would
    // already be dead too.
    launch_detached(&spec).await.unwrap();

    let mut pid = None;
    for _ in 0..100 {
        if let Ok(s) = std::fs::read_to_string(&pid_file) {
            if let Ok(p) = s.trim().parse() {
                pid = Some(p);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let pid: u32 = pid.expect("fixture never wrote its pid");

    assert!(
        pid_alive(pid),
        "the launched process must survive independent of the launcher"
    );

    std::fs::write(&stop_file, "").unwrap();
    for _ in 0..100 {
        if !pid_alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output();
    panic!("fixture did not exit after the stop file appeared");
}
