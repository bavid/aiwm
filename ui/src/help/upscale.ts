import type { HelpSetting, HelpTopic } from "./types.ts";

/** Upscale: the one-click RTX Video Super Resolution step on a finished
 *  image or clip. Facts from `features/image/Image.tsx` / `video/Video.tsx`
 *  (`handleUpscale`, `upscaleSummary`), `lib/ipc.ts` (`UpscaleParams`) and
 *  the RTX VSR entry in `docs/TODO.md` (2026-09-15). */

const SETTINGS: readonly HelpSetting[] = [
  {
    key: "upscale",
    label: "Upscale",
    what: "Sends a finished image or clip through NVIDIA's RTX Video Super Resolution node in ComfyUI and saves the result as a new job.",
    why: "A one-megapixel render or a 480p clip is fine to judge and small to store; the keeper deserves more pixels.",
    effect: "A new job of type \"upscale\" is queued with the source job as its input; the defaults on the Rust side are 2× and ULTRA quality, so there is nothing to fill in. It runs through the same scheduler as a render (VRAM slot, queue), not in place — the Result card switches to it and shows \"upscaling…\". The source stays untouched; the output is a new file in the generated-media folder.",
    benefit: "Twice the resolution of the picture or clip you already like, in one click.",
    pitfalls: "The button is offered on completed image and video jobs, not on an upscale result itself. A longer clip takes a while. The node is part of the managed ComfyUI install; an installation from before the feature reports \"Node 'RTXVideoSuperResolution' not found\" until the node pack is installed again.",
    measured: "2026-09-15: a real 256×256 SDXL image upscaled 2× through the core's API came back as a 512×512 PNG (294 KB), no error.",
  },
  {
    key: "factor",
    label: "RTX Video Super Resolution (result line)",
    what: "The result's meta line: the scale (2.00×) or a fixed width × height, and the quality level (ULTRA).",
    why: "It records what the upscale job actually did, since the form has no fields for it.",
    effect: "Informational. The core supports a scale or explicit dimensions and a quality level per job; the buttons on the Image and Video tabs always submit the defaults.",
    benefit: "Know which settings produced the file.",
  },
];

export const UPSCALE_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "upscale",
    title: "What Upscale is",
    summary: "An RTX Video Super Resolution pass on a finished image or clip — a separate ComfyUI job, 2× by default.",
    body: [
      {
        kind: "p",
        text: "The official Comfy-Org RTX node pack (Apache-2.0) is installed into the managed ComfyUI next to the GGUF node pack. An upscale is a real job: it takes a VRAM slot, waits in the queue behind renders, and writes a new output rather than overwriting the source. The source job's id is recorded on the result (\"Upscaled from\").",
      },
    ],
    settings: SETTINGS,
  },
  {
    id: "flow",
    area: "upscale",
    title: "Step by step",
    summary: "Render, select the result, press Upscale, wait, download.",
    body: [
      {
        kind: "steps",
        items: [
          "On the Image or Video tab, select a completed result (from the gallery, the queue or right after it finishes).",
          "Press Upscale. The Result card switches to the new job.",
          "Wait: an image is quick; a clip is processed frame by frame and takes a while.",
          "Download the result, or delete it — the original is still there.",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "upscale",
    title: "Limits and pitfalls",
    summary: "Fixed defaults, one pass, GPU-bound.",
    body: [
      {
        kind: "list",
        items: [
          "The UI submits 2× / ULTRA; there is no field for another factor or size yet, although the core accepts them.",
          "An upscale result cannot be upscaled again from the button.",
          "It is not DLSS: RTX VSR is NVIDIA's video-upscaling filter, run as a batch node here; it needs an RTX card and the node pack in ComfyUI.",
          "Like any ComfyUI job it is blocked while VRAM is short and cancelled from the queue like a render.",
        ],
      },
    ],
  },
];
