import type { HelpSetting, HelpTopic } from "./types.ts";

/** Video tab: text-to-video and image-to-video, the size / frames / fps
 *  fields, the time estimate, sessions and queue. Facts from
 *  `features/video/Video.tsx` and the Wan / LTX notes in `docs/TODO.md`
 *  (2026-09-12, 2026-09-16). */

const SETTINGS: readonly HelpSetting[] = [
  {
    key: "start-frame",
    label: "Start from an image",
    what: "A finished image from this app, or an image file, that the clip starts from (image-to-video). \"None\" is text-to-video.",
    why: "A start frame fixes the subject, framing and colours; the prompt then describes the motion.",
    effect: "The image is sent as the first frame and the model animates from it. The thumbnail below the field shows what was picked. Sizes should match the image's aspect ratio or the frame is resized.",
    benefit: "Turn a still you like into a clip with the same look.",
    pitfalls: "A file path only works from the desktop app. Wan 2.2 TI2V-5B needs the umt5 text encoder and the Wan VAE in the library; the form says which is missing.",
  },
  {
    key: "size",
    label: "Width and height",
    what: "The clip's frame size: 128 to 1,280 px per side, in steps of 16. Presets: Landscape 832×480, Portrait 480×832, Square 512×512, HD 720p 1280×720.",
    why: "Video cost grows with pixels × frames; Wan's comfortable zone on a 16 GB card is 832×480.",
    effect: "Values are snapped to a multiple of 16 when you generate. Above 480p the note turns to a warning: much slower, and it can run out of VRAM.",
    benefit: "480p for iteration, 720p only for a keeper.",
  },
  {
    key: "frames",
    label: "Frames",
    what: "How many frames the clip has: 5 to 121, snapped to 4k + 1 (5, 9, 13, … 81, … 121) because Wan requires it. 81 by default.",
    why: "Frames × fps is the clip's length; frames also drive time and VRAM.",
    effect: "81 frames at 24 fps is about 3.4 s. Above 81 the note warns about time and VRAM. The number is snapped when you generate.",
    benefit: "Length under your control, in the model's own grid.",
    pitfalls: "The scheduler's VRAM need for a video job scales with its pixels and frames, so a long HD clip is the first thing to be blocked.",
  },
  {
    key: "fps",
    label: "FPS",
    what: "Frames per second of the written file: 8 to 30, 24 by default.",
    why: "It sets how long the same frames play — and how smooth motion looks.",
    effect: "Rendering cost does not change with fps; only the playback speed and the clip's length in seconds do (the note shows \"~x.x s clip\").",
    benefit: "Lower fps stretches a short clip; higher makes it smoother but shorter.",
  },
  {
    key: "steps",
    label: "Steps",
    what: "Sampler steps per clip: 1 to 60, 30 by default.",
    why: "Fewer steps are faster; very few leave the clip noisy.",
    effect: "Time scales with steps, on top of pixels and frames.",
    benefit: "Drop to a handful of steps for a quick motion test.",
    measured: "2026-09-16: Wan 2.2 TI2V-5B and LTX-Video-2B at 128×128, 5 and 9 frames, 4 steps each rendered in about 30–48 s on an RTX 4080 SUPER — the smallest possible clips, as a smoke test.",
  },
  {
    key: "cfg",
    label: "CFG",
    what: "How strongly the sampler follows the prompt: 1 to 15, 5 by default.",
    why: "Video models drift with too little guidance and flicker with too much.",
    effect: "Time per step is unchanged.",
    benefit: "Stay near the default unless the prompt is ignored.",
  },
  {
    key: "seed",
    label: "Seed",
    what: "The random number the render starts from; blank is a new one each time. The result shows the seed used, with \"reuse\".",
    why: "Same seed, same settings, same clip — so a single change is visible.",
    effect: "Only the starting noise changes.",
    benefit: "Compare prompts or steps fairly.",
  },
  {
    key: "model",
    label: "Model",
    what: "Auto, or one video model from the library (Wan 2.2 TI2V-5B, LTX-Video).",
    why: "The two families need different companion files and behave differently.",
    effect: "Auto uses the most-recently-used video model. The VRAM line under the picker compares the model's estimate with what is free right now.",
    benefit: "Pin a model for a series of clips.",
    pitfalls: "Wan also needs the umt5 text encoder and the wan2.2_vae; the form lists what is missing.",
  },
  {
    key: "time-estimate",
    label: "Time estimate",
    what: "The line under the form: the clip's length in seconds and a very rough guess of minutes on a 16 GB card, scaled from 832×480, 81 frames, 25 steps.",
    why: "Video is slow — minutes, not seconds — and the guess stops you from waiting on something that will take an hour.",
    effect: "Purely informational; it is not calibrated against real runs yet. The window stays usable while a clip renders.",
    benefit: "Know before pressing Generate whether to go smaller.",
    pitfalls: "A guess, not a measurement: the only measured clips are the tiny 128×128 smoke tests.",
  },
  {
    key: "sessions",
    label: "Sessions",
    what: "The list on the left: New session, rename, archive, delete, and \"Ungrouped clips\" for clips that belong to no session. The same list the Chat and Image tabs have.",
    why: "A project's clips stay together in their own gallery instead of one endless stream.",
    effect: "New clips are tagged with the picked session; the Gallery below shows only this session's clips (24 per page). A new session opens its name field right away. Archive hides a session under \"Show archived\"; Delete removes the session but keeps its clips, just ungrouped.",
    benefit: "Galleries that match your projects, and the same session list on every tab.",
    pitfalls: "The Queue still lists every video job across all sessions — switching sessions changes the Gallery, not the queue.",
  },
];

