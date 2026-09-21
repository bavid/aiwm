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

/// The parsed JSON header at `path` — every tensor entry plus `__metadata__`
/// — after the same bounds checks every reader here relies on.
fn read_header(path: &Path) -> Result<serde_json::Map<String, Value>> {
    Ok(open_header(path)?.0)
}

/// [`read_header`] plus the open file and the offset its data section starts
/// at (`8 + header length`), for the one reader that needs a tensor's bytes.
fn open_header(path: &Path) -> Result<(serde_json::Map<String, Value>, File, u64)> {
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
    match root {
        Value::Object(obj) => Ok((obj, file, header_len + 8)),
        _ => Err(st_err("header is not an object")),
    }
}

/// The names that mark a LoRA's down projection: `lora_down.weight` in the
/// kohya naming ai-toolkit writes for SDXL, `lora_A.weight` in the
/// diffusers/PEFT naming it writes for FLUX.
const DOWN_PROJECTION_MARKERS: [&str; 2] = ["lora_down", "lora_A"];

/// The first down projection in `header`: its name and its first dimension,
/// which is the rank — both namings are `[rank, in_features]`. `None` when
/// no tensor of at least two dimensions carries either marker.
fn first_down_projection(header: &serde_json::Map<String, Value>) -> Option<(&str, u32)> {
    header.iter().find_map(|(name, spec)| {
        if name == "__metadata__" || !DOWN_PROJECTION_MARKERS.iter().any(|m| name.contains(m)) {
            return None;
        }
        let dims = spec.get("shape")?.as_array()?;
        if dims.len() < 2 {
            return None;
        }
        let rank = dims.first()?.as_u64().and_then(|d| u32::try_from(d).ok())?;
        Some((name.as_str(), rank))
    })
}

/// The LoRA rank recorded in a `.safetensors` header — see
/// [`first_down_projection`]. `Ok(None)` means the file is not a LoRA.
/// Reading the header at all can still fail (missing or malformed file),
/// which is an error rather than "no rank".
pub fn lora_rank_from_header(path: &Path) -> Result<Option<u32>> {
    let header = read_header(path)?;
    Ok(first_down_projection(&header).map(|(_, rank)| rank))
}

/// The alpha a LoRA was trained with, when the file records one: the
/// `<module>.alpha` scalar next to the first down projection (kohya format),
/// else kohya's `ss_network_alpha` metadata. `Ok(None)` when neither is
/// there — a PEFT-format file, which ai-toolkit loads with alpha equal to
/// the rank (`lora_special.py`, `peft_format`) — or when the file is not a
/// LoRA at all. A recorded scalar whose bytes cannot be read is an error.
pub fn lora_alpha_from_header(path: &Path) -> Result<Option<f32>> {
    let (header, mut file, data_start) = open_header(path)?;
    let Some((down_name, _)) = first_down_projection(&header) else {
        return Ok(None);
    };
    let module = DOWN_PROJECTION_MARKERS
        .iter()
        .find_map(|m| down_name.find(m).map(|at| &down_name[..at]));
    let alpha_spec = module
        .map(|prefix| format!("{prefix}alpha"))
        .and_then(|key| header.get(&key));
    if let Some(spec) = alpha_spec {
        return read_scalar(&mut file, data_start, spec).map(Some);
    }
    let from_metadata = header
        .get("__metadata__")
        .and_then(|m| m.get("ss_network_alpha"))
        .and_then(Value::as_str)
        .and_then(|s| s.trim().parse::<f32>().ok());
    Ok(from_metadata)
}

