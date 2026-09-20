import type { HelpTopic } from "./types.ts";

/** Getting started: what the app is, the first-run checklist, how the tabs
 *  fit together, and how every job goes through one queue on one GPU. Facts
 *  from `App.tsx`, `Dashboard.tsx` (the setup checklist), `Diagnostics.tsx`
 *  and the scheduler notes in `docs/TODO.md`. */

export const GETTING_STARTED_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "getting-started",
    title: "What AIWM is",
    summary: "One desktop app that runs chat, image, video, voice, agents and LoRA training on your own GPU — nothing leaves the machine unless you download a model.",
    body: [
      {
        kind: "p",
        text: "AI Workstation Manager wraps the tools that would otherwise each need their own install and terminal — llama.cpp for chat, ComfyUI for images and video, a text-to-speech sidecar, OpenCode or Hermes for coding agents, ai-toolkit for training — behind one window. Every model file is imported once into a model library, every piece of work is a job in one queue, and one scheduler hands the GPU from job to job.",
      },
      {
        kind: "p",
        text: "The core runs as a local service on 127.0.0.1 (the port is on the Diagnostics tab). The only outbound traffic is what you ask for: model downloads, runtime installs, Hugging Face and Civitai searches, the tool-update check and the \"Better?\" upgrade check. Offline mode (Settings → Network) blocks all of it.",
      },
    ],
  },
  {
    id: "first-run",
    area: "getting-started",
    title: "First run: three steps",
    summary: "Install a runtime, import a model, generate something — the Dashboard's checklist walks you through it and disappears when all three are done.",
    body: [
      {
        kind: "steps",
        items: [
          "Diagnostics → Runtimes: press \"Set up llama.cpp\" (about 645 MB) for chat, coding and benchmarks; \"Set up ComfyUI\" (several GB — PyTorch is a large download) for images, video and upscaling. Both run in the background; the row shows the download and unpack progress.",
          "Models → Add models: pick a curated stack. \"Download entire stack\" queues every file a model needs (a FLUX stack is four files); the fit badge tells you beforehand whether it fits your card. A .gguf or .safetensors you already have can be imported by path on the same page.",
          "Generate: type a message on the Chat tab, or a prompt on the Image tab. The first image job of a session also starts the ComfyUI server and loads the model, so it is slower than the ones after it.",
        ],
      },
      {
        kind: "p",
        text: "The Dashboard's \"Get set up\" card ticks each step off as it happens and hides itself once all three are done (or when you dismiss it — it never comes back).",
      },
    ],
  },
  {
    id: "tabs",
    area: "getting-started",
    title: "The tabs, in one line each",
    summary: "Where to go for what.",
    body: [
      {
        kind: "list",
        items: [
          "Dashboard — GPU, RAM and disk meters, what is loaded right now, the queue, recent activity.",
          "Chat — conversations with a local chat model; personas; /image, /video and /edit inside a conversation; documents attached to a chat.",
          "Image — text-to-image and image editing with SDXL, FLUX.1 and FLUX.2 [klein]; LoRA stacks; Hi-res fix; upscaling.",
          "Video — text-to-video and image-to-video with Wan 2.2 or LTX-Video; upscaling.",
          "Voice — an off-screen narrator: Kokoro (fast, 28 English voices) or Dia (expressive, voice cloning from a reference clip).",
          "Stories — Story Studio: characters, locations, a scene timeline, character-consistent images.",
          "Dataset — turn videos and images into a curated, captioned training set.",
          "Training — train a LoRA on an exported dataset, watch samples, test it on the Image tab.",
          "Jobs — every job of every kind, with filters and Cancel.",
          "Agents — OpenCode or Hermes coding sessions in a sandboxed workspace, or an external terminal.",
          "Models — the library, the curated catalogue, Discover (Hugging Face and Civitai), downloads, storage and maintenance.",
          "Benchmark — tokens per second of a chat model on fixed test sets; compare models and runs.",
          "Diagnostics — environment paths, runtime installs, tool updates, bring-your-own engine, GPU processes, the log.",
          "Settings — theme, data locations, retention, VRAM budget, model selection, llama.cpp and ComfyUI options, tokens, offline mode, backup.",
          "Help — this page. Ctrl+K also lists every topic.",
        ],
      },
    ],
  },
  {
    id: "jobs-and-gpu",
    area: "getting-started",
    title: "How work reaches the GPU",
    summary: "Everything is a job; one runs at a time; \"blocked\" means it is waiting for VRAM, not that it failed.",
    body: [
      {
        kind: "p",
        text: "A message, a render, a narration, a benchmark, a dataset prep, a training run: each becomes a job with a type, a state and a result. The scheduler runs one at a time. Before it starts a job it compares the model's VRAM estimate with the VRAM that is really free (read live from the GPU) and, when another model in memory is in the way, unloads that model first — but only when doing so would actually make room.",
      },
      {
        kind: "list",
        items: [
          "queued — waiting its turn.",
          "blocked — not enough free VRAM right now. The job waits and is retried whenever something else finishes or unloads; the reason (\"needs 15,329 MB, this GPU has 14,840 MB usable\" and the like) is shown on the job. Blocked jobs never lock a form: you can submit a smaller request meanwhile.",
          "preparing / running / post — loading the model, generating, finishing up.",
          "completed / failed / cancelled — done; the output, the error text, or nothing.",
        ],
      },
      {
        kind: "p",
        text: "A model that does not fit the card at all (its weights alone exceed the usable VRAM) is refused outright rather than queued forever — the message says so and suggests a smaller quant. Other applications holding VRAM (a browser, a game) count too; \"Unload\" on the Dashboard frees what AIWM itself holds.",
      },
    ],
  },
  {
    id: "where-things-are",
    area: "getting-started",
    title: "Where things are on disk",
    summary: "One data folder next to the app, with a subfolder per kind; every one of them is listed with its size under Settings → Storage & data.",
    body: [
      {
        kind: "list",
        items: [
          "Model store — every imported model file; usually the largest folder. Import moves the file in unless you tick \"keep the original\".",
          "Generated media — every image and video clip a job produced; it grows with every job and is the only folder with an automatic retention rule.",
          "Dataset work folders and Training runs — one folder per prep run or training run; both can be pointed at another drive per run.",
          "Runtime installs — llama.cpp, ComfyUI, the trainer; several GB.",
          "Cache — the model-registry cache; disposable.",
          "config.toml — every setting on the Settings tab; the database — jobs, sessions, models, stories, datasets, runs.",
        ],
      },
      {
        kind: "p",
        text: "Diagnostics → Environment lists the exact paths; \"Copy\" puts them on the clipboard for a bug report.",
      },
    ],
  },
];