export const VIDEO_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "video",
    title: "What the Video tab is for",
    summary: "Short clips from a prompt, or from a still image, with Wan 2.2 or LTX-Video on ComfyUI — with the same sessions, queue, gallery and upscale as the Image tab.",
    body: [
      {
        kind: "p",
        text: "A video job builds the family's ComfyUI graph (text-to-video, or image-to-video from the start frame), applies any LoRAs, renders the frames and writes an MP4 to the generated-media folder. Progress per step streams to the Result card; the engine's own log line (\"rendering …\") is shown while no step count is available.",
      },
    ],
  },
  {
    id: "flow",
    area: "video",
    title: "Step by step",
    summary: "Optionally pick a start frame, write the motion, keep it small, generate, wait minutes, upscale if it is a keeper.",
    body: [
      {
        kind: "steps",
        items: [
          "Import a video model on the Models tab (the Video stack brings the encoder and VAE with it). ComfyUI must be set up (Diagnostics).",
          "Optional: pick a finished image or a file as the start frame.",
          "Write the prompt (the assistant can draft one) and a negative prompt.",
          "Keep Landscape 832×480, 81 frames, 24 fps, 30 steps for a first clip; read the estimate line.",
          "Generate. Expect minutes. The Queue shows it; the Result card shows progress and, when done, the player.",
          "Upscale (RTX Video Super Resolution, 2×) or Download; reuse the seed to vary the prompt.",
        ],
      },
    ],
    settings: SETTINGS,
  },
  {
    id: "disk-gpu",
    area: "video",
    title: "What happens on disk and on the GPU",
    summary: "The whole card for minutes; an MP4 per job; RAM matters too.",
    body: [
      {
        kind: "list",
        items: [
          "GPU: the video model, its text encoder and VAE are loaded into ComfyUI; the scheduler's VRAM need scales with the clip's pixels and frames. Chat and image models are evicted if they are in the way.",
          "RAM: with ComfyUI's offloading, part of the model can live in system RAM; a warning fires when the model's size plus 6 GB exceeds the available RAM (a heuristic, checked against real Wan and LTX runs on 2026-09-16 but not tuned further).",
          "Disk: one MP4 per job in the generated-media folder — video files are large and this folder grows fast; set a retention rule under Settings → Storage & data.",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "video",
    title: "Limits and pitfalls",
    summary: "Grid, size, time, and what is not measured.",
    body: [
      {
        kind: "list",
          items: [
          "128–1,280 px per side in multiples of 16; 5–121 frames on the 4k + 1 grid; 8–30 fps; 1–60 steps.",
          "Above 832×480 or 81 frames is much slower and can run out of VRAM on a 16 GB card.",
          "The minute estimate is uncalibrated; the only measured clips are 128×128 smoke tests (about 30–48 s).",
          "A start frame from a file needs the desktop app.",
          "A LoRA must be of the video model's family.",
        ],
      },
    ],
  },
];
