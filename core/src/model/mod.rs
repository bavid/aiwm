//! Model files: GGUF header inspection and importing into the canonical store.

pub mod catalog;
mod gguf;
mod import;
mod kind;

pub use catalog::{KnownModel, KNOWN_MODELS};
pub use gguf::{read_gguf_info, GgufInfo};
pub use import::{import_model, ImportOutcome, ImportRequest};
pub use kind::ModelKind;

/// Bytes per MiB, used for the VRAM/size estimates.
pub(crate) const MIB: u64 = 1024 * 1024;

/// A filesystem-safe, lowercase slug (letters, digits, single dashes).
pub(crate) fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let slug = out.trim_matches('-');
    let slug: String = slug.chars().take(60).collect();
    if slug.is_empty() {
        "model".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugify_cases() {
        assert_eq!(slugify("Qwen2 7B Instruct"), "qwen2-7b-instruct");
        assert_eq!(slugify("  Meta-Llama-3.1-8B  "), "meta-llama-3-1-8b");
        assert_eq!(slugify("!!!"), "model");
        assert_eq!(slugify(""), "model");
    }
}
