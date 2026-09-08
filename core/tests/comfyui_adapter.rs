//! `ComfyUiAdapter` driven against the `aiwm-fake-comfy` stand-in server:
//! a real child process, real `RuntimeSupervisor`, real loopback HTTP.
//!
//! Windows only — the production target and where process supervision (Job
//! Objects) actually matters; `cargo test` elsewhere skips this file.

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Duration;

use aiwm_core::db::Database;
use aiwm_core::runtime::{ComfyDirs, ComfyLaunch, ComfyUiAdapter, Health, RuntimeAdapter};

fn fake_comfy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aiwm-fake-comfy"))
}

fn fixture_launch(extra_args: &[&str]) -> ComfyLaunch {
    ComfyLaunch {
        program: fake_comfy_bin(),
        main: None,
        extra_args: extra_args.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn dirs(tmp: &std::path::Path) -> ComfyDirs {
    ComfyDirs {
        base: tmp.join("comfyui-data"),
        output: tmp.join("outputs"),
    }
}

/// Adapter wired to the fixture exe, with its data/output dirs under `tmp`.
async fn adapter(tmp: &std::path::Path) -> ComfyUiAdapter {
    let db = Database::connect_in_memory().await.unwrap();
    ComfyUiAdapter::with_launch(db, Some(fixture_launch(&[])), dirs(tmp))
}

#[tokio::test]
async fn starts_the_server_on_first_load_and_stays_up_across_unload() {
    let tmp = tempfile::tempdir().unwrap();
    let a = adapter(tmp.path()).await;

    assert_eq!(a.health().await, Health::Unknown); // not started yet

    a.load_model("sdxl", 7_000).await.unwrap();
    assert_eq!(a.health().await, Health::Healthy);
    assert_eq!(a.loaded_models().len(), 1);
    assert_eq!(a.vram_used_mb(), 7_000);
    assert!(a.detail().unwrap().starts_with("running on :"));

    // ComfyUI created its data + output directories.
    assert!(tmp.path().join("comfyui-data").is_dir());
    assert!(tmp.path().join("outputs").is_dir());

    // system_stats comes back from the live fixture.
    let stats = a.system_stats().await.unwrap();
    assert!(stats.version.unwrap().contains("fake"));
    assert!(stats.vram_total_mb > 15_000);

    // Unload frees the slot but the server keeps running (Python boot is dear).
    a.unload_model("sdxl").await.unwrap();
    assert!(a.loaded_models().is_empty());
    assert_eq!(a.health().await, Health::Healthy);

    a.stop().await.unwrap();
    assert_eq!(a.health().await, Health::Unknown);
}

#[tokio::test]
async fn second_model_swaps_the_slot_without_restarting_the_server() {
    let tmp = tempfile::tempdir().unwrap();
    let a = adapter(tmp.path()).await;

    a.load_model("sdxl", 7_000).await.unwrap();
    let stats_a = a.system_stats().await.unwrap();

    a.load_model("flux", 13_000).await.unwrap();
    let loaded = a.loaded_models();
    assert_eq!(loaded.len(), 1, "one resident model at a time");
    assert_eq!(loaded[0].model_id, "flux");
    assert_eq!(a.vram_used_mb(), 13_000);

    // Same server process — its reported stats are unchanged.
    assert_eq!(a.system_stats().await.unwrap(), stats_a);

    a.stop().await.unwrap();
}

#[tokio::test]
async fn waits_out_a_slow_cold_start() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    // The fixture binds its socket only after ~800 ms.
    let a = ComfyUiAdapter::with_launch(
        db,
        Some(fixture_launch(&["--fake-ready-ms", "800"])),
        dirs(tmp.path()),
    );

    let start = std::time::Instant::now();
    a.load_model("sdxl", 7_000).await.unwrap();

    assert!(
        start.elapsed() >= Duration::from_millis(700),
        "should have waited out the cold start"
    );
    assert_eq!(a.health().await, Health::Healthy);
    a.stop().await.unwrap();
}

#[tokio::test]
async fn load_without_an_install_is_a_clear_error() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let a = ComfyUiAdapter::with_launch(db, None, dirs(tmp.path()));

    let err = a.load_model("sdxl", 7_000).await.unwrap_err();
    assert!(err.to_string().contains("not installed"), "got: {err}");
    assert_eq!(a.health().await, Health::Unknown);
}
