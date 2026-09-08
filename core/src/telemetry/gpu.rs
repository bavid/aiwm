//! GPU telemetry via NVML, with a no-op fallback when NVML is unavailable.

use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::enums::device::UsedGpuMemory;
use nvml_wrapper::Nvml;
use serde::Serialize;

const BYTES_PER_MB: u64 = 1024 * 1024;
/// Cap on how many GPU processes we report (top VRAM consumers).
const MAX_PROCESSES: usize = 16;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GpuStatus {
    Available(GpuInfo),
    Unavailable { reason: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub vram_total_mb: u64,
    pub vram_used_mb: u64,
    pub vram_free_mb: u64,
    pub utilization_pct: u8,
    pub temperature_c: u8,
    pub processes: Vec<GpuProcess>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuProcess {
    pub pid: u32,
    pub vram_mb: u64,
}

pub(super) trait GpuSource: Send {
    fn sample(&self) -> GpuStatus;
}

/// Build the real source, falling back to [`NoGpu`] when NVML cannot start.
pub(super) fn real_source() -> Box<dyn GpuSource> {
    match Nvml::init() {
        Ok(nvml) => Box::new(NvmlSource { nvml }),
        Err(e) => Box::new(NoGpu {
            reason: format!("NVML unavailable: {e}"),
        }),
    }
}

struct NvmlSource {
    nvml: Nvml,
}

impl GpuSource for NvmlSource {
    fn sample(&self) -> GpuStatus {
        match self.read() {
            Ok(info) => GpuStatus::Available(info),
            Err(reason) => GpuStatus::Unavailable { reason },
        }
    }
}

impl NvmlSource {
    fn read(&self) -> Result<GpuInfo, String> {
        let device = self
            .nvml
            .device_by_index(0)
            .map_err(|e| format!("no GPU at index 0: {e}"))?;

        let name = device.name().map_err(|e| format!("name: {e}"))?;
        let mem = device.memory_info().map_err(|e| format!("memory: {e}"))?;

        // Non-critical metrics: fall back to zero rather than failing the reading.
        let utilization_pct = device
            .utilization_rates()
            .map(|u| u.gpu.min(100) as u8)
            .unwrap_or(0);
        let temperature_c = device
            .temperature(TemperatureSensor::Gpu)
            .map(|t| t.min(u32::from(u8::MAX)) as u8)
            .unwrap_or(0);

        // A process can appear in both the compute and graphics lists; keep the
        // largest VRAM figure per pid.
        let mut by_pid: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
        for p in device
            .running_compute_processes()
            .unwrap_or_default()
            .into_iter()
            .chain(device.running_graphics_processes().unwrap_or_default())
        {
            let mb = match p.used_gpu_memory {
                UsedGpuMemory::Used(bytes) => bytes / BYTES_PER_MB,
                UsedGpuMemory::Unavailable => 0,
            };
            by_pid
                .entry(p.pid)
                .and_modify(|v| *v = (*v).max(mb))
                .or_insert(mb);
        }
        let mut processes: Vec<GpuProcess> = by_pid
            .into_iter()
            .map(|(pid, vram_mb)| GpuProcess { pid, vram_mb })
            .collect();
        processes.sort_by_key(|p| std::cmp::Reverse(p.vram_mb));
        processes.truncate(MAX_PROCESSES);

        Ok(GpuInfo {
            name,
            vram_total_mb: mem.total / BYTES_PER_MB,
            vram_used_mb: mem.used / BYTES_PER_MB,
            vram_free_mb: mem.free / BYTES_PER_MB,
            utilization_pct,
            temperature_c,
            processes,
        })
    }
}

struct NoGpu {
    reason: String,
}

impl GpuSource for NoGpu {
    fn sample(&self) -> GpuStatus {
        GpuStatus::Unavailable {
            reason: self.reason.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_gpu_reports_unavailable_with_reason() {
        let src = NoGpu {
            reason: "driver missing".into(),
        };
        match src.sample() {
            GpuStatus::Unavailable { reason } => assert_eq!(reason, "driver missing"),
            GpuStatus::Available(_) => panic!("expected unavailable"),
        }
    }

    #[test]
    fn real_source_never_panics() {
        // On CI this yields Unavailable; on the dev box, Available. Either is fine.
        let _ = real_source().sample();
    }

    #[test]
    fn gpu_status_serializes_with_a_state_tag() {
        let json = serde_json::to_string(&GpuStatus::Unavailable { reason: "x".into() }).unwrap();
        assert!(json.contains("\"state\":\"unavailable\""));
    }
}
