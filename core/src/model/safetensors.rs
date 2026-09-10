//! A bounded reader for `.safetensors` headers — enough to know a model's
//! parameter count and weight precision without loading it or trusting a
//! third-party parser.
//!
//! Format: an 8-byte little-endian `u64` header length `N`, then `N` bytes of
//! UTF-8 JSON — `{ "<tensor>": { "dtype", "shape", "data_offsets" }, …,
//! "__metadata__": { … } }` — then the raw tensor bytes (never read here). The
//! header length is range-checked before the read.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use crate::{CoreError, Result};

/// Real headers are KB to a few MB; anything past this is not a model we care
/// to inspect.
const MAX_HEADER_BYTES: u64 = 64 * 1024 * 1024;

/// What we learn from a `.safetensors` header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SafetensorsInfo {
    /// Sum of element counts over every real tensor.
    pub parameter_count: Option<u64>,
    /// The dtype that holds the most parameters, normalised (`F16`, `BF16`,
    /// `FP8`, `F32`, `I8`).
    pub precision: Option<String>,
    /// String entries from `__metadata__` (e.g. `modelspec.architecture`).
    pub metadata: BTreeMap<String, String>,
}

fn st_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("safetensors: {msg}"))
}

/// Read and identify the `.safetensors` header at `path`.
pub fn read_safetensors_info(path: &Path) -> Result<SafetensorsInfo> {
    let mut file = File::open(path).map_err(|e| st_err(format!("open {}: {e}", path.display())))?;

    let file_len = file
        .seek(SeekFrom::End(0))
        .map_err(|e| st_err(format!("stat: {e}")))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| st_err(format!("seek: {e}")))?;

    let mut len_buf = [0u8; 8];
    file.read_exact(&mut len_buf)
        .map_err(|_| st_err("file is shorter than an 8-byte header length"))?;
    let header_len = u64::from_le_bytes(len_buf);
    if header_len == 0 || header_len > MAX_HEADER_BYTES || header_len + 8 > file_len {
        return Err(st_err(format!("implausible header length {header_len}")));
    }

    let mut json = vec![0u8; header_len as usize];
    file.read_exact(&mut json)
        .map_err(|_| st_err("header is truncated"))?;
    let root: Value =
        serde_json::from_slice(&json).map_err(|e| st_err(format!("header is not JSON: {e}")))?;
    let obj = root
        .as_object()
        .ok_or_else(|| st_err("header is not an object"))?;

    let mut info = SafetensorsInfo::default();
    let mut per_dtype: BTreeMap<String, u64> = BTreeMap::new();

    for (name, spec) in obj {
        if name == "__metadata__" {
            if let Some(meta) = spec.as_object() {
                for (k, v) in meta {
                    if let Some(s) = v.as_str() {
                        info.metadata.insert(k.clone(), s.to_string());
                    }
                }
            }
            continue;
        }
        let Some(spec) = spec.as_object() else {
            continue;
        };
        let dtype = spec.get("dtype").and_then(Value::as_str).unwrap_or("");
        let elems: u64 = spec
            .get("shape")
            .and_then(Value::as_array)
            .map(|dims| {
                dims.iter()
                    .filter_map(Value::as_u64)
                    .product::<u64>()
                    .max(if dims.is_empty() { 1 } else { 0 })
            })
            .unwrap_or(0);
        if elems > 0 && !dtype.is_empty() {
            *per_dtype.entry(normalise_dtype(dtype)).or_default() += elems;
        }
    }

    let total: u64 = per_dtype.values().sum();
    if total > 0 {
        info.parameter_count = Some(total);
        info.precision = per_dtype
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(d, _)| d);
    }
    Ok(info)
}

/// Collapse the fp8 variants; leave the rest as the header spelled them.
fn normalise_dtype(d: &str) -> String {
    let u = d.to_ascii_uppercase();
    if u.starts_with("F8") || u.starts_with("FP8") {
        "FP8".to_string()
    } else {
        u
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal `.safetensors`: 8-byte length + JSON header + `pad` zero
    /// bytes of "tensor data".
    fn write_st(dir: &Path, name: &str, header: &Value, pad: usize) -> std::path::PathBuf {
        let json = serde_json::to_vec(header).unwrap();
        let mut bytes = (json.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(&json);
        bytes.extend(std::iter::repeat_n(0u8, pad));
        let p = dir.join(name);
        File::create(&p).unwrap().write_all(&bytes).unwrap();
        p
    }

    #[test]
    fn reads_param_count_and_the_dominant_precision() {
        let dir = tempfile::tempdir().unwrap();
        let header = serde_json::json!({
            "__metadata__": { "format": "pt", "modelspec.architecture": "flux-1-dev" },
            "double_blocks.0.img_attn.qkv.weight": {
                "dtype": "BF16", "shape": [9216, 3072], "data_offsets": [0, 56_623_104]
            },
            "final_layer.linear.weight": {
                "dtype": "F32", "shape": [64, 3072], "data_offsets": [56_623_104, 57_409_536]
            }
        });
        let p = write_st(dir.path(), "flux.safetensors", &header, 64);

        let info = read_safetensors_info(&p).unwrap();
        assert_eq!(info.parameter_count, Some(9216 * 3072 + 64 * 3072));
        assert_eq!(info.precision.as_deref(), Some("BF16")); // holds the most params
        assert_eq!(
            info.metadata
                .get("modelspec.architecture")
                .map(String::as_str),
            Some("flux-1-dev")
        );
    }

    #[test]
    fn fp8_variants_collapse_to_fp8() {
        let dir = tempfile::tempdir().unwrap();
        let header = serde_json::json!({
            "w": { "dtype": "F8_E4M3", "shape": [1024, 1024], "data_offsets": [0, 1_048_576] }
        });
        let p = write_st(dir.path(), "t.safetensors", &header, 8);
        assert_eq!(
            read_safetensors_info(&p).unwrap().precision.as_deref(),
            Some("FP8")
        );
    }

    #[test]
    fn a_bogus_header_length_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.safetensors");
        // Claims a 1 TB header in a 16-byte file.
        let mut f = File::create(&p).unwrap();
        f.write_all(&1_000_000_000_000u64.to_le_bytes()).unwrap();
        f.write_all(b"12345678").unwrap();
        assert!(read_safetensors_info(&p).is_err());
    }

    #[test]
    fn not_json_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.safetensors");
        let mut f = File::create(&p).unwrap();
        f.write_all(&4u64.to_le_bytes()).unwrap();
        f.write_all(b"junk").unwrap();
        assert!(read_safetensors_info(&p).is_err());
    }
}
