# Dataset Curation Implementation Plan (Plan 6)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-18-dataset-curation-design.md`.

**Goal:** Delete datasets and frames with their files, clean up discarded frames, find duplicates across the whole dataset, and sort frames fast in a two-column keep/discard grid.

**Architecture:** New core module `capability::dataset::housekeeping` (usage, delete, cleanup, dedup) with a path-prefix guard; bulk frame routes; UI split of the curation grid into Keep/Discard columns with rubber-band selection and drag & drop.

**Tech Stack:** Rust (tokio, sqlx/SQLite, axum, `image_hasher`, `walkdir`), React 19 + TS.

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build`; never edit an existing migration; never delete outside the dataset's app-owned folders; never touch source files.

---

### Task 1: Core — housekeeping, bulk routes, API, Tauri, ipc, dev mock

- [ ] `capability/dataset/housekeeping.rs`: `usage`, `delete_dataset_with_files`, `delete_frames`, `cleanup(dry_run)`, `dedup(threshold)`; path guard (canonicalised prefix of the work dir / app-owned export); active-training refusal; tests incl. a malicious `frame_path`.
- [ ] Repo additions: bulk `set_excluded` (+ clear `rejection_reason` on keep), delete frames by ids, list discarded frames, training runs by dataset with active states.
- [ ] Routes + DTOs + Tauri commands + `ipc.ts` + hooks + dev mock per the spec; API tests.
- [ ] Commit `feat(dataset): delete datasets and frames with their files, cleanup, global dedup`.

### Task 2: UI — Keep/Discard grid, rubber band, drag & drop, delete + cleanup + dedup

- [ ] Two columns, per-column Select all, click/Ctrl/Shift, rubber-band rectangle, drag & drop between columns, keyboard shortcuts, toolbar buttons (Keep, Discard, Delete…); concept toolbar keeps working.
- [ ] Dataset delete button with usage sizes and confirm; cleanup panel with dry-run preview; dedup action with threshold and result summary.
- [ ] Accessible (keyboard equivalents for every mouse action, focus management, both themes), files under 800 lines (split `Dataset.tsx` if needed). Live-verify in the dev preview. Commit `feat(ui): fast keep/discard curation with rubber-band selection and drag & drop`.

### Task 3: Real run + docs

- [ ] Back up `E:\AI\data\aiwm.db` first. Real `aiwm-cored` run on `D:\Data\Test` (read-only source): prep, dedup, cleanup, delete; record counts and bytes at each step; confirm the source file is untouched (size + mtime before/after).
- [ ] `docs/TODO.md` ✅ entries; full gates; commit `docs(dataset): curation shipped, measured`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.
