//! System telemetry: GPU (NVML) plus RAM/CPU (sysinfo), sampled on a background
//! task and published on a `watch` channel.
//!
//! Degrades gracefully: with no NVIDIA driver (or in CI) [`GpuStatus`] becomes
//! `Unavailable { reason }` and host metrics keep flowing.

mod gpu;
mod host;

pub use gpu::{GpuInfo, GpuProcess, GpuStatus};
pub use host::HostStatus;

use std::time::Duration;

use serde::Serialize;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use gpu::GpuSource;
use host::{HostSource, SysinfoHost};

/// How often the sampler refreshes.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// One point-in-time reading of the whole machine.
#[derive(Debug, Clone, Serialize)]
pub struct SystemTelemetry {
    /// Milliseconds since the Unix epoch when this reading was taken.
    pub captured_at_ms: u64,
    pub gpu: GpuStatus,
    pub host: HostStatus,
}

impl SystemTelemetry {
    fn capture(gpu: &dyn GpuSource, host: &mut dyn HostSource) -> Self {
        Self {
            captured_at_ms: unix_millis(),
            gpu: gpu.sample(),
            host: host.sample(),
        }
    }
}

/// Owns the background sampling task and hands out `watch` receivers. Dropping
/// the sampler stops the task.
#[derive(Debug)]
pub struct Sampler {
    rx: watch::Receiver<SystemTelemetry>,
    task: JoinHandle<()>,
}

impl Sampler {
    /// Start sampling with the real NVML + sysinfo sources.
    pub fn spawn() -> Self {
        Self::spawn_with(
            SAMPLE_INTERVAL,
            gpu::real_source(),
            Box::new(SysinfoHost::new()),
        )
    }

    fn spawn_with(
        interval: Duration,
        gpu: Box<dyn GpuSource>,
        mut host: Box<dyn HostSource>,
    ) -> Self {
        let first = SystemTelemetry::capture(gpu.as_ref(), host.as_mut());
        let (tx, rx) = watch::channel(first);

        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticker.tick().await; // consume the immediate first tick
            loop {
                ticker.tick().await;
                if tx
                    .send(SystemTelemetry::capture(gpu.as_ref(), host.as_mut()))
                    .is_err()
                {
                    break; // no receivers left
                }
            }
        });

        Self { rx, task }
    }

    /// The most recent reading (always available).
    pub fn latest(&self) -> SystemTelemetry {
        self.rx.borrow().clone()
    }

    /// A receiver that resolves on every new reading.
    pub fn subscribe(&self) -> watch::Receiver<SystemTelemetry> {
        self.rx.clone()
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::{GpuSource, GpuStatus, HostSource, HostStatus, Sampler};
    use std::time::Duration;

    struct FakeGpu(GpuStatus);
    impl GpuSource for FakeGpu {
        fn sample(&self) -> GpuStatus {
            self.0.clone()
        }
    }

    struct CountingHost {
        calls: Arc<AtomicU32>,
    }
    impl HostSource for CountingHost {
        fn sample(&mut self) -> HostStatus {
            self.calls.fetch_add(1, Ordering::SeqCst);
            HostStatus {
                ram_total_mb: 32_000,
                ram_used_mb: 8_000,
                cpu_total_pct: 12,
                cpu_per_core_pct: vec![10; 16],
            }
        }
    }

    fn fake_sampler(interval_ms: u64, calls: Arc<AtomicU32>) -> Sampler {
        Sampler::spawn_with(
            Duration::from_millis(interval_ms),
            Box::new(FakeGpu(GpuStatus::Unavailable {
                reason: "test".into(),
            })),
            Box::new(CountingHost { calls }),
        )
    }

    #[tokio::test]
    async fn latest_is_available_immediately() {
        let calls = Arc::new(AtomicU32::new(0));
        let sampler = fake_sampler(10, calls.clone());

        let snap = sampler.latest();
        assert!(matches!(snap.gpu, GpuStatus::Unavailable { .. }));
        assert_eq!(snap.host.ram_total_mb, 32_000);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sampler_updates_on_tick() {
        let calls = Arc::new(AtomicU32::new(0));
        let sampler = fake_sampler(10, calls.clone());
        let mut rx = sampler.subscribe();

        rx.changed().await.unwrap();
        assert!(calls.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn task_stops_when_sampler_dropped() {
        let calls = Arc::new(AtomicU32::new(0));
        let sampler = fake_sampler(5, calls);
        let handle = sampler.task.abort_handle();

        drop(sampler);
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(handle.is_finished());
    }
}
