import type { HelpSetting, HelpTopic } from "./types.ts";

/** Training tab: purpose, flow, the run form, presets and fine-tuning,
 *  sample prompts, preflight, run actions, the LoRA overview with continued
 *  training, disk/GPU, limits and measured numbers. Numbers from
 *  `docs/TODO.md` (dates given). */

const FORM_SETTINGS: readonly HelpSetting[] = [
  {
    key: "target-model",
    label: "Target model",
    what: "The base model the LoRA is trained for and will later be used with. Grouped by training profile: FLUX.2 [klein] 4B, FLUX.2 [klein] 9B, SDXL, Wan 2.2 TI2V 5B (video).",
    why: "A LoRA only works with the family it was trained on; the profile also sets the presets, the caption order it prefers and how much VRAM it needs.",
    effect: "Changing it swaps the preset values, re-checks the base weights in preflight and drops a \"Start from\" LoRA of another family. The label after each model says how it fits this machine's VRAM.",
    benefit: "Pick the model you generate with, and the rest of the form is set up for it.",
    pitfalls: "Models without a training profile are not offered. The 4B profile is the one measured on a 16 GB card; 9B and Wan are set up but not yet measured here.",
  },
  {
    key: "start-from",
    label: "Start from",
    what: "Fresh LoRA (default), or an existing LoRA of the same family whose weights this run continues from.",
    why: "Continuing lets you teach an existing LoRA a second dataset instead of starting over.",
    effect: "The run loads the chosen LoRA's weights as its starting point and writes a new LoRA to the library; the original stays untouched. The rank is fixed to the chosen LoRA's rank. Only LoRAs of the target model's family are listed.",
    benefit: "Build on what you already trained.",
    pitfalls: "Training only on the new dataset drifts towards it and partly overwrites what the earlier datasets taught. To keep earlier material, add some of it to this dataset.",
    measured: "2026-09-19: continuing a rank-16 FLUX.2 [klein] 4B LoRA for 50 steps took 190.7 s (about 1.6 s per step), VRAM peaked at 11,915 MiB; the earlier LoRA's file was unchanged.",
  },
  {
    key: "dataset",
    label: "Dataset",
    what: "Which exported dataset the run trains on.",
    why: "The trainer reads the NNNN.png + NNNN.txt pairs an export wrote, not the curation board — so only exported datasets are listed.",
    effect: "The run records the dataset and its image count; the LoRA's history later shows which dataset each run used.",
    benefit: "Curate freely on the Dataset tab; what you export is what gets trained.",
    pitfalls: "Changes on the board after an export are not in the pairs until you export again.",
  },
  {
    key: "run-name",
    label: "Run name",
    what: "The name of this run and of the LoRA it produces in the library.",
    why: "You will pick it from a list later, on the Image tab and in the LoRA overview.",
    effect: "Naming only; the safetensors file in the library is named after it.",
    benefit: "\"Kenji Character v2\" is findable; \"run 7\" is not.",
  },
  {
    key: "trigger-word",
    label: "Trigger word",
    what: "The made-up token the captions start with — normally the dataset's trigger word, taken over from the export.",
    why: "It is the word that will call up the LoRA in a prompt.",
    effect: "Stored on the run; the sample prompts start from it and \"Test now\" prefills it on the Image tab. Spaces are removed as you type; 30 characters at most.",
    benefit: "One word in a prompt, and the trained look appears.",
    pitfalls: "It has to match the word in the exported captions. A real word collides with what the model already knows; the field warns about those.",
  },
  {
    key: "store-run-in",
    label: "Store run in",
    what: "Where the run's work folder (checkpoints, samples, logs) is written. Blank uses the app's training folder.",
    why: "A run keeps a checkpoint and an optimizer state on disk; long runs on a small drive fail late.",
    effect: "Everything for this run goes to <folder>\\<run id>. Resume and cleanup follow the run there. The run refuses to start with less than 20 GiB free on that drive.",
    benefit: "Long runs can live on a roomy drive.",
    measured: "2026-09-19: the work folder of a 50-step FLUX.2 [klein] 4B run held 93,536,565 B — a 46,223,656 B checkpoint, a 47,115,531 B optimizer state, two samples, config and log.",
  },
];

