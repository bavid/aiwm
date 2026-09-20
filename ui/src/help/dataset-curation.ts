import type { HelpSetting, HelpTopic } from "./types.ts";

/** Dataset tab, part 2: the curation board and its discard reasons,
 *  concepts and Learn mode, housekeeping, export, disk/GPU, limits and the
 *  measured numbers. Numbers from `docs/TODO.md` (dates given). */

const CURATION_SETTINGS: readonly HelpSetting[] = [
  {
    key: "keep-discard",
    label: "Keep / Discard columns",
    what: "Every item of the dataset sits in one of two columns. Keep is what the export writes; Discard is what a filter rejected or you excluded by hand.",
    why: "A dataset is only as good as what you leave out. The board makes leaving out cheap and reversible.",
    effect: "Moving a frame (drag and drop, or select it and press K / D) changes only a flag in the database; the file stays on disk. Nothing is deleted until you use Delete… or Clean up.",
    benefit: "You can be ruthless in Discard and change your mind later.",
  },
  {
    key: "excluded",
    label: "Excluded",
    what: "The Discard filter for frames you moved there yourself, as opposed to frames a filter rejected.",
    why: "Your own decisions and the pipeline's verdicts are worth reviewing separately.",
    effect: "Shows only frames with no pipeline reason. Nothing changes on disk.",
    benefit: "A quick way to double-check what you threw out by hand.",
  },
  {
    key: "reason-black",
    label: "Black",
    what: "The frame is (almost) entirely black — a fade, a dropout, a title card.",
    why: "Black frames carry no picture to learn from and would teach the model to produce them.",
    effect: "Rejected automatically during prep; the file is kept and the frame can be moved back to Keep.",
    benefit: "Fades and leaders are out of the dataset without you looking for them.",
  },
  {
    key: "reason-transition",
    label: "Transition",
    what: "The frame sits on a cut or a dissolve between two shots and mixes both.",
    why: "A half-and-half image is not a real picture of anything.",
    effect: "Rejected automatically; movable back.",
    benefit: "Scene changes stop leaking into the training material.",
  },
  {
    key: "reason-blur",
    label: "Blur",
    what: "The frame measured below the blur threshold set on the prep form.",
    why: "Blurry training images make blurry generations.",
    effect: "Rejected automatically; movable back. To change the cut-off, run prep again with a different threshold.",
    benefit: "Sharp material without a manual pass.",
  },
  {
    key: "reason-duplicate",
    label: "Duplicate",
    what: "The frame looks like the frame kept just before it in the same video (within the duplicate distance).",
    why: "Runs of near-identical stills over-teach one moment.",
    effect: "Rejected automatically; movable back.",
    benefit: "The dataset keeps the variety, not the padding.",
  },
  {
    key: "reason-duplicate-global",
    label: "Duplicate (dataset)",
    what: "Marked by \"Find duplicates\" under Housekeeping: the frame is a near-copy of another frame anywhere in the dataset, not just its neighbour.",
    why: "Two videos of the same shot, or an image and a frame of its video, look the same to the model.",
    effect: "The sharpest frame of each group stays in Keep; the rest move to Discard with this reason and can be moved back.",
    benefit: "Cross-video repeats are found in one click.",
  },
  {
    key: "reason-cap",
    label: "Cap",
    what: "The video had more surviving frames than \"Max frames per clip\" allows; the most varied ones were kept and this one was not among them.",
    why: "Keeps one long video from dominating the dataset.",
    effect: "The frames past the cap are in Discard and can be moved back if you want more from that video.",
    benefit: "Every source gets a fair share.",
  },
  {
    key: "reason-unusable",
    label: "Unusable",
    what: "The frame could not be decoded or measured — a broken file, an unreadable image.",
    why: "A file the pipeline cannot open would fail the trainer too.",
    effect: "Rejected automatically. Check the file if you expected it to work.",
    benefit: "Broken inputs are visible instead of failing a training run hours later.",
  },
  {
    key: "delete-frames",
    label: "Delete… (selected frames)",
    what: "Removes the selected frames and their files from disk, after a confirmation.",
    why: "Moving to Discard keeps the file; deleting frees the space.",
    effect: "The frame rows and their files are gone for good. Source videos and images are never touched. Disabled while the prep run is still writing.",
    benefit: "Reclaims disk space from frames you will never want.",
    pitfalls: "This is the one board action that cannot be undone. Prefer Discard while you are still deciding.",
  },
];

