import { invoke } from "@tauri-apps/api/core";

/** Typed bridge to the Rust core. Every function forwards to a
 *  `#[tauri::command]` that calls the same `aiwm_core::api::handlers` function
 *  the loopback HTTP server uses. */

export interface AboutInfo {
  core_version: string;
  data_dir: string;
  store_path: string;
  core_api_port: number;
  vram_budget_mb: number;
  offline_mode: boolean;
}

export interface GpuProcess {
  pid: number;
  vram_mb: number;
}

export type GpuStatus =
  | {
      state: "available";
      name: string;
      vram_total_mb: number;
      vram_used_mb: number;
      vram_free_mb: number;
      utilization_pct: number;
      temperature_c: number;
      processes: GpuProcess[];
    }
  | { state: "unavailable"; reason: string };

export interface HostStatus {
  ram_total_mb: number;
  ram_used_mb: number;
  cpu_total_pct: number;
  cpu_per_core_pct: number[];
}

export interface SystemTelemetry {
  captured_at_ms: number;
  gpu: GpuStatus;
  host: HostStatus;
}

export type JobState =
  | "queued"
  | "scheduled"
  | "blocked"
  | "preparing"
  | "running"
  | "post"
  | "completed"
  | "failed"
  | "cancelled";

export interface Job {
  id: string;
  job_type: string;
  capability: string | null;
  state: JobState;
  params: unknown;
  runtime_id: string | null;
  model_id: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  error_text: string | null;
  output_path: string | null;
}

export interface RuntimeStatus {
  id: string;
  kind: string;
  health: "unknown" | "starting" | "healthy" | "unhealthy";
  vram_used_mb: number;
}

export const about = () => invoke<AboutInfo>("about");
export const getTelemetry = () => invoke<SystemTelemetry>("get_telemetry");
export const getSettings = () => invoke<Record<string, string>>("get_settings");
export const getRuntimes = () => invoke<RuntimeStatus[]>("get_runtimes");
export const getRecentLogs = (lines = 200) =>
  invoke<string[]>("get_recent_logs", { lines });
export const listJobs = (opts?: { states?: JobState[]; limit?: number }) =>
  invoke<Job[]>("list_jobs", { states: opts?.states ?? null, limit: opts?.limit ?? null });
