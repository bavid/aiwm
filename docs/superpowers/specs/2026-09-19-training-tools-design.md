# Training & Captioning Tools in Discover — Design

Status: written 2026-09-19 (night run) from the user's backlog wish (`docs/TODO.md`,
"Dataset-Kuratierung & Trainings-Werkzeuge"): "Add in Download / Discover models — models / tools /
addons for training / image description. Currently I get *No captioner installed — import
Florence-2 or the WD tagger on the Models tab…* but I don't want to install manually — it should be
recommended on the model page."

## What exists (verified in code)

- Captioners are resolved from the model library **by role** (`core/src/capability/dataset/captioner.rs`):
  `florence2` (role `vision_florence2`, any row with the role counts), `wd-eva02-tagger-v3`
  (role of `ModelKind::WdTagger`, `required_files = model.onnx + selected_tags.csv` in one folder),
  Qwen2.5-VL escalation (role `vision_qwen2_5_vl`, a model directory).
- The catalog (`core/src/model/catalog.rs`) already has the **WD EVA02-Large Tagger v3** as two
  `KnownModel` entries (`model.onnx` 1,260,435,999 B, sha256 `9e76…bfc`; `selected_tags.csv`
  308,468 B, sha256 `2986…441217`), kind `wd_tagger`, media `image`, but no `ModelStack` and no
  one-click path from the Dataset tab.
- Multi-file Hugging Face repos that a Python loader reads as a **directory** are already solved for
  Dia (`ModelKind::DiaEngine` / `DiaCodec`): one `KnownModel` per file, pinned URL, real sha256,
  co-located with original file names, never hash-suffixed (`model::import::unique_destination`).
- Downloads go through the download manager (resume, sha256 verification, import with roles),
  behind the offline gate.

## Decisions

| Question | Decision |
|---|---|
| Where in the UI? | A **"Training & captioning"** section in the Models tab's catalog/Discover area, next to the image/video/voice stacks, listing each tool as a stack: what it is for, size, license, runs on CPU/GPU, installed state, one "Install" button. |
| Recommended default | **WD EVA02 Tagger v3** — CPU-only, 1.26 GB, no GPU contention with training; marked "Recommended". Florence-2 as the prose alternative; Qwen2.5-VL as the optional "second opinion" for Florence-2 (large). |
| Dataset tab hint | The "No captioner installed …" hint gets an **"Install the recommended captioner (WD tagger, 1.3 GB)"** button that starts the stack download, shows progress, and refreshes the captioner list when done. The captioner picker shows an Install button next to each uninstalled captioner. The text stops telling the user to import manually. |
| Florence-2 files | New directory-shaped kind (pattern of `DiaEngine`) for `microsoft/Florence-2-large`: every file the loader needs (weights, configs, tokenizer, **and the `trust_remote_code` Python files**), **pinned to one commit revision** in the URL (`/resolve/<commit>/…`), because remote code must never change under us. Role `vision_florence2` on import. |
| Qwen2.5-VL files | Same pattern for `Qwen/Qwen2.5-VL-7B-Instruct` (about 16 GB), pinned to a commit, role `vision_qwen2_5_vl`. Optional; clearly labelled large. |
| Hashes | **Never guessed.** Each file's sha256 is computed from a file actually downloaded during development (or taken from the Hub's LFS metadata **and** confirmed by hashing the downloaded file). Sizes likewise. The commit hash of each pinned revision is recorded in the catalog note. |
| Licenses | Florence-2: MIT; Qwen2.5-VL-7B-Instruct: Apache-2.0; WD tagger: Apache-2.0 — each verified on the model card at build time, not from memory. |
| Offline | Installing is a network action → offline gate as for every download. |

## Tests / proof

Catalog invariants (every stack member exists, unique ids, sha256 format, pinned revision in
URLs for the directory kinds), import places directory-kind files side by side with original
names, captioner detection finds an installed Florence-2/WD stack, API returns the new section,
UI live-verified in the dev preview. **Real proof:** install the WD tagger through the app's
download path and run a dataset prep on `D:\Data\Test` (read-only source) with it; then the same
with Florence-2 if the install succeeds; record captions produced, time and VRAM.

## Not in this plan

Other training add-ons (upscalers for datasets, face detectors), automatic captioner choice,
bundling models with the installer.
