import { invoke } from "@tauri-apps/api/core";

/** Typed bridge to the Rust core. Every function forwards to a
 *  `#[tauri::command]` that calls the same `aiwm_core::api::handlers` function
 *  the loopback HTTP server uses. */

export interface AboutInfo {
  core_version: string;
  data_dir: string;
  store_path: string;
  /** Where generated images land. */
  outputs_dir: string;
  core_api_port: number;
  vram_budget_mb: number;
  offline_mode: boolean;
}

/** `llama-server` launch options — the `[llama]` table in config.toml. */
export interface LlamaConfig {
  /** `-ngl` — layers offloaded to the GPU (999 = all). */
  gpu_layers: number;
  /** `-c` — context window; 0 lets the core cap the model's trained context. */
  ctx_size: number;
  flash_attention: boolean;
  load_timeout_secs: number;
}

/** ComfyUI server options — the `[comfyui]` table in config.toml. */
export interface ComfyConfig {
  /** VRAM mode: auto | highvram | normalvram | lowvram | novram. */
  vram_mode: string;
}

/** The full config.toml as the core sees it. */
export interface AppConfig {
  store_path: string;
  core_api_port: number;
  offline_mode: boolean;
  log_filter: string;
  /** VRAM budget (MB) for the scheduler; 0 = auto-detect from the GPU. */
  vram_budget_mb: number;
  llama: LlamaConfig;
  comfyui: ComfyConfig;
}

/** The user-editable subset the Settings tab sends back. */
export interface ConfigUpdate {
  store_path: string;
  offline_mode: boolean;
  vram_budget_mb: number;
  llama: LlamaConfig;
  comfyui: ComfyConfig;
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
  /** Progressively-updated generated text for chat/completion jobs. */
  result: string | null;
}

export interface RuntimeStatus {
  id: string;
  kind: string;
  health: "unknown" | "starting" | "healthy" | "unhealthy";
  vram_used_mb: number;
  /** Short human-readable status, e.g. "not installed" or "serving qwen on :48213". */
  detail: string | null;
}

export interface Model {
  id: string;
  publisher: string | null;
  name: string;
  family: string | null;
  format: string;
  quant: string | null;
  arch: string | null;
  param_count: number | null;
  file_path: string;
  sha256: string | null;
  size_bytes: number;
  ctx_max: number | null;
  /** Estimated VRAM to serve this model at the default chat context (weights + KV cache + overhead). */
  vram_estimate_mb: number | null;
  ram_estimate_mb: number | null;
  source: string;
  imported_at: string;
  last_used_at: string | null;
  use_count: number;
  /** GGUF architecture dims (for the VRAM estimate); null when the header lacked them. */
  n_layers: number | null;
  n_embd: number | null;
  n_heads: number | null;
  n_kv_heads: number | null;
  roles: string[];
  /** Runtime ids that can use this model (from the link manager). */
  runtimes: string[];
}

export interface ImportOutcome {
  model: Model;
  already_present: boolean;
}

export interface JobEvent {
  ts: string;
  level: "info" | "warn" | "error";
  message: string;
}

export interface JobDetail {
  job: Job;
  events: JobEvent[];
}

export interface SubmitJobBody {
  job_type: string;
  capability?: string;
  runtime_id?: string;
  model_id?: string;
  vram_needed_mb?: number;
  agent_session?: boolean;
  params?: unknown;
}

export const about = () => invoke<AboutInfo>("about");
export const getTelemetry = () => invoke<SystemTelemetry>("get_telemetry");
export const getSettings = () => invoke<Record<string, string>>("get_settings");
export const getConfig = () => invoke<AppConfig>("get_config");
/** Persist the editable config fields. Offline mode applies at once; the rest
 *  need an app restart (the caller tells the user). Returns the saved config. */
export const saveConfig = (update: ConfigUpdate) =>
  invoke<AppConfig>("save_config", { update });
export const getRuntimes = () => invoke<RuntimeStatus[]>("get_runtimes");
/** Start the pinned llama.cpp download+install (background). Returns "started" or "already_installed". */
export const installLlamacpp = () => invoke<string>("install_llamacpp");
/** Start the pinned ComfyUI install — uv + source + venv + PyTorch + deps (background). */
export const installComfyui = () => invoke<string>("install_comfyui");
export const getRecentLogs = (lines = 200) =>
  invoke<string[]>("get_recent_logs", { lines });
export const listJobs = (opts?: { states?: JobState[]; limit?: number }) =>
  invoke<Job[]>("list_jobs", { states: opts?.states ?? null, limit: opts?.limit ?? null });
/** Ask a job to stop. `true` = applied/signalled, `false` = too late, `null` = no such job. */
export const cancelJob = (id: string) => invoke<boolean | null>("cancel_job", { id });
export const submitJob = (body: SubmitJobBody) => invoke<Job>("submit_job", { body });
export const jobDetail = (id: string) => invoke<JobDetail | null>("job_detail", { id });

export const listModels = () => invoke<Model[]>("list_models");

/** One entry of the curated image-model catalogue (`GET /models/known`). */
export interface KnownModel {
  id: string;
  name: string;
  kind: ModelType;
  family: string | null;
  publisher: string;
  repo: string;
  file: string;
  url: string;
  sha256: string;
  size_bytes: number;
  license: string;
  note: string;
}

export const listKnownModels = () => invoke<KnownModel[]>("list_known_models");

/** The parameters of a `job_type=image` job. After the engine runs, `params`
 *  holds these resolved values (a random seed is pinned back). */
export interface ImageParams {
  prompt: string;
  negative: string;
  width: number;
  height: number;
  steps: number;
  cfg: number;
  sampler: string;
  scheduler: string;
  seed: number;
}

/** URL the loopback core serves a finished image job's picture from. Used as an
 *  `<img>` src — the CSP allows `http://127.0.0.1:*` for images. */
export const imageOutputUrl = (coreApiPort: number, jobId: string) =>
  `http://127.0.0.1:${coreApiPort}/jobs/${jobId}/output`;

/** What kind of file is being imported. `chat` → GGUF LLM for llama.cpp; the
 *  rest are ComfyUI image models routed to their typed store folder. */
export type ModelType =
  | "chat"
  | "checkpoint"
  | "diffusion_model"
  | "vae"
  | "lora"
  | "text_encoder";

export const importModel = (
  sourcePath: string,
  roles: string[],
  keepOriginal: boolean,
  modelType: ModelType,
) =>
  invoke<ImportOutcome>("import_model", {
    request: {
      source_path: sourcePath,
      roles,
      keep_original: keepOriginal,
      model_type: modelType,
    },
  });
