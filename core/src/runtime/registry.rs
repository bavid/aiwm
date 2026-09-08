//! A shared, mutable set of live [`RuntimeAdapter`]s, keyed by id. Shared by the
//! job engine and the scheduler.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use super::RuntimeAdapter;

#[derive(Clone, Default, Debug)]
pub struct RuntimeRegistry {
    adapters: Arc<RwLock<BTreeMap<String, Arc<dyn RuntimeAdapter>>>>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, adapter: Arc<dyn RuntimeAdapter>) {
        self.write().insert(adapter.id().to_string(), adapter);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn RuntimeAdapter>> {
        self.read().get(id).cloned()
    }

    pub fn all(&self) -> Vec<Arc<dyn RuntimeAdapter>> {
        self.read().values().cloned().collect()
    }

    /// Total VRAM (MB) in use across every registered runtime.
    pub fn total_vram_used_mb(&self) -> u64 {
        self.read().values().map(|a| a.vram_used_mb()).sum()
    }

    /// The runtime that currently has `model_id` loaded, if any.
    pub fn runtime_with_model(&self, model_id: &str) -> Option<Arc<dyn RuntimeAdapter>> {
        self.read()
            .values()
            .find(|a| a.loaded_models().iter().any(|m| m.model_id == model_id))
            .cloned()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, Arc<dyn RuntimeAdapter>>> {
        self.adapters.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<String, Arc<dyn RuntimeAdapter>>> {
        self.adapters
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{FakeRuntimeAdapter, RuntimeAdapter};

    #[tokio::test]
    async fn tracks_registered_adapters_and_vram() {
        let reg = RuntimeRegistry::new();
        let a = Arc::new(FakeRuntimeAdapter::healthy("llamacpp"));
        let b = Arc::new(FakeRuntimeAdapter::healthy("comfyui"));
        reg.register(a.clone());
        reg.register(b.clone());

        a.load_model("qwen", 9_000).await.unwrap();
        b.load_model("flux", 12_000).await.unwrap();

        assert_eq!(reg.total_vram_used_mb(), 21_000);
        assert_eq!(reg.all().len(), 2);
        assert_eq!(reg.runtime_with_model("flux").unwrap().id(), "comfyui");
        assert!(reg.runtime_with_model("absent").is_none());
    }
}