const CONCEPT_SETTINGS: readonly HelpSetting[] = [
  {
    key: "concept",
    label: "Concept (name + token)",
    what: "One token per thing the dataset should teach besides the overall look — a character, a prop, a place. Frames are assigned to it in the grid or in Learn mode.",
    why: "With one trigger word everything in the dataset melts into one idea. A concept token per subject lets a prompt ask for that subject on its own.",
    effect: "Creating a concept stores a row; assigning frames writes the token into their exported captions after the trigger word. Deleting a concept keeps the frames — they only lose the token.",
    benefit: "\"<trigger>, kenji_xy, a wide shot\" brings that character, not the whole dataset.",
    pitfalls: "Tokens follow the same rule as the trigger word: made-up, one word, no spaces; the field warns about real words. Wide shots teach position, close-ups teach form — mix both, and vary the backgrounds.",
  },
  {
    key: "concept-description",
    label: "Description (optional)",
    what: "A note for you about what the concept covers, such as \"bare feet, visible\".",
    why: "Weeks later, a token alone does not say what you meant by it.",
    effect: "Stored with the concept; not written into captions.",
    benefit: "A reminder of the assignment rule you used.",
  },
  {
    key: "learn-grouping",
    label: "Learn mode: Group",
    what: "How Learn mode slices the kept frames into sets: by source clip, or by coarse visual similarity.",
    why: "Assigning concepts one set at a time is faster than hunting through the whole grid.",
    effect: "Only changes the order and grouping you walk through; nothing is written until you press Assign.",
    benefit: "Select a whole set (A), drop the odd ones (click), Enter assigns — hundreds of frames in minutes.",
  },
];

const HOUSEKEEPING_SETTINGS: readonly HelpSetting[] = [
  {
    key: "dedup-threshold",
    label: "Duplicate threshold",
    what: "How similar two frames anywhere in the dataset may be before \"Find duplicates\" groups them (0–16; default 6).",
    why: "The prep-time duplicate filter only compares neighbours in one video; this search compares across the whole dataset.",
    effect: "Lower catches only near-identical frames; higher also catches similar shots. Unavailable for clip datasets.",
    benefit: "One slider decides how aggressive the cross-dataset cleanup is.",
  },
  {
    key: "find-duplicates",
    label: "Find duplicates",
    what: "Scans the kept frames for near-copies at the threshold above and moves all but the sharpest of each group to Discard.",
    why: "Repeats across videos slip past the prep-time filter.",
    effect: "Marked frames get the reason \"Duplicate (dataset)\" and can be moved back; no file is deleted.",
    benefit: "Cross-video repeats are handled in seconds.",
    measured: "2026-09-19: 40 frames at threshold 6 checked in 1.2 s — 9 groups, 24 marked, 16 left in Keep.",
  },
  {
    key: "clean-up",
    label: "Clean up discarded…",
    what: "Deletes every frame in Discard with its file, after showing how many frames and bytes that is.",
    why: "Discarded frames still take disk space — usually most of the work folder, since the filters reject far more than they keep.",
    effect: "The confirmation names the count and size; on confirm the files are removed and the Disk use line updates. Kept frames and source files are never touched. Disabled while a prep run is writing.",
    benefit: "Frees the bulk of the work folder while keeping what you curated.",
    measured: "2026-09-19: 1,356 discarded frames deleted in 0.28 s, freeing 1,163,358,187 B (about 1.16 GB); the work folder went from 1,368 files to 12.",
  },
  {
    key: "delete-dataset",
    label: "Delete dataset…",
    what: "Removes the dataset with its frames, captions, concepts and work folder, after a confirmation that lists exactly what goes and what stays.",
    why: "A dataset you will not train from again is only disk space.",
    effect: "Deleted: the dataset rows, its work folder (or only its own frame files when other data shares the folder), and an export that lives inside the app's folder. Kept: your source videos and images, an export in a folder of your own, and every LoRA trained from it. A file that cannot be deleted keeps the dataset so you can retry.",
    benefit: "Reclaims everything the run wrote, without risk to your sources or your finished LoRAs.",
    measured: "2026-09-19: deleting a dataset removed 1,368 files and freed 1,173,444,536 B.",
  },
];

const EXPORT_SETTINGS: readonly HelpSetting[] = [
  {
    key: "export-destination",
    label: "Destination folder",
    what: "The folder the export writes NNNN.png + NNNN.txt pairs into (trimmed videos in Clips mode).",
    why: "The trainer reads a flat folder of image/caption pairs, not the curation database.",
    effect: "Kept items are copied there with numbered names; each .txt holds the trigger word, concept tokens and caption in the chosen order. Re-exporting after changes overwrites the pairs. A folder of your own is never deleted by the app, not even with the dataset.",
    benefit: "The Training tab only offers exported datasets — this is what makes one trainable.",
    measured: "2026-09-19: 50 images with 50 captions (80,277,647 B) were written in 104 ms.",
  },
  {
    key: "caption-order",
    label: "Caption order",
    what: "Whether each caption starts with the prose sentence (FLUX.2) or with the tag list (anime / SDXL).",
    why: "Different model families were trained on different caption styles and pay most attention to the start of the text.",
    effect: "Only the order inside each .txt changes; the trigger word always comes first.",
    benefit: "Captions in the shape the target model expects.",
    pitfalls: "Pick the order for the model you will train — the training profile names its preferred order.",
  },
];

