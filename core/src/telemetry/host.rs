//! RAM and CPU telemetry via sysinfo.

use serde::Serialize;
use sysinfo::{MemoryRefreshKind, RefreshKind, System};

const BYTES_PER_MB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct HostStatus {
    pub ram_total_mb: u64,
    pub ram_used_mb: u64,
    pub cpu_total_pct: u8,
    pub cpu_per_core_pct: Vec<u8>,
}

pub(super) trait HostSource: Send {
    /// Refresh and read. The first call after construction reports 0% CPU
    /// (usage needs two samples); subsequent calls are accurate.
    fn sample(&mut self) -> HostStatus;
}

pub(super) struct SysinfoHost {
    sys: System,
}

impl SysinfoHost {
    pub(super) fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_memory(MemoryRefreshKind::everything())
                .with_cpu(sysinfo::CpuRefreshKind::nothing().with_cpu_usage()),
        );
        Self { sys }
    }
}

impl HostSource for SysinfoHost {
    fn sample(&mut self) -> HostStatus {
        self.sys
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        self.sys.refresh_cpu_usage();

        let per_core: Vec<u8> = self
            .sys
            .cpus()
            .iter()
            .map(|c| c.cpu_usage().round().clamp(0.0, 100.0) as u8)
            .collect();

        HostStatus {
            ram_total_mb: self.sys.total_memory() / BYTES_PER_MB,
            ram_used_mb: self.sys.used_memory() / BYTES_PER_MB,
            cpu_total_pct: self.sys.global_cpu_usage().round().clamp(0.0, 100.0) as u8,
            cpu_per_core_pct: per_core,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_plausible_memory() {
        let mut host = SysinfoHost::new();
        let a = host.sample();
        assert!(a.ram_total_mb > 0, "total RAM should be non-zero");
        assert!(a.ram_used_mb <= a.ram_total_mb);

        // Two more samples so CPU usage has a delta to work with.
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        let b = host.sample();
        assert_eq!(b.ram_total_mb, a.ram_total_mb);
        assert!(!b.cpu_per_core_pct.is_empty());
        assert!(b.cpu_total_pct <= 100);
    }
}
