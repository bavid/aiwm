import type { HelpSetting, HelpTopic } from "./types.ts";

/** Settings tab: appearance, data locations, retention and cleanup, the
 *  scheduler budget and Auto preference, llama.cpp and ComfyUI options,
 *  tokens and the local API, offline mode, backup. Facts from
 *  `features/settings/Settings.tsx`, `DataLocations.tsx`, `BackupCard.tsx`,
 *  `Cleanup.tsx`, `lib/ipc.ts` and `docs/TODO.md` (2026-09-16, 2026-09-19). */

const GENERAL_SETTINGS: readonly HelpSetting[] = [
  {
    key: "theme",
    label: "Theme",
    what: "System, Light or Dark.",
    why: "Follow the OS, or pin one.",
    effect: "Applies instantly and is remembered in this browser profile — not in config.toml, so it is not part of a backup.",
    benefit: "Your eyes, your choice.",
  },
];

const STORAGE_SETTINGS: readonly HelpSetting[] = [
  {
    key: "data-locations",
    label: "Data locations",
    what: "Every folder the app writes to — generated media, dataset work folders, training runs, the model store, runtime installs, cache, download staging — with its size, file count, its drive's free space, an Open folder button, and (where configurable) a custom folder.",
    why: "Video, datasets and training fill drives; moving a location to a bigger drive is the fix.",
    effect: "A custom folder applies after Save and a restart. Existing data stays where it is; new datasets, runs and outputs go to the new folder. Blank means the portable default next to the app (the model store always has a path). \"Refresh sizes\" walks the folders again — a recursive walk, so it is on demand, not on a timer. The Cleanup section lists what the app itself generated and could be removed; deleting a model happens under Models → Maintenance.",
    benefit: "See where the gigabytes are and move them without moving the app.",
    pitfalls: "Only new work goes to the new folder; the old folder keeps what it has. Windows paths are compared case-insensitively.",
    measured: "2026-09-19: a dataset prep with a custom \"Store frames in\" folder wrote all 1,368 files (1,173,444,536 B) under <folder>\\<job id> in 296.6 s while the default datasets folder stayed empty.",
  },
  {
    key: "retention-age",
    label: "Delete outputs older than (days)",
    what: "Generated media files last modified more than this many days ago are deleted automatically; 0 disables the age rule.",
    why: "The generated-media folder grows with every render and video files are large.",
    effect: "Applies immediately after Save, no restart: the rule runs on \"Clean up now\" and on an optional sweep at start. Only files are deleted; the job rows stay, so an old job shows without a preview. Datasets, training runs, models and anything outside the outputs folder are never touched.",
    benefit: "A folder that stops growing forever.",
    pitfalls: "Deleted files are gone; keep what you want by downloading it first.",
  },
  {
    key: "retention-size",
    label: "Keep total size under (MB)",
    what: "When the folder exceeds this many MB, the oldest files are deleted first until it fits; 0 disables the size rule. Applied after the age rule.",
    why: "A hard cap is easier to reason about than an age.",
    effect: "Same as the age rule: files only, immediately after Save.",
    benefit: "A known maximum for the folder.",
  },
  {
    key: "clean-up-now",
    label: "Clean up now",
    what: "Runs the saved retention policy against the generated-media folder right now and reports how many files it removed and how much it freed.",
    why: "You do not have to wait for the next start.",
    effect: "Reads config.toml fresh — an unsaved edit above is not what runs; Save first. Disabled while both limits are 0.",
    benefit: "Free the space when you need it.",
  },
];