const PRESET_SETTINGS: readonly HelpSetting[] = [
  {
    key: "preset",
    label: "Preset",
    what: "Fast, Balanced or Thorough: a bundle of steps, rank, learning rate and resolution the profile defines for the target model.",
    why: "The four numbers interact; the presets are combinations that are known to work, ordered by how long they take.",
    effect: "Fast trains fewer steps at a lower resolution (for FLUX.2 [klein] 4B: 600 steps at 768 px); Balanced and Thorough train longer at 1,024 px, Thorough with a bigger rank and a slightly lower learning rate. The line under each preset shows the exact numbers for the chosen model.",
    benefit: "A first result in minutes with Fast, a final one with Balanced or Thorough.",
    measured: "2026-09-17: FLUX.2 [klein] 4B, Fast (600 steps, rank 16, lr 1e-4, 768 px) on 50 images finished in 16 min 55 s — 1.69 s per step — with a VRAM peak of 12,340 MB on a 16 GB card.",
  },
  {
    key: "rank",
    label: "Rank",
    what: "How much room the LoRA has to store what it learns (the size of its extra weight matrices). Blank keeps the preset's value.",
    why: "A small rank captures a look; a bigger one captures more detail — and makes a bigger file that is easier to over-train.",
    effect: "Higher rank: a larger safetensors file, a little more VRAM and time, more capacity. When continuing a LoRA the rank is fixed to that LoRA's rank, because the weights could not be reused otherwise.",
    benefit: "Match the capacity to the job: 16 for a style, 32 when fine detail matters.",
  },
  {
    key: "learning-rate",
    label: "Learning rate",
    what: "How big each training step's change to the weights is. Blank keeps the preset's value (1e-4 for Fast and Balanced, 8e-5 for Thorough).",
    why: "Too high and the run overshoots and produces garbage; too low and it takes forever to learn anything.",
    effect: "Only the size of each update changes; time per step stays the same.",
    benefit: "Leave it on the preset unless the samples tell you otherwise.",
    pitfalls: "Change it in small factors (halve or double), never by orders of magnitude.",
  },
  {
    key: "resolution",
    label: "Resolution",
    what: "The size, in pixels, the training images are scaled to (multiples of 64). Blank keeps the preset's value.",
    why: "Training at the resolution you generate at gives the sharpest result; lower is faster and needs less VRAM.",
    effect: "Higher resolution: more VRAM and more time per step. The images are resized on the fly; the export on disk is not changed.",
    benefit: "Fast at 768 px for a quick look, 1,024 px for the real thing.",
    pitfalls: "On a 16 GB card, 1,024 px with the 4B model is what the Balanced preset does; going higher is uncharted here.",
  },
  {
    key: "steps",
    label: "Steps",
    what: "How many training steps the run does. Blank keeps the preset's value; the allowed range is 50 to 20,000.",
    why: "Steps are the main dial for how much the LoRA learns — and for how long you wait.",
    effect: "Time scales directly with steps: at the measured 1.69 s per step, 600 steps are about 17 minutes and 1,500 about 42.",
    benefit: "Estimate the wait before you start.",
    pitfalls: "More steps is not always better; an over-trained LoRA reproduces the training images instead of the idea. Watch the samples.",
  },
  {
    key: "sample-prompts",
    label: "Sample prompts (1–3)",
    what: "Prompts the trainer renders at intervals during the run, using the LoRA as it is at that point.",
    why: "They are the only way to see whether the run is learning what you meant, before it ends.",
    effect: "Each sample is one extra image generation on the GPU, saved in the run's work folder and shown on the run card. The prompts start from the trigger word and track it until you edit one.",
    benefit: "See the LoRA come together, and stop early if it goes wrong.",
    pitfalls: "Put the trigger word in every prompt, or the sample will not use the LoRA's knowledge.",
  },
];

