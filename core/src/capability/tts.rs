//! The text-to-speech capability: drive one `job_type=tts` body — the Story
//! Studio narrator. Renders `params.text` with the local Kokoro model via the
//! Python sidecar's `synthesize_speech`, and writes the result to
//! `<outputs>/<job_id>.wav`.

use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::{json, Value};

use super::media::write_output;
use crate::db::Database;
use crate::runtime::TtsAdapter;
use crate::{CoreError, Result};

const DEFAULT_VOICE: &str = "am_michael";
const DEFAULT_SPEED: f64 = 1.0;
const MIN_SPEED: f64 = 0.5;
const MAX_SPEED: f64 = 2.0;

fn tts_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "tts".into(),
        message: msg.to_string(),
    }
}

/// A resolved narration request, pulled from a job's `params`. Only `text` is
/// required; `voice`/`speed` fall back to a sensible default narrator preset.
#[derive(Debug, Clone, PartialEq)]
pub struct TtsRequest {
    pub text: String,
    pub voice: String,
    pub speed: f64,
}

impl TtsRequest {
    pub fn from_params(params: &Value) -> Result<Self> {
        let text = params
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| tts_err("tts job has no `text`"))?
            .to_string();
        let voice = params
            .get("voice")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(DEFAULT_VOICE)
            .to_string();
        let speed = params
            .get("speed")
            .and_then(Value::as_f64)
            .map_or(DEFAULT_SPEED, |v| v.clamp(MIN_SPEED, MAX_SPEED));
        Ok(Self { text, voice, speed })
    }
}

/// A finished narration.
#[derive(Debug, Clone)]
pub struct TtsDone {
    pub output_path: PathBuf,
    pub duration_secs: f64,
}

/// Render `req` on the sidecar's Kokoro engine, save the result under
/// `outputs_dir`. The Kokoro model + its voices file are found by role
/// (`voice_model` / `voice_data`) — there is exactly one of each in practice
/// (the catalog's "Download entire stack" imports both together), so `Auto`
/// needs no picker UI, just "is one imported yet".
pub async fn run(
    db: &Database,
    tts: &TtsAdapter,
    outputs_dir: &Path,
    job_id: &str,
    req: TtsRequest,
) -> Result<TtsDone> {
    let model = db
        .models()
        .for_role("voice_model")
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| {
            tts_err(
                "no voice model imported — import Kokoro on the Models tab \
                 (Add models \u{2192} Voice)",
            )
        })?;
    let voices = db
        .models()
        .for_role("voice_data")
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| {
            tts_err(
                "no voice data imported — import Kokoro's voices file on the Models tab \
                 (Add models \u{2192} Voice)",
            )
        })?;

    let client = tts.client().await?;
    let result = client
        .call(
            "synthesize_speech",
            json!({
                "text": req.text,
                "model_path": model.file_path,
                "voices_path": voices.file_path,
                "voice": req.voice,
                "speed": req.speed,
            }),
        )
        .await?;

    let audio_b64 = result
        .get("audio_base64")
        .and_then(Value::as_str)
        .ok_or_else(|| tts_err("the sidecar returned no audio"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio_b64)
        .map_err(|e| tts_err(format!("bad audio from the sidecar: {e}")))?;
    let duration_secs = result
        .get("duration_secs")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);

    let output_path = write_output(outputs_dir, job_id, "wav", &bytes).await?;
    Ok(TtsDone {
        output_path,
        duration_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_params_requires_non_empty_text() {
        assert!(TtsRequest::from_params(&json!({})).is_err());
        assert!(TtsRequest::from_params(&json!({ "text": "   " })).is_err());
    }

    #[test]
    fn from_params_defaults_voice_and_speed() {
        let r = TtsRequest::from_params(&json!({ "text": "hello there" })).unwrap();
        assert_eq!(r.text, "hello there");
        assert_eq!(r.voice, DEFAULT_VOICE);
        assert_eq!(r.speed, DEFAULT_SPEED);
    }

    #[test]
    fn from_params_keeps_an_explicit_voice_and_speed() {
        let r = TtsRequest::from_params(&json!({
            "text": "a mysterious tale",
            "voice": "am_fenrir",
            "speed": 0.85,
        }))
        .unwrap();
        assert_eq!(r.voice, "am_fenrir");
        assert_eq!(r.speed, 0.85);
    }

    #[test]
    fn from_params_clamps_an_extreme_speed() {
        let too_fast = TtsRequest::from_params(&json!({ "text": "x", "speed": 9.0 })).unwrap();
        assert_eq!(too_fast.speed, MAX_SPEED);
        let too_slow = TtsRequest::from_params(&json!({ "text": "x", "speed": 0.01 })).unwrap();
        assert_eq!(too_slow.speed, MIN_SPEED);
    }

    // `TtsAdapter::new()` never spawns a process (or even resolves `uv`) --
    // both tests below fail the "no voice model / voices" check before
    // `tts.client()` (and so any real spawn) would ever run.

    #[tokio::test]
    async fn run_reports_a_clear_error_when_no_voice_model_is_imported() {
        let db = Database::connect_in_memory().await.unwrap();
        let tts = TtsAdapter::new();
        let req = TtsRequest::from_params(&json!({ "text": "hello" })).unwrap();

        let err = run(&db, &tts, std::path::Path::new("/tmp/out"), "job-1", req)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no voice model imported"), "{err}");
    }

    #[tokio::test]
    async fn run_reports_a_clear_error_when_the_voice_model_has_no_voices_file() {
        use crate::db::NewModel;

        let db = Database::connect_in_memory().await.unwrap();
        db.models()
            .insert(NewModel {
                name: "kokoro-v1.0.int8".into(),
                format: "onnx".into(),
                file_path: "E:\\AI\\models\\voice\\kokoro-v1.0.int8.onnx".into(),
                size_bytes: 114_119_327,
                source: "manual".into(),
                roles: vec!["voice_model".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
        let tts = TtsAdapter::new();
        let req = TtsRequest::from_params(&json!({ "text": "hello" })).unwrap();

        let err = run(&db, &tts, std::path::Path::new("/tmp/out"), "job-1", req)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no voice data imported"), "{err}");
    }
}
