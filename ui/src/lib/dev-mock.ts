/** A canned Tauri IPC bridge for running the UI in a plain browser (`pnpm dev`
 *  outside the Tauri shell). Dev-only: `main.tsx` loads this via a dynamic
 *  import that is dead code in a production build. It exists so the studios can
 *  be eyeballed without booting the whole stack; it is not a test double for
 *  logic. Media URLs (`/jobs/{id}/output`) will not resolve here — the layout
 *  and controls are what this is for. */
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

type AnyRecord = Record<string, unknown>;

const now = () => new Date().toISOString();

const MODELS: AnyRecord[] = [
  mkModel("m-wan", "wan2.2_ti2v_5B_fp16", { family: "wan", roles: ["base_video"], runtimes: ["comfyui"], vram_estimate_mb: 11800 }),
  mkModel("m-umt5", "umt5_xxl_fp8_e4m3fn_scaled", { roles: ["text_encoder"], runtimes: ["comfyui"] }),
  mkModel("m-wanvae", "wan2.2_vae", { family: "wan", roles: ["vae"], runtimes: ["comfyui"] }),
  mkModel("m-sdxl", "SDXL Base 1.0", { family: "sdxl", roles: ["base_diffusion"], runtimes: ["comfyui"], vram_estimate_mb: 8200 }),
  mkModel("m-qwen", "Qwen2.5 7B Instruct", { family: "qwen2", format: "gguf", quant: "Q5_K_M", param_count: 7_615_616_512, ctx_max: 32_768, roles: ["chat"], runtimes: ["llamacpp"], vram_estimate_mb: 6400 }),
];

const JOBS: AnyRecord[] = [
  mkJob("j-vid-1", "video", "completed", {
    model_id: "m-wan",
    output_path: "E:\\AI\\data\\outputs\\j-vid-1.mp4",
    params: { prompt: "a paper boat drifting down a rain-soaked street", negative: "blurry", width: 832, height: 480, length: 81, fps: 24, steps: 30, cfg: 5, seed: 4212981 },
  }),
  mkJob("j-img-1", "image", "completed", {
    model_id: "m-sdxl",
    output_path: "E:\\AI\\data\\outputs\\j-img-1.png",
    params: { prompt: "a bay at dawn, wide angle", negative: "", width: 1024, height: 1024, steps: 25, cfg: 7, seed: 99 },
  }),
];

function mkModel(id: string, name: string, over: AnyRecord): AnyRecord {
  return {
    id, name, publisher: null, family: null, format: "safetensors", quant: null, arch: null,
    param_count: null, file_path: `E:\\AI\\models\\${name}`, sha256: null, size_bytes: 6_000_000_000,
    ctx_max: null, vram_estimate_mb: null, ram_estimate_mb: null, source: "manual",
    imported_at: now(), last_used_at: null, use_count: 0,
    n_layers: null, n_embd: null, n_heads: null, n_kv_heads: null, roles: [], runtimes: [],
    ...over,
  };
}

function mkJob(id: string, jobType: string, state: string, over: AnyRecord): AnyRecord {
  return {
    id, job_type: jobType, capability: null, state, params: {}, runtime_id: "comfyui",
    model_id: null, created_at: now(), started_at: now(), finished_at: now(),
    error_text: null, output_path: null, result: null, session_id: null, ...over,
  };
}

function mkSession(id: string, capability: string, name: string): AnyRecord {
  return { id, capability, name, created_at: now(), archived_at: null };
}

const SESSIONS: AnyRecord[] = [
  mkSession("sess-chat-1", "chat", "General"),
  mkSession("sess-img-1", "image", "Anime pics"),
];

const RUNTIMES: AnyRecord[] = [
  { id: "llamacpp", kind: "llama_cpp", health: "unknown", vram_used_mb: 0, detail: "installed · idle" },
  { id: "comfyui", kind: "comfy_ui", health: "healthy", vram_used_mb: 0, detail: "running on :48096 · ComfyUI 0.34.0 · model reserved" },
  { id: "colibri", kind: "colibri", health: "unknown", vram_used_mb: 0, detail: "not installed" },
];

const COLIBRI_MODELS: AnyRecord[] = [
  {
    id: "qwen3.6-35b-a3b-colibri", label: "Qwen3.6-35B-A3B (Colibri)",
    repo: "Kreuzzelg/qwen36-35b-a3b-colibri-i4-gs64", ram_estimate_mb: 24_576,
    disk_estimate_bytes: 20 * 1024 * 1024 * 1024, license: "Apache-2.0",
    note: "A 35B-parameter MoE (3B active) too large for GGUF/llama.cpp on a 16 GB card — runs CPU-only via Colibri instead. Needs ~24 GB RAM resident; tight on a 32 GB machine.",
  },
];

