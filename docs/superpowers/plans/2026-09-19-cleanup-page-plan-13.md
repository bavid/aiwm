# Settings "Cleanup" Page Implementation Plan (Plan 13)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-19-cleanup-page-design.md`.

**Goal:** A Settings "Cleanup" section that scans what the app generated (grouped, sized, selectable), previews exactly what would be deleted, deletes only after an explicit warning + confirmation through the existing deletion gates, and logs every cleanup.

**Architecture:** `core/src/cleanup/scan.rs` (report over DB + `locations::walk`), `core/src/cleanup/apply.rs` (routes every selection through the housekeeping guard / `check_purge_target` / retention rules), migration 0021 `cleanup_log`, routes `GET /cleanup/scan`, `POST /cleanup/apply`, five more rows in `cleanup::locations`, Settings "Cleanup" section with preview dialog and history.

**Tech Stack:** Rust (axum, sqlx/SQLite), Tauri 2, React 19 + TS.

Rules: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits, never `git add -A`, never bare `git stash`, never `--no-verify`; rustfmt immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build`; never edit an existing migration; **every deletion goes through an existing gate — no new "may I delete this" rules outside `guard.rs` / `check_purge_target` / `RetentionPolicy`.**

---

### Task 1: Scan + locations

- [x] `cleanup::locations`: add `logs`, `exports`, `voice_identities`, `comfyui_data`, `pending_import` (non-configurable; voice identities labelled user assets); tests; UI `DataLocations` passes unknown keys through (verify) — add labels.
- [x] `core/src/cleanup/scan.rs`: `CleanupReport { scanned_at, groups, protected }`, `CleanupGroup { key, label, entries: [CleanupEntry { id, label, files, bytes, rows, detail }], total_files, total_bytes }`, `ProtectedNote { what, reason }`; group builders per spec (media by retention + orphan media files; discarded frames per dataset; unclaimed folders under the datasets root; missing-file frame rows; finished runs' folders — whole only when the result LoRA is present in the library, else protected; caches incl. stale staging/ComfyUI/pending-import; logs > 30 days; DB backups). Walks in `spawn_blocking`, links never followed. Tests per group from fixture trees + DB, plus "never listed" tests (store, runtimes, source roots, busy dataset/run/download, roots themselves) with mutation checks.
- [x] Route/Tauri/ipc/dev mock for `GET /cleanup/scan`. Commit `feat(cleanup): scan app-generated content into groups`.

### Task 2: Apply + log

- [x] Migration `0021_cleanup_log.sql`; repo + tests.
- [x] `core/src/cleanup/apply.rs`: `ApplyRequest { selections: [{ group, entry_ids }], dry_run }` → per group: media via `RetentionPolicy`/explicit file list inside `outputs_dir` (flat, never `datasets/` or export subfolders), discarded frames via `housekeeping::cleanup`, unclaimed folders and missing rows via the guard, runs via `purge_run_folder`/per-file inside a `check_purge_target`-verified folder, caches/logs/backups via a simple "strictly inside that root, regular file, not today's log" rule; `refuse_if_busy` for datasets/runs; dry-run returns the exact list; apply returns `{ deleted_files, freed_bytes, removed_rows, skipped }` and writes `cleanup_log`. Tests: dry-run deletes nothing; apply deletes exactly the preview; skip reasons; log row written; mutation checks on every gate call.
- [x] Route/Tauri/ipc/dev mock for `POST /cleanup/apply` + `GET /cleanup/log`. Commit `feat(cleanup): apply selected cleanup through the existing gates; cleanup log`.

### Task 3: UI

- [x] `ConfirmDialog` lifted to `ui/src/components/`; Settings "Cleanup" section: intro warning, Scan, group cards with per-group/per-entry checkboxes, sticky footer with selected total, Preview dialog (dry-run list, capped), Delete (danger) → result + skipped list, Protected list, Cleanup history (last 20); Help hint. Live-verify; commit `feat(ui): Settings Cleanup page`.

### Task 4: Real run + docs

- [x] DB backup; real daemon; `GET /cleanup/scan` on the user's data — record every group with sizes and the protected list; apply ONLY on entries created by this session's own earlier test runs (e.g. Plan 11's continue run work folder is NOT one of them — it is the user's lineage; leave it) or nothing; report honestly. `docs/TODO.md` ✅ entry; commit `docs(cleanup): Cleanup page shipped, measured`.
- [x] Hand back: whole-branch review (approved) → controller gates (1415 lib tests, UI clean) → merge `--no-ff` → push.

## Measured

Task 4 real run, 2026-09-20, real `aiwm-cored` built from this worktree (`8922e8c`), `AIWM_DATA_DIR=E:\AI\data`, port 48160 (desktop app not running). DB backed up to the session scratchpad first (`aiwm.db` SHA-256 `0D070E042FB1A4F3CEC83389A68938F1112276841DC3679DB763D765943F5DBD`, byte-identical afterwards; only `-wal`/`-shm` changed because migration 0021 `cleanup log` was applied live). `config.toml` untouched. **Look-only: nothing was deleted** — only `GET /cleanup/scan` and `POST /cleanup/apply` with `dry_run: true`; `cleanup_log` has 0 rows and `GET /cleanup/log` is `[]` before and after. Nothing on this machine was created by this session, so there was nothing of "our own" to apply on; the user decides on the page. The full table (every group, entry, byte count, cross-check and dry-run preview) is in `docs/TODO.md` under "Aufräumen: Settings-Seite »Cleanup«".

| Step | Result |
|---|---|
| `GET /storage/locations` | 12 rows (the 5 new ones present: logs 1,522,418 B / 10, exports missing, voice_identities missing, comfyui_data 152,028 B / 3, pending_import missing); models 148,320,782,317 B / 117 and runtimes 21,878,826,432 B / 199,116 listed but protected; `volume_free_bytes` filled (1,174,374,703,104 of 1,738,237,014,016) |
| `GET /cleanup/scan` | 0.257 s; `media_retention` 0, `media_orphans` 0 (`outputs/` empty), `discarded_frames` 2 entries with 0 files / 0 B (1,985 + 4 rows — the present frame files are the user's source pictures, guard keeps them), `unclaimed_dataset_folders` 0, `missing_frame_rows` 1 entry "missveronika milkpreg" 1,960 of 2,051 rows (re-counted independently via read-only SQL: 1,926 discarded + 34 kept, drive `E:` present, folder gone), `finished_runs` 1 entry "myrender-v3 — whole folder" 10 files / 93,536,565 B (library LoRA is a separate file, single hardlink; run 1's vanished folder silently skipped), `caches` 1 entry `cache` 60 files / 283,550 B, `old_logs` 0, `db_backups` 0; `protected`: model store, runtime installs, voice identities |
| Dry run (a) `caches/cache` + (b) whole `missing_frame_rows` + (c) `finished_runs/01a0bab4…` | 0.146 s; `dry_run: true`, 70 files / 93,820,115 B / 1,960 rows, 0 skipped; every path on `E:\` (60 `cache\registry\*.json`, 10 run-folder files), 0 paths for the rows entry |
| Dry run, both `discarded_frames` entries | 0.041 s; 0 files / 0 B / 1,989 rows, 0 paths — no `D:\` or scratchpad source path listed |
| Proof nothing changed | cache 60 / 283,550 B, run folder 10 / 93,536,565 B, training 161 / 202,148,331 B, logs 10, outputs 0 (identical to the baseline); `dataset_frames` 2,051 + 54 rows; `cleanup_log` 0 rows; source folder `D:\…\Pics` still 91 files; daemon log shows exactly two `cleanup apply dry_run=true` lines |
| Teardown | daemon PID 5040 stopped, no `aiwm-cored`/`cargo` process left, port 48160 has no listener |

No wrong entry found (nothing offered that must not go). Noted for later, not Plan 13: the export folder `E:\AI\data\training\datasets\myrenders-v1` (151 files / 108,611,766 B incl. `_latent_cache` 50 / 28,331,608 B) is neither offered nor mentioned (exports are not a group per spec); the DB backups next to the database in the data root (`aiwm.db.bak-*`) are invisible to the page (they are not in `exports/`); `jobs` rows whose output file is gone (49) have no group; one dataset can appear in both `discarded_frames` and `missing_frame_rows` (1,926 shared rows here — selecting both is best-effort by design).
