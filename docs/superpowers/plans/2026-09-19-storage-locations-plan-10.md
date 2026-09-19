# Storage Locations Implementation Plan (Plan 10)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-19-storage-locations-design.md`.

**Goal:** Choose where dataset frames and training runs are stored (per prep/run, optional) and see/change every data location centrally in Settings with sizes, free space and "Open folder".

**Architecture:** Two new configurable roots (`datasets_path`, `training_path`), per-dataset `work_dir` column (migration 0019) honoured by the housekeeping guard, runner reading the stored run `work_dir`, a `/storage/locations` usage route, and UI pickers.

**Tech Stack:** Rust (axum core, sqlx/SQLite), Tauri 2, React 19 + TS.

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits (`git commit -- <paths>`), never `git add -A`, never bare `git stash`, never `--no-verify`; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build` (in `ui/`); never edit an existing migration.

---

### Task 1: Central roots + locations route

- [ ] `PathsConfig.datasets_path` / `training_path` (`Option<PathBuf>`); `AppPaths::datasets_dir()` (default `outputs_dir()/datasets`) and `training_dir()` (default `root/training`) with `with_*_override`; wire in `app.rs` (engine dataset work root, `TrainingRunner` root); `ConfigUpdate`/`PathsUpdateDto`/`AboutDto` + `save_config` (`non_empty_path`); tests.
- [ ] `GET /storage/locations` (outputs, datasets, training, model store, runtimes, cache, downloads): recursive bytes/files in `spawn_blocking`, no symlink/junction following, unreadable entries skipped and counted, `exists`, volume free/total via `cleanup::volume_free`; Tauri command + `ipc.ts` + dev mock; tests.
- [ ] Commit `feat(paths): configurable dataset and training roots; storage locations report`.

### Task 2: Per-dataset location

- [ ] `DatasetPrepRequest.data_dir` + validation (absolute; not inside/equal source root; source root not inside `<data_dir>`; not inside model store); engine uses `data_dir.unwrap_or(datasets_dir())`; free-space preflight `MIN_PREP_FREE_BYTES = 5 GB` on that drive (probe injectable for tests).
- [ ] Migration `0019_dataset_work_dir.sql` (`ALTER TABLE datasets ADD COLUMN work_dir TEXT`); `NewDataset.work_dir`; every new dataset records its absolute work folder; DTO exposes it.
- [ ] Guard: row `work_dir` → app-owned work folder (canonical; refused if drive root, overlaps any `source_root` or another dataset's work folder); legacy rows unchanged; usage/cleanup/delete/delete_frames honour it; extend safety tests (custom location deleted; neighbour, source, overlapping cases untouched).
- [ ] Commit `feat(dataset): choose where a dataset's frames are stored`.

### Task 3: Per-run location

- [ ] `StartRequest.data_dir` / `StartRunDto.data_dir`; validation; run folder `<data_dir>/<run_id>` stored in `training_runs.work_dir`; one helper returns the stored `work_dir` and every recomputing site (runner.rs, runner/settle.rs) uses it; `check_disk` on the run's folder; tests (resume, settle, purge, preflight on chosen drive).
- [ ] Commit `feat(training): choose where a training run is stored`.

### Task 4: UI

- [ ] Settings "Data locations": every location with path, size, free space, Open folder, Choose… picker for configurable ones, Refresh, "existing data stays where it is" note.
- [ ] PrepForm "Store frames in" and NewRunForm "Store run in": default shown, Choose…/Reset, free space of the chosen drive, send `data_dir` only when changed; ipc types; dev mock.
- [ ] Live-verify in the dev preview; commit `feat(ui): storage locations in Settings, prep and training forms`.

### Task 5: Real run + docs

- [ ] Back up `E:\AI\data\aiwm.db` (+ -wal/-shm). Real `aiwm-cored`: prep on `D:\Data\Test` (SHA-256 before/after) storing frames in a scratchpad folder; verify files there and none under `outputs/datasets`; usage + cleanup + delete work on it; record `/storage/locations`.
- [ ] `docs/TODO.md` ✅ entry + measurements; full gates; commit `docs(paths): storage locations shipped, measured`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.
