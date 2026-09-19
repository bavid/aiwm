# LoRA Overview & Continue Training — Design (Plan 11)

Status: written 2026-09-19 from the backlog section "LoRA-Übersicht & Weitertrainieren" in
`docs/TODO.md`. The user's goal: improve one LoRA step by step with further datasets instead of
training ~1 TB of raw material in one run, and always see what a LoRA has been trained with.

## What exists (verified in code 2026-09-19)

- ai-toolkit (pinned `e65c4d0`) **does** support starting a new job from an existing LoRA file:
  `network.pretrained_lora_path` (`toolkit/config_modules.py:226`). It is used only when the job's own
  `training_folder/<name>` holds no checkpoint (`BaseSDTrainProcess.py:859-866`) — always the case for
  an AIWM run because every run has its own `work_dir`. Step counter restarts at 0; no optimizer
  state is needed.
- **It fails silently** in three ways that AIWM must catch itself: missing file → trains from scratch
  after a print; rank mismatch → zero-pads/truncates (`network_mixins.py:737-775`); different
  architecture → keys dropped, near-no-op init. On SDXL the alpha scale is not recomputed after load,
  so alpha must equal the source's.
- AIWM: `NetworkBlock` (`core/src/training/config.rs:184-190`) renders `type/linear/linear_alpha`
  only. `training_runs` (migration 0016) has no parent-LoRA column. The finished LoRA is imported with
  `models.source = "training:<run_id>"` and `training_runs.result_model_id` points back
  (`runner/settle.rs`). `models` has no kind column; LoRA-ness is the store subdir. The Training tab's
  `RunCard` shows no dataset/preset/rank/timestamps; the Models tab has no LoRA list. Rank is readable
  from the safetensors header (`core/src/model/safetensors.rs` `read_safetensors_info`).

## Decisions

| Question | Decision |
|---|---|
| Lineage storage | Migration **0020**: `ALTER TABLE training_runs ADD COLUMN init_lora_model_id TEXT REFERENCES models(id) ON DELETE SET NULL` — the library LoRA a run started from (NULL = from scratch). A LoRA's **lineage** is the chain run → `init_lora_model_id` → that model's `source = training:<id>` → its run → … computed on read, no extra table. |
| Continue training | `StartRequest.init_lora_model_id: Option<String>` (+ DTO/TS). Preflight refuses, before anything is created: model missing or file missing on disk; model not a LoRA (store subdir); `library_family(profile_family)` ≠ the LoRA's `family`; requested rank ≠ the LoRA's rank (from the safetensors header — `linear`/`lora_down` first dim); LoRA belongs to a run that is still running. The form pins rank to the source's and shows why. Config renders `pretrained_lora_path: <file_path>` (Windows path as-is; YAML-escaped) and `linear_alpha = rank`. |
| Result | Always a **new** library model (new hash → new row) named `<run name>`; the source LoRA is untouched. Both are visible in the lineage. |
| Forgetting caveat ("Mischen") | v1 of this plan: **explain, don't automate.** The form shows a notice when continuing: "Training only on the new dataset drifts towards it and partly overwrites what the earlier datasets taught. To keep earlier material, add some of it to this dataset." A dataset-mixing feature (feeding a share of earlier datasets) is recorded as a follow-up, because ai-toolkit's multi-dataset config and the AIWM dataset export would both need work; not built here. |
| LoRA overview | New Training-tab section **"Your LoRAs"** (above runs): every library model with `source LIKE 'training:%'`, newest first: name, family, rank, size, date, run count, total steps, total images across lineage. Selecting one opens its **history**: each contributing run (oldest first) with dataset name + source root, images/frames used (count of kept frames at run start — stored, see below), captioner, trigger word, profile, preset, rank/lr/resolution, steps, duration, result state, the run's samples (existing thumbnails), and buttons **"Test in Image tab"** (existing) and **"Continue with another dataset"** (opens the new-run form with target/profile/rank prefilled, `init_lora_model_id` set). Hand-imported LoRAs (no `training:` source) are listed under "Imported" with what is known (name, family, size) and can also be continued. |
| Image count per run | Migration 0020 also adds `training_runs.image_count INTEGER` — the number of kept frames/clips at start (already counted during export/preflight; store it). NULL for old rows → shown as "unknown". |
| Data-amount guidance | The overview header states: "A LoRA typically needs a few hundred to a few thousand well-chosen images; the pipeline samples frames (default 1.5 fps) and filters blur and duplicates." Numbers for duration/VRAM come from the measured tables in `docs/TODO.md`, not estimates. |
| API | `GET /training/loras` → list with aggregates; `GET /training/loras/{model_id}` → lineage detail (runs in order). Tauri commands + ipc + dev mock. |
| Cleanup interaction | A LoRA in the library is never offered for cleanup (existing rule); its runs' work folders remain purgeable as today — purging a run does not delete the library LoRA (imported copy). |

## Tests / proof

Unit: config renders `pretrained_lora_path` only when set; preflight refusals (missing file, family
mismatch, rank mismatch via a synthetic safetensors header, non-LoRA model, running parent); lineage
walk (3-deep chain, cycle-safe, missing link ends chain); aggregates; migration adds columns. UI
verified in the dev preview (overview, history, continue prefill, rank pinned + notice).

**Real run:** with the real daemon, start a short continue-run (fast preset, minimal steps) from an
existing library LoRA on a dataset export, confirm the YAML contains `pretrained_lora_path`, the
trainer log shows the pretrained LoRA being loaded (ai-toolkit prints the path), the result is a new
library model, and the overview shows the two-run lineage. If no suitable base model/LoRA pair is
installed, record exactly what is missing instead of pretending.

## Not in this plan

Automatic dataset mixing; editing/merging LoRAs; training from a LoRA of another family.
