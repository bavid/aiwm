import type { HelpSetting, HelpTopic } from "./types.ts";

/** Diagnostics tab: environment, runtime setup, tool updates, bring your
 *  own engine, GPU processes, registry, log. Facts from
 *  `features/diagnostics/Diagnostics.tsx` and `lib/ipc.ts`. */

const SETTINGS: readonly HelpSetting[] = [
  {
    key: "environment-copy",
    label: "Copy (environment)",
    what: "Copies the Environment card — core version, data dir, store, outputs, runtimes, cache, API port, VRAM budget, offline flag, theme, GPU — to the clipboard as text.",
    why: "It is the first thing to paste into a bug report.",
    effect: "Clipboard only.",
    benefit: "One click instead of eleven.",
  },
  {
    key: "runtime-setup",
    label: "Set up llama.cpp / ComfyUI",
    what: "Installs a managed runtime into the runtimes folder: llama.cpp (about 645 MB) or ComfyUI (several GB — PyTorch is a large download).",
    why: "The app does not ship them; each is set up once, on this machine.",
    effect: "Runs in the background; the row shows downloading / extracting / installing. The button is only shown while the runtime is missing or its last setup failed. ComfyUI's install includes the GGUF and RTX node packs.",
    benefit: "No terminals, no Python environments to manage by hand.",
    pitfalls: "Refused while Offline mode is on.",
  },
  {
    key: "unload-all",
    label: "Unload all models",
    what: "Frees every model every runtime currently holds — one unload request per loaded model.",
    why: "Before a big render or a training run, an empty card is the surest way to avoid a blocked job.",
    effect: "VRAM and RAM are released; the next job that needs a model loads it again. Nothing on disk changes. Errors per model are listed if one refuses.",
    benefit: "A clean slate in one click.",
  },
  {
    key: "tool-updates",
    label: "Check for updates",
    what: "Compares the pinned versions of ComfyUI, llama.cpp, Colibri and Hermes with the latest published upstream (GitHub Releases / PyPI); OpenCode is reported as \"bring your own\".",
    why: "The app pins versions for reproducibility; this says how far behind the pins are.",
    effect: "A handful of real HTTP requests, only when you press it (never polled). Amber = an update exists, red = the check failed. It reports; it does not upgrade.",
    benefit: "Know when a newer runtime is worth the reinstall.",
    pitfalls: "Needs network; blocked by Offline mode.",
  },
  {
    key: "byo-engine",
    label: "Attach (bring your own engine)",
    what: "Uses an LLM server that is already running on this machine — Ollama (:11434), LM Studio (:1234) or any llama-server on 127.0.0.1 — as the chat runtime instead of the managed llama.cpp.",
    why: "If you already run a model server, there is no reason to load the model twice.",
    effect: "Chat and Agents use it exactly like a managed server; its process is never started or stopped by AIWM. Detach returns to the managed runtime.",
    benefit: "One resident model for every tool on the machine.",
    pitfalls: "127.0.0.1 only. The runtime row then reads \"attached to …\".",
  },
  {
    key: "byo-vram",
    label: "VRAM estimate (MB)",
    what: "How much VRAM the attached server's model holds — your figure, in MB.",
    why: "AIWM cannot inspect a process it does not manage; the scheduler needs a number for its budget maths.",
    effect: "Used only for planning: a too-low value lets image jobs start and hit real out-of-memory; a too-high one blocks them needlessly.",
    benefit: "Honest scheduling next to a foreign process.",
  },
  {
    key: "gpu-processes",
    label: "GPU processes",
    what: "Every process holding VRAM right now, by PID, with its share.",
    why: "When a job is blocked and AIWM holds nothing, this shows who does.",
    effect: "Read-only telemetry.",
    benefit: "Find the browser tab eating 3 GB.",
  },
  {
    key: "registry",
    label: "Model registry",
    what: "The Hugging Face registry client's state: last fetch, cache entries, rate-limit budget or back-off, and whether a token is set.",
    why: "Discover and the catalogue depend on it; a back-off explains a slow search.",
    effect: "Read-only. The token is set under Settings → Network & API.",
    benefit: "See why a search is stale or slow.",
  },
  {
    key: "log",
    label: "Log",
    what: "The tail of aiwm.log, the core's own log file, auto-scrolled.",
    why: "Every job, install, deletion and error is a line here.",
    effect: "Read-only; the file lives in the logs folder under the data dir.",
    benefit: "The full story behind a failed job.",
  },
];

export const DIAGNOSTICS_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "diagnostics",
    title: "What the Diagnostics tab is for",
    summary: "Where the app lives, which runtimes are installed and healthy, what holds the GPU, and the log — plus the two setup buttons everything else depends on.",
    body: [
      {
        kind: "p",
        text: "Runtimes are listed with a health dot, their detail line (\"not installed\", a version, \"attached to …\", an install phase) and the VRAM they hold. Setup buttons appear only where they apply.",
      },
    ],
    settings: SETTINGS,
  },
  {
    id: "flow",
    area: "diagnostics",
    title: "Step by step: first setup",
    summary: "Set up llama.cpp for chat, ComfyUI for media; then import models.",
    body: [
      {
        kind: "steps",
        items: [
          "Runtimes → Set up llama.cpp. Wait for the row to turn healthy.",
          "Runtimes → Set up ComfyUI. Several GB; it runs in the background while you continue.",
          "Models tab → import or download a model for each.",
          "Optional: attach an Ollama or LM Studio you already run instead of step 1.",
        ],
      },
    ],
  },
  {
    id: "disk",
    area: "diagnostics",
    title: "What is on disk",
    summary: "The paths in the Environment card are the whole footprint.",
    body: [
      {
        kind: "list",
        items: [
          "data dir — database, config.toml, logs, exports, voice identities.",
          "store — the model library's files.",
          "outputs — generated media (and, by default, dataset work folders).",
          "runtimes — llama.cpp, ComfyUI, the trainer.",
          "cache — the registry cache; disposable.",
          "Every one of them can be measured and, where configurable, moved under Settings → Storage & data.",
        ],
      },
    ],
  },
];