export const DATASET_CURATION_TOPICS: readonly HelpTopic[] = [
  {
    id: "curation",
    area: "dataset",
    title: "Curation board and discard reasons",
    summary: "Keep on the left, Discard on the right, a reason on every rejected frame.",
    body: [
      {
        kind: "p",
        text: "Select with click, Ctrl-click, Shift-click or a rubber band; arrow keys move between cards, Space selects, K and D move the selection between columns, Delete removes it. The Discard column can be filtered by the reason a frame landed there. Thumbnails shows more cards; Details shows the caption under each one, editable in place.",
      },
    ],
    settings: CURATION_SETTINGS,
  },
  {
    id: "concepts",
    area: "dataset",
    title: "Concepts and Learn mode",
    summary: "One token per subject, assigned in the grid or set by set.",
    body: [
      {
        kind: "p",
        text: "Sort shows the board; Learn walks the kept frames one set at a time with the concept summary beside it. Click to pick, Shift+click for a range; ← / → change set, A selects all, I inverts, Esc clears, Enter assigns.",
      },
    ],
    settings: CONCEPT_SETTINGS,
  },
  {
    id: "housekeeping",
    area: "dataset",
    title: "Housekeeping",
    summary: "Disk use, cross-dataset duplicate search, cleanup and deleting the dataset.",
    body: [
      {
        kind: "p",
        text: "Disk use is measured on demand (the app walks the folders). Every deletion is previewed and confirmed, and the confirmation says what is never touched: your source videos and images, exports in your own folders, finished LoRAs.",
      },
    ],
    settings: HOUSEKEEPING_SETTINGS,
  },
  {
    id: "export",
    area: "dataset",
    title: "Export",
    summary: "Write the image/caption pairs the trainer reads, then hand the dataset to Training.",
    body: [
      {
        kind: "p",
        text: "Only kept items are exported. After a successful export the Train LoRA button opens the Training tab with this dataset preselected.",
      },
    ],
    settings: EXPORT_SETTINGS,
  },
  {
    id: "disk-gpu",
    area: "dataset",
    title: "What happens on disk and on the GPU",
    summary: "Where the files go, what uses VRAM, and what is unloaded afterwards.",
    body: [
      {
        kind: "list",
        items: [
          "Sources: only read. Never moved, renamed or deleted — not by the filters, not by Clean up, not by Delete dataset.",
          "Work folder: <datasets folder>\\<job id> (or the folder set under \"Store frames in\"). Every extracted frame is written there before filtering, so the folder is largest right after prep and shrinks with Clean up.",
          "Export folder: the NNNN.png + NNNN.txt pairs; inside the app's folder it is deleted with the dataset, in a folder of your own it is kept.",
          "GPU: extraction and the filters run on the CPU. The WD tagger runs on the CPU too. Florence-2 takes about 2,187 MiB of VRAM while resident; with Qwen2.5-VL escalation the card peaked at 11,867 MiB (2026-09-19). Both are unloaded when the prep job ends, so the next run loads them again.",
          "Only one job runs at a time: a prep run with a prose captioner waits for, and is waited on by, image, video and training jobs.",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "dataset",
    title: "Limits and pitfalls",
    summary: "The things that cost the most time or surprise people.",
    body: [
      {
        kind: "list",
        items: [
          "The filter stage is the slow part — about 200 ms per extracted frame (2026-09-19). A high sample rate on long videos means a long wait and a large work folder.",
          "Files directly in the root folder are tagged with the root folder's name; mixing them with subfolders gives two tags.",
          "Without captions, everything that recurs in the footage becomes part of the trigger word.",
          "Real words as trigger word or concept token collide with what the model already knows.",
          "Only exported datasets can be trained from; edits after an export are not in the pairs until you export again.",
          "Delete… on frames and the two housekeeping deletions are permanent; Discard is not.",
          "A prep run needs at least 5 GiB free on the drive it writes to, or it refuses to start.",
        ],
      },
    ],
  },
  {
    id: "measured",
    area: "dataset",
    title: "Measured numbers",
    summary: "Real timings from docs/TODO.md, RTX 4080 SUPER 16 GB, one source video, frames mode with defaults.",
    body: [
      {
        kind: "list",
        items: [
          "2026-09-19, no captioner: 298.6 s in all — extraction 19.6 s, filters 275.7 s; 1,368 frames extracted (1,173,444,536 B), 40 kept, 1,328 rejected as duplicates.",
          "2026-09-19, WD tagger: 84.0 s of captioning for 40 frames on the CPU (about 2.1 s per frame including loading); no extra VRAM.",
          "2026-09-19, Florence-2: 38.3 s of captioning for 40 frames including loading; 31.2 s when already loaded (about 0.78 s per frame); about 2,187 MiB VRAM resident.",
          "2026-09-19, Florence-2 + Qwen2.5-VL escalation of every 5th frame: 373 s, VRAM 1,818 MiB idle → 11,867 MiB peak → 2,145 MiB one second after the job.",
          "2026-09-19, Find duplicates at threshold 6: 1.2 s for 40 frames, 24 marked.",
          "2026-09-19, Clean up: 0.28 s, 1,356 files, 1,163,358,187 B freed.",
          "2026-09-19, Export: 50 pairs, 80,277,647 B, 104 ms.",
          "2026-09-19, custom \"Store frames in\" folder: 296.6 s, all files under <folder>\\<job id>.",
        ],
      },
    ],
  },
];
