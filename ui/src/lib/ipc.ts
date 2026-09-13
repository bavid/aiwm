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
  /** Where managed runtime installs (llama.cpp, ComfyUI) land. */
  runtimes_dir: string;
  /** Where the disposable registry cache lands. */
  cache_dir: string;
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

/** How `Auto` weighs speed vs heft (`balanced` | `fast` | `quality`). */
export type AutoPreference = "balanced" | "fast" | "quality";

/** The `[models]` table — how `Auto` picks a model for a role (6.6). */
export interface ModelsConfig {
  auto_preference: AutoPreference;
}

/** The `[paths]` table — per-folder location overrides. `null` = the portable
 *  default next to the app (see `AboutInfo.runtimes_dir` / `.cache_dir` for
 *  the effective path). */
export interface PathsConfig {
  outputs_path: string | null;
  runtimes_path: string | null;
  cache_path: string | null;
}

/** The `paths` slice of a `ConfigUpdate` — plain strings; blank means "use
 *  the portable default", not a literal path. */
export interface PathsUpdate {
  outputs_path: string;
  runtimes_path: string;
  cache_path: string;
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
  models: ModelsConfig;
  paths: PathsConfig;
}

/** The user-editable subset the Settings tab sends back. */
export interface ConfigUpdate {
  store_path: string;
  offline_mode: boolean;
  vram_budget_mb: number;
  llama: LlamaConfig;
  comfyui: ComfyConfig;
  models: ModelsConfig;
  paths: PathsUpdate;
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
  /** The session this job belongs to, if any (`null` = ungrouped). */
  session_id: string | null;
}

/** A model currently resident on a runtime. */
export interface LoadedModel {
  model_id: string;
  vram_mb: number;
}

export interface RuntimeStatus {
  id: string;
  kind: string;
  health: "unknown" | "starting" | "healthy" | "unhealthy";
  vram_used_mb: number;
  /** Short human-readable status, e.g. "not installed" or "serving qwen on :48213". */
  detail: string | null;
  /** Structured, not parsed out of `detail` -- empty when nothing is loaded. */
  loaded_models: LoadedModel[];
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
  /** Group this job under a session (Chat/Image/Video "project"). */
  session_id?: string;
  params?: unknown;
}

// --- sessions -----------------------------------------------------------

/** `"chat"` | `"image"` | `"video"`. */
export type SessionCapability = "chat" | "image" | "video";

export interface Session {
  id: string;
  capability: SessionCapability;
  name: string;
  created_at: string;
  /** `null` = active (shown in the switcher); set = archived. */
  archived_at: string | null;
}