/// Read a one-element float tensor described by `spec` from the data
/// section that starts at `data_start`.
fn read_scalar(file: &mut File, data_start: u64, spec: &Value) -> Result<f32> {
    let dtype = spec.get("dtype").and_then(Value::as_str).unwrap_or("");
    let width = match dtype {
        "F16" | "BF16" => 2,
        "F32" => 4,
        "F64" => 8,
        other => return Err(st_err(format!("alpha has non-float dtype {other:?}"))),
    };
    let offsets = spec
        .get("data_offsets")
        .and_then(Value::as_array)
        .ok_or_else(|| st_err("alpha has no data_offsets"))?;
    let (Some(begin), Some(end)) = (
        offsets.first().and_then(Value::as_u64),
        offsets.get(1).and_then(Value::as_u64),
    ) else {
        return Err(st_err("alpha has malformed data_offsets"));
    };
    if end.saturating_sub(begin) != width {
        return Err(st_err(format!(
            "alpha is not a single {dtype} value ({} bytes)",
            end.saturating_sub(begin)
        )));
    }
    file.seek(SeekFrom::Start(data_start + begin))
        .map_err(|e| st_err(format!("seek to alpha: {e}")))?;
    let mut raw = [0u8; 8];
    file.read_exact(&mut raw[..width as usize])
        .map_err(|_| st_err("alpha bytes are missing from the file"))?;
    Ok(match dtype {
        "F16" => f16_to_f32(u16::from_le_bytes([raw[0], raw[1]])),
        "BF16" => f32::from_bits(u32::from(u16::from_le_bytes([raw[0], raw[1]])) << 16),
        "F32" => f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
        // Checked above; an alpha is a small integer-valued float, so the
        // narrowing is exact for any real LoRA.
        _ => f64::from_le_bytes(raw) as f32,
    })
}

/// IEEE 754 binary16 → binary32, by the usual bit route (no `half` crate).
fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let mantissa = u32::from(bits & 0x3ff);
    let magnitude = match exponent {
        // Zero or subnormal: value = mantissa * 2^-24.
        0 => (mantissa as f32) * 2f32.powi(-24),
        // Infinity or NaN.
        0x1f => {
            return f32::from_bits(sign | 0x7f80_0000 | (mantissa << 13));
        }
        e => f32::from_bits(((e + 112) << 23) | (mantissa << 13)),
    };
    if sign == 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// Read and identify the `.safetensors` header at `path`.