const PREFLIGHT_SETTINGS: readonly HelpSetting[] = [
  {
    key: "preflight-trainer",
    label: "Trainer installed",
    what: "Whether the training program (a pinned ai-toolkit in its own Python environment) is installed and healthy.",
    why: "The app does not ship the trainer; it is set up once, on this machine.",
    effect: "\"Install trainer\" downloads and installs it; \"Set up again\" repairs a broken environment. Start stays disabled until this row is green.",
    benefit: "A one-time setup, checked before every run.",
  },
  {
    key: "preflight-base-weights",
    label: "Base weights in the library",
    what: "Whether the target model's base weights (the files the trainer needs, several GB) are registered in the model library.",
    why: "A LoRA is trained on top of a base model; without its weights there is nothing to train on.",
    effect: "If they are missing, the row shows the exact download command and where to register the folder on the Models tab. This is a one-time download per model family.",
    benefit: "The first run of a family costs a download; every later run does not.",
  },
  {
    key: "preflight-gpu",
    label: "GPU is taken during training",
    what: "A reminder rather than a check: the chat model and ComfyUI are unloaded when the run starts, and image/video jobs wait until it ends.",
    why: "Training needs most of the card — the measured 4B run peaked at 12,340 MB of 16,376 MB (2026-09-17).",
    effect: "Chat and generation are unavailable during the run; they come back afterwards.",
    benefit: "No half-finished generation crashes your training run.",
  },
];

const RUN_SETTINGS: readonly HelpSetting[] = [
  {
    key: "pause",
    label: "Pause",
    what: "Stops the trainer process now and frees the GPU; the run stays resumable from its last saved checkpoint.",
    why: "You need the card for something else, or want to look at the samples in peace.",
    effect: "The last saved checkpoint and optimizer state stay in the work folder; the state becomes \"paused\". Steps since that save are done again on resume.",
    benefit: "A long run does not have to be all-or-nothing.",
  },
  {
    key: "resume",
    label: "Resume",
    what: "Continues a paused or interrupted run from its last checkpoint with the same settings.",
    why: "A run interrupted by a restart or a pause should not start over.",
    effect: "The trainer reloads the base model and the checkpoint (about a minute) and carries on from the step it reached.",
    benefit: "Only the steps since the last checkpoint are repeated.",
    pitfalls: "This is not \"train more\" — the total steps stay what the run was started with. To train an existing LoRA further, use \"Continue with another dataset\" in the LoRA overview.",
  },
  {
    key: "cancel",
    label: "Cancel",
    what: "Stops the run for good; it ends as \"cancelled\" and produces no LoRA.",
    why: "The samples show it is going wrong, or the settings were wrong.",
    effect: "The work folder stays until you delete the run. Not offered while a finished run is being imported into the library — that step finishes on its own.",
    benefit: "Stop wasting GPU time early.",
  },
  {
    key: "delete-run",
    label: "Delete",
    what: "Removes a finished, failed or cancelled run from the history, after a confirmation.",
    why: "The list is clearer without dead runs.",
    effect: "Deletes the run's row. The LoRA it produced stays in the library, and the work folder stays unless you tick the purge box.",
    benefit: "Tidy history, nothing lost by accident.",
  },
  {
    key: "purge-work-folder",
    label: "also delete its work folder",
    what: "With Delete: also removes the run's work folder — checkpoints, optimizer state, samples and logs.",
    why: "A work folder holds a checkpoint plus an optimizer state of about the same size, per run.",
    effect: "The folder is deleted from disk. The LoRA in the library is a separate file and stays.",
    benefit: "Reclaims the space a finished run no longer needs.",
    measured: "2026-09-19: a 50-step run's work folder was 93,536,565 B, of which the checkpoint was 46,223,656 B and the optimizer state 47,115,531 B.",
  },
  {
    key: "test-now",
    label: "Test now",
    what: "Opens the Image tab with this run's LoRA selected and a prompt starting with its trigger word.",
    why: "The samples are one thing; a real generation with your own prompt is the proof.",
    effect: "Only navigation and prefill; nothing is generated until you press Generate there.",
    benefit: "From \"completed\" to a first real image in two clicks.",
  },
];

