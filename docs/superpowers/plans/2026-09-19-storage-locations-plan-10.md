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

## Measured

Task 5 real run, 2026-09-19, real `aiwm-cored` built from this worktree (`0649387`), `AIWM_DATA_DIR=E:\AI\data`, port 48160 (desktop app not running). DB backed up to the session scratchpad first; migration 0019 (`dataset work dir`) applied live. Source `D:\Data\Test\20250111-_1.mp4`: 153,603,700 B, SHA-256 `15F22FD09A75574B4469073D3A0AC16072409CC7B5E43B0594225D939869BF28` before and after, mtime unchanged. `config.toml` untouched (hash identical); `data_dir` passed per request, `data_dir` = `<scratchpad>\plan10-frames` on `C:`.

| Step | Result |
|---|---|
| `GET /storage/locations` | outputs `E:\AI\data\outputs` 0 B / 0 files; datasets `E:\AI\data\outputs\datasets` 0 / 0; training `E:\AI\data\training` 0 / 0; models `E:\AI\models` 148,274,558,661 B / 116; runtimes `E:\AI\data\runtimes` 21,878,826,432 B / 199,116; cache `E:\AI\data\cache` 283,550 B / 60; downloads `E:\AI\data\.downloads` 0 / 0 (not configurable). All `exists: true`, `skipped: 0`. `volume_free_bytes`/`volume_total_bytes` **null for every row** (see below) |
| `POST /jobs` dataset_prep (`root: D:\Data\Test`, `mode: frames`, `captioner: null`, `data_dir` set) | HTTP 201; job `01a0ba52-1846-7c72-87bd-1f540c021543`; started 15:38:57.862Z, finished 15:43:54.473Z = **296.6 s** (extraction 21.8 s → "1368 still(s)", filter 271.5 s → "kept 40 of 1368"); no error events |
| Files land in the chosen folder | 1,368 files / 1,173,444,536 B under `<data_dir>\<job_id>\raw\Test\20250111-_1\frame_000001.png…`; `GET /datasets` row `01a0ba52-19cd…` has `work_dir` = that folder (legacy rows `null`); `E:\AI\data\outputs\datasets\` empty before and after, directory mtime unchanged (07:57:19) |
| Negative: `data_dir = D:\Data\Test` | 400 "frames cannot be stored inside the source folder D:\Data\Test — choose a folder outside it" |
| Negative: `data_dir = D:\Data` | 400 "the source folder D:\Data\Test lies inside D:\Data — choose a folder to store frames in that does not hold your source media" |
| Negative: `data_dir = E:\AI\models\x` | 400 "frames cannot be stored inside the model store E:\AI\models — choose another folder" |
| Negative: `data_dir = frames` | 400 "the folder to store frames in must be an absolute path, got \"frames\"" |
| Negative: `data_dir = <data_dir>\<job_id>` | 400 "frames cannot be stored in …\<job_id>: it lies inside its work folder …\<job_id> of dataset \"Test\" — choose another folder" |
| Negative: `POST /training/runs` with `data_dir = D:\Data\Test` | without `sample_prompts` the prompt check fires first (400 "add at least one sample prompt so you can see what the run learns"); with one prompt: 400 "the training run cannot be stored in D:\Data\Test: it lies inside its source folder D:\Data\Test of dataset \"Test\" — choose another folder" — the location check runs before any dataset/export/model lookup (ids were `none`) |
| Nothing created by negatives | job list: only the one prep job after 15:38Z; training runs unchanged (1 pre-existing) |
| `GET /datasets/{id}/usage` | `work_walkable: true`, `work_files: 1368`, `work_bytes: 1173444536` (identical to an independent `find` tally), `discarded_frames: 1328`, `discarded_bytes: 1139669340`, no export |
| `POST /datasets/{id}/cleanup` `{"dry_run":true}` | `frames: 1328, bytes: 1139669340, deleted_files: 0`; file count still 1,368 |
| `POST /datasets/{id}/cleanup` `{"dry_run":false}` | `deleted_files: 1328`, `skipped_files: []`, **434 ms**; folder now 40 files / 33,775,196 B; usage agrees (`discarded_frames: 0`) |
| `DELETE /datasets/{id}` | `frames: 40, deleted_files: 40, freed_bytes: 33775196, dataset_deleted: true`, **68 ms**; `<data_dir>\<job_id>` gone; `<data_dir>` and its `canary.txt` intact; `/datasets` no longer lists it |
| Teardown | source re-hashed (identical); daemon PID 23644 stopped, port 48160 free, no `aiwm-cored` process left; canary + empty custom folder removed |

**Pre-existing finding (not Plan 10 code, left as-is):** `cleanup::volume_free` (`core/src/cleanup/mod.rs`, since `25e99e4`) canonicalizes the path — on Windows that yields a `\\?\E:\…` verbatim prefix — then filters sysinfo disks with `target.starts_with(mount_point)` where the mount point is `E:\`; the prefix never matches, so free/total are `None` everywhere. Seen live: every `/storage/locations` row reports `null` free space, and the prep preflight logged `DEBUG could not determine free disk space for the dataset work folder` and let the run proceed (fail-open, as designed for the unknown case). Fix candidate for a follow-up: strip the verbatim prefix (or use `std::path::absolute`) plus a Windows test.
