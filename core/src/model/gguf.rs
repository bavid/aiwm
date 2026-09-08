//! A bounded reader for GGUF headers — enough to identify a model without
//! loading the file or trusting a third-party parser.
//!
//! Format reference: <https://github.com/ggml-org/ggml/blob/master/docs/gguf.md>.
//! Every count and length is range-checked before use.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::{CoreError, Result};

const GGUF_MAGIC: u32 = 0x4655_4747; // "GGUF" little-endian
const MAX_KV: u64 = 100_000;
const MAX_TENSORS: u64 = 200_000;
const MAX_STRING_LEN: u64 = 64 * 1024;
const MAX_NDIMS: u32 = 8;
const MAX_ARRAY_ELEMS: u64 = 200_000_000;

/// What we learn from a GGUF header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GgufInfo {
    pub version: u32,
    pub architecture: Option<String>,
    pub name: Option<String>,
    pub quantization: Option<String>,
    pub context_length: Option<u64>,
    pub parameter_count: Option<u64>,
    /// Transformer layers (`<arch>.block_count`) — for the KV-cache estimate.
    pub block_count: Option<u64>,
    /// Hidden size (`<arch>.embedding_length`).
    pub embedding_length: Option<u64>,
    /// Attention heads (`<arch>.attention.head_count`).
    pub head_count: Option<u64>,
    /// KV heads (`<arch>.attention.head_count_kv`); `head_count` for plain MHA.
    pub head_count_kv: Option<u64>,
    pub tensor_count: u64,
    pub metadata_count: u64,
}

fn gguf_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("GGUF: {msg}"))
}

/// Read and identify the GGUF header at `path`.
pub fn read_gguf_info(path: &Path) -> Result<GgufInfo> {
    let file = File::open(path).map_err(|e| gguf_err(format!("open {}: {e}", path.display())))?;
    let mut r = Reader {
        inner: BufReader::new(file),
    };

    if r.u32()? != GGUF_MAGIC {
        return Err(gguf_err("not a GGUF file (bad magic)"));
    }
    let version = r.u32()?;
    if !(2..=3).contains(&version) {
        return Err(gguf_err(format!("unsupported version {version}")));
    }
    let tensor_count = r.u64()?;
    let kv_count = r.u64()?;
    if tensor_count > MAX_TENSORS || kv_count > MAX_KV {
        return Err(gguf_err("header counts are implausible"));
    }

    let mut info = GgufInfo {
        version,
        tensor_count,
        metadata_count: kv_count,
        ..GgufInfo::default()
    };
    let mut file_type: Option<u32> = None;

    for _ in 0..kv_count {
        let key = r.gguf_string()?;
        let value = r.read_value()?;
        match key.as_str() {
            "general.architecture" => info.architecture = value.into_string(),
            "general.name" => info.name = value.into_string(),
            "general.file_type" => file_type = value.as_u64().and_then(|v| u32::try_from(v).ok()),
            "general.parameter_count" => info.parameter_count = value.as_u64(),
            k if k.ends_with(".context_length") && info.context_length.is_none() => {
                info.context_length = value.as_u64();
            }
            k if k.ends_with(".block_count") && info.block_count.is_none() => {
                info.block_count = value.as_u64();
            }
            k if k.ends_with(".embedding_length") && info.embedding_length.is_none() => {
                info.embedding_length = value.as_u64();
            }
            k if k.ends_with(".attention.head_count") && info.head_count.is_none() => {
                info.head_count = value.as_u64();
            }
            k if k.ends_with(".attention.head_count_kv") && info.head_count_kv.is_none() => {
                info.head_count_kv = value.as_u64();
            }
            _ => {}
        }
    }

    info.quantization = file_type.map(ftype_name);

    // Exact parameter count from the tensor table when the metadata didn't say.
    if info.parameter_count.is_none() {
        let mut total: u64 = 0;
        for _ in 0..tensor_count {
            let _name = r.gguf_string()?;
            let n_dims = r.u32()?;
            if n_dims > MAX_NDIMS {
                return Err(gguf_err("tensor has too many dimensions"));
            }
            let mut elems: u64 = 1;
            for _ in 0..n_dims {
                elems = elems.saturating_mul(r.u64()?);
            }
            let _ggml_type = r.u32()?;
            let _offset = r.u64()?;
            total = total.saturating_add(elems);
        }
        if total > 0 {
            info.parameter_count = Some(total);
        }
    }

    Ok(info)
}

/// GGML file-type id -> a human label. The common quants are stable; newer ids
/// drift with llama.cpp, so unknowns fall through to `ftype-<n>`.
fn ftype_name(id: u32) -> String {
    let name = match id {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XS",
        23 => "IQ3_XXS",
        24 => "IQ1_S",
        25 => "IQ4_NL",
        26 => "IQ3_S",
        27 => "IQ3_M",
        28 => "IQ2_S",
        29 => "IQ2_M",
        30 => "IQ4_XS",
        31 => "IQ1_M",
        32 => "BF16",
        36 => "TQ1_0",
        37 => "TQ2_0",
        _ => return format!("ftype-{id}"),
    };
    name.to_string()
}

