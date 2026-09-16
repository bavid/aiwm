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

const DEFAULT_KOKORO_VOICE: &str = "am_michael";
/// Dia has no named voice presets (see [`TtsEngine::Dia`]) -- `voice` is
/// repurposed as a free-text "narrator identity" label the sidecar hashes
/// into a stable seed, so the default just needs to be a stable label too.
const DEFAULT_DIA_NARRATOR: &str = "narrator";
const DEFAULT_SPEED: f64 = 1.0;
const MIN_SPEED: f64 = 0.5;
const MAX_SPEED: f64 = 2.0;

fn tts_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "tts".into(),
        message: msg.to_string(),
    }
}

/// Which sidecar narration engine renders a request. Kokoro (the original,
/// default engine) is fast and flat; Dia (`nari-labs/Dia-1.6B-0626`) is
/// slower but supports real non-verbal tags (`(laughs)`, `(sighs)`, ...) and
/// `[S1]`/`[S2]` speaker turns -- see `aiwm_sidecar.dia` for what it
/// actually does and does not support (no freeform emotion tags either
/// way).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtsEngine {
    Kokoro,
    Dia,
}

impl TtsEngine {
    fn from_hint(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "kokoro" => Some(Self::Kokoro),
            "dia" => Some(Self::Dia),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kokoro => "kokoro",
            Self::Dia => "dia",
        }
    }
}

/// A resolved narration request, pulled from a job's `params`. Only `text` is
/// required; `engine` falls back to Kokoro, and `voice`/`speed` fall back to
/// a sensible default for whichever engine was picked.
#[derive(Debug, Clone, PartialEq)]
pub struct TtsRequest {
    pub text: String,
    pub voice: String,
    pub speed: f64,
    pub engine: TtsEngine,
    /// A saved [`crate::db::VoiceIdentity`] to voice-clone from (Dia only —
    /// see the module docs on [`TtsEngine::Dia`]). When set, `run` resolves
    /// its reference clip + transcript and hands them to the sidecar instead
    /// of relying on `voice`'s seed-only stable-but-generic identity.
    pub voice_identity_id: Option<String>,
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
        let engine = match params
            .get("engine")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(s) => TtsEngine::from_hint(s).ok_or_else(|| {
                tts_err(format!("unknown tts engine {s:?} (expected kokoro or dia)"))
            })?,
            None => TtsEngine::Kokoro,
        };
        let default_voice = match engine {
            TtsEngine::Kokoro => DEFAULT_KOKORO_VOICE,
            TtsEngine::Dia => DEFAULT_DIA_NARRATOR,
        };
        let voice = params
            .get("voice")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(default_voice)
            .to_string();
        let speed = params
            .get("speed")
            .and_then(Value::as_f64)
            .map_or(DEFAULT_SPEED, |v| v.clamp(MIN_SPEED, MAX_SPEED));
        let voice_identity_id = params
            .get("voice_identity_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        Ok(Self {
            text,
            voice,
            speed,
            engine,
            voice_identity_id,
        })
    }
}

/// A finished narration.
#[derive(Debug, Clone)]
pub struct TtsDone {
    pub output_path: PathBuf,
    pub duration_secs: f64,
}

/// The `voice_model` / `voice_data` pair to render with. Picks the *largest*
/// file in each role rather than just "the first one" — when both Kokoro
/// variants are imported (e.g. someone tried int8 first, then added fp32 for
/// its cleaner audio), the bigger, un-quantized file wins automatically
/// instead of depending on DB insertion order.
async fn resolve_voice_files(db: &Database) -> Result<(crate::db::Model, crate::db::Model)> {
    let model = db
        .models()
        .for_role("voice_model")
        .await?
        .into_iter()
        .max_by_key(|m| m.size_bytes)
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
        .max_by_key(|m| m.size_bytes)
        .ok_or_else(|| {
            tts_err(
                "no voice data imported — import Kokoro's voices file on the Models tab \
                 (Add models \u{2192} Voice)",
            )
        })?;
    Ok((model, voices))
}

