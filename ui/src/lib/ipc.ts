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
  /** Where dataset-prep work folders land. */
  datasets_dir: string;
  /** Where training-run work folders land. */
  training_dir: string;
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
  datasets_path: string | null;
  training_path: string | null;
}

/** The `paths` slice of a `ConfigUpdate` — plain strings; blank means "use
 *  the portable default", not a literal path. */
export interface PathsUpdate {
  outputs_path: string;
  runtimes_path: string;
  cache_path: string;
  datasets_path: string;
  training_path: string;
}

/** The `[retention]` table — automatic cleanup of `<outputs_dir>`. Either
 *  field `0` disables that rule; both `0` (the default) disables retention
 *  entirely. Unlike most of `AppConfig`, this applies immediately — no
 *  restart needed — both the manual "clean up now" trigger and an optional
 *  startup sweep re-read `config.toml` fresh. */
export interface RetentionConfig {
  /** Delete output files last modified more than this many days ago. */
  max_age_days: number;
  /** Keep the outputs folder's total size under this many MB, oldest deleted
   *  first, applied after the age rule. */
  max_total_mb: number;
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
  retention: RetentionConfig;
  civitai: CivitaiConfig;
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
  retention: RetentionConfig;
  civitai: CivitaiConfig;
}

/** Which Civitai front door Discover searches: `com` is the safe-for-work
 *  catalogue, `red` Civitai's own domain that also carries adult models. Both
 *  answer the same API; what comes back is still governed by the "Show NSFW"
 *  tick in Discover. */
export type CivitaiFrontDoor = "com" | "red";

