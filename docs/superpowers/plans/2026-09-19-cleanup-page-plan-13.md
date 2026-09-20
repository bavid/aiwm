# Settings "Cleanup" Page Implementation Plan (Plan 13)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-19-cleanup-page-design.md`.

**Goal:** A Settings "Cleanup" section that scans what the app generated (grouped, sized, selectable), previews exactly what would be deleted, deletes only after an explicit warning + confirmation through the existing deletion gates, and logs every cleanup.

**Architecture:** `core/src/cleanup/scan.rs` (report over DB + `locations::walk`), `core/src/cleanup/apply.rs` (routes every selection through the housekeeping guard / `check_purge_target` / retention rules), migration 0021 `cleanup_log`, routes `GET /cleanup/scan`, `POST /cleanup/apply`, five more rows in `cleanup::locations`, Settings "Cleanup" section with preview dialog and history.

**Tech Stack:** Rust (axum, sqlx/SQLite), Tauri 2, React 19 + TS.

Rules: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits, never `git add -A`, never bare `git stash`, never `--no-verify`; rustfmt immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build`; never edit an existing migration; **every deletion goes through an existing gate — no new "may I delete this" rules outside `guard.rs` / `check_purge_target` / `RetentionPolicy`.**

---

### Task 1: Scan + locations

- [ ] `cleanup::locations`: add `logs`, `exports`, `voice_identities`, `comfyui_data`, `pending_import` (non-configurable; voice identities labelled user assets); tests; UI `DataLocations` passes unknown keys through (verify) — add labels.
- [ ] `core/src/cleanup/scan.rs`: `CleanupReport { scanned_at, groups, protected }`, `CleanupGroup { key, label, entries: [CleanupEntry { id, label, files, bytes, rows, detail }], total_files, total_bytes }`, `ProtectedNote { what, reason }`; group builders per spec (media by retention + orphan media files; discarded frames per dataset; unclaimed folders under the datasets root; missing-file frame rows; finished runs' folders — whole only when the result LoRA is present in the library, else protected; caches incl. stale staging/ComfyUI/pending-import; logs > 30 days; DB backups). Walks in `spawn_blocking`, links never followed. Tests per group from fixture trees + DB, plus "never listed" tests (store, runtimes, source roots, busy dataset/run/download, roots themselves) with mutation checks.
- [ ] Route/Tauri/ipc/dev mock for `GET /cleanup/scan`. Commit `feat(cleanup): scan app-generated content into groups`.

### Task 2: Apply + log

- [ ] Migration `0021_cleanup_log.sql`; repo + tests.
- [ ] `core/src/cleanup/apply.rs`: `ApplyRequest { selections: [{ group, entry_ids }], dry_run }` → per group: media via `RetentionPolicy`/explicit file list inside `outputs_dir` (flat, never `datasets/` or export subfolders), discarded frames via `housekeeping::cleanup`, unclaimed folders and missing rows via the guard, runs via `purge_run_folder`/per-file inside a `check_purge_target`-verified folder, caches/logs/backups via a simple "strictly inside that root, regular file, not today's log" rule; `refuse_if_busy` for datasets/runs; dry-run returns the exact list; apply returns `{ deleted_files, freed_bytes, removed_rows, skipped }` and writes `cleanup_log`. Tests: dry-run deletes nothing; apply deletes exactly the preview; skip reasons; log row written; mutation checks on every gate call.
- [ ] Route/Tauri/ipc/dev mock for `POST /cleanup/apply` + `GET /cleanup/log`. Commit `feat(cleanup): apply selected cleanup through the existing gates; cleanup log`.

### Task 3: UI

- [ ] `ConfirmDialog` lifted to `ui/src/components/`; Settings "Cleanup" section: intro warning, Scan, group cards with per-group/per-entry checkboxes, sticky footer with selected total, Preview dialog (dry-run list, capped), Delete (danger) → result + skipped list, Protected list, Cleanup history (last 20); Help hint. Live-verify; commit `feat(ui): Settings Cleanup page`.

### Task 4: Real run + docs

- [ ] DB backup; real daemon; `GET /cleanup/scan` on the user's data — record every group with sizes and the protected list; apply ONLY on entries created by this session's own earlier test runs (e.g. Plan 11's continue run work folder is NOT one of them — it is the user's lineage; leave it) or nothing; report honestly. `docs/TODO.md` ✅ entry; commit `docs(cleanup): Cleanup page shipped, measured`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.