const AGENTS: AnyRecord[] = [
  {
    id: "a-coder", name: "Repo coder", adapter: "opencode", model_id: null,
    workspace_path: "E:\\AI", allowed_paths: [], toolset: null, created_at: now(),
  },
];

type DevSession = {
  id: string;
  agent_id: string;
  state: string;
  answered: boolean;
  stopped: boolean;
  firstMessage?: string | null;
};
let DEV_SESSION: DevSession | null = null;
let HERMES_INSTALLED = false;

const DOWNLOADS: AnyRecord[] = [];
const BENCHMARKS: AnyRecord[] = [];
const TAGS: Record<string, string[]> = { "m-qwen": ["coding", "favourite"] };
let HF_TOKEN = "";

/** Flip a running `bench` job to `completed` a few seconds in and drop a
 *  benchmark row — the dev-mock stand-in for `core::bench`. */
function progressBenchJobs(): void {
  for (const j of JOBS) {
    if (j.job_type !== "bench" || j.state !== "running") continue;
    const started = Date.parse(String(j.started_at ?? j.created_at));
    if (Date.now() - started < 3500) continue;
    j.state = "completed";
    j.finished_at = now();
    const m = MODELS.find((x) => x.id === j.model_id);
    const gen = 38 + Math.round(Math.random() * 42);
    BENCHMARKS.unshift({
      id: `bench-${seq++}`,
      model_id: j.model_id,
      job_id: j.id,
      kind: "llm",
      runs: 3,
      prompt_tps: 260 + Math.round(Math.random() * 220),
      gen_tps: gen,
      load_ms: 1500 + Math.round(Math.random() * 2200),
      vram_peak_mb: Number(m?.vram_estimate_mb ?? 6000),
      ram_peak_mb: 14000 + Math.round(Math.random() * 2000),
      stability_score: 0.9 + Math.random() * 0.09,
      overall_score: Math.min(100, Math.round(gen * 1.15)),
      notes: "dev-mock",
      created_at: now(),
    });
  }
}

/** Flip a running `upgrade_check` job to `completed` with a canned report. */
function progressUpgradeJobs(): void {
  for (const j of JOBS) {
    if (j.job_type !== "upgrade_check" || j.state !== "running") continue;
    const started = Date.parse(String(j.started_at ?? j.created_at));
    if (Date.now() - started < 3000) continue;
    const m = MODELS.find((x) => x.id === (j.params as AnyRecord)?.target_model_id);
    j.state = "completed";
    j.finished_at = now();
    j.result = JSON.stringify({
      target: String(m?.name ?? "model"),
      query: String(m?.family ?? "qwen2"),
      note: "Ranked by your local model over the objective signals (release date, downloads, size, fit). Quality is not locally verifiable.",
      freshness: { kind: "live" },
      candidates: [
        {
          id: "Qwen/Qwen2.5-Coder-14B-Instruct-GGUF",
          why: "same family, ~2× the parameters, still fits your 16 GB budget, 400k downloads",
          downloads: 402113,
          likes: 88,
          last_modified: "2026-04-18",
          param_count: 14_770_000_000,
          format: "gguf",
          gated: false,
          fit: { level: "yellow", reason: "needs ~9 GB — little head-room" },
          installed: false,
          llm_ranked: true,
        },
        {
          id: "bartowski/Qwen2.5-7B-Instruct-GGUF",
          why: "same size, newer quant set, more actively maintained",
          downloads: 1_780_676,
          likes: 436,
          last_modified: "2026-05-02",
          param_count: 7_615_616_512,
          format: "gguf",
          gated: false,
          fit: { level: "green" },
          installed: false,
          llm_ranked: true,
        },
      ],
    });
  }
}

const DISCOVER_MODELS: AnyRecord[] = [
  {
    id: "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF", author: "Qwen", downloads: 1_780_676, likes: 436,
    trending_score: 12, created_at: now(), last_modified: now(), pipeline_tag: "text-generation",
    library_name: "transformers", gated: "no", license: "apache-2.0",
    base_model: "Qwen/Qwen2.5-Coder-7B-Instruct", tags: ["gguf", "code"],
    param_count: 7_615_616_512, arch: "qwen2", ctx_max: 131_072, precision: null, format: "gguf",
  },
  {
    id: "bartowski/Qwen2.5-Coder-14B-Instruct-GGUF", author: "bartowski", downloads: 402_113, likes: 88,
    trending_score: 5, created_at: now(), last_modified: now(), pipeline_tag: "text-generation",
    library_name: null, gated: "no", license: "apache-2.0",
    base_model: "Qwen/Qwen2.5-Coder-14B-Instruct", tags: ["gguf"],
    param_count: 14_770_000_000, arch: "qwen2", ctx_max: 131_072, precision: null, format: "gguf",
  },
];

