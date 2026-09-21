import type { HelpSetting, HelpTopic } from "./types.ts";

/** Image tab: the form (size, steps, CFG, seed, model), editing, the LoRA
 *  stack, Hi-res fix, sessions and queue, the result actions. Facts from
 *  `features/image/Image.tsx`, `HiresFixField.tsx`, `hires-fix.ts`,
 *  `components/LoraPicker.tsx`, `VramEstimateHint.tsx`, `SessionSidebar.tsx`,
 *  `QueueList.tsx` and the Hi-Res-Fix measurements in `docs/TODO.md`
 *  (2026-09-17). */

const FORM_SETTINGS: readonly HelpSetting[] = [
  {
    key: "edit-source",
    label: "Edit an existing image",
    what: "A finished image from this app, or an image file on disk, that the prompt then edits instead of starting from nothing.",
    why: "Changing one thing about a picture you already have (\"make the hair blonde\") is a different job from generating a new one.",
    effect: "The form switches to edit mode: the prompt becomes an edit instruction, size and negative prompt disappear (the edited image keeps the source's size), Hi-res fix is not offered. Editing needs the FLUX.2 [klein] 9B stack — pick it as the model.",
    benefit: "Iterate on a result instead of re-rolling it.",
    pitfalls: "A file path only works from the desktop app (the browser preview has no file picker). Set the field back to \"None\" to generate from a prompt again.",
  },
  {
    key: "negative-prompt",
    label: "Negative prompt",
    what: "What the image should not contain — \"blurry, low quality, watermark\".",
    why: "Steering away from typical failure modes is often more effective than adding more positive words.",
    effect: "Sent with the prompt; the preset picker below it appends saved phrases. Not offered in edit mode.",
    benefit: "Fewer artefacts for one line of text.",
    pitfalls: "FLUX runs at CFG 1, where the negative prompt has little or no effect.",
  },
  {
    key: "size",
    label: "Width and height",
    what: "The size of the first render, in pixels: 512 to 2,048 on each side, in steps of 64. The Square / Portrait / Landscape chips set 1024×1024, 832×1216 and 1216×832; the arrow swaps the two.",
    why: "Every model has a native size (SDXL and FLUX.2 [klein]: about one megapixel); far from it, composition falls apart.",
    effect: "More pixels: more VRAM and more time, roughly with the pixel count. Values are snapped to a multiple of 64 when you generate. With Hi-res fix on, this is the first pass; the final image is bigger.",
    benefit: "Stay near a megapixel for the first pass and let Hi-res fix add the pixels.",
    measured: "2026-09-17: SDXL at 1024×1024, 25 steps, warm: 5.7 s and a 10,573 MB VRAM peak (whole card, desktop included); FLUX.2 [klein] 9B fp8: 15.6 s, 13,075 MB.",
  },
  {
    key: "steps",
    label: "Steps",
    what: "How many denoising steps the sampler takes: 1 to 60.",
    why: "Each step refines the image; too few leaves it mushy, too many wastes time with no visible gain.",
    effect: "Time scales with steps. Distilled models need far fewer: FLUX.2 [klein] is typically 8; SDXL 20–30.",
    benefit: "The single biggest lever on time per image.",
    pitfalls: "The smartphone preset sets 8 steps for FLUX.2 [klein]; switching to SDXL afterwards needs the steps raised again.",
  },
  {
    key: "cfg",
    label: "CFG / Guidance",
    what: "How strongly the sampler follows the prompt (1 to 15). For FLUX.1 the field sets FluxGuidance instead (1 to 10; the model itself runs at CFG 1).",
    why: "Too low and the prompt is ignored; too high and colours saturate and the image burns out.",
    effect: "Time per step is unchanged. Typical: SDXL 5–8, FLUX.1 guidance 3–4, FLUX.2 [klein] 1.5–2.",
    benefit: "Match the value to the model family and the prompt behaves.",
  },
  {
    key: "sd15-defaults",
    label: "SD 1.5 defaults",
    what: "When the chosen checkpoint is Stable Diffusion 1.5, the size, steps and CFG fields start at 512×512, 20 steps and CFG 8 instead of 1024×1024, 25 steps and CFG 7.",
    why: "SD 1.5 was trained at 512 px; at 1024 px it repeats subjects and falls apart. The values are those of ComfyUI's own SD 1.5 example.",
    effect: "Picking an SD 1.5 checkpoint moves every field that still holds the previous default; a value you changed yourself stays. Picking an SDXL or FLUX checkpoint moves them back the same way.",
    benefit: "A usable first SD 1.5 render without looking up its native size.",
    pitfalls: "The family comes from the checkpoint's recorded base family or its import family; an SD 1.5 file imported by hand without either gets the 1024 px defaults — set the fields yourself, or get it from the catalogue. The reference image (IP-Adapter) is refused on SD 1.5: the installed IP-Adapter is SDXL's.",
  },
  {
    key: "seed",
    label: "Seed",
    what: "The random number the render starts from. Blank means a new random seed every time; the result shows the seed it used, with a \"reuse\" button.",
    why: "The same seed with the same settings gives the same picture; change one thing and you see what that one thing did.",
    effect: "Only the starting noise changes. Reusing a seed and changing the prompt keeps the composition roughly in place.",
    benefit: "Reproducible results and controlled experiments.",
    pitfalls: "ComfyUI caches node outputs: repeating a seed with identical settings returns the same image in about 1.5 s without rendering — fine for you, misleading for timing.",
  },
  {
    key: "model",
    label: "Model",
    what: "Auto, or one image checkpoint from the library.",
    why: "SDXL, FLUX.1 and FLUX.2 [klein] behave differently and take different steps and CFG values; editing needs FLUX.2 [klein] 9B.",
    effect: "The form adapts (CFG becomes Guidance for FLUX, the VRAM line below shows whether the pick fits right now). Auto uses the most-recently-used checkpoint. A model that is not resident is loaded first; switching families mid-session costs a reload.",
    benefit: "Pin the model for a series; Auto for one-offs.",
    measured: "2026-09-17: the first image job of a session (ComfyUI start + SDXL load) took 45.6 s; the first FLUX.2 [klein] job after it (SDXL evicted, klein + text encoder loaded) 23.3 s; switching back and forth between the two families cost 24–41 s per job instead of the warm 15.6 s.",
  },
  {
    key: "smartphone-preset",
    label: "Smartphone photo preset",
    what: "One click that sets FLUX.2 [klein] 9B, its realistic-detail LoRA at 0.8, CFG 1.5, 8 steps, and appends \"natural window light, soft shadows, candid framing, shot on iphone, high detail skin\" to the prompt.",
    why: "The realistic phone-photo look needs several settings at once; this is that recipe saved.",
    effect: "Changes the model, CFG, steps, the LoRA stack and the prompt text. Nothing is generated until you press Generate.",
    benefit: "A known-good starting point for realism.",
    pitfalls: "Needs the FLUX.2 [klein] 9B stack in the library (Models → Discover); otherwise it says so and changes nothing. Every field can be adjusted afterwards.",
  },
  {
    key: "use-as-base",
    label: "Use as base",
    what: "Puts the finished image into \"Edit an existing image\" so the next prompt edits it.",
    why: "Iterating on a result should not need a file browse.",
    effect: "The form switches to edit mode with this image as the source.",
    benefit: "Generate, then refine, in two clicks.",
  },
  {
    key: "sessions",
    label: "Sessions",
    what: "The list on the left: New session, rename, archive, delete, and \"Ungrouped images\" for images that belong to no session. The same list the Chat and Video tabs have.",
    why: "A project's images stay together in their own gallery instead of one endless stream.",
    effect: "New renders are tagged with the picked session; the Gallery below shows only this session's images (24 per page). A new session opens its name field right away. Archive hides a session under \"Show archived\"; Delete removes the session but keeps its images, just ungrouped.",
    benefit: "Galleries that match your projects, and the same session list on every tab.",
    pitfalls: "The Queue still lists every image job across all sessions — switching sessions changes the Gallery, not the queue.",
  },
  {
    key: "queue",
    label: "Queue",
    what: "Every image job that is queued, running or blocked — across all sessions — with a cancel button; click a row to watch it in Result.",
    why: "A job started from another session or from Chat's /image would otherwise be invisible here.",
    effect: "Selecting a row only changes what Result shows. × cancels that job.",
    benefit: "Nothing waits unseen.",
  },
];