const CLEANUP_SETTINGS: readonly HelpSetting[] = [
  {
    key: "cleanup-page",
    label: "Cleanup",
    what: "Scan finds what the app itself generated and could remove, grouped: generated media beyond the retention rule or without a job, discarded dataset frames, dataset work folders no dataset claims, frame entries whose files are missing, finished training runs' work folders, caches and leftovers, logs older than 30 days, database backups. Each group shows its files and size; pick whole groups or single entries, Preview lists the exact files, Delete removes them.",
    why: "Renders, curation and training leave gigabytes behind in places you would have to know about; one page finds them all and says what each is.",
    effect: "Scan only reads (a recursive walk of the app's folders — on demand, never on a timer). Preview is a dry run: nothing is deleted. Delete goes through the same guards as a dataset's Housekeeping and a run's purge, best-effort per file with the skipped ones listed and why, then scans again and writes the Cleanup history. Frame entries whose files are missing free no space; removing them clears broken thumbnails.",
    benefit: "See where the space went and get it back in two clicks, with a list of exactly what left.",
    pitfalls: "Deleted files are gone. Never listed: models and the model store, runtimes, library LoRAs, voice identities, your source media, and anything a running or paused job, run or download still needs — the Protected list says what was held back and why. While a selected dataset, run or download is busy the whole request is refused, also for a preview; nothing is deleted then and the selection stays. A finished run's whole folder is offered only when its result LoRA is in the library; otherwise only checkpoints, optimizer state, samples and the log.",
  },
];

const PERFORMANCE_SETTINGS: readonly HelpSetting[] = [
  {
    key: "vram-budget",
    label: "VRAM budget (MB)",
    what: "How much VRAM the scheduler plans against; 0 auto-detects the card's total. The current figure is shown next to the field.",
    why: "It is the ceiling for every fit verdict and every \"blocked\" decision.",
    effect: "After a restart. The scheduler also caps this by the VRAM that is really free (other applications count), so a lower budget only makes it more cautious. A model whose weights alone exceed the budget is refused outright.",
    benefit: "Leave headroom for a game or a browser by lowering it; leave 0 to use the card.",
  },
  {
    key: "auto-preference",
    label: "Model selection (Auto)",
    what: "How Auto picks among a role's models: Balanced (speed, stability and size together), Prefer fast (highest tok/s that fits) or Prefer quality (biggest model that fits). A model that fits the budget always wins over one that does not; without benchmark data Auto uses the most-recently-used model.",
    why: "Auto is what Chat, the prompt assistant and the ranked search use when you do not pick a model.",
    effect: "After a restart. Run Test on the Models tab to give Auto numbers to rank by.",
    benefit: "Auto that reflects your priority instead of your last click.",
  },
];

const LLAMA_SETTINGS: readonly HelpSetting[] = [
  {
    key: "gpu-layers",
    label: "GPU layers (-ngl)",
    what: "How many of the model's layers llama.cpp puts on the GPU; 999 offloads everything.",
    why: "Layers on the GPU are fast; layers left on the CPU save VRAM at a large speed cost.",
    effect: "On the next model load. Fewer layers: less VRAM, slower answers.",
    benefit: "Squeeze a model that is a little too big onto the card.",
    measured: "2026-09-16: Mistral-Small-3.2-24B IQ3_M with -ngl 999 at 8,192 context took 11,690 MB.",
  },
  {
    key: "ctx-size",
    label: "Context size (-c)",
    what: "The context window in tokens; 0 caps at the model's trained context.",
    why: "Context costs VRAM (the KV cache) and sets how much of a long prompt or document the model can see.",
    effect: "On the next load. The chat default is 8,192; agent sessions use the model's full context regardless.",
    benefit: "Lower it to fit a bigger model; raise it for long documents.",
    pitfalls: "Hermes needs 64K; Qwen2.5-Coder 14B is 32K natively (2026-09-12).",
  },
  {
    key: "load-timeout",
    label: "Load timeout (seconds)",
    what: "How long a model load may take before it counts as failed: 10 to 3,600.",
    why: "A huge model from a slow disk can legitimately take minutes.",
    effect: "On the next load.",
    benefit: "No false failures on a slow drive.",
  },
  {
    key: "flash-attention",
    label: "Flash attention",
    what: "Passes --flash-attn on to llama-server.",
    why: "A faster attention kernel that also saves VRAM on long contexts.",
    effect: "On the next load.",
    benefit: "Free speed on a supported card.",
  },
  {
    key: "jinja",
    label: "Jinja chat template",
    what: "Passes --jinja: use the model's embedded chat template with Jinja rendering.",
    why: "Needed for tool calls (agents) and correct for chat.",
    effect: "On the next load. Turn off only if a model's embedded template misbehaves.",
    benefit: "Tool calling works.",
  },
  {
    key: "chat-template",
    label: "Chat template override",
    what: "A --chat-template name (e.g. qwen2.5-coder); empty uses the GGUF's own.",
    why: "Some GGUFs ship a wrong or missing template.",
    effect: "On the next load, for every model.",
    benefit: "Fix a broken template without re-downloading.",
    pitfalls: "It applies to every model, not just the one that needs it.",
  },
];