/// How many catalog files make up one of Dia's directory-shaped components
/// (`dia_engine` or `dia_codec`) -- read live off the catalog rather than
/// hand-duplicated here, so the two can never quietly drift apart.
fn dia_component_total(kind: &str) -> usize {
    crate::model::KNOWN_MODELS
        .iter()
        .filter(|m| m.kind == kind)
        .count()
}

/// The shared directory holding every imported file with role `role` --
/// Dia's engine and codec files are imported individually (one `KnownModel`
/// row per file) but land as siblings in one fixed subdirectory per kind
/// (see `ModelKind::DiaEngine`/`DiaCodec`), so any one of them names the
/// directory the sidecar needs. Mirrors [`resolve_voice_files`]'s
/// role-based lookup, adapted for a kind that's a directory of many files
/// rather than one or two.
async fn resolve_dia_component_dir(db: &Database, role: &str, label: &str) -> Result<PathBuf> {
    let files = db.models().for_role(role).await?;
    if files.is_empty() {
        return Err(tts_err(format!(
            "no {label} files imported — import Dia on the Models tab \
             (Add models \u{2192} Voice \u{2192} \u{201c}Download entire stack\u{201d})"
        )));
    }
    let expected = dia_component_total(role);
    if expected > 0 && files.len() < expected {
        return Err(tts_err(format!(
            "{label} is incomplete ({} of {expected} files imported) — re-run \u{201c}Download \
             entire stack\u{201d} on the Models tab to get the rest",
            files.len()
        )));
    }
    let path = Path::new(&files[0].file_path);
    path.parent().map(Path::to_path_buf).ok_or_else(|| {
        tts_err(format!(
            "{label} file has no parent directory: {}",
            files[0].file_path
        ))
    })
}

/// Dia's two required directories -- its own weights/config/tokenizer, and
/// its separate DAC audio codec -- found by role (`dia_engine` / `dia_codec`).
async fn resolve_dia_dirs(db: &Database) -> Result<(PathBuf, PathBuf)> {
    let engine_dir = resolve_dia_component_dir(db, "dia_engine", "Dia engine").await?;
    let codec_dir = resolve_dia_component_dir(db, "dia_codec", "Dia's audio codec (DAC)").await?;
    Ok((engine_dir, codec_dir))
}

/// Resolves an optional saved [`crate::db::VoiceIdentity`] by id — `None` in,
/// `None` out (today's seed-only Dia path, unchanged); `Some(id)` in, a clear
/// error if that identity was deleted out from under a still-referencing job
/// body rather than a confusing sidecar-side failure.
async fn resolve_voice_identity(
    db: &Database,
    voice_identity_id: Option<&str>,
) -> Result<Option<crate::db::VoiceIdentity>> {
    let Some(id) = voice_identity_id else {
        return Ok(None);
    };
    let identity = db.voice_identities().get(id).await?.ok_or_else(|| {
        tts_err("the selected voice identity no longer exists — it may have been deleted")
    })?;
    Ok(Some(identity))
}

/// The `synthesize_speech` request body for a Dia render — a pure function so
/// the "does a saved voice identity actually add `reference_audio_path` /
/// `reference_transcript`?" question is testable without ever touching the
/// sidecar client.
fn dia_request_body(
    req: &TtsRequest,
    engine_dir: &Path,
    codec_dir: &Path,
    reference: Option<&crate::db::VoiceIdentity>,
) -> Value {
    let mut body = json!({
        "engine": "dia",
        "text": req.text,
        "model_dir": engine_dir.to_string_lossy(),
        "dac_dir": codec_dir.to_string_lossy(),
        "voice": req.voice,
    });
    if let Some(identity) = reference {
        body["reference_audio_path"] = json!(identity.reference_audio_path);
        body["reference_transcript"] = json!(identity.reference_transcript);
    }
    body
}

/// Pulls the finished clip out of a `synthesize_speech` result, common to
/// both engines.
fn decode_synth_result(result: &Value) -> Result<(Vec<u8>, f64)> {
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
    Ok((bytes, duration_secs))
}