function devSession(): AnyRecord {
  const s = DEV_SESSION!;
  return {
    id: s.id, agent_id: s.agent_id, adapter_session_id: "ses_dev", state: s.state,
    error_text: null, checkpoint_path: null, started_at: now(),
    ended_at: s.stopped ? now() : null,
  };
}

function devEvents(): AnyRecord[] {
  const s = DEV_SESSION!;
  const events: AnyRecord[] = [
    { ts: now(), kind: "text", payload: { type: "text", text: "I'll read the README to get oriented." } },
    { ts: now(), kind: "tool", payload: { type: "tool", id: "c1", name: "bash", status: s.answered ? "done" : "running", input: { command: "cat README.md" }, output: s.answered ? "AI Workstation Manager — a local control plane…\n" : null } },
  ];
  if (!s.answered) {
    events.push({ ts: now(), kind: "permission", payload: { type: "permission", id: "per_1", kind: "bash", summary: "cat README.md", always_pattern: "cat *" } });
  } else {
    events.push({ ts: now(), kind: "text", payload: { type: "text", text: "The README says it's an offline-first orchestrator for local AI runtimes." } });
    events.push({ ts: now(), kind: "idle", payload: { type: "idle" } });
  }
  return events;
}

const ABOUT: AnyRecord = {
  core_version: "0.0.1-dev", data_dir: "E:\\AI\\data", store_path: "E:\\AI\\models",
  outputs_dir: "E:\\AI\\data\\outputs", outputs_bytes: 4_812_300_000,
  runtimes_dir: "E:\\AI\\data\\runtimes", cache_dir: "E:\\AI\\data\\cache",
  core_api_port: 48096, vram_budget_mb: 14848, offline_mode: false,
};

const CONFIG: AnyRecord = {
  store_path: "E:\\AI\\models", core_api_port: 48096, offline_mode: false, log_filter: "info",
  vram_budget_mb: 0,
  llama: {
    gpu_layers: 999, ctx_size: 0, flash_attention: true,
    jinja: true, chat_template: "", load_timeout_secs: 180,
  },
  comfyui: { vram_mode: "auto", reserve_vram_mb: 0, extra_args: "" },
  models: { auto_preference: "balanced" },
  paths: { outputs_path: null, runtimes_path: null, cache_path: null },
};

