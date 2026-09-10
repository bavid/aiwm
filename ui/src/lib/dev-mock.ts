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
  mkModel("m-qwen", "Qwen2.5 7B Instruct", { family: "qwen2", roles: ["chat"], runtimes: ["llamacpp"], vram_estimate_mb: 6400 }),
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
    error_text: null, output_path: null, result: null, ...over,
  };
}

const RUNTIMES: AnyRecord[] = [
  { id: "llamacpp", kind: "llama_cpp", health: "unknown", vram_used_mb: 0, detail: "installed · idle" },
  { id: "comfyui", kind: "comfy_ui", health: "healthy", vram_used_mb: 0, detail: "running on :48096 · ComfyUI 0.34.0 · model reserved" },
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
};

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
        return MODELS;
      case "list_known_models":
        return [];
      case "list_jobs":
        return JOBS;
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
        });
        JOBS.unshift(job);
        return job;
      }
      case "cancel_job":
        return true;
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
          created_at: now(), updated_at: now(),
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
