# Settings "Cleanup" Page — Design (Plan 13)

Status: written 2026-09-19 from the backlog section "Aufräumen: Settings-Seite »Cleanup«" in
`docs/TODO.md`. The user's wish: one page that, from the configured paths and the app's own database,
finds what the app itself generated and could be removed — grouped by kind with size and count —
selectable per group/entry, a "X GB will be freed" preview, deletion only behind a clear warning and
confirmation, and a log of what was deleted. **Never offered:** models (library, weight downloads,
installed runtimes), the user's source files outside app folders, finished LoRAs in the library, and
anything a running/paused job or training run still needs. The one-off developer-machine sweep
(cargo `target`, worktrees, test artefacts) is a separate, later, measure-only step — not this page.

## What exists (verified in code 2026-09-19)

- Path registry `core/src/paths.rs` (`outputs`, `datasets` — default inside outputs —, `training`,
  `cache`, `downloads` staging, `comfyui_data_dir` with `input/temp/output`, `voice_identities_dir`,
  `logs_dir` — daily files, never pruned —, `exports_dir` (DB backups), `pending_import_dir`).
  `cleanup::locations` sizes only 7 keys; logs, exports, voice identities, comfyui-data,
  `.pending-import` are invisible today.
- Two test-covered deletion gates that must stay the only ones: the dataset housekeeping guard
  (`capability/dataset/housekeeping/guard.rs`: never source files/roots, never other datasets or run
  folders, only strictly inside own roots, links never followed, `refuse_if_busy`) and the training
  purge check (`training/location.rs::check_purge_target`). Retention sweep for generated media
  (`cleanup/outputs.rs`) deletes files only, never rows; `job_output_path` tolerates a gone file.
- Orphans: `StorageReport.models[].file_present` is the only "row without file" flag; dataset frames
  have none (the "missveronika milkpreg" finding: 1,960 rows whose folder never existed); no "file
  without row" scan exists anywhere. Download staging and ComfyUI input staging self-clean unless
  the process died.
- Captioners write no HF cache (explicit `model_dir`, `local_files_only`). Dia keeps a hardlink
  `.hf-cache` **inside the model store** — model-adjacent, never offered.
- No audit table; deletions are only `tracing` lines in `logs/aiwm.log`.
- UI: `Settings.tsx` `SECTIONS` + `SectionNav`; `ConfirmDialog` (native `<dialog>`, focus on Cancel)
  is reusable; `HousekeepingPanel` is the measure → dry-run → confirm → summary precedent.

## Decisions

