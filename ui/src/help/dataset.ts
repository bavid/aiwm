import type { HelpSetting, HelpTopic } from "./types.ts";

/** Dataset tab, part 1: purpose, flow, the prep form and captioning. The
 *  curation, housekeeping and export settings live in `dataset-curation.ts`.
 *  Numbers come from the measured tables in `docs/TODO.md` (dates given). */

const PREP_SETTINGS: readonly HelpSetting[] = [
  {
    key: "root",
    label: "Root folder",
    what: "The folder the pipeline reads your videos and images from (.mp4, .png, .jpg, .jpeg, .webp).",
    why: "Every frame needs a tag, and the folder tree is where that tag comes from: each immediate subfolder becomes a tag for everything inside it. Files directly in the root are tagged with the root folder's own name.",
    effect: "Nothing is written into this folder. The pipeline only reads it; extracted frames go to the app's datasets folder (or the folder you pick under \"Store frames in\"). Your source files are never moved, renamed or deleted — not even when you delete the dataset later.",
    benefit: "One folder per look or subject gives you tags for free, without renaming a single file.",
    pitfalls: "A folder that holds both loose files and subfolders mixes two tags; sort the loose files into a subfolder if that is not what you want.",
  },
  {
    key: "mode",
    label: "Mode",
    what: "Frames pulls still images out of every video (and takes the images as they are); Clips keeps whole videos as items, for video models.",
    why: "An image LoRA learns from stills, a video LoRA from short clips — the two need different material on disk.",
    effect: "Frames: the sampling, blur and duplicate settings below apply and each item on the board is one still. Clips: each item is one video with a length and optional in/out points you set on its card, and the duplicate search is unavailable.",
    benefit: "You choose once, and the rest of the form, the board and the export follow.",
  },
  {
    key: "sample-fps",
    label: "Sample rate (fps)",
    what: "How many stills per second of video the pipeline extracts, before any filtering.",
    why: "Video runs at 24–60 frames per second, but neighbouring frames look almost the same. Sampling a few per second keeps variety without thousands of near-copies.",
    effect: "Higher values extract more frames (more time, more disk, more duplicates to filter out); lower values extract fewer. The count is set before the filters run, so it also sets how long the filter stage takes.",
    benefit: "The frame count, and with it the prep time and the disk use, are under your control.",
    pitfalls: "The frame count is set before the filters run — extracting 10 fps from long videos means a long filter stage (about 200 ms per frame) and a big work folder until you clean up.",
    measured: "2026-09-19: one video at the default 1.5 fps gave 1,368 frames; extraction took 19.6 s, the filter stage 275.7 s (298.6 s in all).",
  },
  {
    key: "blur-threshold",
    label: "Blur threshold",
    what: "How sharp a frame has to be to survive. The pipeline measures the sharpness of every frame; below this number it is marked \"Blur\" and moved to Discard.",
    why: "Motion blur and out-of-focus frames teach the model to make blurry images.",
    effect: "Raising it discards more frames (stricter); lowering it keeps softer ones; 0 keeps everything. Discarded frames are not deleted — they sit in the Discard column with the reason \"Blur\" and can be moved back to Keep.",
    benefit: "Fewer blurry frames means a sharper LoRA, without looking at every frame by hand.",
    pitfalls: "Soft-lit or intentionally hazy footage measures as blurry; if too much of it lands in Discard, lower the threshold or move the good ones back.",
  },
  {
    key: "duplicate-distance",
    label: "Duplicate distance",
    what: "How similar two neighbouring frames may be before the second one counts as a duplicate. The pipeline compares a tiny fingerprint of each frame with the last kept one.",
    why: "At 1.5 fps a static shot yields long runs of identical stills; hundreds of copies of one image over-teach it.",
    effect: "Higher numbers treat more frames as duplicates (a slow pan collapses into a few stills); 0 keeps only exact copies apart. Duplicates go to Discard with the reason \"Duplicate\" and can be moved back.",
    benefit: "You keep the variety of the footage and drop the padding.",
    measured: "2026-09-19: one video, default settings — 1,368 frames extracted, 1,328 rejected as duplicates, 40 kept.",
  },
  {
    key: "max-frames-per-clip",
    label: "Max frames per clip",
    what: "An upper limit on the kept frames from any one video; 0 means no limit.",
    why: "Without it a long video dominates the dataset and the LoRA learns that one video's look.",
    effect: "When a video has more surviving frames than this, the pipeline keeps the most varied ones and marks the rest \"Cap\" — they go to Discard, still on disk, still movable back.",
    benefit: "Every video gets a fair share of the dataset.",
  },
  {
    key: "min-clip-secs",
    label: "Min clip length (s)",
    what: "Clips mode only: videos shorter than this are skipped.",
    why: "A video model needs a few seconds of motion per item; a half-second stub teaches nothing.",
    effect: "Shorter videos are left out of the dataset. Nothing on disk changes.",
    benefit: "The clip dataset only holds items long enough to train on.",
  },
  {
    key: "store-frames-in",
    label: "Store frames in",
    what: "Where the extracted frames, previews and the dataset's work folder are written. Blank uses the app's datasets folder.",
    why: "A prep run writes every extracted frame to disk before filtering (in the measured run 1,368 frames came to 1.17 GB); a big source folder can fill the app's drive.",
    effect: "Everything for this run goes to <folder>\\<job id>; the source files stay where they are. The run refuses to start with less than 5 GiB free on that drive. Cleanup, dedup and delete follow the dataset to that folder.",
    benefit: "A large dataset can live on a roomy drive without moving the app.",
    measured: "2026-09-19: with a custom folder set, a run took 296.6 s and wrote all 1,368 files (1,173,444,536 B) under <folder>\\<job id>; the default datasets folder stayed empty.",
  },
];