const KNOWN_MOCK: AnyRecord[] = [
  {
    id: "sdxl-base-1.0", name: "Stable Diffusion XL 1.0 (base)", kind: "checkpoint",
    family: "sdxl", publisher: "Stability AI", repo: "stabilityai/stable-diffusion-xl-base-1.0",
    file: "sd_xl_base_1.0.safetensors",
    url: "https://huggingface.co/stabilityai/stable-diffusion-xl-base-1.0/resolve/main/sd_xl_base_1.0.safetensors",
    sha256: "3".repeat(64), size_bytes: 6_938_078_334,
    license: "CreativeML Open RAIL++-M (commercial use allowed)",
    note: "The default. Fits comfortably in 16 GB, huge LoRA/ControlNet ecosystem.",
    is_default: true, media: "image", fit: { level: "green" },
  },
  {
    id: "flux1-dev-q8", name: "FLUX.1-dev — Q8_0 (GGUF)", kind: "diffusion_model",
    family: "flux", publisher: "Black Forest Labs / city96", repo: "city96/FLUX.1-dev-gguf",
    file: "flux1-dev-Q8_0.gguf",
    url: "https://huggingface.co/city96/FLUX.1-dev-gguf/resolve/main/flux1-dev-Q8_0.gguf",
    sha256: "1".repeat(64), size_bytes: 12_708_281_504,
    license: "FLUX.1 [dev] Non-Commercial License",
    note: "Best prompt fidelity + in-image text. Needs the T5, CLIP-L and VAE below.",
    is_default: false, media: "image",
    fit: { level: "yellow", reason: "needs ~13.4 GB of your ~14.5 GB VRAM budget — little head-room for a longer context or a second resident model" },
  },
  {
    id: "t5xxl-fp8", name: "T5-XXL — fp8 (Flux text encoder)", kind: "text_encoder",
    family: null, publisher: "Comfy-Org", repo: "comfyanonymous/flux_text_encoders",
    file: "t5xxl_fp8_e4m3fn.safetensors",
    url: "https://huggingface.co/comfyanonymous/flux_text_encoders/resolve/main/t5xxl_fp8_e4m3fn.safetensors",
    sha256: "5".repeat(64), size_bytes: 4_893_934_904,
    license: "Apache-2.0",
    note: "Flux prompt encoder. ComfyUI offloads it after encoding, so it is not resident during sampling.",
    is_default: false, media: "image", fit: { level: "green" },
  },
  {
    id: "clip-l", name: "CLIP-L (Flux text encoder)", kind: "text_encoder",
    family: null, publisher: "Comfy-Org", repo: "comfyanonymous/flux_text_encoders",
    file: "clip_l.safetensors",
    url: "https://huggingface.co/comfyanonymous/flux_text_encoders/resolve/main/clip_l.safetensors",
    sha256: "6".repeat(64), size_bytes: 246_144_152,
    license: "MIT", note: "The second Flux encoder. Small.",
    is_default: false, media: "image", fit: { level: "green" },
  },
  {
    id: "flux-vae", name: "FLUX.1 VAE (ae)", kind: "vae",
    family: "flux", publisher: "Black Forest Labs", repo: "second-state/FLUX.1-dev-GGUF",
    file: "ae.safetensors",
    url: "https://huggingface.co/second-state/FLUX.1-dev-GGUF/resolve/main/ae.safetensors",
    sha256: "8".repeat(64), size_bytes: 335_304_388,
    license: "FLUX.1 [dev] Non-Commercial License",
    note: "The Flux autoencoder. Byte-identical across every Flux re-upload.",
    is_default: false, media: "image", fit: { level: "green" },
  },
  {
    id: "wan22-ti2v-5b", name: "Wan 2.2 TI2V-5B — fp16 (video, default)", kind: "video",
    family: "wan", publisher: "Alibaba / Comfy-Org", repo: "Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
    file: "wan2.2_ti2v_5B_fp16.safetensors",
    url: "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_ti2v_5B_fp16.safetensors",
    sha256: "4".repeat(64), size_bytes: 9_999_658_848,
    license: "Apache-2.0 (commercial use allowed)",
    note: "The default video model. One model for text→video and image→video. Needs the umt5 encoder and the Wan VAE.",
    is_default: true, media: "video", fit: { level: "green" },
  },
  {
    id: "wan-umt5-xxl-fp8", name: "umt5-XXL — fp8 (Wan text encoder)", kind: "text_encoder",
    family: null, publisher: "Comfy-Org", repo: "Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
    file: "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
    url: "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors",
    sha256: "9".repeat(64), size_bytes: 6_735_906_897,
    license: "Apache-2.0",
    note: "Wan's prompt encoder (multilingual T5). Offloaded to the CPU during sampling.",
    is_default: false, media: "video", fit: { level: "green" },
  },
  {
    id: "wan22-vae", name: "Wan 2.2 VAE", kind: "vae",
    family: "wan", publisher: "Alibaba / Comfy-Org", repo: "Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
    file: "wan2.2_vae.safetensors",
    url: "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/vae/wan2.2_vae.safetensors",
    sha256: "a".repeat(64), size_bytes: 1_409_400_960,
    license: "Apache-2.0", note: "The Wan 2.2 autoencoder. Import as VAE.",
    is_default: false, media: "video", fit: { level: "green" },
  },
  {
    id: "ltx-video-2b-095", name: "LTX-Video 2B v0.9.5 (video, second template)", kind: "video",
    family: "ltx", publisher: "Lightricks", repo: "Lightricks/LTX-Video",
    file: "ltx-video-2b-v0.9.5.safetensors",
    url: "https://huggingface.co/Lightricks/LTX-Video/resolve/main/ltx-video-2b-v0.9.5.safetensors",
    sha256: "7".repeat(64), size_bytes: 6_340_729_500,
    license: "LTXV License (OpenRAIL-M-style; check the repo for commercial terms)",
    note: "Fast, light. One .safetensors carries model + VAE — only needs a t5xxl encoder.",
    is_default: false, media: "video", fit: { level: "green" },
  },
];

