import type { HelpSetting, HelpTopic } from "./types.ts";

/** Dashboard: the meters, what is resident, the queue, the setup checklist.
 *  Facts from `features/dashboard/Dashboard.tsx`, `components/Meter.tsx`,
 *  `lib/units.ts` and the scheduler notes in `docs/TODO.md`. */

const SETTINGS: readonly HelpSetting[] = [
  {
    key: "unload",
    label: "Unload",
    what: "Frees the VRAM and RAM one resident model holds, right now, without waiting for the scheduler to evict it.",
    why: "A chat model left loaded takes most of a 16 GB card; the next image or video job would have to wait for it, or be blocked.",
    effect: "The runtime drops the model. The next job that needs it loads it again (a load is seconds to a minute, depending on size). Nothing on disk changes.",
    benefit: "Make room deliberately before a big render instead of finding out from a blocked job.",
    pitfalls: "A model being used by a running job cannot be freed this way — cancel the job first.",
  },
  {
    key: "setup-checklist",
    label: "Get set up",
    what: "A first-run checklist: install a runtime, import a model, generate something. Each step has an Open button that jumps to the right tab.",
    why: "The three steps are the minimum before anything works, and each lives on a different tab.",
    effect: "Steps tick themselves off as they happen. The card hides itself when all three are done, or when you press Dismiss — and then never returns.",
    benefit: "Nothing to remember on the first day.",
    pitfalls: "Dismiss is remembered in this browser profile; the card does not come back after a reinstall of a runtime.",
  },
];

export const DASHBOARD_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "dashboard",
    title: "What the Dashboard is for",
    summary: "The one screen that says what the machine is doing right now: GPU, RAM, disk, what is loaded, what is queued.",
    body: [
      {
        kind: "p",
        text: "Every number here is read live from the core (GPU and host telemetry, the runtime list, the job list, the storage report). Nothing is stored as a time series — the seven-day usage bars are counted from the jobs table on the client, so they cover only what is still in the job history.",
      },
    ],
    settings: SETTINGS,
  },
  {
    id: "cards",
    area: "dashboard",
    title: "The cards",
    summary: "GPU, Host, Resident right now, Usage, Jobs, What do you want to do, Recent activity.",
    body: [
      {
        kind: "list",
        items: [
          "GPU — the card's name, VRAM in use versus total (shown in GiB, since memory is reported in MiB), utilisation, temperature, and the Budget: the VRAM the scheduler plans against (Settings → Performance; 0 means \"the whole card\"). When telemetry is unavailable the reason is shown instead.",
          "Host — RAM in use, CPU load, and the model store's drive: used versus total (in decimal GB, the unit file sizes are quoted in).",
          "Resident right now — every runtime (llama.cpp, ComfyUI, Colibri, the sidecars) with its health dot, the models it has loaded, the VRAM it holds, and an Unload button per loaded model.",
          "Usage, last 7 days — jobs created per calendar day, from the last 300 jobs in the history.",
          "Jobs — \"1 running · N queued\", the newest twelve jobs with a Cancel button while they can still be cancelled, and \"View all\" to the Jobs tab.",
          "What do you want to do? — shortcuts to Chat, Image, Video and Agents.",
          "Recent activity — the last six finished jobs with their state and, for a failure, the error text.",
        ],
      },
    ],
  },
  {
    id: "reading-vram",
    area: "dashboard",
    title: "Reading the VRAM meter",
    summary: "What the scheduler sees, why a job is blocked, and why other applications count.",
    body: [
      {
        kind: "p",
        text: "The meter shows the whole card, including what other applications hold (a browser, a game, the desktop). The scheduler uses that same live figure: a job is blocked when the model's estimate does not fit into what is free, even if AIWM itself holds nothing. Unload frees AIWM's own models; closing other applications frees the rest.",
      },
      {
        kind: "list",
        items: [
          "2026-09-16: loading Mistral-Small-3.2-24B (IQ3_M GGUF, 8,192-token context, everything on the GPU) took the card from a 909 MB idle desktop to a 12,599 MB peak — 11,690 MB for the model. The estimate's fixed overhead was recalibrated to 350 MB from that run.",
          "The idle desktop alone holds part of the card before any job runs: about 1.4 GB during the 2026-09-17 image measurements, 1,808–1,818 MiB during the 2026-09-19 captioner measurements (RTX 4080 SUPER, 16 GB).",
        ],
      },
    ],
  },
];
