# Dataset Curation Implementation Plan (Plan 6)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-18-dataset-curation-design.md`.

**Goal:** Delete datasets and frames with their files, clean up discarded frames, find duplicates across the whole dataset, and sort frames fast in a two-column keep/discard grid.

**Architecture:** New core module `capability::dataset::housekeeping` (usage, delete, cleanup, dedup) with a path-prefix guard; bulk frame routes; UI split of the curation grid into Keep/Discard columns with rubber-band selection and drag & drop.

**Tech Stack:** Rust (tokio, sqlx/SQLite, axum, `image_hasher`, `walkdir`), React 19 + TS.

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build`; never edit an existing migration; never delete outside the dataset's app-owned folders; never touch source files.

---

### Task 1: Core — housekeeping, bulk routes, API, Tauri, ipc, dev mock

- [x] `capability/dataset/housekeeping.rs`: `usage`, `delete_dataset_with_files`, `delete_frames`, `cleanup(dry_run)`, `dedup(threshold)`; path guard (canonicalised prefix of the work dir / app-owned export); active-training refusal; tests incl. a malicious `frame_path`.
- [x] Repo additions: bulk `set_excluded` (+ clear `rejection_reason` on keep), delete frames by ids, list discarded frames, training runs by dataset with active states.
- [x] Routes + DTOs + Tauri commands + `ipc.ts` + hooks + dev mock per the spec; API tests.
- [x] Commit `feat(dataset): delete datasets and frames with their files, cleanup, global dedup`.

### Task 2: UI — Keep/Discard grid, rubber band, drag & drop, delete + cleanup + dedup

- [x] Two columns, per-column Select all, click/Ctrl/Shift, rubber-band rectangle, drag & drop between columns, keyboard shortcuts, toolbar buttons (Keep, Discard, Delete…); concept toolbar keeps working.
- [x] Dataset delete button with usage sizes and confirm; cleanup panel with dry-run preview; dedup action with threshold and result summary.
- [x] Accessible (keyboard equivalents for every mouse action, focus management, both themes), files under 800 lines (split `Dataset.tsx` if needed). Live-verify in the dev preview. Commit `feat(ui): fast keep/discard curation with rubber-band selection and drag & drop`.

### Task 3: Real run + docs

- [x] Back up `E:\AI\data\aiwm.db` first. Real `aiwm-cored` run on `D:\Data\Test` (read-only source): prep, dedup, cleanup, delete; record counts and bytes at each step; confirm the source file is untouched (size + mtime before/after).
- [x] `docs/TODO.md` ✅ entries; full gates; commit `docs(dataset): curation shipped, measured`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.

## Measured 2026-09-19

Headless `aiwm-cored` from this branch, data dir `E:\AI\data` (DB backed up first), source
`D:\Data\Test` = one video `20250111-_1.mp4` (153,603,700 B). Frames mode, default settings,
no captioner. All items were tagged `Test` (the folder name — files directly in the root).

| Step | Result |
|---|---|
| Prep job | 298.6 s wall (extraction 19.6 s, filtering 275.7 s); **1,368 frames** extracted, 1,368 files / 1,173,444,536 B in the work folder |
| Filter | **40 kept**, 1,328 rejected: duplicate 1,328, blur 0, dead 0, transition 0 |
| Global dedup (threshold 6) | 1.2 s; scanned 40, 9 groups, **24 marked**, 0 unreadable → 16 kept |
| Bulk move | 5 Keep → Discard, 1 `duplicate_global` → Keep (its `rejection_reason` cleared) → 12 kept, 1,356 discarded |
| Cleanup `dry_run` | 1,356 frames / 1,163,358,187 B announced, 0 files deleted (disk still 1,368 files) |
| Cleanup real | 0.28 s; 1,356 files deleted, **1,163,358,187 B freed**, 0 skipped; disk 1,368 → 12 files (10,086,349 B), `usage` matches |
| Delete 3 frames | 3 rows + 3 files, 2,561,739 B freed, 0 skipped |
| Delete dataset | 9 frames, 9 files, **7,524,610 B freed**, 0 skipped, no export; work folder gone, dataset absent from `GET /datasets` (404) |
| Source video | size, mtime and SHA-256 (`15F22FD0…9869BF28`) identical before and after |

Findings during the run: the filter stage dominates prep time (~200 ms per frame); the
pre-existing dataset "missveronika milkpreg" has 1,960 frame rows pointing into a work
folder that did not exist before this run (orphaned rows, not caused by it) — both noted in
`docs/TODO.md`.