const LORA_SETTINGS: readonly HelpSetting[] = [
  {
    key: "lora-stack",
    label: "LoRAs (optional)",
    what: "Zero or more LoRAs from the library layered onto the base model for this render, each with its own strength.",
    why: "A LoRA adds a character, style or detail the base model does not have — including one you trained on the Training tab.",
    effect: "Ticking a LoRA adds it at strength 0.80; it is applied in ComfyUI's LoRA chain before sampling. Only LoRAs whose family matches the picked model (or whose family is unknown) are listed. Nothing ticked means the render is exactly what it would be without this section.",
    benefit: "Stack a trained character with a style LoRA in one render.",
    pitfalls: "A LoRA needs its trigger word in the prompt to do anything (Training's \"Test now\" prefills both). LoRAs of another family are hidden, not disabled — switch the model to see them.",
  },
  {
    key: "lora-strength",
    label: "Strength",
    what: "How much of the LoRA's effect is applied: 0 (none) to 2 (double), in steps of 0.05; 0.8 by default.",
    why: "Full strength often over-applies a LoRA — faces distort, styles swamp the prompt; a lower value blends it in.",
    effect: "Changes the weight the LoRA's deltas are scaled with. Time and VRAM are unchanged.",
    benefit: "Dial a LoRA in instead of on/off.",
    pitfalls: "Above 1.0 is exaggeration territory; useful for a weak LoRA, ugly for a strong one.",
  },
];