pub fn read_safetensors_info(path: &Path) -> Result<SafetensorsInfo> {
    let obj = read_header(path)?;

    let mut info = SafetensorsInfo::default();
    let mut per_dtype: BTreeMap<String, u64> = BTreeMap::new();

    for (name, spec) in &obj {
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

/// Every tensor's name and shape plus the `__metadata__` strings — what
/// family detection ([`crate::model::family`]) looks at. Header only: the
/// tensor data is never read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SafetensorsHeader {
    /// Tensor name -> shape (dimensions that are not unsigned integers are
    /// dropped, leaving a shorter shape rather than a guessed one).
    pub tensors: BTreeMap<String, Vec<u64>>,
    pub metadata: BTreeMap<String, String>,
}

/// Read the tensor names and shapes of the `.safetensors` file at `path`.
pub fn read_safetensors_header(path: &Path) -> Result<SafetensorsHeader> {
    let obj = read_header(path)?;
    let mut out = SafetensorsHeader::default();
    for (name, spec) in obj {
        if name == "__metadata__" {
            if let Value::Object(meta) = spec {
                out.metadata.extend(
                    meta.into_iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))),
                );
            }
            continue;
        }
        let shape = spec
            .get("shape")
            .and_then(Value::as_array)
            .map(|dims| dims.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        out.tensors.insert(name, shape);
    }
    Ok(out)
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
    fn header_lists_every_tensor_shape_and_the_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let header = serde_json::json!({
            "__metadata__": { "format": "pt" },
            "head.modulation": { "dtype": "F32", "shape": [1, 2, 3072], "data_offsets": [0, 24576] },
            "scalar.alpha": { "dtype": "F32", "shape": [], "data_offsets": [24576, 24580] }
        });
        let p = write_st(dir.path(), "wan.safetensors", &header, 24580);

        let h = read_safetensors_header(&p).unwrap();
        assert_eq!(h.tensors.len(), 2);
        assert_eq!(h.tensors.get("head.modulation"), Some(&vec![1, 2, 3072]));
        assert_eq!(h.tensors.get("scalar.alpha"), Some(&vec![]));
        assert_eq!(h.metadata.get("format").map(String::as_str), Some("pt"));
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
    fn lora_rank_is_the_down_projections_first_dim() {
        let dir = tempfile::tempdir().unwrap();
        // The kohya naming ai-toolkit writes for SDXL: `lora_down` is
        // `[rank, in]`, `lora_up` is `[out, rank]`. A 1-D alpha scalar and
        // the metadata block must not be mistaken for a projection.
        let header = serde_json::json!({
            "__metadata__": { "ss_network_dim": "16" },
            "lora_unet_down_blocks_0_attentions_0_proj_in.alpha": {
                "dtype": "F16", "shape": [], "data_offsets": [0, 2]
            },
            "lora_unet_down_blocks_0_attentions_0_proj_in.lora_up.weight": {
                "dtype": "F16", "shape": [320, 16], "data_offsets": [2, 10242]
            },
            "lora_unet_down_blocks_0_attentions_0_proj_in.lora_down.weight": {
                "dtype": "F16", "shape": [16, 320], "data_offsets": [10242, 20482]
            }
        });
        let p = write_st(dir.path(), "sdxl_lora.safetensors", &header, 20482);
        assert_eq!(lora_rank_from_header(&p).unwrap(), Some(16));

        // The diffusers/PEFT naming ai-toolkit writes for FLUX: `lora_A` is
        // the down projection, again `[rank, in]`.
        let header = serde_json::json!({
            "transformer.single_transformer_blocks.0.attn.to_q.lora_B.weight": {
                "dtype": "BF16", "shape": [3072, 32], "data_offsets": [0, 196608]
            },
            "transformer.single_transformer_blocks.0.attn.to_q.lora_A.weight": {
                "dtype": "BF16", "shape": [32, 3072], "data_offsets": [196608, 393216]
            }
        });
        let p = write_st(dir.path(), "flux_lora.safetensors", &header, 393216);
        assert_eq!(lora_rank_from_header(&p).unwrap(), Some(32));
    }

    /// A kohya-style LoRA file: one module's `lora_down`/`lora_up` pair plus,
    /// optionally, its `.alpha` scalar with the given dtype and raw bytes in
    /// the data section, and optional `__metadata__` entries.
    fn write_kohya_lora(
        dir: &Path,
        name: &str,
        alpha: Option<(&str, Vec<u8>)>,
        metadata: Option<Value>,
    ) -> std::path::PathBuf {
        let module = "lora_unet_down_blocks_0_attentions_0_proj_in";
        let mut header = serde_json::json!({
            format!("{module}.lora_down.weight"): {
                "dtype": "F16", "shape": [16, 320], "data_offsets": [0, 10240]
            },
            format!("{module}.lora_up.weight"): {
                "dtype": "F16", "shape": [320, 16], "data_offsets": [10240, 20480]
            }
        });
        let mut data = vec![0u8; 20480];
        if let Some((dtype, bytes)) = alpha {
            header[format!("{module}.alpha")] = serde_json::json!({
                "dtype": dtype, "shape": [], "data_offsets": [20480, 20480 + bytes.len()]
            });
            data.extend_from_slice(&bytes);
        }
        if let Some(meta) = metadata {
            header["__metadata__"] = meta;
        }
        let json = serde_json::to_vec(&header).unwrap();
        let mut bytes = (json.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(&json);
        bytes.extend_from_slice(&data);
        let p = dir.join(name);
        File::create(&p).unwrap().write_all(&bytes).unwrap();
        p
    }

    #[test]
    fn lora_alpha_is_read_from_the_modules_alpha_scalar_in_any_float_dtype() {
        let dir = tempfile::tempdir().unwrap();

        // F32 alpha == rank.
        let p = write_kohya_lora(
            dir.path(),
            "f32.safetensors",
            Some(("F32", 16.0f32.to_le_bytes().to_vec())),
            None,
        );
        assert_eq!(lora_alpha_from_header(&p).unwrap(), Some(16.0));

        // F16 alpha 8.0 is 0x4800.
        let p = write_kohya_lora(
            dir.path(),
            "f16.safetensors",
            Some(("F16", 0x4800u16.to_le_bytes().to_vec())),
            None,
        );
        assert_eq!(lora_alpha_from_header(&p).unwrap(), Some(8.0));

        // BF16 alpha 8.0 is the top half of the F32 bits: 0x4100.
        let p = write_kohya_lora(
            dir.path(),
            "bf16.safetensors",
            Some(("BF16", 0x4100u16.to_le_bytes().to_vec())),
            None,
        );
        assert_eq!(lora_alpha_from_header(&p).unwrap(), Some(8.0));

        // F64 too, since nothing forbids it.
        let p = write_kohya_lora(
            dir.path(),
            "f64.safetensors",
            Some(("F64", 4.0f64.to_le_bytes().to_vec())),
            None,
        );
        assert_eq!(lora_alpha_from_header(&p).unwrap(), Some(4.0));
    }

    #[test]
    fn lora_alpha_falls_back_to_the_kohya_metadata_and_is_none_for_peft_files() {
        let dir = tempfile::tempdir().unwrap();

        // No `.alpha` tensor, but kohya's `ss_network_alpha` in the metadata.
        let p = write_kohya_lora(
            dir.path(),
            "meta.safetensors",
            None,
            Some(serde_json::json!({ "ss_network_alpha": "8.0", "ss_network_dim": "16" })),
        );
        assert_eq!(lora_alpha_from_header(&p).unwrap(), Some(8.0));

        // The tensor wins over the metadata when both are there.
        let p = write_kohya_lora(
            dir.path(),
            "both.safetensors",
            Some(("F32", 16.0f32.to_le_bytes().to_vec())),
            Some(serde_json::json!({ "ss_network_alpha": "8.0" })),
        );
        assert_eq!(lora_alpha_from_header(&p).unwrap(), Some(16.0));

        // Neither — a PEFT-format file (what ai-toolkit writes for FLUX):
        // alpha is defined to equal the rank, and nothing is recorded.
        let header = serde_json::json!({
            "transformer.blocks.0.attn.to_q.lora_A.weight": {
                "dtype": "BF16", "shape": [32, 3072], "data_offsets": [0, 8]
            },
            "transformer.blocks.0.attn.to_q.lora_B.weight": {
                "dtype": "BF16", "shape": [3072, 32], "data_offsets": [8, 16]
            }
        });
        let p = write_st(dir.path(), "peft.safetensors", &header, 16);
        assert_eq!(lora_alpha_from_header(&p).unwrap(), None);

        // Not a LoRA at all: no rank tensor, so no alpha either.
        let header = serde_json::json!({
            "w": { "dtype": "F32", "shape": [4, 4], "data_offsets": [0, 64] }
        });
        let p = write_st(dir.path(), "plain.safetensors", &header, 64);
        assert_eq!(lora_alpha_from_header(&p).unwrap(), None);

        // An alpha tensor whose bytes lie outside the file is an error.
        let p = write_kohya_lora(
            dir.path(),
            "truncated.safetensors",
            Some(("F32", Vec::new())),
            None,
        );
        assert!(lora_alpha_from_header(&p).is_err());
    }

    #[test]
    fn a_file_without_lora_projections_has_no_rank() {
        let dir = tempfile::tempdir().unwrap();
        // A full checkpoint: weights, but nothing shaped like an adapter.
        let header = serde_json::json!({
            "double_blocks.0.img_attn.qkv.weight": {
                "dtype": "BF16", "shape": [9216, 3072], "data_offsets": [0, 56_623_104]
            }
        });
        let p = write_st(dir.path(), "base.safetensors", &header, 64);
        assert_eq!(lora_rank_from_header(&p).unwrap(), None);

        // A 1-D tensor with a LoRA-ish name is not a projection either.
        let header = serde_json::json!({
            "x.lora_down.bias": { "dtype": "F16", "shape": [16], "data_offsets": [0, 32] }
        });
        let p = write_st(dir.path(), "odd.safetensors", &header, 32);
        assert_eq!(lora_rank_from_header(&p).unwrap(), None);

        // And an unreadable file is an error, not "no rank".
        assert!(lora_rank_from_header(&dir.path().join("missing.safetensors")).is_err());
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