const LORA_SETTINGS: readonly HelpSetting[] = [
  {
    key: "lora-overview",
    label: "Your LoRAs",
    what: "Every LoRA in the library with what its training adds up to: runs, total steps, total images. Pick one to open its history.",
    why: "After a few runs, the question is \"what went into this LoRA?\" — this answers it.",
    effect: "Reading only. Imported LoRAs are listed too, marked \"Imported\", without a history.",
    benefit: "One place to see what you have and how it was made.",
  },
  {
    key: "lora-history",
    label: "Training history",
    what: "The runs that contributed to the selected LoRA, oldest first: dataset, images, captioner, trigger word, profile, preset and settings, duration, result, and the samples.",
    why: "A continued LoRA is the sum of its runs; the history keeps that chain.",
    effect: "Reading only. \"Test in Image tab\" and \"Continue with another dataset\" act from here.",
    benefit: "Reproduce a good LoRA, or see why a bad one went wrong.",
  },
  {
    key: "continue-training",
    label: "Continue with another dataset",
    what: "Opens the run form with this LoRA as the starting point, its last target model and trigger word prefilled — you pick the new dataset.",
    why: "Teach an existing LoRA a second character, a second set of renders, without starting from zero.",
    effect: "The run trains from the LoRA's weights and produces a new library entry; the original is untouched. Same family and same rank are enforced.",
    benefit: "A v2 that keeps what v1 knew — as long as some of v1's material is in the new dataset too.",
    pitfalls: "Training only on the new dataset drifts towards it and partly overwrites what the earlier datasets taught.",
  },
];