const COMFY_SETTINGS: readonly HelpSetting[] = [
  {
    key: "vram-mode",
    label: "VRAM mode",
    what: "The --*vram flag ComfyUI starts with: Auto, High VRAM (keep everything on the GPU), Normal, Low VRAM (offload aggressively — helps FLUX on 16 GB), No VRAM (minimal GPU use, slow).",
    why: "ComfyUI's own offloading decides what stays on the card between nodes.",
    effect: "Applies live — a running ComfyUI server is restarted for you. \"Normal\" starts ComfyUI with no flag (ComfyUI has no --normalvram; the app used to pass one and crash-loop, fixed).",
    benefit: "Low VRAM lets a model that is just too big render at a speed cost.",
    pitfalls: "The scheduler's fit check does not know about the mode: a model it judges too big is refused before ComfyUI's offloading gets a chance.",
  },
  {
    key: "reserve-vram",
    label: "Reserve VRAM (GB)",
    what: "--reserve-vram: VRAM ComfyUI keeps free for the OS; 0 is off; 0 to 8 in steps of 0.5.",
    why: "Helps avoid out-of-memory on long video clips.",
    effect: "Applies live (server restart).",
    benefit: "A safety margin for video.",
  },
  {
    key: "extra-args",
    label: "Extra args",
    what: "Arguments appended verbatim to ComfyUI's command line, e.g. --fast --use-sage-attention.",
    why: "Power-user flags the form does not know.",
    effect: "Applies live (server restart). A wrong flag stops ComfyUI from starting — the Diagnostics log shows why.",
    benefit: "Every ComfyUI option, without editing a script.",
  },
];

const NETWORK_SETTINGS: readonly HelpSetting[] = [
  {
    key: "hf-token",
    label: "Hugging Face access token",
    what: "Optional; only for gated repos or if you hit the anonymous rate limit.",
    why: "Gated FLUX and some LLM repos refuse anonymous downloads.",
    effect: "Stored on this machine only (never in a backup); after a restart. Clear removes it.",
    benefit: "Gated stacks download like any other.",
  },
  {
    key: "civitai-key",
    label: "Civitai API key",
    what: "Optional; for gated or early-access content and higher rate limits. Anonymous browsing works without one.",
    why: "Some community models are members-only.",
    effect: "Machine-local, never in a backup, after a restart.",
    benefit: "Access to what your account can see.",
  },
  {
    key: "local-api",
    label: "Local API",
    what: "One OpenAI-compatible endpoint (/v1/…) on this machine that forwards to whichever model is loaded on llama.cpp; gated by a bearer token you set here.",
    why: "External tools (continue.dev, aider, …) can point at one stable address instead of tracking runtime ports.",
    effect: "The token applies immediately (the proxy re-reads it per request). Without a token the endpoint refuses every request; with nothing loaded it answers 503.",
    benefit: "Your editor talks to your local model.",
    pitfalls: "The endpoint is this machine only; the token is not in a backup.",
  },
  {
    key: "offline-mode",
    label: "Offline mode",
    what: "Blocks every outbound network call: model downloads, stack and captioner installs, runtime installs, Hugging Face and Civitai searches, the ranked search, upgrade and tool-update checks.",
    why: "A guarantee that nothing leaves the machine — and no surprise downloads.",
    effect: "Applies instantly on Save. Every refused action says \"Offline mode is on\" where you pressed it; Discover shows its cached results with their age. The sidebar badge reads \"offline\".",
    benefit: "Air-gap the app with one checkbox.",
    pitfalls: "Installs and downloads simply fail until you turn it off — the message tells you where.",
  },
];