/** Mirrors `core::model::MODEL_STACKS` — base model first, then companions. */
const STACK_MEMBER_IDS: Record<string, string[]> = {
  sdxl: ["sdxl-base-1.0"],
  flux: ["flux1-dev-q8", "t5xxl-fp8", "clip-l", "flux-vae"],
  wan22: ["wan22-ti2v-5b", "wan-umt5-xxl-fp8", "wan22-vae"],
  ltx: ["ltx-video-2b-095", "t5xxl-fp8"],
};
const STACK_META: Record<string, AnyRecord> = {
  sdxl: {
    label: "Stable Diffusion XL", media: "image", is_default: true,
    note: "One file — the checkpoint carries its own VAE and text encoder.",
    fit: { level: "green" },
  },
  flux: {
    label: "FLUX.1-dev", media: "image", is_default: false,
    note: "Best prompt fidelity + in-image text. Four files: the diffusion model plus its T5 and CLIP-L text encoders and its VAE.",
    // All four members individually look green/yellow, but a real render
    // needs them all resident at once (~17 GB combined) — the whole-stack
    // verdict is what actually answers "can I run this", and on a 16 GB
    // card it's red even though no single file looks that bad alone.
    fit: { level: "red", reason: "needs ~17 GB combined (model + T5 + CLIP-L + VAE) of your ~16 GB VRAM budget" },
  },
  wan22: {
    label: "Wan 2.2 TI2V-5B", media: "video", is_default: true,
    note: "The default video setup. Three files: the model, its umt5 text encoder, and its VAE.",
    fit: { level: "green" },
  },
  ltx: {
    label: "LTX-Video 0.9.5 (2B)", media: "video", is_default: false,
    note: "Fast and light. Two files: model + VAE bundled in one, plus a shared T5 text encoder.",
    fit: { level: "green" },
  },
};
const STACKS_MOCK: AnyRecord[] = Object.entries(STACK_MEMBER_IDS).map(([id, memberIds]) => ({
  id,
  ...STACK_META[id],
  members: memberIds.map((mid) => KNOWN_MOCK.find((m) => m.id === mid)),
}));

const FEATURED_MOCK: AnyRecord[] = [
  {
    id: "qwen2.5-7b-instruct", role: "chat", label: "Qwen2.5-7B-Instruct",
    repo: "bartowski/Qwen2.5-7B-Instruct-GGUF", quant_hint: "Q5_K_M", typical_vram_mb: 8704,
    license: "Apache-2.0", note: "Strong general-purpose chat model with native tool-calling.",
    import_roles: ["chat"], is_default: true, fit: { level: "green" },
  },
  {
    id: "qwen2.5-14b-instruct", role: "chat", label: "Qwen2.5-14B-Instruct",
    repo: "bartowski/Qwen2.5-14B-Instruct-GGUF", quant_hint: "Q4_K_M", typical_vram_mb: 12800,
    license: "Apache-2.0", note: "Bigger and sharper if you can spare the VRAM — fills most of a 16 GB card.",
    import_roles: ["chat"], is_default: false,
    fit: { level: "yellow", reason: "needs ~12.5 GB of your ~14.5 GB VRAM budget — little head-room for a longer context or a second resident model" },
  },
  {
    id: "qwen2.5-coder-7b-instruct", role: "coding", label: "Qwen2.5-Coder-7B-Instruct",
    repo: "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF", quant_hint: "Q5_K_M", typical_vram_mb: 8704,
    license: "Apache-2.0", note: "Best tool-calling/VRAM ratio for agent sessions (OpenCode) on 16 GB.",
    import_roles: ["chat", "coding"], is_default: true, fit: { level: "green" },
  },
  {
    id: "hermes-3-llama-3.1-8b", role: "coding", label: "Hermes-3-Llama-3.1-8B",
    repo: "NousResearch/Hermes-3-Llama-3.1-8B-GGUF", quant_hint: "Q5_K_M", typical_vram_mb: 9216,
    license: "Llama-3.1 Community License",
    note: "Nous Research's own agent-tuned model — the natural pick for the Hermes runtime.",
    import_roles: ["chat", "coding"], is_default: false, fit: { level: "green" },
  },
];

const TELEMETRY: AnyRecord = {
  captured_at_ms: Date.now(),
  gpu: { state: "available", name: "NVIDIA RTX 4080 SUPER", vram_total_mb: 16376, vram_used_mb: 2100, vram_free_mb: 14276, utilization_pct: 3, temperature_c: 41, processes: [] },
  host: { ram_total_mb: 32000, ram_used_mb: 14200, cpu_total_pct: 8, cpu_per_core_pct: [] },
};

let seq = 100;