export interface CivitaiConfig {
  front_door: CivitaiFrontDoor;
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

/** A `job_type: "chat"` completion submitted by the Prompt Assistant (Image
 *  or Video's "talk through what you want" helper) rather than typed by the
 *  user in the Chat tab -- both are plain chat jobs at the API level, told
 *  apart only by this params marker. Chat's own turn history excludes these
 *  (they're drafting help, not a conversation the user had), and the Jobs
 *  page labels them distinctly instead of just "chat". */
export function assistantKindOf(job: Job): "image" | "video" | "edit" | "narrate" | null {
  const p = job.params;
  if (!p || typeof p !== "object" || !("assistant_for" in p)) return null;
  const v = (p as { assistant_for?: unknown }).assistant_for;
  return v === "image" || v === "video" || v === "edit" || v === "narrate" ? v : null;
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
  /** Legacy/runtime family string (`sdxl`, `flux`, `flux2`, `wan`, …) the
   *  recipes and LoRA pickers compare against. */
  family: string | null;
  /** The base family it is made for, as a registry id (`sdxl`, `pony`,
   *  `flux2-klein-9b`, …) — recorded at download, by the user, or from a
   *  saved detection; `null` until then (Plan 14). */
  base_family: string | null;
  /** How `base_family` was decided. */
  family_source: FamilySource | null;
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
  /** How this chat picks its persona — `"inherit"` for every session that
   *  predates personas. */
  persona_mode: PersonaMode;
  /** Only meaningful with `persona_mode: "persona"`. May name a persona that
   *  has since been deleted; the core heals that on the next resolve. */
  persona_id: string | null;
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

/** Whether AIWM's pinned version for one of the five externally-sourced tools
 *  (ComfyUI, llama.cpp, Colibri, Hermes, OpenCode) is behind the latest one
 *  published upstream. Not the per-model "is there a better model" advisor
 *  (see `upgradeCheck`) -- this is a deterministic version-string compare
 *  against GitHub Releases / PyPI, wherever each tool's installer already
 *  sources its pinned archive from. */
export type ToolUpdateStatus =
  | { state: "up_to_date" }
  | { state: "update_available"; latest: string }
  /** AIWM does not pin a version for this tool (OpenCode: bring your own
   *  binary) -- `latest` is shown for reference only, never compared. */
  | { state: "unmanaged"; latest: string }
  /** The upstream check itself failed (network, unexpected shape, ...). */
  | { state: "check_failed"; error: string };

export interface ToolVersionCheck {
  id: "comfyui" | "llamacpp" | "colibri" | "hermes" | "opencode";
  /** AIWM's pinned version, or `null` for the unmanaged OpenCode. */
  current: string | null;
  status: ToolUpdateStatus;
}

/** Fetches all five tools' upstream versions right now (a few real HTTP
 *  calls) -- not polled, since it hits GitHub/PyPI on every call. Rejects
 *  with "offline mode is on ..." when offline mode is active. */
export const checkToolVersions = () => invoke<ToolVersionCheck[]>("check_tool_versions");
export const getRecentLogs = (lines = 200) =>
  invoke<string[]>("get_recent_logs", { lines });
export const listJobs = (opts?: { states?: JobState[]; limit?: number }) =>
  invoke<Job[]>("list_jobs", { states: opts?.states ?? null, limit: opts?.limit ?? null });
/** Ask a job to stop. `true` = applied/signalled, `false` = too late, `null` = no such job. */
export const cancelJob = (id: string) => invoke<boolean | null>("cancel_job", { id });
/** Permanently remove a finished job — history entry, events, and output
 *  file. Rejects if the job is still running (cancel it first). */
export const deleteJob = (id: string) => invoke<void>("delete_job", { id });
/** Runs a DSP cleanup pass (DC-offset removal, a gentle high-pass filter,
 *  spectral-gate noise reduction) on an already-rendered narration clip,
 *  overwriting it in place. Only valid for finished `tts` jobs. Returns the
 *  resulting duration (unchanged by the pass, but returned for parity with
 *  the original render). */
export const cleanAudio = (id: string) => invoke<number>("clean_audio", { id });
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

// --- personas -----------------------------------------------------------

/** A named preset the chat job prepends as a system prompt (spec
 *  `2026-09-18-personas-design`). One can be active globally; a chat session
 *  can override it with another one or with "none". */
export interface Persona {
  id: string;
  name: string;
  /** One emoji. */
  icon: string;
  /** Passed to the model verbatim — the tool applies no content filter. */
  system_prompt: string;
  created_at: string;
  updated_at: string;
}

/** The three editable fields, for create and update alike. The core trims them
 *  and rejects (400) a name outside 1–60 characters, an icon that is empty or
 *  over 64 bytes, a control character in either, and a prompt outside 1–8000
 *  characters. The prompt itself is passed to the model verbatim — the only
 *  limits are technical. */
export interface PersonaBody {
  name: string;
  icon: string;
  system_prompt: string;
}

/** How a chat session picks its persona: follow the global default, opt out
 *  entirely, or use `persona_id`. */
export type PersonaMode = "inherit" | "none" | "persona";

/** Where the resolved persona came from. `"none"` covers both "nothing is set"
 *  and "this chat opted out" — tell them apart via `Session.persona_mode`. */
export type PersonaOrigin = "session" | "global" | "none";

/** The persona a chat will actually use, resolved by the core so the UI never
 *  re-implements the rule. */
export interface EffectivePersona {
  persona: Persona | null;
  origin: PersonaOrigin;
}

/** The globally active persona's id, or `null` for "no global persona". */
export interface ActivePersona {
  id: string | null;
}

/** `"stored"`, or which of the two ids was unknown (the HTTP twin's 404s). */
export type SessionPersonaOutcome = "stored" | "unknown_session" | "unknown_persona";

/** What a chat job records about the persona that answered it — stamped into
 *  the job's params by the core at run time, so the history still shows it
 *  after the persona has been renamed or deleted. */
export interface PersonaMark {
  id: string;
  name: string;
  icon: string;
}

/** The persona stamped into a finished (or running) chat job's params, or
 *  `null` when that job ran without one. */
export function personaOf(job: Job): PersonaMark | null {
  const p = job.params;
  if (!p || typeof p !== "object") return null;
  const mark = (p as { persona?: unknown }).persona;
  if (!mark || typeof mark !== "object") return null;
  const { id, name, icon } = mark as { id?: unknown; name?: unknown; icon?: unknown };
  if (typeof id !== "string" || typeof name !== "string" || typeof icon !== "string") return null;
  return { id, name, icon };
}

export const listPersonas = () => invoke<Persona[]>("list_personas");
/** Rejects with the core's own message when a limit is violated. */
export const createPersona = (body: PersonaBody) => invoke<Persona>("create_persona", { body });
/** `null` = no such persona. */
export const updatePersona = (id: string, body: PersonaBody) =>
  invoke<Persona | null>("update_persona", { id, body });
/** `false` = no such persona. Sessions pointing at a deleted persona fall back
 *  to `inherit`, and the global default is cleared if it matched. */
export const deletePersona = (id: string) => invoke<boolean>("delete_persona", { id });
export const activePersona = () => invoke<ActivePersona>("active_persona");
/** `null` clears the global default. `false` = unknown id, nothing changed. */
export const setActivePersona = (id: string | null) =>
  invoke<boolean>("set_active_persona", { id });
export const setSessionPersona = (id: string, body: { mode: PersonaMode; persona_id?: string }) =>
  invoke<SessionPersonaOutcome>("set_session_persona", { id, body });
/** The persona that would answer right now. `null` session = "Ungrouped",
 *  where only the global default applies. */
export const effectivePersona = (sessionId: string | null) =>
  invoke<EffectivePersona>("effective_persona", { sessionId });

/** A saved Dia voice-cloning identity — a reference clip + its own transcript,
 *  set up once under a name (e.g. "Old Man Gareth") and reused across many
 *  narration calls instead of re-picking a file and re-typing the transcript
 *  every time. `reference_audio_path` always sits under AIWM's own data dir —
 *  the file picked when creating it is copied there, never referenced in
 *  place. */
export interface VoiceIdentity {
  id: string;
  name: string;
  reference_audio_path: string;
  reference_transcript: string;
  created_at: string;
}
export interface NewVoiceIdentityBody {
  name: string;
  /** A path on this machine, resolved by the caller (a native file picker). */
  source_audio_path: string;
  reference_transcript: string;
}
export const listVoiceIdentities = () => invoke<VoiceIdentity[]>("list_voice_identities");
export const createVoiceIdentity = (body: NewVoiceIdentityBody) =>
  invoke<VoiceIdentity>("create_voice_identity", { body });
export const deleteVoiceIdentity = (id: string) => invoke<void>("delete_voice_identity", { id });

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

/** One row of `GET /storage/locations` — a folder the app writes to,
 *  measured on disk (Plan 10). */
export interface StorageLocation {
  /** Stable id: `outputs` | `datasets` | `training` | `models` | `runtimes`
   *  | `cache` | `downloads` | `logs` | `exports` | `voice_identities`
   *  | `comfyui_data` | `pending_import`. */
  key: string;
  /** Short human label for the Settings UI. */
  label: string;
  path: string;
  /** Whether this location can be pointed elsewhere in Settings. */
  configurable: boolean;
  exists: boolean;
  bytes: number;
  files: number;
  /** Entries the walk could not read — never fatal. */
  skipped: number;
  volume_free_bytes: number | null;
  volume_total_bytes: number | null;
}

/** Every location the app writes to, with its size and volume free space.
 *  Computed on demand (a recursive walk) — call it when the Settings "Data
 *  locations" card opens and on "Refresh", never on a timer. */
export const storageLocations = () => invoke<StorageLocation[]>("storage_locations");

/** What one output-retention sweep did (`POST /outputs/cleanup`). */
export interface SweepResult {
  deleted_files: number;
  freed_bytes: number;
  errors: string[];
}
/** Apply the currently-saved retention policy to `<outputs_dir>` right now —
 *  the Settings "Clean up now" button. A no-op (`deleted_files: 0`) when no
 *  policy is configured (both fields `0`); save one via `saveConfig` first. */
export const cleanupOutputs = () => invoke<SweepResult>("cleanup_outputs");

// --- cleanup scan (Plan 13) --------------------------------------------

/** Group keys of `GET /cleanup/scan`, in display order. */
export type CleanupGroupKey =
  | "media_retention"
  | "media_orphans"
  | "discarded_frames"
  | "unclaimed_dataset_folders"
  | "missing_frame_rows"
  | "finished_runs"
  | "caches"
  | "old_logs"
  | "db_backups";

/** One selectable item of a cleanup group: a file, a dataset, a run, a
 *  folder. `id` is stable within its group (a file name, a dataset or run
 *  id, a folder name) — what a later apply names. */
export interface CleanupEntry {
  id: string;
  label: string;
  /** Files that would be deleted. */
  files: number;
  bytes: number;
  /** Database rows that would be removed (frame rows), else 0. */
  rows: number;
  /** A few lines for the expanded entry (paths, dates, names) — capped. */
  detail: string[];
}

/** One kind of removable content — present even when empty. */
export interface CleanupGroup {
  key: CleanupGroupKey | string;
  label: string;
  entries: CleanupEntry[];
  total_files: number;
  total_bytes: number;
}

/** Something a user might expect to see offered, and why it is not. */
export interface ProtectedNote {
  what: string;
  reason: string;
}

/** `GET /cleanup/scan` — what the app generated and could remove, grouped,
 *  plus what was deliberately not offered. Models, runtimes, voice
 *  identities, source media and anything a running job, run or download
 *  needs are never in `groups`. */
export interface CleanupReport {
  /** RFC 3339, when the scan ran. */
  scanned_at: string;
  groups: CleanupGroup[];
  protected: ProtectedNote[];
}

/** Scan for removable content. Walks every app folder — call it on demand
 *  (the Cleanup page's "Scan" button), never on a timer. Reports only;
 *  nothing is deleted. */
export const cleanupScan = () => invoke<CleanupReport>("cleanup_scan");

/** The entries of one group to apply — the scan's ids. An id the scan does
 *  not (or no longer) offer is skipped with reason `not_offered`. */
export interface CleanupSelection {
  group: CleanupGroupKey | string;
  entry_ids: string[];
}

/** `POST /cleanup/apply`'s body. `dry_run: true` lists exactly what would
 *  go and deletes nothing. */
export interface CleanupApplyRequest {
  selections: CleanupSelection[];
  dry_run: boolean;
}

/** One entry's outcome. In a dry run `files`/`bytes`/`rows` are what would
 *  go and `paths` the exact files; after a real run, what went. */
export interface CleanupEntryResult {
  group: CleanupGroupKey | string;
  id: string;
  label: string;
  files: number;
  bytes: number;
  rows: number;
  skipped: SkippedFile[];
  paths: string[];
}

/** What an apply did — or, in a dry run, would do. */
export interface CleanupApplyResult {
  dry_run: boolean;
  deleted_files: number;
  freed_bytes: number;
  removed_rows: number;
  /** Every skip of every entry, with its reason. */
  skipped: SkippedFile[];
  entries: CleanupEntryResult[];
}

/** Apply a selection from the scan through the existing deletion gates —
 *  or, with `dry_run`, preview it. The whole request is refused (an
 *  error, nothing deleted) while a selected dataset is busy, a selected
 *  run has not finished or a selected download is still going; within a
 *  group each entry is best-effort with skip reasons. Only a real run
 *  writes the cleanup history. */
export const cleanupApply = (req: CleanupApplyRequest) =>
  invoke<CleanupApplyResult>("cleanup_apply", { body: req });

/** One line of the cleanup history (`GET /cleanup/log`). */
export interface CleanupLogEntry {
  id: string;
  /** RFC 3339, UTC. */
  ts: string;
  group_key: CleanupGroupKey | string;
  entry_id: string;
  entry_label: string;
  deleted_files: number;
  freed_bytes: number;
  removed_rows: number;
  skipped_count: number;
  /** Skip reasons and a capped path list. */
  detail: { skipped?: SkippedFile[]; paths?: string[] };
}

/** The newest cleanup history rows, newest first (the page shows 20). */
export const cleanupLog = (limit = 20) => invoke<CleanupLogEntry[]>("cleanup_log", { limit });
/** Delete a model — its file, links and DB rows. Permanent; refused while
 *  the model is loaded. */
export const deleteModel = (id: string) => invoke<DeleteOutcome>("delete_model", { id });
/** Manually free a resident model's VRAM/RAM right now, without waiting for
 *  the scheduler to evict it for something else. Works for any runtime --
 *  looked up by whichever one currently has the model loaded. */
export const unloadModel = (id: string) => invoke<void>("unload_model", { id });

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

/** Rename a model's display name -- never touches the file on disk. */
export const renameModel = (id: string, name: string) =>
  invoke<Model>("rename_model", { id, name });

// --- packages (Plan 14) ---------------------------------------------------

/** How a model's base family was decided — `models.family_source`. */
export type FamilySource = "civitai" | "hf" | "catalog" | "header" | "name" | "user";

export type Runnable = { kind: "yes" } | { kind: "no"; reason: string };

export interface FamilyRef {
  /** Registry id, e.g. `pony`, `flux2-klein-9b`. */
  id: string;
  label: string;
  /** Families in one group load each other's LoRAs (Pony works with SDXL). */
  arch_group: string;
  /** `null` on a library group (its members carry their own). */
  source: FamilySource | null;
  runnable: Runnable;
}

/** `base` | `vae` | `text_encoder`, or another catalog kind verbatim. */
export type NeedRole = "base" | "vae" | "text_encoder" | (string & {});

/** A checkpoint Civitai offers for a base label (top by downloads). */
export interface CheckpointCandidate {
  model_id: string;
  version_id: string;
  name: string;
  downloads: number;
  base_model: string;
  nsfw: boolean;
  preview_image_url: string | null;
  file: {
    path: string;
    size: number;
    sha256: string | null;
    download_url: string | null;
  } | null;
}

export type NeedStatus =
  | { kind: "installed"; model_id: string; name: string; made_for: boolean }
  | {
      kind: "catalog";
      known_model_id: string;
      name: string;
      size_bytes: number;
      sha256: string;
      url: string;
    }
  | {
      kind: "findable";
      base_label: string;
      candidates: CheckpointCandidate[];
      /** Why `candidates` is empty (offline, lookup failed). */
      note: string | null;
    }
  | { kind: "not_runnable"; reason: string };

export interface Need {
  role: NeedRole;
  label: string;
  /** A suggestion (the base it was made for), not a requirement. */
  optional: boolean;
  status: NeedStatus;
}

export type PackageVerdict =
  | { kind: "ready" }
  | { kind: "needs_download" }
  | { kind: "not_runnable"; reason: string }
  | { kind: "unknown_base" };

export interface PackageItem {
  name: string;
  kind: "lora" | "checkpoint" | "other";
  base_label: string | null;
  model_id: string | null;
  size_bytes: number | null;
}

export interface Package {
  item: PackageItem;
  family: FamilyRef | null;
  needs: Need[];
  /** Required catalog bytes still missing. */
  missing_bytes: number;
  verdict: PackageVerdict;
}

export interface GroupModel {
  model: Model;
  family_source: FamilySource;
}

export interface LibraryGroup {
  family: FamilyRef;
  base: GroupModel | null;
  /** When `base` is null: what would provide one. */
  base_needs: Need[];
  companions: Need[];
  loras: GroupModel[];
  missing_bytes: number;
  complete: boolean;
}

export interface LibraryPackages {
  groups: LibraryGroup[];
  /** LoRAs whose base family could not be inferred. */
  orphans: Model[];
}

export type ResolvePackageQuery =
  | { source: "civitai"; model_id: string; version_id?: string; nsfw?: boolean }
  | { source: "library"; model_id: string };

export interface BaseFamilyChoice {
  model_id: string;
  family: string;
}

export interface SavedFamily {
  model_id: string;
  outcome: "written" | "kept_user_choice" | "skipped";
  reason: string | null;
}

/** The package a Civitai pick or a library model needs (`GET /packages/resolve`). */
export const resolvePackage = (query: ResolvePackageQuery) =>
  invoke<Package>("resolve_package", { query });
/** The library grouped by base family — reads only (`GET /packages/library`). */
export const libraryPackages = () => invoke<LibraryPackages>("library_packages");
/** "What base is this?" — the user's choice, never overwritten by a detection. */
export const setModelBaseFamily = (id: string, family: string) =>
  invoke<Model>("set_model_base_family", { id, family });
/** Persist a previewed batch of detected families; user choices are kept. */
export const saveBaseFamilies = (batch: BaseFamilyChoice[]) =>
  invoke<SavedFamily[]>("save_base_families", { batch });

/** The registry health line for Diagnostics (`GET /registry/status`). */
export interface RegistryStatus {
  source_id: string;
  last_fetch: string | null;
  rate_limit_remaining: number | null;
  /** Seconds until the rate-limit window clears — set only while limited. */
  rate_limited_secs: number | null;
  token_set: boolean;
  cache_entries: number;
  /** The host this source talks to, e.g. `https://civitai.red` — link a
   *  result to the front door the app actually uses. */
  base_url: string;
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

/** `KnownModel.media` / `ModelStack.media` — the Models-tab catalog
 *  category (`core::model::catalog`). */
export type CatalogMedia = "image" | "video" | "voice" | "training";

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
  /** Which media type / catalog category this entry belongs to —
   *  `"training"` = the dataset captioners (Training & captioning). */
  media: CatalogMedia;
  fit: FitVerdict;
}

/** A base image/video model bundled with every companion file it needs to
 *  actually run (`GET /models/stacks`) — e.g. Flux's diffusion model + T5 +
 *  CLIP-L + VAE, one "Download entire stack" instead of hunting down each
 *  piece separately. */
export interface ModelStack {
  id: string;
  label: string;
  media: CatalogMedia;
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

/** Hi-Res-Fix: render once at `width`×`height`, then upscale that latent and
 *  re-sample it at a low denoise. Only the text-to-image paths honour it —
 *  an image *edit* and Story Studio's reference-anchored renders ignore it.
 *  Every field is clamped on the Rust side (`scale_by` 1.25–2, `denoise`
 *  0.2–0.7, `steps` 4–60) and written back resolved. */
export interface HiresFixParams {
  /** How much bigger the second pass runs. Default 1.5. */
  scale_by: number;
  /** The second pass's denoise. Default 0.45. */
  denoise: number;
  /** Second-pass steps; defaults to half the first pass's. */
  steps?: number;
  /** One of ComfyUI's `LatentUpscaleBy` methods (`nearest-exact` — the
   *  default —, `bilinear`, `area`, `bicubic`, `bislerp`). */
  upscale_method?: string;
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
  /** Set on an instruction-based *edit* of an existing image (`prompt` is
   *  the instruction, not a generation prompt) — a finished job's id, or a
   *  path. Only FLUX.2 [klein] supports this. */
  source_image?: string;
  /** Set (or `null`) once the engine has resolved the request. */
  hires?: HiresFixParams | null;
  /** The finished pixel size, written back only when `hires` is set — a
   *  single-pass render always finishes at `width`×`height`. */
  output_width?: number;
  output_height?: number;
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

/** The parameters of a `job_type=upscale` job — NVIDIA's RTX Video Super
 *  Resolution, run on an already-finished `image`/`video` job's output (or a
 *  raw file path). Only `source` is required; the rest fall back to the
 *  node's own defaults (2x scale, `ULTRA` quality) on the Rust side, so a
 *  one-click "Upscale" button can submit just `{ source }`. */
export interface UpscaleParams {
  /** A finished `image`/`video` job's id, or a path to a file on disk. */
  source: string;
  /** `"scale"` (default) or `"dimensions"`. */
  resize_mode?: "scale" | "dimensions";
  /** Used when `resize_mode` is `"scale"` (or omitted) — default 2.0. */
  scale?: number;
  /** Used when `resize_mode` is `"dimensions"`. */
  width?: number;
  height?: number;
  /** `"LOW"` | `"MEDIUM"` | `"HIGH"` | `"ULTRA"` (default). */
  quality?: "LOW" | "MEDIUM" | "HIGH" | "ULTRA";
}

/** URL the loopback core serves a finished job's output file from — a PNG for
 *  image jobs, an MP4 for video jobs. Used as an `<img>` / `<video>` src; the
 *  CSP allows `http://127.0.0.1:*` for both `img-src` and `media-src`. */
export const jobOutputUrl = (coreApiPort: number, jobId: string) =>
  `http://127.0.0.1:${coreApiPort}/jobs/${jobId}/output`;
/** @deprecated use {@link jobOutputUrl} */
export const imageOutputUrl = jobOutputUrl;

// --- live render progress (real ComfyUI /ws, not polling) -----------------

/** One reading of a job's render progress, sourced straight from ComfyUI's
 *  own `/ws` (`core::progress`). In-memory on the core side only -- it exists
 *  purely to watch a render in flight, not to look up after the fact. */
export interface JobProgress {
  job_id: string;
  /** 0..100 when ComfyUI's `progress` event carried both a value and a max. */
  percent: number | null;
  step: number | null;
  steps_total: number | null;
  /** The node ComfyUI is currently executing, when it has reported one. */
  node: string | null;
}

/** `ws://127.0.0.1:<port>/ws/jobs/<jobId>` — the loopback server's real
 *  per-job progress stream (`GET /ws/jobs/{id}`). Connected directly from the
 *  webview with a plain `WebSocket`, the same way the `<img>`/`<video>`
 *  output URLs hit the loopback server directly — there is no Tauri command
 *  for this. */
export const jobProgressWsUrl = (coreApiPort: number, jobId: string) =>
  `ws://127.0.0.1:${coreApiPort}/ws/jobs/${jobId}`;

// --- gallery: save-to-disk (Tauri sandbox needs a real save dialog) -------

/** Copy a finished job's output file to `destPath` on disk (the save dialog
 *  already picked it). `<a download>` is inert inside Tauri's webview
 *  sandbox, so the actual byte-copy happens here, Rust-side. */
export const saveJobOutput = (id: string, destPath: string) =>
  invoke<void>("save_job_output", { id, destPath });

/** Download a finished job's output: opens the native save dialog, then
 *  copies the bytes there via [`saveJobOutput`]. Resolves quietly — no
 *  thrown error — when the user cancels the dialog, or when there is no
 *  Tauri dialog plugin to call at all (e.g. the browser dev-mock preview),
 *  mirroring how the existing "Browse…" file pickers treat that case. */
export async function downloadJobOutput(
  job: Pick<Job, "id" | "job_type" | "output_path">,
): Promise<void> {
  const ext =
    job.output_path?.split(".").pop()?.toLowerCase() || (job.job_type === "video" ? "mp4" : "png");
  let dest: string | null;
  try {
    const { save } = await import("@tauri-apps/plugin-dialog");
    dest = await save({
      defaultPath: `${job.id}.${ext}`,
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
    });
  } catch {
    return; // not running inside Tauri (or the plugin refused) — no-op
  }
  if (!dest) return; // user cancelled the dialog
  await saveJobOutput(job.id, dest);
}

/** Whether a dataset's items are still frames or whole (trimmed) clips. */
export type DatasetMode = "frames" | "clips";

/** How a composed caption is worded: tags first (Anime/SDXL) or prose first
 *  (FLUX.2). Chosen per export, not per dataset. */
export type CaptionOrder = "tags_first" | "prose_first";

/** What a captioner produces: a sentence or a comma-separated tag list. */
export type CaptionStyle = "prose" | "tags";

/** A curation set as a first-class object — it outlives the `dataset_prep`
 *  job that produced it (`prep_job_id` can dangle once that job is deleted). */
export interface Dataset {
  id: string;
  name: string;
  mode: DatasetMode;
  source_root: string;
  /** Prepended to every composed caption at export; `""` when unset. */
  trigger_word: string;
  prep_job_id: string | null;
  /** Where it was last exported to, for the "again, same folder" button. */
  export_dir: string | null;
  /** The absolute work folder its frames were extracted into
   *  (`<data_dir>/<prep_job_id>`); `null` for datasets prepared before the
   *  location was recorded. */
  work_dir: string | null;
  created_at: string;
}

/** One entry of the captioner registry plus whether its files are in the
 *  model library — what the "Beschreiben mit" dropdown filters on. */
export interface Captioner {
  id: string;
  name: string;
  style: CaptionStyle;
  /** Model-library role its files are imported under. */
  role: string;
  /** 0 for a CPU-only onnxruntime tagger. */
  vram_mb: number;
  license: string;
  /** Whether frame-X-vs-X+N temporal escalation applies on top of it. */
  supports_escalation: boolean;
  required_files: string[];
  /** `false` too when the files are all there but fail the load-time
   *  integrity check — see `unusable`. */
  installed: boolean;
  /** Why a complete set of files cannot be used (tampered, missing or extra
   *  file in the pinned folder); `null` when installed or simply absent. */
  unusable: string | null;
  /** A known problem that keeps this captioner from working right now (in
   *  words for the person picking one). While set, the UI offers neither
   *  selection nor install — and the core refuses a run that asks for it. */
  known_issue: string | null;
}

/** The Qwen2.5-VL escalation model (`GET /captioners/escalation`) — not a
 *  captioner of its own, but the same "files there, yet unusable" case. */
export interface EscalationStatus {
  files_present: boolean;
  usable: boolean;
  reason: string | null;
}

/** A concept the dataset teaches, with its frame count and the inline warning
 *  shown when the token is an ordinary word the base model already knows. */
export interface DatasetConcept {
  id: string;
  dataset_id: string;
  name: string;
  token: string;
  description: string;
  created_at: string;
  frame_count: number;
  token_warning: string | null;
}

/** How much of an "Alle im Set" batch actually landed — frames already
 *  carrying the concept, and frames from another dataset, are skipped. */
export interface AssignedSummary {
  requested: number;
  attached: number;
}

/** The parameters of a `job_type=dataset_prep` job — the dataset-prep
 *  pipeline (ingest -> ffmpeg frame extraction -> blur/duplicate filtering ->
 *  captioning). Only `root` is required; the rest fall back to the Rust
 *  side's own defaults. */
export interface DatasetPrepParams {
  /** Folder tree root — files directly in it are tagged with its name, each
   *  immediate subfolder becomes a tag of its own. */
  root: string;
  /** Still frames (default) or whole clips as the dataset's items. */
  mode?: DatasetMode;
  /** Captioner id (see {@link listCaptioners}); `null` skips captioning. */
  captioner?: string | null;
  /** Clips mode: cap on frames sampled per clip for its caption. */
  max_frames_per_clip?: number;
  /** Clips mode: shorter clips are dropped. */
  min_clip_secs?: number;
  /** Frames sampled per second of video (default ~1.5). */
  sample_fps?: number;
  /** Variance-of-Laplacian cutoff below which a frame is dropped as blurry. */
  blur_threshold?: number;
  /** Hamming-distance cutoff (of a 64-bit perceptual hash) for near-duplicates. */
  phash_max_distance?: number;
  /** Re-caption a low-confidence video frame with Qwen2.5-VL + a nearby frame. */
  escalate?: boolean;
  escalate_every_nth?: number;
  context_offset?: number;
  /** "Store frames in": the work folder becomes `<data_dir>\<prep job id>`.
   *  Omit for the default datasets folder (Settings → Data locations). The
   *  core refuses a folder that overlaps a source or another dataset. */
  data_dir?: string;
}

/** One item of a curation set (`GET /datasets/{id}/frames`) — a still frame,
 *  or in clips mode the clip itself. */
export interface DatasetFrame {
  id: string;
  /** The prep job that produced it; `null` once that job has been deleted —
   *  the item lives on with its dataset. */
  job_id: string | null;
  dataset_id: string | null;
  /** The folder name this item's source lived under. */
  tag: string;
  source_path: string;
  frame_path: string;
  /** Position within the source video; `null` for a plain image file. */
  timestamp_secs: number | null;
  caption: string;
  /** `"florence2"` | `"wd-eva02-tagger-v3"` | `"qwen2.5-vl"` | `""` (not
   *  captioned yet, or hand-edited). */
  caption_engine: string;
  excluded: boolean;
  /** `""` = kept; otherwise why the pipeline dropped it (`"blur"`, …, or
   *  `"duplicate_global"` from the dataset-wide dedup). A rejected item stays
   *  visible so the curator can put it back. */
  rejection_reason: string;
  /** Clips mode: the clip's own length; `null` for a still frame. */
  duration_secs: number | null;
  /** Curator-set in/out points; `null` means the clip's natural bound. */
  clip_start_secs: number | null;
  clip_end_secs: number | null;
  created_at: string;
}

export const listDatasetFrames = (jobId: string) =>
  invoke<DatasetFrame[]>("list_dataset_frames", { jobId });

/** URL the loopback core serves one curated frame's still image from — same
 *  shape as {@link jobOutputUrl}. Job-keyed, so only usable while the item
 *  still has a `job_id`; prefer {@link datasetFrameImageUrlByDataset}. */
export const datasetFrameImageUrl = (coreApiPort: number, jobId: string, frameId: string) =>
  `http://127.0.0.1:${coreApiPort}/jobs/${jobId}/dataset-frames/${frameId}/image`;

/** The dataset-keyed still image — what the curation grid uses now that
 *  `frame.job_id` can be `null` (an item outlives its prep job). */
export const datasetFrameImageUrlByDataset = (
  coreApiPort: number,
  datasetId: string,
  frameId: string,
) => `http://127.0.0.1:${coreApiPort}/datasets/${datasetId}/frames/${frameId}/image`;

export const updateDatasetFrame = (
  frameId: string,
  body: {
    caption?: string;
    excluded?: boolean;
    /** Clear an automatic rejection — "doch behalten". */
    restore?: boolean;
    /** Omit to keep the stored bound, `null` to clear it back to the clip's
     *  natural start/end, a number to set it. */
    clip_start_secs?: number | null;
    clip_end_secs?: number | null;
  },
) => invoke<DatasetFrame>("update_dataset_frame", { frameId, body });

export interface ExportDatasetSummary {
  exported: number;
  dest_dir: string;
}

export const exportDataset = (jobId: string, destDir: string) =>
  invoke<ExportDatasetSummary>("export_dataset", { jobId, destDir });

export const listCaptioners = () => invoke<Captioner[]>("list_captioners");
export const escalationStatus = () => invoke<EscalationStatus>("escalation_status");

export const listDatasets = () => invoke<Dataset[]>("list_datasets");

export const getDataset = (id: string) => invoke<Dataset | null>("get_dataset", { id });

export const updateDataset = (id: string, body: { trigger_word?: string }) =>
  invoke<Dataset>("update_dataset", { id, body });

/** A file a deletion left alone. `reason` is `"outside_app_folders"` (not the
 *  app's to delete), `"source_file"` (a source video/image — never deleted),
 *  `"in_use"` (another remaining frame still shows it), `"used_by_other_dataset"`
 *  (another dataset's frame or export uses it), `"not_a_file"`, or
 *  `"error: …"` (deleting failed; the frame row is kept for a retry). */
export interface SkippedFile {
  path: string;
  reason: string;
}

/** `GET /datasets/{id}/usage` — measured by walking the folders. */
export interface DatasetUsage {
  /** The app-owned work folder; `null` when it no longer exists or the
   *  dataset has no prep job. */
  work_dir: string | null;
  /** `true`: deleting the dataset removes the whole work folder. `false`:
   *  only this dataset's own frame files go, because other data points into
   *  the folder (or there is none). */
  work_walkable: boolean;
  /** What deleting the dataset frees from the work folder — the whole folder
   *  when `work_walkable`, else this dataset's own deletable frame files. */
  work_bytes: number;
  work_files: number;
  /** The last export destination, app-owned or not. */
  export_dir: string | null;
  /** Bytes of the export's numbered files, measured only when app-owned. */
  export_bytes: number;
  /** `true` when the export lies inside the outputs folder, so deleting the
   *  dataset removes its numbered files too. */
  export_app_owned: boolean;
  /** Excluded + rejected frames, and the bytes a cleanup would free. */
  discarded_frames: number;
  discarded_bytes: number;
}

/** What deleting a dataset did — its files went too. */
export interface DatasetDeleteSummary {
  frames: number;
  deleted_files: number;
  freed_bytes: number;
  skipped_files: SkippedFile[];
  /** A user-chosen (or shared) export folder, left as is. */
  export_dir_kept: string | null;
  /** `false` when a file could not be deleted (see `skipped_files`, reason
   *  `"error: …"`): the dataset stays so the delete can be retried. */
  dataset_deleted: boolean;
}

/** Deletes the dataset's rows *and* files (work folder, app-owned export).
 *  Rejects while a training run of the dataset is still active. */
export const deleteDataset = (id: string) =>
  invoke<DatasetDeleteSummary>("delete_dataset", { id });

export const datasetUsage = (datasetId: string) =>
  invoke<DatasetUsage>("dataset_usage", { datasetId });

export interface BulkUpdatedSummary {
  requested: number;
  /** Unknown ids and ids of another dataset are skipped. */
  updated: number;
}

/** Move a whole selection to Keep (`excluded: false`, which also clears a
 *  filter rejection) or Discard (`excluded: true`) in one request. */
export const bulkUpdateDatasetFrames = (
  datasetId: string,
  frameIds: string[],
  excluded: boolean,
) =>
  invoke<BulkUpdatedSummary>("bulk_update_dataset_frames", {
    datasetId,
    body: { frame_ids: frameIds, excluded },
  });

export interface FramesDeleteSummary {
  /** Frame rows deleted. */
  deleted: number;
  deleted_files: number;
  freed_bytes: number;
  skipped_files: SkippedFile[];
}

/** Irreversible: deletes the frames' files and rows. */
export const deleteDatasetFrames = (datasetId: string, frameIds: string[]) =>
  invoke<FramesDeleteSummary>("delete_dataset_frames", {
    datasetId,
    body: { frame_ids: frameIds },
  });

/** In a dry run `frames`/`bytes` are what a real run would delete and free;
 *  after a real run, what it did. */
export interface CleanupSummary {
  dry_run: boolean;
  frames: number;
  bytes: number;
  deleted_files: number;
  skipped_files: SkippedFile[];
}

/** Delete every discarded (excluded or rejected) frame with its file, or
 *  with `dryRun` only measure what that would free. */
export const cleanupDataset = (datasetId: string, dryRun: boolean) =>
  invoke<CleanupSummary>("cleanup_dataset", { datasetId, body: { dry_run: dryRun } });

/** Default and upper bound of the dedup threshold (Hamming distance). */
export const DEFAULT_DEDUP_THRESHOLD = 6;
export const MAX_DEDUP_THRESHOLD = 16;

/** The core refuses more frame ids than this in one bulk move or frame
 *  delete (`check_frame_ids`); callers split larger selections. */
export const MAX_FRAME_IDS = 10_000;

export interface DedupSummary {
  /** The threshold actually used, after defaulting and clamping. */
  threshold: number;
  /** Kept frames looked at. */
  scanned: number;
  /** Groups of near-duplicates; each keeps its sharpest frame. */
  groups: number;
  /** Frames newly marked `duplicate_global`. */
  marked: number;
  /** Frames whose image could not be read; left untouched. */
  unreadable: number;
}

/** Mark near-duplicates across the whole dataset `duplicate_global`
 *  (reversible: moving a frame to Keep clears the mark). */
export const dedupDataset = (datasetId: string, threshold?: number) =>
  invoke<DedupSummary>("dedup_dataset", {
    datasetId,
    body: threshold === undefined ? {} : { threshold },
  });

export const listDatasetFramesForDataset = (datasetId: string) =>
  invoke<DatasetFrame[]>("list_dataset_frames_for_dataset", { datasetId });

/** frame id -> concept ids; items without a concept are simply absent. */
export const frameConceptMap = (datasetId: string) =>
  invoke<Record<string, string[]>>("frame_concept_map", { datasetId });

export const listConcepts = (datasetId: string) =>
  invoke<DatasetConcept[]>("list_concepts", { datasetId });

/** The created row as stored. It carries no `frame_count`/`token_warning`:
 *  those are computed by {@link listConcepts}, and a fresh concept has no
 *  frames yet — refetch the list rather than splicing this in. */
export const createConcept = (
  datasetId: string,
  body: { name: string; token: string; description?: string },
) =>
  invoke<Omit<DatasetConcept, "frame_count" | "token_warning">>("create_concept", {
    datasetId,
    body,
  });

export const updateConcept = (
  id: string,
  body: { name: string; token: string; description?: string },
) => invoke<void>("update_concept", { id, body });

export const deleteConcept = (id: string) => invoke<void>("delete_concept", { id });

export const assignConcept = (conceptId: string, frameIds: string[]) =>
  invoke<AssignedSummary>("assign_concept", { conceptId, body: { frame_ids: frameIds } });

export const unassignConcept = (conceptId: string, frameIds: string[]) =>
  invoke<void>("unassign_concept", { conceptId, body: { frame_ids: frameIds } });

/** The full export: composed captions in the chosen order, and in clips mode
 *  the trimmed source clips. */
export const exportDatasetById = (
  datasetId: string,
  destDir: string,
  captionOrder: CaptionOrder,
) =>
  invoke<ExportDatasetSummary>("export_dataset_by_id", {
    datasetId,
    body: { dest_dir: destDir, caption_order: captionOrder },
  });

/** What kind of file is being imported. `chat` → GGUF LLM for llama.cpp; the
 *  rest are ComfyUI image / video models routed to their typed store folder. */
export type ModelType =
  | "chat"
  | "checkpoint"
  | "diffusion_model"
  | "vae"
  | "lora"
  | "text_encoder"
  | "video"
  | "voice_model"
  | "voice_data"
  /** A CLIP vision encoder, or an IP-Adapter weight file — the two
   *  supporting models Story Studio Phase 2's SDXL character-consistency
   *  path needs (`core::model::ModelKind::ClipVision`/`IpAdapter`). */
  | "clip_vision"
  | "ip_adapter"
  /** The WD EVA02 tagger's `model.onnx` + `selected_tags.csv` — the
   *  Danbooru-tag captioner the Dataset tab offers alongside Florence-2
   *  (`core::model::ModelKind::WdTagger`). */
  | "wd_tagger"
  /** One file of the Dia narrator engine or its separate DAC audio codec —
   *  see `core::model::ModelKind::DiaEngine`/`DiaCodec`. Each lands in its
   *  own fixed subdirectory under its original Hugging Face filename;
   *  unlike every other kind, importing one file at a time is expected —
   *  "Download entire stack" on the Models tab does that automatically. */
  | "dia_engine"
  | "dia_codec"
  /** One file of the Florence-2 large / Qwen2.5-VL 7B captioner snapshot
   *  directories (`core::model::ModelKind::Florence2Engine`/`QwenVlEngine`)
   *  — same one-file-at-a-time, original-filename shape as Dia; installed
   *  via "Download entire stack". */
  | "florence2_engine"
  | "qwen_vl_engine";

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

/** One search hit — shared by every Discover source (`GET /registry/search`
 *  for Hugging Face, `GET /civitai/search` for Civitai). A field a given
 *  source has no concept of stays at its neutral default rather than being
 *  fabricated — see each field's comment for which source(s) populate it. */
/** One sample of a model's gallery. `nsfw_level` is Civitai's own per-image
 *  rating (1 = safe): a model that is not flagged NSFW can still carry spicier
 *  samples, so anything above 1 stays hidden unless "Show NSFW" is on. */
export interface RemotePreview {
  url: string;
  is_video: boolean;
  nsfw_level: number;
}

export interface RemoteModel {
  /** Hugging Face: `owner/repo`. Civitai: the numeric model id, as a string. */
  id: string;
  /** A separate display title — Civitai's `name` (e.g. "Pony Diffusion V6
   *  XL"). `null` for Hugging Face, where `id` already reads as a title. */
  name: string | null;
  author: string | null;
  downloads: number;
  likes: number;
  trending_score: number | null;
  created_at: string | null;
  last_modified: string | null;
  pipeline_tag: string | null;
  library_name: string | null;
  gated: "no" | "auto" | "manual";
  /** Licence slug (Hugging Face only — Civitai has no slug; see
   *  `allow_commercial_use`). */
  license: string | null;
  /** The upstream repo this is a quant / fine-tune of. Hugging Face only. */
  base_model: string | null;
  tags: string[];
  param_count: number | null;
  arch: string | null;
  ctx_max: number | null;
  precision: string | null;
  format: "gguf" | "safetensors" | "other";
  /** Civitai's own NSFW flag. Always `false` for Hugging Face. The Discover
   *  UI's Civitai search already defaults to excluding NSFW results — this
   *  is only surfaced as defense in depth for a mislabelled result. */
  nsfw: boolean;
  /** A representative preview image (Civitai only). */
  preview_image_url: string | null;
  /** The primary version's sample gallery (Civitai), capped server-side at 12;
   *  empty for a source without one. Nothing is loaded until the row's
   *  "Previews" button is pressed. */
  previews: RemotePreview[];
  /** Civitai's `allowCommercialUse` flags (e.g. `["Image", "Sell"]`) — this,
   *  not `license`, is Civitai's real commercial-terms signal. Empty for
   *  Hugging Face. */
  allow_commercial_use: string[];
  /** Civitai's own `type` (`"Checkpoint"`, `"LORA"`, …), verbatim — a hint at
   *  which `ModelType` to import a file from this model as. `null` for
   *  Hugging Face (use `format` / `importTypeFor` instead). */
  model_kind_hint: string | null;
  /** Civitai's foundation-model family label (e.g. `"SDXL 1.0"`, `"Pony"`) —
   *  distinct from `base_model`, which is an upstream *repo id*. `null` for
   *  Hugging Face. */
  base_model_family: string | null;
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
  /** The source's own reported hash — a pre-download sanity check / dedup
   *  key only. The download manager always re-hashes the bytes it actually
   *  receives and treats *that* as the source of truth, never this field. */
  sha256: string | null;
  quant: string | null;
  /** `[index, total]` for a split file. Hugging Face only. */
  shard: [number, number] | null;
  /** Hugging Face: the `/resolve/` URL. Civitai: the file's own `downloadUrl`.
   *  Either way — fed to `enqueueDownload` (6.4) or "Copy link". */
  download_url: string;
  vram_estimate_mb: number | null;
  fit: FitVerdict;
  /** Civitai's own malware-scan verdicts (`"Success"`, `"Danger"`,
   *  `"Pending"`, …), verbatim — always show a non-`"Success"` value
   *  prominently, never hide it. `null` for Hugging Face. Informational
   *  only: never a substitute for the server's own import-time Pickle-format
   *  guard, which runs unconditionally regardless of what a source reports. */
  pickle_scan_result: string | null;
  virus_scan_result: string | null;
}

/** One version of a remote model (`registry::RemoteVersion`). */
export interface RemoteVersion {
  id: string;
  name: string | null;
  /** Civitai's `baseModel` label (`"Pony"`, `"SDXL 1.0"`, …). */
  base_model: string | null;
}

export interface RegistryDetails extends RemoteModel {
  revision: string;
  files: RegistryFile[];
  /** Every version, newest first, with the base each targets — a Civitai
   *  model's versions can target different bases. Empty for Hugging Face. */
  versions?: RemoteVersion[];
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

// --- Civitai (image/video checkpoints + LoRAs) ---

export interface CivitaiSearchParams {
  q?: string;
  /** Civitai `types` values, e.g. `["Checkpoint", "LORA"]`. */
  types?: string[];
  sort?: "downloads" | "likes" | "trending" | "new";
  /** Include NSFW-flagged results. **Omit or `false`** to exclude them — the
   *  Discover UI's Civitai panel defaults its own toggle to off and the
   *  server independently defaults to `false` too, so a caller can never
   *  accidentally widen this by forgetting the flag. */
  nsfw?: boolean;
  limit?: number;
}

/** The Civitai registry health line for Diagnostics (`GET /civitai/status`). */
export const civitaiStatus = () => invoke<RegistryStatus>("civitai_status");
/** Set (blank clears) the Civitai API key — a machine-local file, never in a
 *  backup. Only needed for gated/early-access content; anonymous browsing
 *  works without one. Takes effect on the next restart. */
export const setCivitaiToken = (token: string) => invoke<void>("set_civitai_token", { token });

/** Search Civitai for image/video checkpoints and LoRAs. Offline-gated
 *  (ADR-009). Defaults to excluding NSFW results — see `CivitaiSearchParams.nsfw`.
 *
 *  `types` is joined into a single comma-separated string on the wire (not a
 *  JSON array): the same `CivitaiSearchDto` also deserializes from an HTTP
 *  query string (the loopback server's `GET /civitai/search`), which can't
 *  reliably parse a `Vec` from repeated keys — see the Rust-side doc on
 *  `CivitaiSearchDto::types`. */
export const civitaiSearch = (params: CivitaiSearchParams) =>
  invoke<RegistrySearchResult>("civitai_search", {
    params: {
      ...params,
      types: params.types && params.types.length > 0 ? params.types.join(",") : undefined,
    },
  });
/** One Civitai model's primary version, with every file's size, hash, quant,
 *  fit, and Civitai's own pickle/virus-scan verdicts. `id` is the numeric
 *  Civitai model id, as a string. */
export const civitaiModel = (id: string) => invoke<RegistryDetails>("civitai_model", { id });

// --- tag/vibe model recommendations (core::recommend) ---
// No dedicated IPC call: a recommendation is a `job_type: "recommend"` job,
// submitted and polled through the same `submitJob`/`jobDetail` every other
// job uses. `job.result` (once `completed`) is this `RecommendReport`,
// JSON-encoded.

export type RecommendKind = "chat" | "coding" | "image" | "video" | "lora";

export interface RecommendCandidate {
  id: string;
  why: string;
  downloads: number;
  likes: number;
  last_modified: string | null;
  param_count: number | null;
  format: "gguf" | "safetensors" | "other";
  gated: boolean;
  tags: string[];
  fit: FitVerdict;
  llm_ranked: boolean;
}

export interface RecommendReport {
  query: string;
  candidates: RecommendCandidate[];
  note: string;
  freshness: Freshness;
}

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
  /** `civitai:<model>/<version>` | `hf:<repo>@<rev>` — recorded on the model. */
  origin: string | null;
  /** Registry id of the base family the source's label maps to. */
  base_family: string | null;
  family_source: FamilySource | null;
}

/** Where a download comes from — recorded on the imported model (Plan 14). */
export interface DownloadOrigin {
  source: "civitai" | "hf";
  /** Civitai's numeric model id, or the Hugging Face `owner/repo`. */
  model_id: string;
  /** Civitai version id, or the Hugging Face revision (`main` when omitted). */
  version?: string;
  /** Civitai's `baseModel` label, or Hugging Face's `base_model`. */
  base_model?: string;
}

export interface EnqueueDownloadBody {
  url: string;
  filename: string;
  model_type?: string;
  sha256?: string;
  size_bytes?: number;
  /** Roles to stamp on the model once imported (e.g. `["chat", "coding"]`). */
  roles?: string[];
  /** Source metadata so the import records origin + base family. */
  origin?: DownloadOrigin;
}

export const listDownloads = () => invoke<Download[]>("list_downloads");
/** Queue a download → verify against the SHA-256 → import into the store.
 *  Offline-gated. */
export const enqueueDownload = (body: EnqueueDownloadBody) =>
  invoke<Download>("enqueue_download", { body });
export const pauseDownload = (id: string) => invoke<void>("pause_download", { id });
export const resumeDownload = (id: string) => invoke<void>("resume_download", { id });
export const cancelDownload = (id: string) => invoke<void>("cancel_download", { id });
/** Remove one finished (done/failed) download from the history — refuses one
 *  still in progress. Never touches an already-imported model. */
export const deleteDownload = (id: string) => invoke<void>("delete_download", { id });
/** Clear every finished download at once; returns how many were removed. */
export const clearFinishedDownloads = () => invoke<number>("clear_finished_downloads");

// --- benchmarks (Phase 6.5) ----------------------------------------------

/** One prompt inside a {@link BenchSuite}. `text` is what the model is sent
 *  verbatim — long enough that the picker should only ever preview it. */
export interface BenchSuitePrompt {
  id: string;
  title: string;
  text: string;
}

/** A fixed, versioned set of prompts run at a fixed generation length. The id
 *  carries the version (`chat-v1`): changing a prompt means a new id, so
 *  stored rows stay comparable. */
export interface BenchSuite {
  id: string;
  title: string;
  /** What the suite measures, in the honest sense — speed, never quality. */
  description: string;
  /** Tokens generated per pass. */
  max_tokens: number;
  prompts: BenchSuitePrompt[];
}

/** What one suite prompt contributed to a run, averaged over its own passes
 *  (`Benchmark.detail[]`). */
export interface BenchPromptResult {
  /** Matches a {@link BenchSuitePrompt.id}. */
  prompt_id: string;
  /** Mean generated tokens per pass, rounded. */
  tokens: number;
  /** The token cap those passes ran at. `tokens` well below it means the model
   *  stopped early, so its tok/s covers a shorter run than the other prompts. */
  max_tokens: number;
  gen_tps: number | null;
  prompt_tps: number | null;
}

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
  /** Id of the suite this run used; null for the Model Library's quick test. */
  suite: string | null;
  /** Per-prompt breakdown; null without a suite. */
  detail: BenchPromptResult[] | null;
  created_at: string;
}

/** Whether `POST /models/{id}/benchmark` will accept this model at all. The
 *  core refuses anything but llama.cpp / GGUF ("benchmarking is llama.cpp /
 *  GGUF models only for now"), so both the Model Library's "Test model" button
 *  and the Benchmark tab's picker gate on exactly this, rather than each
 *  guessing its own rule and drifting from the API. */
export const isBenchmarkable = (model: Model): boolean => model.format === "gguf";

/** Whether each job state still has work ahead of it — queued, waiting for
 *  VRAM, or actually working. A `bench` job in one of the `true` ones is a
 *  test in flight.
 *
 *  A total `Record<JobState, …>` rather than a set of the active ones: adding
 *  a state to {@link JobState} fails to compile until someone says which side
 *  of this line it falls on, instead of defaulting to "finished". */
const JOB_ACTIVE: Record<JobState, boolean> = {
  queued: true,
  scheduled: true,
  blocked: true,
  preparing: true,
  running: true,
  post: true,
  completed: false,
  failed: false,
  cancelled: false,
};

/** True while a job in this state still has work ahead of it. */
export const isJobActive = (state: JobState): boolean => JOB_ACTIVE[state];

/** Every state {@link isJobActive} calls active, derived from {@link JOB_ACTIVE}
 *  rather than hand-listed — a poll filter that cannot silently drift from the
 *  function it is supposed to mirror. A stable module-level constant, so it is
 *  safe as a hook dependency / poll key. */
export const ACTIVE_JOB_STATES = (Object.keys(JOB_ACTIVE) as JobState[]).filter(isJobActive);

/** The latest benchmark for every model that has one — join by `model_id`. */
export const listBenchmarks = () => invoke<Benchmark[]>("list_benchmarks");
/** Every benchmark run for one model, newest first. */
export const modelBenchmarks = (id: string) =>
  invoke<Benchmark[]>("model_benchmarks", { id });
/** The built-in, versioned suites a run may name — the Benchmark tab's picker. */
export const listBenchSuites = () => invoke<BenchSuite[]>("bench_suites");
/** Queue a "Test model" job (GGUF models only). Returns the job.
 *  Without `opts` this is the single-prompt quick test the Model Library
 *  button has always queued; `suite` runs a versioned suite instead and `runs`
 *  sets the passes per prompt (the job clamps it to 1..10). */
export const benchmarkModel = (id: string, opts?: { suite?: string; runs?: number }) =>
  invoke<Job>("benchmark_model", { id, body: opts ?? null });
/** Benchmarks across all models, newest first — the Benchmark tab's history.
 *  No `suite` returns every row (quick tests included); `limit` defaults to 50
 *  and is clamped to 1..200. */
export const benchmarkHistory = (opts?: { suite?: string; limit?: number }) =>
  invoke<Benchmark[]>("benchmark_history", {
    suite: opts?.suite ?? null,
    limit: opts?.limit ?? null,
  });

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

// --- Story Studio (Phase 1: text + plain image, docs/TODO.md) -----------
//
// Portrait/reference images and Scene images are never generated by these
// calls -- they are plain `job_id`s from a `job_type=image` job submitted
// the ordinary way via `submitJob`. A caller assembles the prompt (story
// art style + character/location/scene fields), submits it, then points
// the Story Studio row at the resulting job id once it exists.

export interface Story {
  id: string;
  name: string;
  /** Era/setting, freeform. */
  setting: string;
  /** Freeform; feeds every generation prompt for this story. */
  art_style: string;
  premise: string;
  created_at: string;
}

/** Body for `createStory` / `updateStory` -- every field, always together. */
export interface StoryBody {
  name: string;
  setting: string;
  art_style: string;
  premise: string;
}

export interface Character {
  id: string;
  story_id: string;
  name: string;
  traits: string;
  backstory: string;
  /** Role/alignment hint, freeform, e.g. "elf, friendly, skilled". */
  alignment: string;
  /** A `job_type=image` job id, or `null` until a portrait is picked. */
  portrait_job_id: string | null;
  inventory: string[];
  created_at: string;
}

export interface CharacterBody {
  name: string;
  traits: string;
  backstory: string;
  alignment: string;
}

export interface CharacterRelationship {
  id: string;
  character_id: string;
  related_character_id: string;
  note: string;
  created_at: string;
}

export interface CharacterLogEntry {
  id: string;
  character_id: string;
  scene_id: string;
  created_at: string;
  text: string;
}

export interface Npc {
  id: string;
  story_id: string;
  name: string;
  role: string;
  location_id: string | null;
  description: string;
  created_at: string;
}

export interface NpcBody {
  name: string;
  role: string;
  location_id: string | null;
  description: string;
}

export interface StoryLocation {
  id: string;
  story_id: string;
  name: string;
  description: string;
  /** A `job_type=image` job id, or `null` until a reference is picked. */
  reference_job_id: string | null;
  created_at: string;
}

export interface LocationBody {
  name: string;
  description: string;
}

export interface DialogueLine {
  id: string;
  scene_id: string;
  character_id: string;
  position: number;
  text: string;
}

/** A dialogue line as sent to `createScene` / `updateScene` -- no id/scene_id
 *  yet, position comes from array order. */
export interface DialogueLineBody {
  character_id: string;
  text: string;
}

export interface Scene {
  id: string;
  story_id: string;
  location_id: string | null;
  narrative: string;
  /** The short scenario prompt that led to this scene. */
  redline: string;
  /** Ordering within the story's timeline; stable across edits (Phase 1 has
   *  no manual reorder). */
  position: number;
  created_at: string;
}

/** Body for `createScene` / `updateScene` -- full replace, participants and
 *  dialogue included. `position` is never client-editable. */
export interface SceneBody {
  location_id: string | null;
  narrative: string;
  redline: string;
  participant_ids: string[];
  dialogue: DialogueLineBody[];
}

export interface SceneImage {
  id: string;
  scene_id: string;
  /** A `job_type=image` job id. */
  job_id: string;
  is_canonical: boolean;
  created_at: string;
}

/** A Scene plus everything the Timeline needs to render one card. */
export interface SceneDetail extends Scene {
  participant_ids: string[];
  dialogue: DialogueLine[];
  images: SceneImage[];
}

export const listStories = () => invoke<Story[]>("list_stories");
export const createStory = (body: StoryBody) => invoke<Story>("create_story", { body });
export const updateStory = (id: string, body: StoryBody) => invoke<void>("update_story", { id, body });
export const deleteStory = (id: string) => invoke<void>("delete_story", { id });

export const listCharacters = (storyId: string) =>
  invoke<Character[]>("list_characters", { storyId });
export const createCharacter = (storyId: string, body: CharacterBody) =>
  invoke<Character>("create_character", { storyId, body });
export const updateCharacter = (id: string, body: CharacterBody) =>
  invoke<void>("update_character", { id, body });
export const deleteCharacter = (id: string) => invoke<void>("delete_character", { id });
/** Pick (`jobId`) or clear (`null`) a character's reference portrait. */
export const setCharacterPortrait = (id: string, jobId: string | null) =>
  invoke<void>("set_character_portrait", { id, jobId });
/** Replaces the whole inventory list. */
export const setCharacterInventory = (id: string, items: string[]) =>
  invoke<void>("set_character_inventory", { id, items });
export const listCharacterRelationships = (id: string) =>
  invoke<CharacterRelationship[]>("list_character_relationships", { id });
export const addCharacterRelationship = (id: string, relatedCharacterId: string, note: string) =>
  invoke<CharacterRelationship>("add_character_relationship", {
    id,
    relatedCharacterId,
    note,
  });
export const removeCharacterRelationship = (id: string) =>
  invoke<void>("remove_character_relationship", { id });
/** This character's append-only Character Log, oldest first. */
export const characterLog = (id: string) => invoke<CharacterLogEntry[]>("character_log", { id });

export const listNpcs = (storyId: string) => invoke<Npc[]>("list_npcs", { storyId });
export const createNpc = (storyId: string, body: NpcBody) =>
  invoke<Npc>("create_npc", { storyId, body });
export const updateNpc = (id: string, body: NpcBody) => invoke<void>("update_npc", { id, body });
export const deleteNpc = (id: string) => invoke<void>("delete_npc", { id });

export const listLocations = (storyId: string) =>
  invoke<StoryLocation[]>("list_locations", { storyId });
export const createLocation = (storyId: string, body: LocationBody) =>
  invoke<StoryLocation>("create_location", { storyId, body });
export const updateLocation = (id: string, body: LocationBody) =>
  invoke<void>("update_location", { id, body });
export const deleteLocation = (id: string) => invoke<void>("delete_location", { id });
/** Pick (`jobId`) or clear (`null`) a location's reference image. */
export const setLocationReference = (id: string, jobId: string | null) =>
  invoke<void>("set_location_reference", { id, jobId });

/** Every scene in a story's timeline, in order, each with its full detail. */
export const listScenes = (storyId: string) => invoke<SceneDetail[]>("list_scenes", { storyId });
export const createScene = (storyId: string, body: SceneBody) =>
  invoke<SceneDetail>("create_scene", { storyId, body });
export const updateScene = (id: string, body: SceneBody) =>
  invoke<SceneDetail>("update_scene", { id, body });
export const deleteScene = (id: string) => invoke<void>("delete_scene", { id });
/** Attaches an already-submitted `job_type=image` job as a new image for a
 *  scene (the first one becomes canonical automatically). */
export const addSceneImage = (sceneId: string, jobId: string) =>
  invoke<SceneImage>("add_scene_image", { sceneId, jobId });
export const setCanonicalSceneImage = (id: string) =>
  invoke<void>("set_canonical_scene_image", { id });
export const deleteSceneImage = (id: string) => invoke<void>("delete_scene_image", { id });

// --- training orchestrator (spec `2026-09-16-training-orchestrator-design`) -

/** The training-run lifecycle. `interrupted` is the one every long run has to
 *  survive: the process vanished (a reboot, a kill, a driver crash), the
 *  checkpoints are intact, and the run is resumable — never `failed`. */
export type TrainingRunState =
  | "preparing"
  | "running"
  | "paused"
  | "interrupted"
  | "resuming"
  | "finishing"
  | "completed"
  | "failed"
  | "cancelled";

/** Speed/quality trade-off applied on top of a profile's own defaults. */
export type TrainingPreset = "fast" | "balanced" | "thorough";

/** Plain-English VRAM headroom, shown before a run starts. */
export type TrainingFit = "comfortable" | "at_the_edge";

/** What kind of dataset a profile trains from. */
export type TrainingDataKind = "frames" | "clips" | "both";

/** Which step of the trainer install is running. */
export type TrainerInstallPhase =
  | "downloading"
  | "extracting"
  | "installing_python"
  | "creating_venv"
  | "installing_torch"
  | "installing_deps";

/** Install progress — same serialized shape as the ComfyUI install card's. */
export type TrainerInstallState =
  | { state: "idle" }
  | { state: "running"; phase: TrainerInstallPhase; done_bytes: number; total_bytes: number }
  | { state: "failed"; error: string };

/** Everything the Training tab's header needs to choose between "set up",
 *  "wait", "repair" and "start a run". */
export interface TrainerStatus {
  installed: boolean;
  installing: boolean;
  /** The last probe found the venv unusable — offer a repair, not a run. */
  env_broken: boolean;
  install_state: TrainerInstallState;
  /** The adapter's own one-line status, ready to show verbatim. */
  detail: string;
  /** The run currently holding the GPU, if any. */
  alive_run_id: string | null;
}

/** What the trainer venv reports about its own PyTorch ("Jetzt testen"). */
export interface TrainerProbe {
  torch_version: string;
  cuda: boolean;
  vram_total_mb: number;
}

/** Rank/LR/resolution/step starting points of one preset. */
export interface TrainingPresetValues {
  steps: number;
  lr: number;
  rank: number;
  resolution: number;
  save_every: number;
  sample_every: number;
}

/** A library model a profile can train — the target-model dropdown. */
export interface TrainableModel {
  id: string;
  name: string;
  family: string;
}

/** One trainable model family, joined with the two facts only the library
 *  knows: whether its base weights are staged, and what resolves to it. */
export interface TrainingProfile {
  family: string;
  label: string;
  /** The ai-toolkit architecture id (`flux2_klein_4b`, `wan22_5b`, …). */
  arch: string;
  data_kind: TrainingDataKind;
  fit: TrainingFit;
  /** `fit` in plain English — show this, never the enum. */
  fit_label: string;
  reserve_mb: number;
  base_repo: string;
  base_role: string;
  base_required_files: string[];
  base_approx_gb: number;
  /** The exact `hf` line that stages this base, built server-side from the
   *  same manifest the runner verifies the download against. */
  base_download_command: string;
  /** A library directory model with `base_role` holding every required file. */
  base_installed: boolean;
  caption_order: CaptionOrder;
  license_note: string;
  presets: {
    fast: TrainingPresetValues;
    balanced: TrainingPresetValues;
    thorough: TrainingPresetValues;
  };
  trainable_models: TrainableModel[];
}

/** Per-run overrides on top of the preset; an absent field keeps the preset's
 *  own value. */
export interface TrainingHyperparams {
  steps?: number | null;
  lr?: number | null;
  rank?: number | null;
  resolution?: number | null;
}

/** One training run, as stored. Unlike a {@link Job} it outlives the app: the
 *  trainer is a detached process, so a run that started yesterday is still
 *  observable (and resumable) after a restart. */
export interface TrainingRun {
  id: string;
  name: string;
  profile_family: string;
  target_model_id: string | null;
  dataset_id: string | null;
  data_kind: DatasetMode;
  trigger_word: string;
  preset: TrainingPreset;
  /** JSON of {@link TrainingHyperparams}, as stored. */
  hyperparams_json: string;
  /** JSON string array, as stored. */
  sample_prompts_json: string;
  state: TrainingRunState;
  step: number;
  total_steps: number;
  last_loss: number | null;
  last_checkpoint_at: string | null;
  pid: number | null;
  work_dir: string;
  /** The imported LoRA, once the run completed. */
  result_model_id: string | null;
  error_text: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  /** The library LoRA this run continued from; `null` = from scratch. */
  init_lora_model_id: string | null;
  /** Images/clips the trainer was fed at start; `null` on older runs. */
  image_count: number | null;
}

/** A run plus the two things that live on disk rather than in the store. */
export interface RunDetail {
  run: TrainingRun;
  /** Library name of `run.init_lora_model_id`, when that LoRA still exists. */
  init_lora_name: string | null;
  /** Opaque tokens for {@link trainingSampleUrl}, newest checkpoint first —
   *  deliberately not file paths. */
  latest_samples: string[];
  /** The tail of `train.log`, already split on `\r` and `\n`. */
  log_tail: string[];
  work_dir: string;
}

/** Body for {@link startTrainingRun} — what the Training form collects. */
export interface StartRunBody {
  name: string;
  target_model_id: string;
  dataset_id: string;
  trigger_word: string;
  preset: TrainingPreset;
  hyperparams: TrainingHyperparams;
  sample_prompts: string[];
  /** "Store run in": the run gets `<data_dir>\<run_id>`. Omit for the
   *  default training folder (Settings → Data locations). */
  data_dir?: string;
  /** "Start from": a library LoRA of the same family and rank to continue.
   *  Omit for a fresh LoRA. The core refuses a missing file, another family,
   *  another rank, a non-LoRA, or a LoRA whose own run is still going. */
  init_lora_model_id?: string;
}

export const trainerStatus = () => invoke<TrainerStatus>("training_status");

/** `"started"` | `"already_installed"`; rejected in offline mode. */
export const installTrainer = () => invoke<string>("install_trainer");

/** Run the import probe — the one check that says a run would actually start.
 *  Sets or clears `env_broken` as a side effect. */
export const probeTrainer = () => invoke<TrainerProbe>("probe_trainer");

export const listTrainingProfiles = () => invoke<TrainingProfile[]>("list_training_profiles");

export const listTrainingRuns = () => invoke<TrainingRun[]>("list_training_runs");

export const startTrainingRun = (body: StartRunBody) =>
  invoke<TrainingRun>("start_training_run", { body });

export const getTrainingRun = (id: string) => invoke<RunDetail | null>("get_training_run", { id });

export const pauseTrainingRun = (id: string) => invoke<TrainingRun>("pause_training_run", { id });

export const resumeTrainingRun = (id: string) => invoke<TrainingRun>("resume_training_run", { id });

export const cancelTrainingRun = (id: string) => invoke<TrainingRun>("cancel_training_run", { id });

/** Drop a finished run from the history. `purge` also deletes its work
 *  directory — checkpoints and preview images included. */
export const deleteTrainingRun = (id: string, purge: boolean) =>
  invoke<void>("delete_training_run", { id, purge });

// --- LoRA overview & lineage (spec `2026-09-19-lora-lineage-design`) --------

/** One library LoRA as the "Your LoRAs" overview lists it, with what its
 *  whole lineage adds up to. */
export interface LoraSummary {
  model_id: string;
  name: string;
  family: string | null;
  /** `true` when the LoRA came out of a training run here; `false` for one
   *  imported by hand. */
  trained: boolean;
  /** From the safetensors header; `null` when the file cannot be read. */
  rank: number | null;
  size_bytes: number;
  /** When the row entered the library. */
  created_at: string;
  /** How many runs contributed, oldest source to this LoRA. */
  runs: number;
  /** Steps of the completed runs in the lineage. */
  total_steps: number;
  /** Images/clips summed over the lineage; `null` when any run has no
   *  count (older than migration 0020). */
  total_images: number | null;
}

/** The dataset a lineage run trained on, as far as it is still known. */
export interface LineageDataset {
  id: string;
  name: string;
  source_root: string;
  /** The captioner its prep job used; `null` when unknown or none. */
  captioner: string | null;
}

/** One run in a LoRA's history. */
export interface LineageRun {
  run_id: string;
  name: string;
  state: TrainingRunState;
  /** `null` when the dataset row has since been deleted. */
  dataset: LineageDataset | null;
  image_count: number | null;
  trigger_word: string;
  profile_family: string;
  preset: TrainingPreset;
  /** The stored overrides, parsed; `null` when the stored JSON is unreadable. */
  hyperparams: TrainingHyperparams | null;
  hyperparams_json: string;
  step: number;
  total_steps: number;
  started_at: string | null;
  finished_at: string | null;
  /** `finished_at - started_at` in whole seconds, when both are set. */
  duration_secs: number | null;
  result_model_id: string | null;
  result_model_name: string | null;
  init_lora_model_id: string | null;
  init_lora_name: string | null;
  /** Opaque tokens for {@link trainingSampleUrl} with `run_id`, newest
   *  checkpoint first — the same tokens as {@link RunDetail.latest_samples}. */
  samples: string[];
}

/** A LoRA with its history, oldest run first. */
export interface LoraLineage {
  lora: LoraSummary;
  runs: LineageRun[];
}

/** Every library LoRA, newest first — trained and imported alike. */
export const listLoras = () => invoke<LoraSummary[]>("list_loras");

/** A LoRA's history; `null` when `modelId` is not a library LoRA. */
export const loraLineage = (modelId: string) =>
  invoke<LoraLineage | null>("lora_lineage", { modelId });

/** URL the loopback core serves the `n`-th latest preview image from — same
 *  shape as {@link jobOutputUrl}. `n` indexes
 *  {@link RunDetail.latest_samples}, and is never a path. */
export const trainingSampleUrl = (coreApiPort: number, runId: string, n: number) =>
  `http://127.0.0.1:${coreApiPort}/training/runs/${runId}/samples/${n}`;