const HIRES_SETTINGS: readonly HelpSetting[] = [
  {
    key: "hires-enable",
    label: "Hi-res fix",
    what: "Render once at the requested size, then upscale that result in latent space and re-sample it at a low denoise for a bigger, more detailed image.",
    why: "Asking a model for 2,048 px directly breaks composition; refining a good one-megapixel image is how large images are made.",
    effect: "Two sampling passes: the first at Width × Height, the second at scale × that. The final size, the denoise, the second-pass steps and the method are shown in the result. Text-to-image only — not in edit mode, not in Story Studio.",
    benefit: "A 1,536 px or 2,048 px image with real added detail, from a form you already know.",
    pitfalls: "VRAM headroom scales with the pixels of the second pass: FLUX.2 [klein] 9B at 1.5× reaches 99.5% of a 16 GB budget and is blocked when something else is resident.",
    measured: "2026-09-17, warm, 1024×1024 first pass, 25 steps, denoise 0.45, 12 second-pass steps: SDXL 1.5× 12.6 s (from 5.7 s), 14,005 MB peak; SDXL 2.0× 19.8 s, 14,701 MB — fits in 16 GB; FLUX.2 [klein] 9B 1.5× 34.4 s (from 15.6 s), 14,095 MB.",
  },
  {
    key: "hires-scale",
    label: "Scale",
    what: "How much bigger the second pass is: 1.5× or 2×. The engine accepts 1.25–2.",
    why: "The scale sets the final size and, squared, the sampling work.",
    effect: "1.5× turns 1024 into 1536 (about 2.25× the work), 2× into 2048 (about 4×). The final size is rounded in latent units (multiples of 8 px), so 1000 px × 1.25 is 1248, not 1250.",
    benefit: "1.5× is the sweet spot measured here: markedly more texture, same composition, about 2.2× the time.",
    pitfalls: "2× at denoise 0.45 already re-interprets the picture (the measured lighthouse moved and shrank) — lower the denoise or stay at 1.5× if you only want it sharper.",
  },
  {
    key: "hires-denoise",
    label: "Denoise",
    what: "How much the second pass may change the image: 0.20 (barely) to 0.70 (a lot), in steps of 0.05; 0.45 by default.",
    why: "Low values sharpen; high values re-imagine. The right value depends on whether you want detail or a new picture.",
    effect: "Only the second pass changes. Higher denoise adds detail and drift alike.",
    benefit: "0.30–0.45 for \"the same, but crisper\"; 0.55+ for a deliberate re-interpretation.",
  },
  {
    key: "hires-steps",
    label: "Steps (second pass)",
    what: "Sampling steps of the second pass. Blank means half the first pass (4 to 60); the placeholder shows that number.",
    why: "The second pass starts from a real image, so it needs fewer steps than the first.",
    effect: "Time of the second pass scales with this. Typed values are clamped to 4–60 when you leave the field. For FLUX.2 [klein] GGUF the engine derives the scheduler's step count so that this is the number actually executed, same as for the other families.",
    benefit: "Leave it blank unless the refined image looks unfinished.",
  },
  {
    key: "hires-upscale-method",
    label: "Upscale method",
    what: "How the latent is enlarged before the second pass: nearest-exact (default), bilinear, area, bicubic or bislerp.",
    why: "The method sets what the second pass starts from; the differences are subtle because the sampler re-draws the detail anyway.",
    effect: "Only the enlargement changes; time is the same.",
    benefit: "nearest-exact is what the measured runs used; try bislerp if you see blockiness at 2×.",
  },
];