export function installDevMock(): void {
  mockIPC(async (cmd, args): Promise<unknown> => {
    const a = (args ?? {}) as AnyRecord;
    switch (cmd) {
      case "about":
        return ABOUT;
      case "get_telemetry":
        return TELEMETRY;
      case "get_runtimes":
        return RUNTIMES;
      case "list_models":
        // Fresh array — `usePolled` needs a changed reference to re-render
        // (e.g. after delete_model splices MODELS).
        return MODELS.map((m) => ({ ...m }));
      case "list_known_models":
        return KNOWN_MOCK;
      case "list_model_stacks":
        return STACKS_MOCK;
      case "list_featured_models":
        return FEATURED_MOCK;
      case "list_colibri_models":
        return COLIBRI_MODELS;
      case "register_colibri_model": {
        const body = (a.body ?? {}) as AnyRecord;
        const catalog = COLIBRI_MODELS.find((m) => m.id === body.catalog_id);
        const model = mkModel(`m-colibri-${seq++}`, String(catalog?.label ?? "Colibri model"), {
          format: "colibri", file_path: String(body.dir ?? ""),
          size_bytes: Number(catalog?.disk_estimate_bytes ?? 0),
          ram_estimate_mb: Number(catalog?.ram_estimate_mb ?? 0),
          roles: ["chat"],
        });
        MODELS.push(model);
        return model;
      }
      case "install_colibri": {
        const rt = RUNTIMES.find((r) => r.id === "colibri");
        if (rt) rt.detail = "installed · idle";
        return "started";
      }
      case "list_jobs":
        progressBenchJobs();
        progressUpgradeJobs();
        // Fresh array — `usePolled` needs a changed reference to re-render.
        return JOBS.map((j) => ({ ...j }));
      case "get_config":
        return CONFIG;
      case "save_config": {
        const u = (a.update ?? {}) as AnyRecord;
        Object.assign(CONFIG, u);
        return CONFIG;
      }
      case "get_settings":
        return {};
      case "get_recent_logs":
        return ["dev-mock: no real logs"];
      case "job_detail": {
        const job = JOBS.find((j) => j.id === a.id);
        return job ? { job, events: [{ ts: now(), level: "info", message: "rendering 832×480 video, 81 frames @ 24 fps (~3.4s), 30 steps, cfg 5, seed 4212981 — wan2.2_ti2v_5B_fp16" }] } : null;
      }
      case "submit_job": {
        const body = (a.body ?? {}) as AnyRecord;
        const job = mkJob(`j-dev-${seq++}`, String(body.job_type ?? "video"), "running", {
          params: body.params ?? {},
          model_id: (body.model_id as string) ?? "m-wan",
          output_path: null,
          session_id: (body.session_id as string) ?? null,
        });
        JOBS.unshift(job);
        return job;
      }
      case "cancel_job":
        return true;
      case "list_sessions": {
        const capability = String(a.capability ?? "");
        return SESSIONS.filter((s) => s.capability === capability).map((s) => ({ ...s }));
      }
      case "create_session": {
        const body = (a.body ?? {}) as AnyRecord;
        const session = mkSession(
          `sess-dev-${seq++}`,
          String(body.capability ?? "chat"),
          String(body.name ?? "Untitled"),
        );
        SESSIONS.unshift(session);
        return session;
      }
      case "rename_session": {
        const s = SESSIONS.find((x) => x.id === a.id);
        if (s) s.name = String(a.name ?? s.name);
        return null;
      }
      case "set_session_archived": {
        const s = SESSIONS.find((x) => x.id === a.id);
        if (s) s.archived_at = a.archived ? now() : null;
        return null;
      }
      case "delete_session": {
        const i = SESSIONS.findIndex((x) => x.id === a.id);
        if (i >= 0) SESSIONS.splice(i, 1);
        return null;
      }
      case "storage_report": {
        const kindOf = (m: AnyRecord): string => {
          const roles = (m.roles as string[]) ?? [];
          if (roles.includes("chat") || roles.includes("coding")) return "llm";
          if (roles.includes("base_video")) return "video";
          return "image";
        };
        const byKind: AnyRecord = {};
        let store = 0;
        for (const m of MODELS) {
          const size = Number(m.size_bytes);
          store += size;
          const k = kindOf(m);
          const e = (byKind[k] as AnyRecord) ?? { kind: k, bytes: 0, count: 0 };
          e.bytes = Number(e.bytes) + size;
          e.count = Number(e.count) + 1;
          byKind[k] = e;
        }
        return {
          store_bytes: store,
          volume_free_bytes: 1_496_000_000_000,
          volume_total_bytes: 2_000_000_000_000,
          by_kind: Object.values(byKind),
          models: MODELS.map((m) => ({
            id: m.id, name: m.name, kind: kindOf(m), size_bytes: Number(m.size_bytes),
            last_used_at: m.last_used_at ?? null, use_count: Number(m.use_count ?? 0),
            roles: m.roles ?? [], file_present: true,
          })),
          duplicates:
            MODELS.length >= 2
              ? [
                  {
                    sha256: "dead00beef00cafe00".repeat(2).slice(0, 64),
                    member_ids: [MODELS[0].id, MODELS[1].id],
                    wasted_bytes: Number(MODELS[1].size_bytes),
                  },
                ]
              : [],
          unused: MODELS.filter((m) => !m.last_used_at).map((m) => m.id),
          stale_days: 45,
        };
      }
      case "delete_model": {
        const i = MODELS.findIndex((m) => m.id === a.id);
        if (i < 0) throw new Error(`model ${a.id} is not in the library`);
        const [m] = MODELS.splice(i, 1);
        delete TAGS[String(m.id)];
        return { id: m.id, name: m.name, file_removed: true, freed_bytes: Number(m.size_bytes) };
      }
      case "model_tags":
        return { ...TAGS };
      case "set_model_tags": {
        if (!MODELS.some((m) => m.id === a.id)) throw new Error(`model ${a.id} is not in the library`);
        const clean = [
          ...new Set(
            ((a.tags as string[]) ?? [])
              .map((t) => t.trim().toLowerCase())
              .filter((t) => t && t.length <= 32),
          ),
        ].sort();
        if (clean.length) TAGS[String(a.id)] = clean;
        else delete TAGS[String(a.id)];
        return clean;
      }
      case "set_model_roles": {
        const m = MODELS.find((x) => x.id === a.id);
        if (!m) throw new Error(`model ${a.id} is not in the library`);
        const clean = [
          ...new Set(((a.roles as string[]) ?? []).map((r) => r.trim()).filter(Boolean)),
        ].sort();
        m.roles = clean;
        return clean;
      }
      case "registry_status":
        return {
          source_id: "huggingface",
          last_fetch: now(),
          rate_limit_remaining: 471,
          rate_limited_secs: null,
          token_set: HF_TOKEN.length > 0,
          cache_entries: 3,
        };
      case "set_hf_token":
        HF_TOKEN = String(a.token ?? "").trim();
        return null;
      case "list_benchmarks":
        progressBenchJobs();
        return [...new Map(BENCHMARKS.map((b) => [b.model_id, b])).values()];
      case "model_benchmarks":
        progressBenchJobs();
        return BENCHMARKS.filter((b) => b.model_id === a.id);
      case "benchmark_model": {
        const job = mkJob(`j-bench-${seq++}`, "bench", "running", {
          model_id: String(a.id),
          started_at: now(),
        });
        JOBS.unshift(job);
        return job;
      }
      case "upgrade_check": {
        const job = mkJob(`j-up-${seq++}`, "upgrade_check", "running", {
          params: { target_model_id: String(a.id) },
          started_at: now(),
        });
        JOBS.unshift(job);
        return job;
      }
      case "list_agents":
        return AGENTS;
      case "list_agent_runtimes":
        return [
          { id: "opencode", installed: true },
          { id: "hermes", installed: HERMES_INSTALLED, install: { state: "idle" } },
        ];
      case "install_hermes":
        HERMES_INSTALLED = true;
        return "started";
      case "registry_search": {
        const q = String((a.params as AnyRecord)?.q ?? "").toLowerCase();
        const all = DISCOVER_MODELS.filter(
          (m) => !q || String(m.id).toLowerCase().includes(q),
        );
        return { data: all, freshness: { kind: "live" } };
      }
      case "registry_model": {
        const id = String(a.id ?? "");
        const m = DISCOVER_MODELS.find((x) => x.id === id) ?? DISCOVER_MODELS[0];
        const mid = String(m.id);
        const repo = mid.split("/")[1].toLowerCase();
        return {
          ...m,
          revision: "main",
          freshness: { kind: "live" },
          files: [
            {
              path: "README.md", size_bytes: 4200, sha256: null, quant: null,
              shard: null, download_url: `https://huggingface.co/${mid}/resolve/main/README.md`,
              vram_estimate_mb: null, fit: { level: "unknown" },
            },
            {
              path: `${repo}-q4_k_m.gguf`, size_bytes: 4_683_073_536,
              sha256: "509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c",
              quant: "Q4_K_M", shard: null,
              download_url: `https://huggingface.co/${mid}/resolve/main/model-q4_k_m.gguf`,
              vram_estimate_mb: 5_800, fit: { level: "green" },
            },
            {
              path: `${repo}-q5_k_m.gguf`, size_bytes: 5_444_832_000,
              sha256: "bb".repeat(32), quant: "Q5_K_M", shard: null,
              download_url: `https://huggingface.co/${mid}/resolve/main/model-q5_k_m.gguf`,
              vram_estimate_mb: 6_900, fit: { level: "green" },
            },
            {
              path: `${repo}-q8_0.gguf`, size_bytes: 8_100_000_000,
              sha256: "aa".repeat(32), quant: "Q8_0", shard: null,
              download_url: `https://huggingface.co/${mid}/resolve/main/model-q8_0.gguf`,
              vram_estimate_mb: 9_200,
              fit: { level: "yellow", reason: "needs ~9.0 GB of your ~14.8 GB VRAM budget — little head-room for a longer context or a second resident model" },
            },
          ],
        };
      }
      case "list_downloads":
        // Nudge any running download forward so the progress bar moves.
        for (const d of DOWNLOADS) {
          if (d.state === "running") {
            const size = Number(d.size_bytes);
            const next = Math.min(size, Number(d.bytes_done) + 6e8);
            d.bytes_done = next;
            if (next >= size) {
              d.state = "done";
              d.model_id = `m-dl-${seq++}`;
            }
          }
        }
        // Fresh array + fresh rows every poll — the real core serializes a new
        // Vec each call, and `usePolled` needs a changed reference to re-render.
        return DOWNLOADS.map((d) => ({ ...d }));
      case "enqueue_download": {
        const body = (a.body ?? {}) as AnyRecord;
        const d: AnyRecord = {
          id: `dl-${seq++}`, url: String(body.url ?? ""),
          filename: String(body.filename ?? "model.gguf").split("/").pop(),
          model_type: body.model_type ?? null, sha256: body.sha256 ?? null,
          size_bytes: body.size_bytes ?? 4_683_073_536, bytes_done: 0, retries: 0,
          state: "running", error_text: null, model_id: null,
          created_at: now(), updated_at: now(), roles: body.roles ?? [],
        };
        DOWNLOADS.unshift(d);
        return d;
      }
      case "pause_download": {
        const d = DOWNLOADS.find((x) => x.id === a.id);
        if (d && d.state === "running") d.state = "paused";
        return null;
      }
      case "resume_download": {
        const d = DOWNLOADS.find((x) => x.id === a.id);
        if (d && (d.state === "paused" || d.state === "failed")) d.state = "running";
        return null;
      }
      case "cancel_download": {
        const i = DOWNLOADS.findIndex((x) => x.id === a.id);
        if (i >= 0) DOWNLOADS.splice(i, 1);
        return null;
      }
      case "export_backup":
        return "E:\\AI\\data\\exports\\aiwm-export-2026-01-01T00-00-00Z.zip";
      case "import_backup":
        return {
          core_version: "0.0.1",
          created_at: now(),
          model_count: MODELS.length,
          missing_models: ["Qwen2.5 Coder (coder.gguf)"],
          restart_required: true,
        };
      case "create_agent": {
        const body = (a.body ?? {}) as AnyRecord;
        const agent = {
          id: `a-dev-${seq++}`, name: String(body.name ?? "Agent"),
          adapter: String(body.adapter ?? "opencode"), model_id: body.model_id ?? null,
          workspace_path: String(body.workspace_path ?? "E:\\AI"),
          allowed_paths: body.allowed_paths ?? [], toolset: body.toolset ?? null,
          created_at: now(),
        };
        AGENTS.unshift(agent);
        return agent;
      }
      case "delete_agent":
        return null;
      case "open_agent_session": {
        DEV_SESSION = {
          id: `s-dev-${seq++}`,
          agent_id: String((a.body as AnyRecord)?.agent_id ?? "a-coder"),
          state: "awaiting_approval",
          answered: false,
          stopped: false,
        };
        const first = (a.body as AnyRecord)?.first_message;
        DEV_SESSION.firstMessage = typeof first === "string" ? first : null;
        return devSession();
      }
      case "agent_session_detail":
        return DEV_SESSION && DEV_SESSION.id === String(a.id)
          ? { session: devSession(), events: devEvents(), live: !DEV_SESSION.stopped }
          : null;
      case "agent_session_message":
        if (DEV_SESSION) DEV_SESSION.state = "working";
        return null;
      case "agent_session_permission":
        if (DEV_SESSION) {
          DEV_SESSION.answered = true;
          DEV_SESSION.state = "idle";
        }
        return null;
      case "stop_agent_session":
        if (DEV_SESSION) {
          DEV_SESSION.stopped = true;
          DEV_SESSION.state = "stopped";
        }
        return null;
      default:
        if (cmd.startsWith("plugin:")) return null; // opener plugin etc. — no-op
        console.warn("dev-mock: unhandled command", cmd);
        return null;
    }
    // `shouldMockEvents` lets mockIPC handle `plugin:event|listen/unlisten`
    // itself, so `listen()` (useTelemetry) resolves cleanly.
  }, { shouldMockEvents: true });
  mockWindows("main");
}
