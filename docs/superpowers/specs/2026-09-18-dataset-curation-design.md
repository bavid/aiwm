# Dataset Curation — Design (delete, cleanup, global dedup, fast sorting)

Status: written 2026-09-18 (night run) from the user's backlog wishes in `docs/TODO.md`,
section "Dataset-Kuratierung & Trainings-Werkzeuge": delete datasets (asked twice), a
cleanup job because extraction leaves "tons of duplicates", real frame deletion, and a
faster keep/discard grid with rubber-band selection, select-all and drag & drop.

## What exists (verified in code)

- `DELETE /datasets/{id}` → `db.datasets().delete(id)` removes only DB rows (frames and
  concepts cascade via FK). The frame files stay on disk. `ipc.ts` has `deleteDataset`, but
  no UI calls it.
- Frames are extracted to `<outputs>/datasets/<prep_job_id>/raw/<tag>/<video stem>/frame_%06d.png`
  (`orchestrator/engine.rs` passes `outputs_dir.join("datasets")`, `pipeline.rs` builds `raw_dir`).
- A frame row carries `excluded: bool` (user choice) and `rejection_reason: String` (filter
  verdict: blur, duplicate, dead frame, transition). Export skips both. Nothing in
  `core/src/capability/dataset/` deletes files.
- Duplicate detection (`filter.rs`) compares a pHash only with the last *kept* frame of the
  *same source*; repeats later in a video or across videos survive. `select_diverse` exists.
- Exports go to a user-chosen `dest_dir` stored as `datasets.export_dir`; a training run
  (`training_runs.dataset_id`, `ON DELETE SET NULL`) trains from that export.
- The curation grid (`ui/src/features/dataset/Dataset.tsx`, `FrameCard.tsx`,
  `SelectionToolbar.tsx`) already has a selection model for concepts, and per-frame exclude.

## Decisions

| Question | Decision |
|---|---|
| What does "delete dataset" remove? | DB rows **and** the app-owned work folder `<outputs>/datasets/<prep_job_id>/`. The export folder is removed only when it lies inside the app's outputs folder; a user-chosen folder elsewhere is shown and left alone (it may hold other files). Source videos/images are never touched. |
| Delete while training uses it? | Refused (409-style refusal, 400 in this codebase's mapping) while any training run for the dataset is `preparing / running / paused / interrupted / resuming / finishing`, naming the run. Finished runs keep their LoRA; their `dataset_id` becomes NULL (existing FK). |
| Size shown before deleting | A `GET /datasets/{id}/usage` returns bytes and file counts for the work folder, the export (if app-owned) and "rejected + excluded frames" — computed by walking the folders, never guessed. |
| Discard vs delete for frames | Two separate actions. **Discard** = set `excluded` (reversible, files stay). **Delete** = remove the frame files and rows (irreversible, confirm). Rejected-by-filter frames count as discarded. |
| Cleanup job | `POST /datasets/{id}/cleanup` deletes the files **and** rows of every frame that is rejected or excluded, after a preview (`dry_run: true` returns count + bytes). Never touches kept frames. Refused while a training run is active on the dataset (same rule as above). Runs inline — it is file deletion, not GPU work. Optional "clean up automatically after extraction" is **not** in this plan. |
| Global duplicates | `POST /datasets/{id}/dedup` computes pHashes for every kept frame across all sources, groups frames within a Hamming threshold (default = the filter's existing duplicate threshold, adjustable 0–16), keeps the sharpest frame per group (Laplacian variance, already implemented) and marks the rest `rejection_reason = "duplicate_global"`. Reversible: the frames are only marked, the cleanup job deletes them later. Returns groups and counts. Runs as a job if the dataset is large enough to block the API for more than a moment (> 500 frames), otherwise inline — the implementer decides from a measured timing, not a guess. |
| Sorting UI | Two columns: **Keep** (left) and **Discard** (right: excluded + rejected, each with its reason chip). Multi-select with click, Ctrl/Shift-click, **rubber-band rectangle** drawn with the mouse, and **Select all** per column. Move the selection by drag & drop onto the other column, or with a keyboard shortcut (e.g. `D` discard / `K` keep) and buttons in the selection toolbar. Moving a rejected frame to Keep clears its `rejection_reason`. Bulk endpoint so moving 500 frames is one request. |
| Existing concept selection | Kept working: the same selection feeds the concept toolbar. |

## API (new or changed)

- `GET /datasets/{id}/usage` → `{ work_dir, work_bytes, work_files, export_dir, export_bytes, export_app_owned, discarded_frames, discarded_bytes }`
- `DELETE /datasets/{id}` → now also removes files per the rules above; returns 200 with `{ frames, deleted_files, freed_bytes, skipped_files: [{path, reason}], export_dir_kept, dataset_deleted }` (as shipped; `dataset_deleted: false` keeps the dataset when a file could not be deleted). Unknown id → 404.
- `POST /datasets/{id}/frames/bulk` `{ frame_ids, excluded: bool }` → sets `excluded` (and clears `rejection_reason` when keeping).
- `POST /datasets/{id}/frames/delete` `{ frame_ids }` → deletes files + rows; returns `{ deleted, freed_bytes }`.
- `POST /datasets/{id}/cleanup` `{ dry_run }` → `{ frames, bytes }`.
- `POST /datasets/{id}/dedup` `{ threshold? }` → `{ groups, marked }` (or a job id if run as a job).
- Tauri commands mirror all of them; `ipc.ts`, hooks and the dev mock follow.

## Safety rules (enforced in core, tested)

- Every deletion path resolves each file and refuses anything not under the dataset's
  app-owned work folder (or the app-owned export folder) — canonicalised prefix check, so a
  crafted `frame_path` in the DB cannot make the app delete elsewhere.
- Never delete source files (`source_path`).
- Deletions are logged as job events or tracing lines with counts and bytes.

## Tests / proof

Unit and integration tests for every endpoint (including the path-prefix guard with a
malicious `frame_path`, the active-training refusal, dry-run vs real cleanup, dedup
grouping across two sources, bulk keep clearing `rejection_reason`). UI live-verified in the
dev preview (rubber band, select all, drag & drop, keyboard, delete confirm with sizes).
A real run: build a dataset from the user's `D:\Data\Test` (one `.mp4`, read-only source),
run dedup, clean up, delete the dataset, and record frame counts and bytes freed at each step.

## Not in this plan

Automatic cleanup after extraction, moving datasets between drives (storage-location plan),
one-click captioner installs (training-tools plan), the global Cleanup settings page.
