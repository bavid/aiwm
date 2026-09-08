import { invoke } from "@tauri-apps/api/core";

/**
 * Typed bridge to the Rust core. The full command surface
 * (`get_system_telemetry`, `list_jobs`, …) lands in WP-6; for now only `about`
 * exists. Callers handle rejection when running outside Tauri.
 */

export interface AboutInfo {
  core_version: string;
  tauri_host_version: string;
}

export function about(): Promise<AboutInfo> {
  return invoke<AboutInfo>("about");
}