export interface NewSessionBody {
  capability: SessionCapability;
  name: string;
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
/** Permanently remove a finished job — history entry, events, and output
 *  file. Rejects if the job is still running (cancel it first). */
export const deleteJob = (id: string) => invoke<void>("delete_job", { id });
export const submitJob = (body: SubmitJobBody) => invoke<Job>("submit_job", { body });
export const jobDetail = (id: string) => invoke<JobDetail | null>("job_detail", { id });

export const listSessions = (capability: SessionCapability) =>
  invoke<Session[]>("list_sessions", { capability });
export const createSession = (body: NewSessionBody) =>
  invoke<Session>("create_session", { body });
export const renameSession = (id: string, name: string) =>
  invoke<void>("rename_session", { id, name });
export const setSessionArchived = (id: string, archived: boolean) =>
  invoke<void>("set_session_archived", { id, archived });
export const deleteSession = (id: string) => invoke<void>("delete_session", { id });

/** A document attached to a chat session for local RAG (7.x) — grounds chat
 *  answers via lexical (keyword) search over its chunks, no embedding model. */
export interface Document {
  id: string;
  session_id: string;
  name: string;
  source_path: string;
  /** `"txt"` | `"md"`. */
  format: string;
  created_at: string;
}
export const listDocuments = (sessionId: string) =>
  invoke<Document[]>("list_documents", { sessionId });
/** Read, chunk, and store a document already on this machine (`path`) for
 *  the given chat session. Only `.txt` / `.md` are supported. */
export const attachDocument = (sessionId: string, path: string) =>
  invoke<Document>("attach_document", { sessionId, path });
export const deleteDocument = (id: string) => invoke<void>("delete_document", { id });

export const listModels = () => invoke<Model[]>("list_models");

// --- storage & cleanup (Phase 6.8) -------------------------------------

export interface KindUsage {
  /** `llm` / `image` / `video` / `other`. */
  kind: string;
  bytes: number;
  count: number;
}

export interface ModelDisk {
  id: string;
  name: string;
  kind: string;
  size_bytes: number;
  last_used_at: string | null;
  use_count: number;
  roles: string[];
  file_present: boolean;
}

export interface DuplicateGroup {
  sha256: string;
  /** Newest first — keep [0], the rest are redundant. */
  member_ids: string[];
  wasted_bytes: number;
}

export interface StorageReport {
  store_bytes: number;
  volume_free_bytes: number | null;
  volume_total_bytes: number | null;
  by_kind: KindUsage[];
  models: ModelDisk[];
  duplicates: DuplicateGroup[];
  /** Model ids never used / not used within `stale_days`. */
  unused: string[];
  stale_days: number;
}

export interface DeleteOutcome {
  id: string;
  name: string;
  file_removed: boolean;
  freed_bytes: number;
}

export const storageReport = () => invoke<StorageReport>("storage_report");
/** Delete a model — its file, links and DB rows. Permanent; refused while
 *  the model is loaded. */
export const deleteModel = (id: string) => invoke<DeleteOutcome>("delete_model", { id });

// --- tags & registry status (Phase 6.9) -------------------------------

/** `model_id -> [tags]` for every tagged model. */
export const modelTags = () => invoke<Record<string, string[]>>("model_tags");
/** Replace one model's tags; returns the cleaned set. */
export const setModelTags = (id: string, tags: string[]) =>
  invoke<string[]>("set_model_tags", { id, tags });

/** Replace a model's roles (e.g. add `coding` to a model downloaded before
 *  that role existed) without re-importing. Returns the cleaned set. */
export const setModelRoles = (id: string, roles: string[]) =>
  invoke<string[]>("set_model_roles", { id, roles });

/** The registry health line for Diagnostics (`GET /registry/status`). */
export interface RegistryStatus {
  source_id: string;
  last_fetch: string | null;
  rate_limit_remaining: number | null;
  /** Seconds until the rate-limit window clears — set only while limited. */
  rate_limited_secs: number | null;
  token_set: boolean;
  cache_entries: number;
}
export const registryStatus = () => invoke<RegistryStatus>("registry_status");
/** Set (blank clears) the Hugging Face token — a machine-local file, never in
 *  a backup. Takes effect on the next restart. */
export const setHfToken = (token: string) => invoke<void>("set_hf_token", { token });

/** The unified local API endpoint (`GET /local-api/status`) — one
 *  OpenAI-compatible address that always forwards to whichever model is
 *  currently resident on llama.cpp. */
export interface LocalApiStatus {
  /** `http://127.0.0.1:<core_api_port>/v1` — what an external tool points at. */
  endpoint: string;
  token_set: boolean;
}
export const localApiStatus = () => invoke<LocalApiStatus>("local_api_status");
/** Set (blank clears) the local API bearer token — a machine-local file,
 *  never in a backup. Takes effect immediately, no restart needed. */
export const setLocalApiToken = (token: string) => invoke<void>("set_local_api_token", { token });

/** An already-running local LLM server found on a well-known port (Ollama,
 *  LM Studio, …) — bring-your-own-engine (`GET /external-engines`). */
export interface ExternalEngine {
  label: string;
  port: number;
  models: string[];
}
export const externalEngines = () => invoke<ExternalEngine[]>("external_engines");
/** Point the llama.cpp runtime slot at an already-running external server
 *  instead of installing AIWM's own. `vramMb` is your own estimate — AIWM
 *  can't introspect a process it doesn't manage. */
export const attachExternalEngine = (port: number, modelId: string, vramMb: number) =>
  invoke<void>("attach_external_engine", {
    body: { port, model_id: modelId, vram_mb: vramMb },
  });
/** Release the runtime slot without touching the external process. */
export const detachEngine = (modelId: string) => invoke<void>("detach_engine", { id: modelId });

/** One entry of the curated image/video catalogue (`GET /models/known`),
 *  enriched with a fit verdict against the current VRAM budget. */
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
  /** The curated "pick this one" model for its role. */
  is_default: boolean;
  /** Which media type this entry belongs to. */
  media: "image" | "video";
  fit: FitVerdict;
}

