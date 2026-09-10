import { invoke } from "@tauri-apps/api/core";

/** Typed bridge to the Rust core. Every function forwards to a
 *  `#[tauri::command]` that calls the same `aiwm_core::api::handlers` function
 *  the loopback HTTP server uses. */

export interface AboutInfo {
  core_version: string;
  data_dir: string;
  store_path: string;
  /** Where generated images + video land. */
  outputs_dir: string;
  /** Total bytes of the files in `outputs_dir` (the retention card shows it). */
  outputs_bytes: number;
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
  /** `--jinja` — the GGUF's embedded chat template. Needed for tool calls (agents). */
  jinja: boolean;
  /** `--chat-template <name>`; empty = use the GGUF's own. */
  chat_template: string;
  load_timeout_secs: number;
}

/** ComfyUI server options — the `[comfyui]` table in config.toml. */
export interface ComfyConfig {
  /** VRAM mode: auto | highvram | normalvram | lowvram | novram. */
  vram_mode: string;
  /** `--reserve-vram <GB>` — VRAM kept free for the OS. In MB; 0 = off. */
  reserve_vram_mb: number;
  /** Extra raw args appended to the ComfyUI command line (whitespace-split). */
  extra_args: string;
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

/** The parameters of a `job_type=video` job. After the engine runs, `params`
 *  holds these resolved values (a random seed is pinned back; `length` is
 *  snapped to Wan's 4k+1 frame grid). `init_image` is the image→video start
 *  frame — a finished image job's id, or a path. */
export interface VideoParams {
  prompt: string;
  negative: string;
  width: number;
  height: number;
  /** Frame count. */
  length: number;
  fps: number;
  steps: number;
  cfg: number;
  seed: number;
  init_image?: string;
}

/** URL the loopback core serves a finished job's output file from — a PNG for
 *  image jobs, an MP4 for video jobs. Used as an `<img>` / `<video>` src; the
 *  CSP allows `http://127.0.0.1:*` for both `img-src` and `media-src`. */
export const jobOutputUrl = (coreApiPort: number, jobId: string) =>
  `http://127.0.0.1:${coreApiPort}/jobs/${jobId}/output`;
/** @deprecated use {@link jobOutputUrl} */
export const imageOutputUrl = jobOutputUrl;

/** What kind of file is being imported. `chat` → GGUF LLM for llama.cpp; the
 *  rest are ComfyUI image / video models routed to their typed store folder. */
export type ModelType =
  | "chat"
  | "checkpoint"
  | "diffusion_model"
  | "vae"
  | "lora"
  | "text_encoder"
  | "video";

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

// --- agents (Phase 5.1c) ---------------------------------------------------

/** A stored agent profile (`GET /agents`). */
export interface Agent {
  id: string;
  name: string;
  /** `"opencode"` (Hermes lands in 5.4). */
  adapter: string;
  /** Explicit coding model, or `null` for Auto over the `coding` role. */
  model_id: string | null;
  workspace_path: string;
  allowed_paths: string[];
  toolset: string[] | null;
  created_at: string;
}

export interface NewAgentBody {
  name: string;
  adapter: string;
  model_id?: string | null;
  workspace_path: string;
  allowed_paths?: string[];
  toolset?: string[] | null;
}

export type AgentSessionState =
  | "starting"
  | "idle"
  | "working"
  | "awaiting_approval"
  | "stopped"
  | "failed";

export interface AgentSession {
  id: string;
  agent_id: string;
  adapter_session_id: string | null;
  state: AgentSessionState;
  error_text: string | null;
  checkpoint_path: string | null;
  started_at: string;
  ended_at: string | null;
}

export type ToolStatus = "pending" | "running" | "done" | "error";

/** One event from a running session — the Rust `AgentEvent`, tag = `type`. */
export type AgentEventPayload =
  | { type: "text"; text: string }
  | {
      type: "tool";
      id: string;
      name: string;
      status: ToolStatus;
      input: unknown;
      output?: string | null;
    }
  | {
      type: "permission";
      id: string;
      kind: string;
      summary: string;
      always_pattern?: string | null;
    }
  | { type: "idle" }
  | { type: "error"; message: string; terminal: boolean };

/** One transcript entry. `payload.type` matches `kind`. */
export interface AgentSessionEvent {
  ts: string;
  kind: string;
  payload: AgentEventPayload;
}

export interface AgentSessionDetail {
  session: AgentSession;
  events: AgentSessionEvent[];
  /** Whether the core is currently driving this session in-process. */
  live: boolean;
}

export type PermissionDecision = "allow_once" | "allow_always" | "deny";

export const listAgents = () => invoke<Agent[]>("list_agents");
export const createAgent = (body: NewAgentBody) =>
  invoke<Agent>("create_agent", { body });
export const deleteAgent = (id: string) =>
  invoke<void>("delete_agent", { id });

/** Open a session for a profile and (optionally) send the first turn. The
 *  coding model is placed + pinned before this returns. */
export const openAgentSession = (agentId: string, firstMessage?: string) =>
  invoke<AgentSession>("open_agent_session", {
    body: { agent_id: agentId, first_message: firstMessage ?? null },
  });
export const agentSessionDetail = (id: string) =>
  invoke<AgentSessionDetail | null>("agent_session_detail", { id });
export const agentSessionMessage = (id: string, text: string) =>
  invoke<void>("agent_session_message", { id, text });
export const agentSessionPermission = (
  id: string,
  requestId: string,
  decision: PermissionDecision,
) =>
  invoke<void>("agent_session_permission", {
    id,
    body: { request_id: requestId, decision },
  });
export const stopAgentSession = (id: string) =>
  invoke<void>("stop_agent_session", { id });

/** Hermes install progress (`GET /agent-runtimes` → `install`). */
export type AgentInstallStatus =
  | { state: "idle" }
  | {
      state: "running";
      phase: "downloading" | "creating_venv" | "installing_hermes" | "post_install";
      done_bytes: number;
      total_bytes: number;
    }
  | { state: "failed"; error: string };

/** One agent runtime's availability. Only Hermes has an AIWM installer;
 *  OpenCode's `install` is absent (bring your own `opencode`). */
export interface AgentRuntime {
  id: "opencode" | "hermes";
  installed: boolean;
  install?: AgentInstallStatus;
}

export const listAgentRuntimes = () =>
  invoke<AgentRuntime[]>("list_agent_runtimes");
/** Start the Hermes install (background). "started" | "already_installed". */
export const installHermes = () => invoke<string>("install_hermes");

// --- backup & restore (Phase 5.5) ---

/** What an import staged, to relay before the restart. */
export interface ImportSummary {
  /** Core version that wrote the archive. */
  core_version: string;
  /** When the archive was created (RFC 3339). */
  created_at: string;
  model_count: number;
  /** `"<name> (<file>)"` for each archived model whose file is not in this
   *  machine's store — the user must re-import these. */
  missing_models: string[];
  restart_required: boolean;
}

/** Write a portable backup (db snapshot + config + model manifest, **not** the
 *  model files) to `<data>/exports/` and return its path. */
export const exportBackup = () => invoke<string>("export_backup");
/** Validate an export archive and stage it for the next startup. The live db is
 *  untouched until the app restarts. */
export const importBackup = (path: string) =>
  invoke<ImportSummary>("import_backup", { path });

// --- model discovery (Phase 6.2) ---

/** Where a registry answer came from. `stale` = the Hub was unreachable and this
 *  is the last cached answer; `offline` = offline mode is on. */
export type Freshness =
  | { kind: "live" }
  | { kind: "stale"; age_secs: number }
  | { kind: "offline"; age_secs: number };

/** One search hit from the Hugging Face Hub (`GET /registry/search`). */
export interface RemoteModel {
  /** `owner/repo`. */
  id: string;
  author: string | null;
  downloads: number;
  likes: number;
  trending_score: number | null;
  created_at: string | null;
  last_modified: string | null;
  pipeline_tag: string | null;
  library_name: string | null;
  gated: "no" | "auto" | "manual";
  license: string | null;
  /** The upstream repo this is a quant / fine-tune of. */
  base_model: string | null;
  tags: string[];
  param_count: number | null;
  arch: string | null;
  ctx_max: number | null;
  precision: string | null;
  format: "gguf" | "safetensors" | "other";
}

/** Will this file run on this machine? Weights + KV + overhead vs the VRAM
 *  budget and free system RAM, with a plain-language reason (`core::compat`). */
export type FitVerdict =
  | { level: "green" }
  | { level: "yellow"; reason: string }
  | { level: "red"; reason: string }
  | { level: "unknown" };

export interface RegistryFile {
  path: string;
  size_bytes: number;
  sha256: string | null;
  quant: string | null;
  /** `[index, total]` for a split file. */
  shard: [number, number] | null;
  /** The HF `/resolve/` URL — fed to `enqueueDownload` (6.4) or "Copy link". */
  download_url: string;
  vram_estimate_mb: number | null;
  fit: FitVerdict;
}

export interface RegistryDetails extends RemoteModel {
  revision: string;
  files: RegistryFile[];
  freshness: Freshness;
}

export interface RegistrySearchParams {
  q?: string;
  base_model?: string;
  gguf?: boolean;
  sort?: "downloads" | "likes" | "trending" | "updated" | "new";
  limit?: number;
}

export interface RegistrySearchResult {
  data: RemoteModel[];
  freshness: Freshness;
}

/** Search the Hugging Face Hub. Offline-gated (ADR-009) — offline mode returns a
 *  cached answer or a plain error. */
export const registrySearch = (params: RegistrySearchParams) =>
  invoke<RegistrySearchResult>("registry_search", { params });
/** One repo with every file (size, SHA-256, quant, fit). `id` is `owner/repo`. */
export const registryModel = (id: string) =>
  invoke<RegistryDetails>("registry_model", { id });

// --- download manager (Phase 6.4) ---

export type DownloadState =
  | "queued"
  | "running"
  | "paused"
  | "verifying"
  | "done"
  | "failed";

/** One queued / in-flight / finished model download (`GET /downloads`). */
export interface Download {
  id: string;
  url: string;
  filename: string;
  model_type: string | null;
  sha256: string | null;
  size_bytes: number | null;
  bytes_done: number;
  retries: number;
  state: DownloadState;
  error_text: string | null;
  /** Set to the store model id after a successful import. */
  model_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface EnqueueDownloadBody {
  url: string;
  filename: string;
  model_type?: string;
  sha256?: string;
  size_bytes?: number;
}

export const listDownloads = () => invoke<Download[]>("list_downloads");
/** Queue a download → verify against the SHA-256 → import into the store.
 *  Offline-gated. */
export const enqueueDownload = (body: EnqueueDownloadBody) =>
  invoke<Download>("enqueue_download", { body });
export const pauseDownload = (id: string) => invoke<void>("pause_download", { id });
export const resumeDownload = (id: string) => invoke<void>("resume_download", { id });
export const cancelDownload = (id: string) => invoke<void>("cancel_download", { id });