// --- reader ---------------------------------------------------------------

struct Reader {
    inner: BufReader<File>,
}

impl Reader {
    fn u8(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.inner.read_exact(&mut b).map_err(gguf_err)?;
        Ok(b[0])
    }

    fn u32(&mut self) -> Result<u32> {
        let mut b = [0u8; 4];
        self.inner.read_exact(&mut b).map_err(gguf_err)?;
        Ok(u32::from_le_bytes(b))
    }

    fn u64(&mut self) -> Result<u64> {
        let mut b = [0u8; 8];
        self.inner.read_exact(&mut b).map_err(gguf_err)?;
        Ok(u64::from_le_bytes(b))
    }

    fn skip(&mut self, n: u64) -> Result<()> {
        self.inner
            .seek(SeekFrom::Current(n as i64))
            .map_err(gguf_err)?;
        Ok(())
    }

    fn gguf_string(&mut self) -> Result<String> {
        let len = self.u64()?;
        if len > MAX_STRING_LEN {
            return Err(gguf_err("string too long"));
        }
        let mut buf = vec![0u8; len as usize];
        self.inner.read_exact(&mut buf).map_err(gguf_err)?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    /// Read a metadata value, materializing scalars/strings and *skipping* array
    /// contents (a tokenizer vocab can be many MB).
    fn read_value(&mut self) -> Result<Value> {
        let vtype = self.u32()?;
        self.read_typed(vtype)
    }

    fn read_typed(&mut self, vtype: u32) -> Result<Value> {
        Ok(match vtype {
            0 => Value::U(u64::from(self.u8()?)),
            1 => Value::I(i64::from(self.u8()? as i8)),
            2 => Value::U(u64::from(read_u16(self)?)),
            3 => Value::I(i64::from(read_u16(self)? as i16)),
            4 => Value::U(u64::from(self.u32()?)),
            5 => Value::I(i64::from(self.u32()? as i32)),
            6 => {
                self.skip(4)?;
                Value::Other
            }
            7 => Value::U(u64::from(self.u8()? != 0)),
            8 => Value::Str(self.gguf_string()?),
            9 => {
                let elem_type = self.u32()?;
                let count = self.u64()?;
                if count > MAX_ARRAY_ELEMS {
                    return Err(gguf_err("array too long"));
                }
                self.skip_array_elems(elem_type, count)?;
                Value::Other
            }
            10 => Value::U(self.u64()?),
            11 => Value::I(self.u64()? as i64),
            12 => {
                self.skip(8)?;
                Value::Other
            }
            other => return Err(gguf_err(format!("unknown value type {other}"))),
        })
    }

    fn skip_array_elems(&mut self, elem_type: u32, count: u64) -> Result<()> {
        let fixed = match elem_type {
            0 | 1 | 7 => Some(1u64),
            2 | 3 => Some(2),
            4..=6 => Some(4),
            10..=12 => Some(8),
            8 => None, // strings: variable
            9 => return Err(gguf_err("nested arrays are not supported")),
            other => return Err(gguf_err(format!("unknown array element type {other}"))),
        };
        match fixed {
            Some(size) => self.skip(size.saturating_mul(count))?,
            None => {
                for _ in 0..count {
                    let len = self.u64()?;
                    if len > MAX_STRING_LEN {
                        return Err(gguf_err("array string too long"));
                    }
                    self.skip(len)?;
                }
            }
        }
        Ok(())
    }
}

fn read_u16(r: &mut Reader) -> Result<u16> {
    Ok(u16::from(r.u8()?) | (u16::from(r.u8()?) << 8))
}

#[derive(Debug)]
enum Value {
    U(u64),
    I(i64),
    Str(String),
    /// A value we read past but don't use (floats, arrays).
    Other,
}

impl Value {
    fn as_u64(&self) -> Option<u64> {
        match self {
            Value::U(v) => Some(*v),
            Value::I(v) if *v >= 0 => Some(*v as u64),
            _ => None,
        }
    }
    fn into_string(self) -> Option<String> {
        match self {
            Value::Str(s) if !s.trim().is_empty() => Some(s),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// Minimal GGUF v3 writer for fixtures.
    struct Builder {
        buf: Vec<u8>,
        kv: u64,
        tensors: u64,
    }

    impl Builder {
        fn new() -> Self {
            Self {
                buf: Vec::new(),
                kv: 0,
                tensors: 0,
            }
        }
        fn gstr(&mut self, s: &str) {
            self.buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
            self.buf.extend_from_slice(s.as_bytes());
        }
        fn kv_str(&mut self, key: &str, val: &str) -> &mut Self {
            self.gstr(key);
            self.buf.extend_from_slice(&8u32.to_le_bytes());
            self.gstr(val);
            self.kv += 1;
            self
        }
        fn kv_u32(&mut self, key: &str, val: u32) -> &mut Self {
            self.gstr(key);
            self.buf.extend_from_slice(&4u32.to_le_bytes());
            self.buf.extend_from_slice(&val.to_le_bytes());
            self.kv += 1;
            self
        }
        fn kv_u64(&mut self, key: &str, val: u64) -> &mut Self {
            self.gstr(key);
            self.buf.extend_from_slice(&10u32.to_le_bytes());
            self.buf.extend_from_slice(&val.to_le_bytes());
            self.kv += 1;
            self
        }
        fn kv_str_array(&mut self, key: &str, items: &[&str]) -> &mut Self {
            self.gstr(key);
            self.buf.extend_from_slice(&9u32.to_le_bytes()); // ARRAY
            self.buf.extend_from_slice(&8u32.to_le_bytes()); // of STRING
            self.buf
                .extend_from_slice(&(items.len() as u64).to_le_bytes());
            for it in items {
                self.gstr(it);
            }
            self.kv += 1;
            self
        }
        fn tensor(&mut self, name: &str, dims: &[u64]) -> &mut Self {
            self.gstr(name);
            self.buf
                .extend_from_slice(&(dims.len() as u32).to_le_bytes());
            for d in dims {
                self.buf.extend_from_slice(&d.to_le_bytes());
            }
            self.buf.extend_from_slice(&0u32.to_le_bytes()); // ggml type F32
            self.buf.extend_from_slice(&0u64.to_le_bytes()); // offset
            self.tensors += 1;
            self
        }
        fn finish(self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
            out.extend_from_slice(&3u32.to_le_bytes());
            out.extend_from_slice(&self.tensors.to_le_bytes());
            out.extend_from_slice(&self.kv.to_le_bytes());
            out.extend_from_slice(&self.buf);
            out
        }
    }

    fn write_fixture(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn reads_metadata_and_derives_quant() {
        let mut b = Builder::new();
        b.kv_str("general.architecture", "qwen2")
            .kv_str("general.name", "Qwen2 7B Instruct")
            .kv_u32("general.file_type", 15) // Q4_K_M
            .kv_u64("qwen2.context_length", 32768)
            .kv_u32("qwen2.block_count", 28)
            .kv_u32("qwen2.embedding_length", 3584)
            .kv_u32("qwen2.attention.head_count", 28)
            .kv_u32("qwen2.attention.head_count_kv", 4)
            .kv_u64("general.parameter_count", 7_615_616_512)
            .kv_str_array("tokenizer.ggml.tokens", &["a", "bb", "ccc"]); // skipped
        let file = write_fixture(&b.finish());

        let info = read_gguf_info(file.path()).unwrap();
        assert_eq!(info.architecture.as_deref(), Some("qwen2"));
        assert_eq!(info.name.as_deref(), Some("Qwen2 7B Instruct"));
        assert_eq!(info.quantization.as_deref(), Some("Q4_K_M"));
        assert_eq!(info.context_length, Some(32768));
        assert_eq!(info.block_count, Some(28));
        assert_eq!(info.embedding_length, Some(3584));
        assert_eq!(info.head_count, Some(28));
        assert_eq!(info.head_count_kv, Some(4));
        assert_eq!(info.parameter_count, Some(7_615_616_512));
        assert_eq!(info.metadata_count, 10);
    }

    #[test]
    fn sums_parameter_count_from_tensors_when_absent() {
        let mut b = Builder::new();
        b.kv_str("general.architecture", "llama");
        b.tensor("token_embd.weight", &[4096, 32000])
            .tensor("blk.0.attn_q.weight", &[4096, 4096]);
        let file = write_fixture(&b.finish());

        let info = read_gguf_info(file.path()).unwrap();
        assert_eq!(info.parameter_count, Some(4096 * 32000 + 4096 * 4096));
        assert_eq!(info.tensor_count, 2);
    }

    #[test]
    fn rejects_non_gguf() {
        let file = write_fixture(b"not a gguf file at all");
        let err = read_gguf_info(file.path()).unwrap_err();
        assert!(err.to_string().contains("bad magic"));
    }

    #[test]
    fn rejects_implausible_counts() {
        let mut out = Vec::new();
        out.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&1u64.to_le_bytes());
        out.extend_from_slice(&u64::MAX.to_le_bytes()); // kv_count
        let file = write_fixture(&out);
        assert!(read_gguf_info(file.path())
            .unwrap_err()
            .to_string()
            .contains("implausible"));
    }

    #[test]
    fn truncated_file_errors_cleanly() {
        let mut b = Builder::new();
        b.kv_str("general.architecture", "llama");
        let mut bytes = b.finish();
        bytes.truncate(bytes.len() - 5);
        let file = write_fixture(&bytes);
        assert!(read_gguf_info(file.path()).is_err());
    }
}