/** A base image/video model bundled with every companion file it needs to
 *  actually run (`GET /models/stacks`) — e.g. Flux's diffusion model + T5 +
 *  CLIP-L + VAE, one "Download entire stack" instead of hunting down each
 *  piece separately. */
export interface ModelStack {
  id: string;
  label: string;
  media: "image" | "video";
  note: string;
  /** The curated "pick this one" stack for its media type. */
  is_default: boolean;
  /** Base model first, then companions — display order. */
  members: KnownModel[];
  /** Fit for every member's size **summed** — a real render needs the base
   *  model and every companion in VRAM at once, so this is the number that
   *  actually answers "can I run this stack" (each member's own `fit` is
   *  just that one file in isolation). */
  fit: FitVerdict;
}

export const listModelStacks = () => invoke<ModelStack[]>("list_model_stacks");

/** One curated chat/coding recommendation (`GET /models/featured`) — a
 *  Hugging Face repo + preferred quant, not a pinned file. `fit` is judged
 *  from `typical_vram_mb`, a documented estimate; click through to Discover
 *  (or "Check exact fit") for the real file list. */
export interface FeaturedModel {
  id: string;
  role: "chat" | "coding";
  label: string;
  /** Hugging Face `owner/repo`. */
  repo: string;
  quant_hint: string;
  typical_vram_mb: number;
  license: string;
  note: string;
  import_roles: string[];
  is_default: boolean;
  fit: FitVerdict;
}

export const listFeaturedModels = () => invoke<FeaturedModel[]>("list_featured_models");

/** A curated Colibri model (`GET /models/colibri`) — AIWM does not download
 *  these itself (a few dozen safetensors shards; Hugging Face's own `hf` CLI
 *  fetches a directory like this far faster than this app's single-stream
 *  downloader would). The UI shows the exact command to run, then
 *  `registerColibriModel` registers wherever it lands. */
export interface ColibriModel {
  id: string;
  label: string;
  /** Hugging Face `owner/repo` to `hf download`. */
  repo: string;
  ram_estimate_mb: number;
  disk_estimate_bytes: number;
  license: string;
  note: string;
}

export const listColibriModels = () => invoke<ColibriModel[]>("list_colibri_models");

export interface RegisterColibriModelBody {
  /** A `ColibriModel.id`. */
  catalog_id: string;
  /** Local path to the already-downloaded model directory. */
  dir: string;
}

export const registerColibriModel = (body: RegisterColibriModelBody) =>
  invoke<Model>("register_colibri_model", { body });

export const installColibri = () => invoke<string>("install_colibri");

export const listKnownModels = () => invoke<KnownModel[]>("list_known_models");

/** A LoRA applied to a render — a library reference plus its strength. */
export interface LoraParam {
  model_id: string;
  strength: number;
}

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
  loras?: LoraParam[];
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
  loras?: LoraParam[];
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

// --- external launcher ---

export type LaunchTool = "opencode" | "hermes";

/** What's pinned for an external launch right now. */
export interface LaunchInfo {
  tool: LaunchTool;
  model_id: string;
  model_name: string;
  base_url: string;
  workspace: string;
  /** Set when the picked model doesn't meet a tool's own requirements
   *  (currently: Hermes' 64K context floor). Not fatal -- the launch still
   *  went through -- just surfaced so a refusal to start isn't a mystery. */
  warning: string | null;
}