const CAPTION_SETTINGS: readonly HelpSetting[] = [
  {
    key: "auto-caption",
    label: "Auto-caption",
    what: "Writes a text description for every kept frame with a captioner model, right after the filters.",
    why: "A LoRA learns what the captions do not name. Whatever is described stays controllable in prompts later; whatever is not described becomes part of the style. Without captions, every recurring thing in the footage flows into the trigger word.",
    effect: "The captioner runs after extraction and filtering; the text is stored per frame and can be edited on the card. This is the slow, GPU-using part of a prep run for prose captioners; the WD tagger runs on the CPU.",
    benefit: "Prompts like \"<trigger>, a wide shot\" work as expected instead of dragging every background of the footage along.",
    pitfalls: "Off by default only when no captioner is installed. Turning it off is a valid choice for a pure style LoRA, but then keep the footage tight — everything in it becomes \"the style\".",
  },
  {
    key: "captioner",
    label: "Captioner",
    what: "Which model writes the captions: the WD tagger (comma-separated tags), Florence-2 (a prose sentence), optionally with a Qwen2.5-VL second opinion.",
    why: "Tags suit SDXL and anime models, prose suits FLUX.2 — the export can put either first, but the captioner decides what there is to write.",
    effect: "The WD tagger runs on the CPU with no VRAM cost; Florence-2 loads onto the GPU for the run and is unloaded afterwards; Qwen escalation adds a much bigger model on top. Changing the captioner does not change already captioned frames.",
    benefit: "You pick speed and style on purpose, and the numbers below tell you what each choice costs.",
    measured: "2026-09-19, 40 kept frames: WD tagger 84.0 s on the CPU (about 2.1 s per frame including loading, no extra VRAM); Florence-2 38.3 s including loading, 31.2 s when already loaded (about 0.78 s per frame), about 2,187 MiB VRAM while resident.",
  },
  {
    key: "escalate",
    label: "Escalate uncertain captions",
    what: "When Florence-2 is unsure about a frame, the frame is shown to the larger Qwen2.5-VL together with a later frame of the same video, and Qwen writes the caption instead.",
    why: "A single still is ambiguous (is that a turn or a fall?); a second frame a moment later settles it. The big model is too slow to run on everything, so it only takes the doubtful cases.",
    effect: "Qwen2.5-VL is loaded on the GPU next to Florence-2 for the duration of captioning; the escalated frames take several seconds each. Only prose captioners can escalate — the WD tagger has no sentence to re-check.",
    benefit: "Better captions on exactly the frames that need them, at a fraction of the cost of captioning everything with the big model.",
    measured: "2026-09-19, Florence-2 with escalation of every 5th frame: VRAM rose from 1,818 MiB idle to a peak of 11,867 MiB and was back to 2,145 MiB one second after the job; 373 s for 1,368 frames (40 kept: 33 by Florence-2, 7 by Qwen2.5-VL).",
  },
  {
    key: "escalate-every-nth",
    label: "Escalate every Nth frame too",
    what: "Besides the uncertain frames, every Nth kept frame is sent to Qwen2.5-VL as well; 0 turns this off.",
    why: "Uncertainty is a guess. Sampling a fixed share of frames through the big model gives every part of the footage some careful captions.",
    effect: "A smaller N means more frames through the slow model and a longer run; a larger N means fewer.",
    benefit: "A steady baseline of careful captions across the whole dataset.",
  },
  {
    key: "context-offset",
    label: "Context offset (frames)",
    what: "How many kept frames later the comparison frame for an escalation is taken from.",
    why: "The second opinion works by seeing what happened next. Too close and the two frames look the same; too far and they no longer belong to the same moment.",
    effect: "Only changes what Qwen2.5-VL sees; no cost difference.",
    benefit: "A sensible offset (the default is 5) keeps the \"before and after\" pair meaningful.",
  },
];