export const TRAINING_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "training",
    title: "What the Training tab is for",
    summary: "Train a LoRA — a small add-on that teaches an existing model one new thing — from an exported dataset.",
    body: [
      {
        kind: "p",
        text: "A LoRA is a small file (tens of MB) that sits on top of a base model and nudges it towards a character, a prop, a style. Training one means showing the base model your image/caption pairs a few hundred to a few thousand times. The Training tab runs that on your GPU with a pinned copy of ai-toolkit, shows samples while it runs, and imports the finished LoRA into the model library, ready for the Image tab.",
      },
    ],
  },
  {
    id: "flow",
    area: "training",
    title: "Step by step",
    summary: "Export a dataset, pick a target and a preset, pass preflight, start, watch the samples, test.",
    body: [
      {
        kind: "steps",
        items: [
          "Export a dataset on the Dataset tab, then press Train LoRA there (or New run here).",
          "Pick the target model. The first run of a family needs its base weights: preflight shows the download command and where to register the folder.",
          "Give the run a name, keep the dataset's trigger word, pick a preset — Fast for a first look.",
          "Keep at least one sample prompt that starts with the trigger word.",
          "Start. Chat and ComfyUI are unloaded; the card shows step, loss, an estimate of the time left and new samples as they arrive.",
          "When it says completed, the LoRA is in the library: Test now opens the Image tab with it.",
          "Later: pick the LoRA under Your LoRAs to see its history, or continue it with another dataset.",
        ],
      },
    ],
  },
  {
    id: "run-form",
    area: "training",
    title: "Run form",
    summary: "Target model, starting point, dataset, name, trigger word and where the run is stored.",
    body: [
      {
        kind: "p",
        text: "Everything blocking is checked before Start is enabled: a trainer, base weights, a dataset, a name, a trigger word, a sample prompt, and fine-tune fields that parse.",
      },
    ],
    settings: FORM_SETTINGS,
  },
  {
    id: "presets",
    area: "training",
    title: "Presets, fine-tuning and samples",
    summary: "What steps, rank, learning rate and resolution mean in practice, and why samples matter.",
    body: [
      {
        kind: "p",
        text: "The presets are the intended way in. The Fine-tune section overrides one or more of the four numbers; an empty field means \"use the preset\". The placeholders show the preset's value.",
      },
    ],
    settings: PRESET_SETTINGS,
  },
  {
    id: "preflight",
    area: "training",
    title: "Before the run starts",
    summary: "The trainer, the base weights, the GPU hand-over and the licence note.",
    body: [
      {
        kind: "p",
        text: "Red rows block Start; dotted rows are notes. The licence row repeats what the base weights' licence says — FLUX.2 [klein] 9B, for example, is non-commercial and its download is gated.",
      },
    ],
    settings: PREFLIGHT_SETTINGS,
  },
  {
    id: "runs",
    area: "training",
    title: "Run cards and actions",
    summary: "Progress, loss, samples, the log, and pause / resume / cancel / delete / test.",
    body: [
      {
        kind: "p",
        text: "Loss is the trainer's own error number; it wanders and only the trend over hundreds of steps means anything. The time-left estimate uses the steps done so far. The Log button shows the trainer's last lines and the work folder path.",
      },
    ],
    settings: RUN_SETTINGS,
  },
  {
    id: "loras",
    area: "training",
    title: "Your LoRAs and continued training",
    summary: "The overview, the per-LoRA history, and training an existing LoRA on another dataset.",
    body: [
      {
        kind: "p",
        text: "A LoRA typically needs a few hundred to a few thousand well-chosen images; the pipeline samples frames (default 1.5 fps) and filters blur and duplicates.",
      },
    ],
    settings: LORA_SETTINGS,
  },
  {
    id: "disk-gpu",
    area: "training",
    title: "What happens on disk and on the GPU",
    summary: "Base weights once per family, a work folder per run, the LoRA in the library, the whole card while it runs.",
    body: [
      {
        kind: "list",
        items: [
          "Base weights: downloaded once per model family (several GB) and registered on the Models tab; every run of that family reuses them.",
          "Work folder: <training folder>\\<run id> (or the folder under \"Store run in\") with config, log, checkpoints, optimizer state and samples. Needs 20 GiB free to start; deleted only with Delete + purge.",
          "Result: the finished LoRA is copied into the model library as its own safetensors file (46,223,656 B for a rank-16 FLUX.2 [klein] 4B LoRA, 2026-09-19) and stays there even if the run is deleted.",
          "GPU: the chat model and ComfyUI are unloaded first; the measured 4B run held 12,340 MB of 16,376 MB at its peak (2026-09-17). Image and video jobs queue behind the run.",
          "The trainer is a Python environment on this machine, installed once from preflight.",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "training",
    title: "Limits and pitfalls",
    summary: "What blocks a run, what cannot be undone, and what is easy to get wrong.",
    body: [
      {
        kind: "list",
        items: [
          "Only exported datasets are offered; re-export after curating further.",
          "Continuing a LoRA needs the same family and the same rank; the form fixes the rank for you.",
          "Continuing on a new dataset alone partly overwrites what earlier datasets taught — mix in some of the old material.",
          "Steps must be between 50 and 20,000; resolution a multiple of 64.",
          "The GPU is taken for the whole run: no chat, no generation until it ends or is paused.",
          "Resume continues a run; it does not add steps. Cancel ends it without a LoRA.",
          "Delete with purge removes checkpoints and samples for good; the LoRA in the library stays.",
          "Only the FLUX.2 [klein] 4B profile has been measured on a 16 GB card here; the others are set up from their documentation.",
        ],
      },
    ],
  },
  {
    id: "measured",
    area: "training",
    title: "Measured numbers",
    summary: "Real runs from docs/TODO.md on an RTX 4080 SUPER 16 GB.",
    body: [
      {
        kind: "list",
        items: [
          "2026-09-17, FLUX.2 [klein] 4B, Fast preset (600 steps, rank 16, lr 1e-4, 768 px), 50 images, captions = trigger word only: 16 min 55 s, 1.69 s per step; a second run of the same settings 16 min 30 s. VRAM peak 12,340 MB of 16,376 MB (about 1,256 MB of that was in use before the run).",
          "2026-09-19, continuing that LoRA for 50 steps on 50 images: 190.7 s wall time; VRAM 5,169 → 9,834 MiB while loading, 11,729–11,731 MiB while training, 11,915 MiB peak at the final sample and save, 963 MiB afterwards.",
          "2026-09-19, work folder of that 50-step run: 93,536,565 B (checkpoint 46,223,656 B, optimizer state 47,115,531 B, two samples with thumbnails, config, log).",
          "2026-09-19, the resulting LoRA in the library: 46,223,656 B, rank 16.",
        ],
      },
    ],
  },
];