export interface LaunchExternalBody {
  tool: LaunchTool;
  /** Explicit coding model, or omitted for `Auto` over the "coding" role. */
  model_id?: string | null;
  workspace: string;
}

export const launcherStatus = () => invoke<LaunchInfo | null>("launcher_status");
export const launchExternal = (body: LaunchExternalBody) =>
  invoke<LaunchInfo>("launch_external", { body });
/** Releases the pinned model. Cannot close the terminal window itself -- the
 *  whole point is that it keeps running independent of AIWM. */
export const stopExternalLaunch = () => invoke<void>("stop_external_launch");

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
  /** Roles stamped on the model once imported (e.g. a Featured coding pick). */
  roles: string[];
}

export interface EnqueueDownloadBody {
  url: string;
  filename: string;
  model_type?: string;
  sha256?: string;
  size_bytes?: number;
  /** Roles to stamp on the model once imported (e.g. `["chat", "coding"]`). */
  roles?: string[];
}

export const listDownloads = () => invoke<Download[]>("list_downloads");
/** Queue a download → verify against the SHA-256 → import into the store.
 *  Offline-gated. */
export const enqueueDownload = (body: EnqueueDownloadBody) =>
  invoke<Download>("enqueue_download", { body });
export const pauseDownload = (id: string) => invoke<void>("pause_download", { id });
export const resumeDownload = (id: string) => invoke<void>("resume_download", { id });
export const cancelDownload = (id: string) => invoke<void>("cancel_download", { id });

// --- benchmarks (Phase 6.5) ----------------------------------------------

/** One "Test model" run, measured on this machine (`GET /benchmarks`). */
export interface Benchmark {
  id: string;
  model_id: string;
  job_id: string | null;
  /** `"llm"` for now. */
  kind: string;
  runs: number;
  /** Prompt (prefill) tokens/sec, mean. */
  prompt_tps: number | null;
  /** Generation tokens/sec, mean. */
  gen_tps: number | null;
  /** Cold load time; null when the model was already resident. */
  load_ms: number | null;
  vram_peak_mb: number | null;
  ram_peak_mb: number | null;
  /** 0..1, 1 = perfectly consistent tokens/sec across the runs. */
  stability_score: number;
  /** 0..100 openly-declared heuristic (speed + fit + stability), **not** a
   *  quality score. */
  overall_score: number;
  notes: string | null;
  created_at: string;
}

/** The latest benchmark for every model that has one — join by `model_id`. */
export const listBenchmarks = () => invoke<Benchmark[]>("list_benchmarks");
/** Every benchmark run for one model, newest first. */
export const modelBenchmarks = (id: string) =>
  invoke<Benchmark[]>("model_benchmarks", { id });
/** Queue a "Test model" job (GGUF models only). Returns the job. */
export const benchmarkModel = (id: string) => invoke<Job>("benchmark_model", { id });

// --- upgrade check (Phase 6.7) ------------------------------------------

/** One model the upgrade check proposes (`UpgradeReport.candidates[]`). */
export interface UpgradeCandidate {
  /** `owner/repo` on Hugging Face. */
  id: string;
  /** One sentence — the local model's reason, or an objective note. */
  why: string;
  downloads: number;
  likes: number;
  last_modified: string | null;
  param_count: number | null;
  format: "gguf" | "safetensors" | "other";
  gated: boolean;
  fit: FitVerdict;
  /** Already in your library. */
  installed: boolean;
  /** The local model ranked this (vs. objective-only). */
  llm_ranked: boolean;
}

/** The result of an `upgrade_check` job — JSON in `job.result`. */
export interface UpgradeReport {
  target: string;
  query: string;
  candidates: UpgradeCandidate[];
  note: string;
  freshness: Freshness;
}

/** Queue an "is there something better?" job for one installed model. Reasons
 *  with an Auto-picked chat/coding model + queries Hugging Face. Offline-gated. */
export const upgradeCheck = (id: string) => invoke<Job>("upgrade_check", { id });