export const IMAGE_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "image",
    title: "What the Image tab is for",
    summary: "Text-to-image and image editing on ComfyUI with SDXL, FLUX.1 or FLUX.2 [klein]; LoRA stacks, Hi-res fix, a queue, a gallery per session, one-click upscaling.",
    body: [
      {
        kind: "p",
        text: "Every render is a job on the ComfyUI runtime. The form builds the request; the core turns it into a ComfyUI graph for the model's family (checkpoint, FLUX.1, FLUX.2 [klein] safetensors or GGUF, edit), applies the LoRA chain, optionally the Hi-res second pass, and streams per-step progress back to the Result card. Finished images land in the generated-media folder and in the session's gallery.",
      },
    ],
  },
  {
    id: "flow",
    area: "image",
    title: "Step by step",
    summary: "Pick a session, write a prompt, set size and steps for the model, generate, then upscale, edit or reuse the seed.",
    body: [
      {
        kind: "steps",
        items: [
          "Pick or create a session (or stay Ungrouped).",
          "Write the prompt; the prompt assistant can draft one with a chat model, and the preset pickers append saved phrases.",
          "Pick a size (Square / Portrait / Landscape or exact pixels), steps and CFG for the model family — the hints under the fields say what is typical.",
          "Optional: tick a LoRA and set its strength; turn on Hi-res fix for a bigger final image.",
          "Generate. The Result card shows the node being executed and a step meter; the Queue lists anything waiting.",
          "Afterwards: Upscale (RTX Video Super Resolution, 2×), Use as base (edit it), Download, reuse the seed, or delete.",
        ],
      },
    ],
    settings: FORM_SETTINGS,
  },
  {
    id: "loras",
    area: "image",
    title: "LoRA stack",
    summary: "Layer LoRAs from the library onto the base model, each with its own strength.",
    body: [
      {
        kind: "p",
        text: "Every recipe the app ships (SDXL and plain checkpoints, FLUX.1, FLUX.2 [klein] in all its forms, Wan 2.2, LTX-Video) threads a LoRA chain through its graph, so the section is shown for every model. The Video tab uses the same picker.",
      },
    ],
    settings: LORA_SETTINGS,
  },
  {
    id: "hires-fix",
    area: "image",
    title: "Hi-res fix",
    summary: "Render at one megapixel, then refine at 1.5× or 2× — the measured way to a detailed large image.",
    body: [
      {
        kind: "p",
        text: "For SDXL and FLUX.1 the second pass is a latent upscale followed by a second sampler with the chosen denoise. For FLUX.2 [klein] GGUF, which uses a different sampler, the engine splits the sigmas so that the steps you set are the steps actually executed. The final size written back to the result is computed with the same formula the graph uses, so the two cannot disagree.",
      },
    ],
    settings: HIRES_SETTINGS,
  },
  {
    id: "disk-gpu",
    area: "image",
    title: "What happens on disk and on the GPU",
    summary: "ComfyUI holds the checkpoint and encoders between jobs; every output is a PNG in the generated-media folder; VRAM is planned per job.",
    body: [
      {
        kind: "list",
        items: [
          "GPU: the first job of a session starts ComfyUI and loads the model (measured 45.6 s for SDXL); later jobs reuse it. Switching families evicts and reloads. The scheduler's need for a job scales with its pixels (and, with Hi-res fix, the second pass's pixels), so a small test image is not blocked as often as a big one.",
          "Disk: one PNG per job in the generated-media folder (Settings → Storage & data shows its size; a retention rule can prune it). Deleting a result or a gallery item deletes the job and its file.",
          "The VRAM line under the model picker compares the model's estimate with what is free right now — a heads-up, not the scheduler's decision.",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "image",
    title: "Limits and pitfalls",
    summary: "Sizes, families, what edit mode drops, what gets blocked.",
    body: [
      {
        kind: "list",
        items: [
          "512–2,048 px per side, multiples of 64; 1–60 steps; CFG 1–15 (guidance 1–10 for FLUX).",
          "Editing needs FLUX.2 [klein] 9B and keeps the source's size; negative prompt and Hi-res fix are not available while editing.",
          "A LoRA only works with its own family and needs its trigger word in the prompt.",
          "Hi-res fix at 2× re-interprets at denoise 0.45; FLUX.2 [klein] 9B at 1.5× is at the edge of a 16 GB card and is blocked while SDXL is still resident.",
          "The FLUX.2 [klein] GGUF branch of Hi-res fix is proven against fixtures only — no GGUF klein model was installed to measure it (2026-09-17).",
          "Repeating a seed with identical settings returns the cached image without rendering.",
        ],
      },
    ],
  },
  {
    id: "measured",
    area: "image",
    title: "Measured numbers",
    summary: "Warm renders on an RTX 4080 SUPER 16 GB, 2026-09-17, median of at least three runs with different seeds.",
    body: [
      {
        kind: "list",
        items: [
          "SDXL base 1.0, 1024×1024, 25 steps, CFG 7: 5.7 s; VRAM peak 10,573 MB (whole card, about 1.4 GB desktop included).",
          "SDXL + Hi-res 1.5× (denoise 0.45, 12 steps, nearest-exact): 12.6 s, 14,005 MB, output 1536×1536 — visibly more rock and spray detail, composition kept.",
          "SDXL + Hi-res 2.0×: 19.8 s, 14,701 MB, output 2048×2048 — fits in 16 GB with about 1.7 GB to spare, but the composition shifted.",
          "FLUX.2 [klein] 9B fp8, 1024×1024, guidance 4: 15.6 s, 13,075 MB. With Hi-res 1.5×: 34.4 s, 14,095 MB, output 1536×1536 — the best quality per second of the series.",
          "Cold start: first SDXL job of a session 45.6 s; first klein job after it 23.3 s; alternating families 24–41 s per job.",
        ],
      },
    ],
  },
];
