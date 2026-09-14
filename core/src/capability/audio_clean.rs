//! Post-process an already-rendered narration clip through the sidecar's
//! `clean_audio` method — a DSP pass (DC-offset removal, a gentle high-pass
//! filter, and spectral-gate noise reduction) that runs on whatever's
//! already on disk, not a new render. No VRAM, no model to pick, nothing
//! that needs the scheduler's Target/Decision pipeline — it's fast and
//! deterministic enough to run synchronously from the HTTP/Tauri handler.

use std::path::Path;

use base64::Engine;
use serde_json::{json, Value};

use crate::runtime::TtsAdapter;
use crate::{CoreError, Result};

fn clean_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "tts".into(),
        message: msg.to_string(),
    }
}

/// Reads the WAV at `path`, sends it through the sidecar's cleanup pass, and
/// overwrites `path` with the result. Returns the resulting duration (the
/// pass doesn't trim or extend audio, so this should match the original).
pub async fn clean_in_place(tts: &TtsAdapter, path: &Path) -> Result<f64> {
    let bytes = tokio::fs::read(path).await.map_err(CoreError::Io)?;
    let audio_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let client = tts.client().await?;
    let result = client
        .call("clean_audio", json!({ "audio_base64": audio_b64 }))
        .await?;

    let cleaned_b64 = result
        .get("audio_base64")
        .and_then(Value::as_str)
        .ok_or_else(|| clean_err("the sidecar returned no cleaned audio"))?;
    let cleaned_bytes = base64::engine::general_purpose::STANDARD
        .decode(cleaned_b64)
        .map_err(|e| clean_err(format!("bad cleaned audio from the sidecar: {e}")))?;
    let duration_secs = result
        .get("duration_secs")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);

    tokio::fs::write(path, cleaned_bytes)
        .await
        .map_err(CoreError::Io)?;
    Ok(duration_secs)
}