const TRIGGER_SETTINGS: readonly HelpSetting[] = [
  {
    key: "trigger-word",
    label: "Trigger word",
    what: "A made-up token, such as ghibli_xy, that will stand for this dataset's look or subject in every caption.",
    why: "The model needs one word to attach the new knowledge to — a word it has never seen, so nothing it already knows gets overwritten. Everything the captions do not describe flows into this word.",
    effect: "Saved on the dataset (Enter or click away). The export puts it first in every caption; the training form takes it over as the default.",
    benefit: "Later, that one word in a prompt calls up what you trained.",
    pitfalls: "Real words (\"woman\", \"anime\") collide with what the model already knows; the field warns about them. Keep it short, one token, no spaces.",
  },
];

export const DATASET_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "dataset",
    title: "What the Dataset tab is for",
    summary: "Turn a folder of videos and images into a curated, captioned dataset a LoRA trainer can read.",
    body: [
      {
        kind: "p",
        text: "A LoRA teaches an existing model one new thing — a character, a prop, a look. It learns from a folder of images with a short text next to each one. The Dataset tab makes that folder for you: it pulls stills out of videos, throws away the blurry and repeated ones, writes captions, lets you sort what is left, and exports image/caption pairs. It does not train anything itself; that is the Training tab.",
      },
      {
        kind: "p",
        text: "The result on disk is a work folder with the extracted frames and previews, and — once you export — a separate folder of NNNN.png + NNNN.txt pairs that the trainer reads. Your source videos and images are only ever read.",
      },
    ],
  },
  {
    id: "flow",
    area: "dataset",
    title: "Step by step",
    summary: "Point at a folder, run the pipeline, curate, assign concepts, export, train.",
    body: [
      {
        kind: "steps",
        items: [
          "Arrange the source material: one folder per look or subject, because each immediate subfolder becomes a tag.",
          "Pick the root folder and the mode (Frames for image models, Clips for video models). The defaults for sampling, blur and duplicates are a good start.",
          "Leave Auto-caption on and pick a captioner, unless you want a pure style LoRA.",
          "Run pipeline. Extraction is quick; the filter stage takes about 200 ms per frame; captioning takes as long as the table under \"Captioner\" says.",
          "Curate on the board: Keep on the left, Discard on the right. Move frames with drag and drop or K / D, check the discard reasons, fix captions on the cards.",
          "Set the trigger word, and create a concept for each thing the dataset should teach; assign frames in the grid or set by set in Learn mode.",
          "Export to a destination folder — that writes the pairs the trainer reads. Then Train LoRA hands the dataset to the Training tab.",
        ],
      },
    ],
  },
  {
    id: "prep-settings",
    area: "dataset",
    title: "Prep form settings",
    summary: "Root folder, mode, sampling rate, blur and duplicate filters, caps and where the frames go.",
    body: [
      {
        kind: "p",
        text: "The filters never delete anything: a rejected frame sits in the Discard column with its reason and can be moved back. Only Clean up and Delete dataset remove files, and both preview first.",
      },
    ],
    settings: PREP_SETTINGS,
  },
  {
    id: "captioning",
    area: "dataset",
    title: "Captioning",
    summary: "Why captions matter, which captioner to pick, and what escalation costs.",
    body: [
      {
        kind: "p",
        text: "A caption tells the model which parts of the image are ordinary (\"a wide shot, a beach, evening light\") so that only the rest is attributed to the trigger word. Tags are lists of nouns, best for SDXL-style models; prose is a sentence, best for FLUX.2. The export can put either first.",
      },
      {
        kind: "p",
        text: "Captioners are installed on the Models tab (\"More captioners…\" takes you there). Prose captioners live on the GPU for the run and are unloaded at the end, so the next run loads them again (about 7 s for Florence-2).",
      },
    ],
    settings: CAPTION_SETTINGS,
  },
  {
    id: "trigger",
    area: "dataset",
    title: "Trigger word",
    summary: "The one made-up word that will stand for what the dataset teaches.",
    body: [
      {
        kind: "p",
        text: "Without captions, everything that recurs in the footage — the lighting, the room, the outfit — is learned as part of the trigger word. With captions, only what the captions leave unnamed is.",
      },
    ],
    settings: TRIGGER_SETTINGS,
  },
];
