//! An in-memory [`RuntimeAdapter`] with no real process, for exercising the
//! scheduler and job engine.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;

use super::{Health, LoadedModel, RuntimeAdapter, RuntimeKind, SpawnSpec};
use crate::{CoreError, Result};

/// Knobs for [`FakeRuntimeAdapter`] behaviour in tests.
#[derive(Debug, Clone)]
pub struct FakeConfig {
    pub id: String,
    pub health: Health,
    /// Artificial delay before a `load_model` resolves.
    pub load_delay: Duration,
    /// Model ids for which `load_model` returns an error.
    pub fail_load_for: Vec<String>,
    pub spawn_spec: Option<SpawnSpec>,
}

impl Default for FakeConfig {
    fn default() -> Self {
        Self {
            id: "fake".to_string(),
            health: Health::Healthy,
            load_delay: Duration::ZERO,
            fail_load_for: Vec::new(),
            spawn_spec: None,
        }
    }
}

#[derive(Debug)]
pub struct FakeRuntimeAdapter {
    config: FakeConfig,
    loaded: Mutex<BTreeMap<String, u64>>,
    health_calls: AtomicU32,
    load_calls: AtomicU32,
    unload_calls: AtomicU32,
}

impl FakeRuntimeAdapter {
    pub fn new(config: FakeConfig) -> Self {
        Self {
            config,
            loaded: Mutex::new(BTreeMap::new()),
            health_calls: AtomicU32::new(0),
            load_calls: AtomicU32::new(0),
            unload_calls: AtomicU32::new(0),
        }
    }

    /// A healthy adapter with the given id and default behaviour.
    pub fn healthy(id: &str) -> Self {
        Self::new(FakeConfig {
            id: id.to_string(),
            ..FakeConfig::default()
        })
    }

    pub fn health_call_count(&self) -> u32 {
        self.health_calls.load(Ordering::SeqCst)
    }

    pub fn load_call_count(&self) -> u32 {
        self.load_calls.load(Ordering::SeqCst)
    }

    /// Every `unload_model` call, including ones for a model that was never
    /// loaded — so a test can tell "asked to release" from "had nothing".
    pub fn unload_call_count(&self) -> u32 {
        self.unload_calls.load(Ordering::SeqCst)
    }

    fn loaded(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, u64>> {
        self.loaded.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl RuntimeAdapter for FakeRuntimeAdapter {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Fake
    }

    fn spawn_spec(&self) -> Option<SpawnSpec> {
        self.config.spawn_spec.clone()
    }

    async fn health(&self) -> Health {
        self.health_calls.fetch_add(1, Ordering::SeqCst);
        self.config.health
    }

    async fn load_model(&self, model_id: &str, vram_mb: u64) -> Result<()> {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        if !self.config.load_delay.is_zero() {
            tokio::time::sleep(self.config.load_delay).await;
        }
        if self.config.fail_load_for.iter().any(|m| m == model_id) {
            return Err(CoreError::Runtime {
                runtime: self.config.id.clone(),
                message: format!("simulated load failure for {model_id}"),
            });
        }
        self.loaded().insert(model_id.to_string(), vram_mb);
        Ok(())
    }

    async fn unload_model(&self, model_id: &str) -> Result<()> {
        self.unload_calls.fetch_add(1, Ordering::SeqCst);
        self.loaded().remove(model_id);
        Ok(())
    }

    fn loaded_models(&self) -> Vec<LoadedModel> {
        self.loaded()
            .iter()
            .map(|(model_id, &vram_mb)| LoadedModel {
                model_id: model_id.clone(),
                vram_mb,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_and_unload_track_vram() {
        let fake = FakeRuntimeAdapter::healthy("llamacpp");
        assert_eq!(fake.vram_used_mb(), 0);

        fake.load_model("qwen-14b", 9_000).await.unwrap();
        fake.load_model("upscaler", 1_500).await.unwrap();
        assert_eq!(fake.vram_used_mb(), 10_500);
        let ids: Vec<_> = fake
            .loaded_models()
            .into_iter()
            .map(|m| m.model_id)
            .collect();
        assert_eq!(ids, ["qwen-14b", "upscaler"]);

        fake.unload_model("qwen-14b").await.unwrap();
        assert_eq!(fake.vram_used_mb(), 1_500);
    }

    #[tokio::test]
    async fn unload_of_absent_model_is_a_noop() {
        let fake = FakeRuntimeAdapter::healthy("x");
        fake.unload_model("never-loaded").await.unwrap();
        assert!(fake.loaded_models().is_empty());
    }

    #[tokio::test]
    async fn configured_load_failures_propagate() {
        let fake = FakeRuntimeAdapter::new(FakeConfig {
            fail_load_for: vec!["broken".to_string()],
            ..FakeConfig::default()
        });

        let err = fake.load_model("broken", 100).await.unwrap_err();
        assert!(matches!(err, CoreError::Runtime { .. }));
        assert_eq!(fake.vram_used_mb(), 0);

        fake.load_model("fine", 100).await.unwrap();
        assert_eq!(fake.vram_used_mb(), 100);
    }

    #[tokio::test]
    async fn health_reports_configured_value_and_counts_calls() {
        let fake = FakeRuntimeAdapter::new(FakeConfig {
            health: Health::Unhealthy,
            ..FakeConfig::default()
        });
        assert_eq!(fake.health().await, Health::Unhealthy);
        assert_eq!(fake.health().await, Health::Unhealthy);
        assert_eq!(fake.health_call_count(), 2);
    }
}