const BACKUP_SETTINGS: readonly HelpSetting[] = [
  {
    key: "backup-export",
    label: "Export backup",
    what: "Writes a portable .zip: a database snapshot (agent profiles, sessions, transcripts, model records, jobs), config.toml and a model manifest (names and hashes) — not the model files.",
    why: "Everything you configured and everything you chatted, in one file; models are re-downloadable.",
    effect: "The archive lands in the exports folder under the data dir; \"reveal\" opens it. Tokens are not included.",
    benefit: "Move to a new machine, or roll back.",
  },
  {
    key: "backup-restore",
    label: "Restore",
    what: "Stages an export archive; it takes effect on the next start, keeping the replaced files as *.pre-import.",
    why: "A restore must not replace a database the running app has open.",
    effect: "The summary says which core version wrote it, when, how many models it references and which of those are not in this machine's store — re-import those after the restart.",
    benefit: "A known-good state back in two steps.",
    pitfalls: "Restart to finish. Model files must be brought back separately.",
  },
];

export const SETTINGS_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "settings",
    title: "What the Settings tab is for",
    summary: "Everything in config.toml, in seven sections: General, Storage & data, Cleanup, Performance, Runtimes, Network & API, Backup. Each card says whether it applies instantly, on the next model load, live, or after a restart.",
    body: [
      {
        kind: "p",
        text: "Save changes writes the whole form to config.toml. Offline mode and the ComfyUI options apply at once (ComfyUI is restarted for you); retention applies at once; store path, data locations, VRAM budget, model selection and llama.cpp options take effect after a restart or the next model load — the confirmation message repeats this.",
      },
    ],
  },
  {
    id: "general",
    area: "settings",
    title: "General",
    summary: "Appearance.",
    body: [],
    settings: GENERAL_SETTINGS,
  },
  {
    id: "storage",
    area: "settings",
    title: "Storage & data",
    summary: "Where every folder is and how big; retention for generated media.",
    body: [
      {
        kind: "p",
        text: "Data locations measure and move; the Generated media card prunes. Deleting generated data also lives where it was made: a dataset's Housekeeping (clean up discarded frames, delete the dataset), a training run's Delete + purge, a model's Delete under Models → Maintenance. The Cleanup section lists all of it with sizes, previews what would be freed and keeps a history.",
      },
    ],
    settings: STORAGE_SETTINGS,
  },
  {
    id: "cleanup",
    area: "settings",
    title: "Cleanup",
    summary: "Find what the app generated and could remove, preview the exact files, delete behind a confirmation, and see the history.",
    body: [
      {
        kind: "p",
        text: "Nothing is scanned until you press Scan, and nothing is deleted until you press Delete in the preview. The page calls the same functions as the dataset's Housekeeping and a run's Delete + purge, so the same guards apply: never source files, never outside the app's own folders, never links, never a busy dataset or run.",
      },
    ],
    settings: CLEANUP_SETTINGS,
  },
  {
    id: "performance",
    area: "settings",
    title: "Performance",
    summary: "The scheduler's VRAM budget and how Auto picks a model.",
    body: [],
    settings: PERFORMANCE_SETTINGS,
  },
  {
    id: "llama",
    area: "settings",
    title: "Runtimes: llama.cpp",
    summary: "The llama-server flags: GPU layers, context, timeout, flash attention, Jinja, chat template. Apply on the next model load.",
    body: [],
    settings: LLAMA_SETTINGS,
  },
  {
    id: "comfyui",
    area: "settings",
    title: "Runtimes: ComfyUI",
    summary: "VRAM mode, reserved VRAM, extra arguments. Apply live — a running server is restarted.",
    body: [],
    settings: COMFY_SETTINGS,
  },
  {
    id: "network",
    area: "settings",
    title: "Network & API",
    summary: "Tokens for Hugging Face and Civitai, the local OpenAI-compatible endpoint, and Offline mode.",
    body: [
      {
        kind: "p",
        text: "Tokens are stored on this machine only and never in a backup. The Hugging Face and Civitai tokens apply after a restart; the local API token applies immediately.",
      },
    ],
    settings: NETWORK_SETTINGS,
  },
  {
    id: "backup",
    area: "settings",
    title: "Backup",
    summary: "Export and restore a portable archive of the database, config and model manifest.",
    body: [],
    settings: BACKUP_SETTINGS,
  },
];