/// Render `req` on the sidecar, save the result under `outputs_dir`. Which
/// engine actually runs is [`TtsRequest::engine`]; each resolves its own
/// model files by role before ever calling the sidecar, so a missing import
/// is a clear message here rather than a Python traceback.
pub async fn run(
    db: &Database,
    tts: &TtsAdapter,
    outputs_dir: &Path,
    job_id: &str,
    req: TtsRequest,
) -> Result<TtsDone> {
    let result = match req.engine {
        TtsEngine::Kokoro => {
            let (model, voices) = resolve_voice_files(db).await?;
            let client = tts.client().await?;
            client
                .call(
                    "synthesize_speech",
                    json!({
                        "engine": "kokoro",
                        "text": req.text,
                        "model_path": model.file_path,
                        "voices_path": voices.file_path,
                        "voice": req.voice,
                        "speed": req.speed,
                    }),
                )
                .await?
        }
        TtsEngine::Dia => {
            let (engine_dir, codec_dir) = resolve_dia_dirs(db).await?;
            let reference = resolve_voice_identity(db, req.voice_identity_id.as_deref()).await?;
            let client = tts.client().await?;
            let body = dia_request_body(&req, &engine_dir, &codec_dir, reference.as_ref());
            client.call("synthesize_speech", body).await?
        }
    };

    let (bytes, duration_secs) = decode_synth_result(&result)?;
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
    fn from_params_defaults_engine_voice_and_speed() {
        let r = TtsRequest::from_params(&json!({ "text": "hello there" })).unwrap();
        assert_eq!(r.text, "hello there");
        assert_eq!(r.engine, TtsEngine::Kokoro);
        assert_eq!(r.voice, DEFAULT_KOKORO_VOICE);
        assert_eq!(r.speed, DEFAULT_SPEED);
    }

    #[test]
    fn from_params_accepts_the_dia_engine_and_its_own_default_narrator_identity() {
        let r = TtsRequest::from_params(&json!({ "text": "hello", "engine": "dia" })).unwrap();
        assert_eq!(r.engine, TtsEngine::Dia);
        assert_eq!(r.voice, DEFAULT_DIA_NARRATOR);
    }

    #[test]
    fn from_params_engine_is_case_insensitive() {
        let r = TtsRequest::from_params(&json!({ "text": "hello", "engine": "DIA" })).unwrap();
        assert_eq!(r.engine, TtsEngine::Dia);
    }

    #[test]
    fn from_params_rejects_an_unknown_engine() {
        let err = TtsRequest::from_params(&json!({ "text": "hello", "engine": "nonexistent" }))
            .unwrap_err();
        assert!(err.to_string().contains("nonexistent"), "{err}");
    }

    #[test]
    fn from_params_keeps_an_explicit_dia_narrator_identity() {
        let r = TtsRequest::from_params(&json!({
            "text": "hello",
            "engine": "dia",
            "voice": "gravelly old man",
        }))
        .unwrap();
        assert_eq!(r.voice, "gravelly old man");
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

    #[test]
    fn from_params_has_no_voice_identity_by_default() {
        let r = TtsRequest::from_params(&json!({ "text": "hello", "engine": "dia" })).unwrap();
        assert_eq!(r.voice_identity_id, None);
    }

    #[test]
    fn from_params_keeps_an_explicit_voice_identity_id() {
        let r = TtsRequest::from_params(&json!({
            "text": "hello",
            "engine": "dia",
            "voice_identity_id": "abc-123",
        }))
        .unwrap();
        assert_eq!(r.voice_identity_id.as_deref(), Some("abc-123"));
    }

    #[test]
    fn from_params_treats_a_blank_voice_identity_id_as_absent() {
        let r = TtsRequest::from_params(&json!({
            "text": "hello",
            "engine": "dia",
            "voice_identity_id": "   ",
        }))
        .unwrap();
        assert_eq!(r.voice_identity_id, None);
    }

    // --- Dia voice cloning: reference resolution --------------------------

    fn a_voice_identity(id: &str) -> crate::db::VoiceIdentity {
        crate::db::VoiceIdentity {
            id: id.to_string(),
            name: "Old Man Gareth".to_string(),
            reference_audio_path: "E:\\AI\\data\\voice-identities\\x\\ref.wav".to_string(),
            reference_transcript: "a scratchy, smokey voice".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn dia_request_body_has_no_reference_fields_without_a_saved_identity() {
        let req = TtsRequest::from_params(&json!({ "text": "hello", "engine": "dia" })).unwrap();
        let body = dia_request_body(&req, Path::new("E:/engine"), Path::new("E:/codec"), None);
        assert!(body.get("reference_audio_path").is_none());
        assert!(body.get("reference_transcript").is_none());
        assert_eq!(body["voice"], DEFAULT_DIA_NARRATOR);
    }

    #[test]
    fn dia_request_body_adds_the_reference_fields_for_a_saved_identity() {
        let req = TtsRequest::from_params(&json!({ "text": "hello", "engine": "dia" })).unwrap();
        let identity = a_voice_identity("id-1");
        let body = dia_request_body(
            &req,
            Path::new("E:/engine"),
            Path::new("E:/codec"),
            Some(&identity),
        );
        assert_eq!(
            body["reference_audio_path"],
            "E:\\AI\\data\\voice-identities\\x\\ref.wav"
        );
        assert_eq!(body["reference_transcript"], "a scratchy, smokey voice");
    }

    #[tokio::test]
    async fn resolve_voice_identity_returns_none_when_none_is_requested() {
        let db = Database::connect_in_memory().await.unwrap();
        assert!(resolve_voice_identity(&db, None).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolve_voice_identity_reports_a_clear_error_for_an_unknown_id() {
        let db = Database::connect_in_memory().await.unwrap();
        let err = resolve_voice_identity(&db, Some("nonexistent"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no longer exists"), "{err}");
    }

    #[tokio::test]
    async fn resolve_voice_identity_finds_a_saved_identity_by_id() {
        let db = Database::connect_in_memory().await.unwrap();
        let created = db
            .voice_identities()
            .create("Old Man Gareth", "path/ref.wav", "a scratchy voice")
            .await
            .unwrap();

        let found = resolve_voice_identity(&db, Some(&created.id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, created.id);
        assert_eq!(found.reference_transcript, "a scratchy voice");
    }

    #[tokio::test]
    async fn run_dia_reports_a_clear_error_when_the_voice_identity_no_longer_exists() {
        let db = Database::connect_in_memory().await.unwrap();
        insert_dia_files(
            &db,
            "dia_engine",
            "E:\\AI\\models\\voice\\dia-engine",
            dia_engine_file_count(),
        )
        .await;
        insert_dia_files(
            &db,
            "dia_codec",
            "E:\\AI\\models\\voice\\dia-codec",
            dia_codec_file_count(),
        )
        .await;
        let tts = TtsAdapter::new();
        let req = TtsRequest::from_params(&json!({
            "text": "hello",
            "engine": "dia",
            "voice_identity_id": "does-not-exist",
        }))
        .unwrap();

        // Fails at `resolve_voice_identity`, still before `tts.client()` (and
        // so any real process spawn) would ever run -- same reasoning as
        // every other error case in this module's tests.
        let err = run(&db, &tts, std::path::Path::new("/tmp/out"), "job-1", req)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no longer exists"), "{err}");
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

    #[tokio::test]
    async fn resolve_voice_files_prefers_the_larger_voice_model_when_both_are_imported() {
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
        db.models()
            .insert(NewModel {
                name: "kokoro-v1.0.fp32".into(),
                format: "onnx".into(),
                file_path: "E:\\AI\\models\\voice\\kokoro-v1.0.onnx".into(),
                size_bytes: 325_505_369,
                source: "manual".into(),
                roles: vec!["voice_model".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();
        db.models()
            .insert(NewModel {
                name: "voices-v1.0".into(),
                format: "bin".into(),
                file_path: "E:\\AI\\models\\voice\\voices-v1.0.bin".into(),
                size_bytes: 28_214_398,
                source: "manual".into(),
                roles: vec!["voice_data".into()],
                ..NewModel::default()
            })
            .await
            .unwrap();

        let (model, _voices) = resolve_voice_files(&db).await.unwrap();
        assert_eq!(model.name, "kokoro-v1.0.fp32");
    }

    fn dia_engine_file_count() -> usize {
        dia_component_total("dia_engine")
    }

    fn dia_codec_file_count() -> usize {
        dia_component_total("dia_codec")
    }

    async fn insert_dia_files(db: &Database, role: &str, dir: &str, count: usize) {
        use crate::db::NewModel;

        for i in 0..count {
            db.models()
                .insert(NewModel {
                    name: format!("{role}-file-{i}"),
                    format: "json".into(),
                    file_path: format!("{dir}\\file-{i}.json"),
                    size_bytes: 100,
                    source: "manual".into(),
                    roles: vec![role.into()],
                    ..NewModel::default()
                })
                .await
                .unwrap();
        }
    }

    // Same reasoning as the Kokoro tests above -- every case here fails the
    // role-based resolution before `run` would ever call `tts.client()`.

    #[tokio::test]
    async fn run_dia_reports_a_clear_error_when_no_dia_engine_files_are_imported() {
        let db = Database::connect_in_memory().await.unwrap();
        let tts = TtsAdapter::new();
        let req = TtsRequest::from_params(&json!({ "text": "hello", "engine": "dia" })).unwrap();

        let err = run(&db, &tts, std::path::Path::new("/tmp/out"), "job-1", req)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("no Dia engine files imported"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn run_dia_reports_a_clear_error_when_the_engine_directory_is_incomplete() {
        let db = Database::connect_in_memory().await.unwrap();
        insert_dia_files(&db, "dia_engine", "E:\\AI\\models\\voice\\dia-engine", 1).await;
        let tts = TtsAdapter::new();
        let req = TtsRequest::from_params(&json!({ "text": "hello", "engine": "dia" })).unwrap();

        let err = run(&db, &tts, std::path::Path::new("/tmp/out"), "job-1", req)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("incomplete"), "{err}");
        assert!(
            err.to_string()
                .contains(&format!("1 of {}", dia_engine_file_count())),
            "{err}"
        );
    }

    #[tokio::test]
    async fn run_dia_reports_a_clear_error_when_only_the_engine_but_not_the_codec_is_imported() {
        let db = Database::connect_in_memory().await.unwrap();
        insert_dia_files(
            &db,
            "dia_engine",
            "E:\\AI\\models\\voice\\dia-engine",
            dia_engine_file_count(),
        )
        .await;
        let tts = TtsAdapter::new();
        let req = TtsRequest::from_params(&json!({ "text": "hello", "engine": "dia" })).unwrap();

        let err = run(&db, &tts, std::path::Path::new("/tmp/out"), "job-1", req)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Dia's audio codec"), "{err}");
    }

    #[tokio::test]
    async fn resolve_dia_dirs_finds_the_shared_directory_for_each_fully_imported_component() {
        let db = Database::connect_in_memory().await.unwrap();
        insert_dia_files(
            &db,
            "dia_engine",
            "E:\\AI\\models\\voice\\dia-engine",
            dia_engine_file_count(),
        )
        .await;
        insert_dia_files(
            &db,
            "dia_codec",
            "E:\\AI\\models\\voice\\dia-codec",
            dia_codec_file_count(),
        )
        .await;

        let (engine_dir, codec_dir) = resolve_dia_dirs(&db).await.unwrap();
        assert_eq!(
            engine_dir.to_string_lossy().replace('\\', "/"),
            "E:/AI/models/voice/dia-engine"
        );
        assert_eq!(
            codec_dir.to_string_lossy().replace('\\', "/"),
            "E:/AI/models/voice/dia-codec"
        );
    }
}
