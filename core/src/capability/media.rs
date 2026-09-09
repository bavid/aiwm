//! Helpers shared by the ComfyUI media capabilities ([`image`](super::image),
//! [`video`](super::video)): parameter parsing, seeds, and writing the finished
//! file into the outputs directory.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{CoreError, Result};

/// JSON stays lossless below 2^53, so random seeds are drawn from that range —
/// the UI can show and re-submit them without precision loss.
pub(super) const SEED_CEILING: u64 = 1 << 53;

pub(super) fn comfy_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "comfyui".into(),
        message: msg.to_string(),
    }
}

/// The bare file name as ComfyUI sees it in its model folders.
pub(super) fn file_name(path: &str) -> Result<&str> {
    Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| comfy_err("the model file has no name"))
}

/// A trimmed string param, or `default` when it is absent / blank.
pub(super) fn str_param(params: &Value, key: &str, default: &str) -> String {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
        .to_string()
}

/// Round `v` to the nearest multiple of `multiple`.
pub(super) fn round_to(v: u64, multiple: u32) -> u32 {
    let m = u64::from(multiple);
    let rounded = ((v + m / 2) / m).saturating_mul(m);
    u32::try_from(rounded).unwrap_or(u32::MAX)
}

/// A non-crypto random seed: hash the current time with a process-random keyed
/// hasher. Good enough here — seeds only need to differ per call.
pub(super) fn random_seed() -> i64 {
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    if let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) {
        h.write_u128(d.as_nanos());
    }
    i64::try_from(h.finish() % SEED_CEILING).unwrap_or(0)
}

/// Read `params["seed"]` when it is a non-negative integer; otherwise a fresh
/// random seed.
pub(super) fn resolve_seed(params: &Value) -> i64 {
    params
        .get("seed")
        .and_then(Value::as_i64)
        .filter(|s| *s >= 0)
        .unwrap_or_else(random_seed)
}

/// Write a finished render to `<outputs_dir>/<job_id>.<ext>`.
pub(super) async fn write_output(
    outputs_dir: &Path,
    job_id: &str,
    ext: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    tokio::fs::create_dir_all(outputs_dir)
        .await
        .map_err(|e| comfy_err(format!("create {}: {e}", outputs_dir.display())))?;
    let path = outputs_dir.join(format!("{job_id}.{ext}"));
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| comfy_err(format!("write {}: {e}", path.display())))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_to_snaps_to_the_nearest_multiple() {
        assert_eq!(round_to(900, 64), 896);
        assert_eq!(round_to(833, 8), 832);
        assert_eq!(round_to(0, 64), 0);
    }

    #[test]
    fn resolve_seed_keeps_explicit_and_randomizes_otherwise() {
        assert_eq!(resolve_seed(&serde_json::json!({ "seed": 123 })), 123);
        assert!(resolve_seed(&serde_json::json!({ "seed": -1 })) >= 0);
        assert!(resolve_seed(&serde_json::json!({})) >= 0);
    }

    #[test]
    fn random_seeds_differ() {
        assert_ne!(random_seed(), random_seed());
    }
}