| Question | Decision |
|---|---|
| Scan | `GET /cleanup/scan` → `CleanupReport { scanned_at, groups: [CleanupGroup], protected: [ProtectedNote] }`. Runs the walks in `spawn_blocking` (reusing `locations::walk`, links/junctions never followed), DB reads async. Computed on demand + "Scan again"; never on a timer. |
| Groups (kind, what counts, how it is identified) | 1 **Generated media, older than N days / beyond the retention size** — flat files in `outputs_dir` whose `jobs.output_path` matches (rows kept, like the retention sweep) plus **orphan media files** (flat file in `outputs_dir` with no `jobs` row → separate entry list). 2 **Discarded dataset frames** — per dataset, `excluded|rejected` frames (existing `cleanup` semantics, through the guard). 3 **Dataset work folders of deleted/orphaned datasets** — folders under `datasets_dir` not claimed by any dataset row (through the guard's "unclaimed under the datasets root" rule only; custom `work_dir`s elsewhere are never guessed). 4 **Frame rows whose files are missing** — per dataset, count of `dataset_frames` rows with no file (zero bytes freed; removing them is a DB cleanup that the grid otherwise shows as broken thumbnails) — offered as "remove N stale entries". 5 **Finished training runs' work folders** — per run in a terminal state: checkpoints (`*_<step>.safetensors` below the final), `optimizer.pt`, samples, `train.log`; the whole folder only if the run's result LoRA is in the library (`result_model_id` present and file present) — otherwise the final checkpoint is the only copy and the run is listed under *protected* with the reason. Deletion goes through `check_purge_target` (whole folder) or per-file inside the verified folder. 6 **Caches** — `cache_dir` contents (always safe), leftover download staging dirs with no active `downloads` row, ComfyUI `input`/`temp`/`output` leftovers (only files older than 1 h and not referenced by a non-terminal job), `.pending-import` leftovers. 7 **Logs older than 30 days** in `logs_dir` (never today's file). 8 **DB backups** in `exports_dir` (listed with dates; user picks). |
| Never listed | The model store and anything inside it (incl. Dia's `.hf-cache`), runtimes, library LoRAs, voice identities (user assets), anything referenced by a non-terminal job/run/download, source roots and everything under them, the datasets/outputs/training roots themselves, exports outside outputs (foreign folders), custom dataset work folders that fail the guard. Each refusal that hides something the user might expect appears under `protected` with a one-line reason. |
| Apply | `POST /cleanup/apply` with `{ selections: [{ group, entry_ids? }], dry_run }`. `dry_run: true` returns the exact files/bytes that would go; `false` deletes, group by group, through the existing gates, best-effort per file (skips with reasons, like housekeeping), and returns `{ deleted_files, freed_bytes, removed_rows, skipped: [{path, reason}] }`. Refuses the whole request if any selected dataset/run is busy (`refuse_if_busy`) — nothing partial across groups is fine, within a group it is per-entry. |
| Log | New migration **0021** `cleanup_log (id, ts, group, entry, deleted_files, freed_bytes, removed_rows, skipped_count, detail_json)` — no FK to jobs so it survives. The page shows the last 20 entries ("Cleanup history"). |
| Page | Settings section **"Cleanup"** (after "Storage & data"): intro warning; "Scan" button; groups as expandable cards with count, size, a checkbox per group and per entry (entries: datasets by name, runs by name, backups by date, orphan files by name), a sticky footer "Selected: X GB in N items — Preview" → dialog listing what will be deleted (`dry_run` result, capped list + "… and N more") with the warning text "This deletes files. Models, source media and anything a running job needs are never included." → **Delete** (danger, `ConfirmDialog` lifted to `components/`). Result summary + skipped list (reuse `SkippedFiles`). "Protected" list at the bottom explaining what was not offered and why. Help hint (Plan 12) on the section. |
| Interaction with existing features | Dataset "Clean up discarded" / "Delete dataset" and run "Delete (+purge)" stay where they are; this page calls the same functions. Retention settings stay on the "Storage & data" card; group 1 reuses `RetentionPolicy` for the age/size rule when set, else lists nothing under "older than" (only orphans). |
| Locations report | Add the five missing folders (`logs`, `exports`, `voice-identities`, `comfyui-data`, `pending-import`) to `cleanup::locations` as non-configurable rows so "Data locations" shows them (voice identities marked "user assets"). |

## Tests / proof

Unit: scan finds each group from a fixture tree + DB (orphan media, discarded frames, unclaimed
work folder, missing-file rows, finished run with/without library LoRA, stale staging dir, old log,
backup); never lists store/runtimes/source roots/busy items and records them under `protected`;
dry-run deletes nothing; apply deletes exactly the preview, skips with reasons, writes `cleanup_log`;
mutation-style checks on every "never" rule (as in Plan 6/10). UI live-verified (scan, select,
preview, confirm, history). **Real run:** real daemon, DB backed up, scan on the user's data —
report the groups and sizes found; apply only on entries this session created earlier (its own
test runs/exports) or nothing at all — the user decides on their data.

## Not in this plan

The one-off developer-machine sweep (separate measure-only step); automatic scheduled cleanup;
deleting models or runtimes; compacting the SQLite file.
