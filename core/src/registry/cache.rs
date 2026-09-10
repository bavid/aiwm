//! A disposable on-disk cache for remote index answers.
//!
//! One JSON file per key under `<data>/cache/registry/`. Entries never expire on
//! their own — [`Registry`](super::Registry) decides how old is too old from the
//! returned age. Every operation is best-effort: a read miss, a corrupt file or
//! an unwritable directory just means "no cache", never an error.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::runtime::download::hex;

#[derive(Debug)]
pub(super) struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub(super) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// The cached value and its age in seconds, or `None` on any miss.
    pub(super) fn get<T: DeserializeOwned>(&self, key: &str) -> Option<(T, u64)> {
        let bytes = std::fs::read(self.path(key)).ok()?;
        let env: EnvelopeDe<T> = serde_json::from_slice(&bytes).ok()?;
        let age = unix_now().saturating_sub(env.stored_at);
        Some((env.value, age))
    }

    /// Best-effort write-through. Failures are swallowed (logged at debug).
    pub(super) fn put<T: Serialize>(&self, key: &str, value: &T) {
        if let Err(e) = self.try_put(key, value) {
            tracing::debug!(%key, error = %e, "registry cache write failed");
        }
    }

    /// Number of cached `.json` entries — best-effort, `0` on any error.
    pub(super) fn count(&self) -> u64 {
        std::fs::read_dir(&self.dir)
            .map(|it| {
                it.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                    .count() as u64
            })
            .unwrap_or(0)
    }

    fn try_put<T: Serialize>(&self, key: &str, value: &T) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let env = EnvelopeSer {
            stored_at: unix_now(),
            value,
        };
        let json = serde_json::to_vec(&env)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // Write to a temp sibling then rename, so a reader never sees half a file.
        let tmp = self.path(&format!("{key}.tmp"));
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, self.path(key))
    }
}

#[derive(Serialize)]
struct EnvelopeSer<'a, T> {
    stored_at: u64,
    value: &'a T,
}

#[derive(serde::Deserialize)]
struct EnvelopeDe<T> {
    stored_at: u64,
    value: T,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A filesystem-safe key from its parts (SHA-256, hex — collision-free, and the
/// query text never lands in a path).
pub(super) fn key(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0]); // separator, so ["ab","c"] != ["a","bc"]
    }
    hex(&h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Item {
        n: u32,
    }

    #[test]
    fn key_is_stable_hex_and_path_safe() {
        let a = key(&["search", "qwen coder / gguf"]);
        assert_eq!(a, key(&["search", "qwen coder / gguf"]));
        assert_ne!(a, key(&["search", "qwen coder / ggug"]));
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn put_then_get_round_trips_with_a_small_age() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cache::new(dir.path().to_path_buf());
        c.put("k", &Item { n: 7 });

        let (got, age): (Item, u64) = c.get("k").unwrap();
        assert_eq!(got, Item { n: 7 });
        assert!(age < 5);
    }

    #[test]
    fn get_is_none_on_a_miss_or_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cache::new(dir.path().to_path_buf());
        assert!(c.get::<Item>("absent").is_none());

        c.put("bad", &Item { n: 1 });
        for e in std::fs::read_dir(dir.path()).unwrap() {
            std::fs::write(e.unwrap().path(), b"not json").unwrap();
        }
        assert!(c.get::<Item>("bad").is_none());
    }

    #[test]
    fn put_is_silent_when_the_dir_cannot_be_created() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, b"x").unwrap();
        let c = Cache::new(file.join("cache"));
        c.put("k", &Item { n: 1 });
        assert!(c.get::<Item>("k").is_none());
    }
}
