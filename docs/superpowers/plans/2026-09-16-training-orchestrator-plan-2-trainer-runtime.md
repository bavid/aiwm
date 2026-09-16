# Training Orchestrator — Plan 2: Trainer Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A LoRA training run can be started from a curated dataset inside AIWM, runs as a detached ostris/ai-toolkit process that survives app restarts, reports progress file-based, can be paused/resumed/cancelled, and imports its result LoRA into the model library.

**Architecture:** `core::training` (profiles, YAML rendering, log/work-dir progress, run repo + state machine, runner/poller) + `core::runtime::training::TrainingAdapter` (isolated `uv` venv for the pinned ai-toolkit commit, import probe, `RuntimeAdapter` impl whose single pinned "loaded model" is the GPU reservation) + a `training_runs` table (migration 0016, separate from `jobs`) + API/Tauri/ipc + a Training tab. Processes are launched with a new quiet variant of `launcher::spawn::launch_detached`; progress comes from `train.log` (tqdm lines split on `\r` and `\n`) and from checkpoint/sample files; a PID check turns a vanished process into `interrupted`, never `failed`.

**Tech Stack:** Rust (tokio, sqlx/SQLite, axum, serde_yaml — add if absent), ostris/ai-toolkit pinned at commit `e65c4d0fb69251e692390574c49873297dc4bae5` (MIT), `uv`, Python 3.12 venv, torch 2.13.0 cu130; React 19/TS UI.

**Spec:** `docs/superpowers/specs/2026-09-16-training-orchestrator-design.md` (Abschnitte 1, 2, 4, 5). Verified facts: memory note `aiwm-plan2-trainer-research` (also appended as the appendix of this plan).

**Hard rules (unchanged):** TDD with observed RED; `cargo fmt/clippy -D warnings/test --workspace`, sidecar `ruff`+`pytest` (`uv run --no-sync`), `pnpm typecheck/lint/build` clean before every commit; never `--no-verify`; zero `unsafe`; no `unwrap`/`expect` in production code; every SHA-256 in the catalog/installer is computed from a file actually downloaded in that task; loopback-only, offline-gated (`app.offline()`); no `git stash`.

---

## File structure

- `core/migrations/0016_training_runs.sql` — table + indexes.
- `core/src/db/training_runs.rs` — `RunState`, `TrainingRun`, `NewTrainingRun`, `TrainingRunRepo`.
- `core/src/training/mod.rs` — module root, `TRAINING_MODEL_ID`, `training_err`.
- `core/src/training/profile.rs` — `TrainingProfile` registry, presets, fit.
- `core/src/training/config.rs` — ai-toolkit YAML rendering from profile + run.
- `core/src/training/progress.rs` — tqdm line parser, work-dir scan (checkpoints/samples), `Progress`.
- `core/src/training/process.rs` — PID liveness, process-tree kill (Windows `taskkill /T`), quiet detached launch (delegates to `launcher::spawn`).
- `core/src/training/runner.rs` — start/pause/resume/cancel, poller, recover-on-start, result import.
- `core/src/runtime/training/{mod.rs,install.rs}` — `TrainingAdapter` (installer, probe, `RuntimeAdapter`).
- `core/src/launcher/spawn.rs` — `launch_detached_quiet` (no console, stdout/err to a file).
- `core/tests/training_run.rs` + `core/src/bin/aiwm-fake-trainer.rs` (or `fixtures/` crate, whichever pattern `aiwm-fake-comfy` uses — check `core/Cargo.toml` `[[bin]]`).
- `core/src/api/{dto.rs,handlers.rs,http.rs}`, `src-tauri/src/lib.rs`, `ui/src/lib/{ipc.ts,hooks.ts,dev-mock.ts}`, `ui/src/features/training/{Training.tsx,training.css,RunCard.tsx,NewRunForm.tsx}`, `ui/src/features/dataset/ExportCard.tsx` ("LoRA trainieren" entry), `ui/src/App.tsx` (tab), `docs/TODO.md`.

---

### Task 1: `training_runs` table, `RunState` machine, `TrainingRunRepo`

**Files:** Create `core/migrations/0016_training_runs.sql`, `core/src/db/training_runs.rs`; modify `core/src/db/mod.rs`.

- [ ] **Step 1: Migration**

```sql
-- 0016_training_runs.sql
-- A training run is NOT a job: it is a detached ai-toolkit process that must
-- survive app restarts (JobEngine::recover marks running jobs failed). State,
-- progress and the PID live here; the process writes checkpoints/samples/log
-- into work_dir, which is the source of truth for progress.
CREATE TABLE training_runs (
    id                 TEXT PRIMARY KEY,                       -- uuid v7
    name               TEXT NOT NULL,
    profile_family     TEXT NOT NULL,                          -- TrainingProfile.family
    target_model_id    TEXT REFERENCES models(id) ON DELETE SET NULL,
    dataset_id         TEXT REFERENCES datasets(id) ON DELETE SET NULL,
    data_kind          TEXT NOT NULL CHECK (data_kind IN ('frames', 'clips')),
    trigger_word       TEXT NOT NULL DEFAULT '',
    preset             TEXT NOT NULL CHECK (preset IN ('fast', 'balanced', 'thorough')),
    hyperparams_json   TEXT NOT NULL DEFAULT '{}',             -- rank/lr/resolution/steps overrides
    sample_prompts_json TEXT NOT NULL DEFAULT '[]',
    state              TEXT NOT NULL CHECK (state IN ('preparing','running','paused','interrupted','resuming','finishing','completed','failed','cancelled')),
    step               INTEGER NOT NULL DEFAULT 0,
    total_steps        INTEGER NOT NULL DEFAULT 0,
    last_loss          REAL,
    last_checkpoint_at TEXT,
    pid                INTEGER,
    work_dir           TEXT NOT NULL,
    result_model_id    TEXT REFERENCES models(id) ON DELETE SET NULL,
    error_text         TEXT,
    created_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    started_at         TEXT,
    finished_at        TEXT
) STRICT;
CREATE INDEX idx_training_runs_state ON training_runs(state);
CREATE INDEX idx_training_runs_dataset ON training_runs(dataset_id);
```

- [ ] **Step 2: Failing tests** (in `training_runs.rs` `mod tests`, using `Database::connect_in_memory()`):
  - `run_state_round_trips_and_rejects_unknown`
  - `only_allowed_transitions_pass` — `preparing→running`, `running→paused|interrupted|finishing|failed|cancelled`, `paused→resuming|cancelled`, `interrupted→resuming|cancelled`, `resuming→running|failed|cancelled`, `finishing→completed|failed`; everything else `Err(CoreError::Config)`; terminal states go nowhere (mirror `orchestrator::state::JobState::can_transition_to`).
  - `create_get_list_and_progress_updates` — `create(NewTrainingRun{..})` → state `preparing`; `set_state(id, Running)` sets `started_at`; `set_progress(id, step, total, loss)`; `set_pid(id, Some(4242))`; `set_result(id, model_id)`; `set_error(id, "…")` + `Failed` sets `finished_at`; `list()` newest first; `list_alive()` = `running|resuming`.
  - `deleting_a_dataset_keeps_the_run_with_a_null_dataset_id`.
- [ ] **Step 3: Implement** `RunState` (`as_str`/`parse`/`can_transition_to`/`ensure_transition`/`is_terminal`), `TrainingRun` (all columns, `Option`s for nullable), `NewTrainingRun { name, profile_family, target_model_id: Option<String>, dataset_id: Option<String>, data_kind: DatasetMode, trigger_word, preset: Preset, hyperparams_json, sample_prompts_json, work_dir }`, `Preset { Fast, Balanced, Thorough }` (`as_str`/`parse`), `TrainingRunRepo` (`create, get, list, list_alive, set_state (validates transition), set_progress, set_pid, set_checkpoint_at, set_result, set_error`). Re-export from `db/mod.rs` with a `training_runs()` accessor.
- [ ] **Step 4:** gates; commit `feat(db): training_runs table, run state machine and repo`.

---

### Task 2: Profile registry

**Files:** Create `core/src/training/mod.rs`, `core/src/training/profile.rs`; modify `core/src/lib.rs` (`pub mod training;`).

- [ ] **Step 1: Failing tests**: `every_profile_has_a_unique_family_and_arch`; `presets_scale_steps_monotonically` (fast < balanced < thorough for every profile); `find_for_family_matches_library_family_strings` (`"flux2-klein-4b"` → `flux2_klein_4b`, unknown → None); `video_profiles_accept_clips_and_image_profiles_do_not`; `fit_labels_are_plain_text` (`Comfortable`/`AtTheEdge` `as_str` = "passt bequem"/"am Limit" — check the UI language convention: the app's user-facing strings are English; use "fits comfortably"/"at the edge").
- [ ] **Step 2: Implement**

```rust
pub enum Fit { Comfortable, AtTheEdge }
pub enum DataKind { Frames, Clips, Both }
pub struct VramStrategy { pub quantize: bool, pub qtype: &'static str, pub quantize_te: bool, pub low_vram: bool, pub layer_offloading: bool, pub fit: Fit, pub reserve_mb: u64 }
pub struct PresetValues { pub steps: u32, pub lr: f64, pub rank: u32, pub resolution: u32, pub save_every: u32, pub sample_every: u32 }
pub struct BaseWeight { pub repo: &'static str, pub local_dir_role: &'static str, pub required_files: &'static [&'static str] }
pub struct TrainingProfile {
    pub family: &'static str, pub label: &'static str, pub arch: &'static str, pub data_kind: DataKind,
    pub vram: VramStrategy, pub base: BaseWeight, pub caption_order: crate::capability::dataset::CaptionOrder,
    pub fast: PresetValues, pub balanced: PresetValues, pub thorough: PresetValues, pub license_note: &'static str,
}
pub const PROFILES: &[TrainingProfile] = &[ /* flux2_klein_4b (Comfortable, quantize+qfloat8, reserve 12288), flux2_klein_9b (AtTheEdge, quantize+low_vram+layer_offloading, reserve 15000), sdxl (Comfortable, no quantize, reserve 10240), wan22_5b (AtTheEdge, Both, reserve 15000) */ ];
pub fn find_for_family(family: &str) -> Option<&'static TrainingProfile>;
pub fn preset_values(p: &TrainingProfile, preset: crate::db::Preset) -> PresetValues;
```

Base weights are **directory models** in the library, one role per profile (`training_base_flux2_klein_4b` etc.); `required_files` for 4B = `["transformer/diffusion_pytorch_model.safetensors","text_encoder/model-00001-of-00002.safetensors","text_encoder/model-00002-of-00002.safetensors","vae/diffusion_pytorch_model.safetensors","model_index.json"]`. Preset numbers (starting points, calibrated in Task 11): 4B fast 600 steps/lr 1e-4/rank 16/res 768, balanced 1500/1e-4/16/1024, thorough 3000/8e-5/32/1024; 9B same steps, rank 16, res 768/1024/1024; SDXL fast 800/1e-4/rank 16/1024; wan22_5b fast 500/1e-4/16/res 512 (clips). Add `ModelKind::default_role` is NOT needed — roles are strings on directory models.
- [ ] **Step 3:** gates; commit `feat(training): profile registry with presets, VRAM strategy and base weights`.

---

### Task 3: YAML rendering

**Files:** Create `core/src/training/config.rs`. Add `serde_yaml` (check `core/Cargo.toml`; if a YAML crate is already present use it) — rendering via a typed struct → `serde_yaml::to_string`, never string concatenation.

- [ ] **Step 1: Failing tests**: `renders_the_exact_yaml_for_a_flux2_klein_4b_frames_run` (assert full string equality against a literal built from the shape in the appendix: `job: extension`, `config.name = run.name`, `process[0].type = sd_trainer`, `training_folder = <work_dir>/output`, `device = cuda:0`, `trigger_word`, `network {type: lora, linear: rank, linear_alpha: rank}`, `save {dtype: float16, save_every, max_step_saves_to_keep: 4}`, `datasets[0] {folder_path: <export_dir>, caption_ext: txt, caption_dropout_rate: 0.05, shuffle_tokens: false, cache_latents_to_disk: true, resolution: [res]}`, `train {batch_size: 1, steps, gradient_accumulation_steps: 1, train_unet: true, train_text_encoder: false, gradient_checkpointing: true, noise_scheduler: flowmatch, optimizer: adamw8bit, lr, ema_config {use_ema: true, ema_decay: 0.99}, dtype: bf16}`, `model {name_or_path: <base dir>, arch: flux2_klein_4b, quantize: true, qtype: qfloat8, quantize_te: true, low_vram: false}`, `sample {sampler: flowmatch, sample_every, sample_start_step: 0, width, height, prompts: [..], neg: "", seed: 42, walk_seed: true, guidance_scale: 4, sample_steps: 20}`, `meta {name: "[name]", version: "1.0"}`); `clips_runs_set_num_frames_and_video_dataset_keys` (wan: `datasets[0].num_frames`, `do_i2v: true`, `is_video`-related keys per the appendix); `windows_paths_are_escaped_for_yaml` (backslashes preserved via serde quoting); `sdxl_uses_ddpm_and_no_quantize`; `hyperparam_overrides_win_over_presets`.
- [ ] **Step 2: Implement** `pub struct RenderInput<'a> { profile, run: &TrainingRun, preset: PresetValues, base_dir: &Path, dataset_dir: &Path, work_dir: &Path, prompts: &[String] }` + `pub fn render_yaml(input: &RenderInput) -> Result<String>`; `pub fn config_path(work_dir) -> PathBuf` (`<work_dir>/config.yaml`).
- [ ] **Step 3:** gates; commit `feat(training): ai-toolkit config rendering`.

---

### Task 4: Progress — log parser and work-dir scan

**Files:** Create `core/src/training/progress.rs`.

- [ ] **Step 1: Failing tests** with the recorded line shapes from the appendix:
  - `parses_a_tqdm_progress_line` — `"my_lora:  12%|██        | 240/2000 [01:02<07:35,  3.86it/s, lr: 1.0e-04 loss: 3.123e-01]"` → `Some(Progress { step: 240, total: 2000, loss: Some(0.3123), lr: Some(1.0e-4), eta_secs: Some(455) })`.
  - `splits_carriage_return_updates_and_keeps_the_last` — one buffer with three `\r`-separated bars → last step.
  - `detects_resume_oom_and_completion_markers` — `#### IMPORTANT RESUMING FROM x ####` → `Marker::Resuming`, `# OOM during training step, skipping batch 2/3 #` → `Marker::Oom(2)`, `OOM during training step 3 times in a row, aborting training` → `Marker::OomAbort`, `Result:` + ` - 1 completed job` → `Marker::Completed`, `Error running job: <msg>` → `Marker::Error(msg)`, `Job stopped` → `Marker::Stopped`.
  - `scans_checkpoints_and_samples` — temp dir with `my_lora_000000250.safetensors`, `my_lora_000000500.safetensors`, `samples/1700000000_000000500_0.png` → `latest_checkpoint = (500, path)`, `latest_samples = [path]`.
- [ ] **Step 2: Implement** `pub struct Progress {..}`, `pub enum Marker {..}`, `pub fn parse_line(line: &str) -> Option<Progress>`, `pub fn parse_marker(line: &str) -> Option<Marker>`, `pub fn split_updates(buf: &str) -> impl Iterator<Item=&str>` (split on `\r` and `\n`, skip empty), `pub async fn tail_log(path: &Path, from_offset: u64) -> Result<(Vec<String>, u64)>` (read new bytes only, lossy UTF-8), `pub fn scan_work_dir(work_dir: &Path, run_name: &str) -> Result<WorkDirState { latest_checkpoint: Option<(u32, PathBuf)>, latest_samples: Vec<PathBuf> }>`. Step numbers via the `_{step:09}` suffix.
- [ ] **Step 3:** gates; commit `feat(training): tqdm log parser, markers and work-dir scan`.

---

### Task 5: Process control — quiet detached launch, liveness, tree kill

**Files:** Modify `core/src/launcher/spawn.rs`; create `core/src/training/process.rs`.

- [ ] **Step 1: Failing tests**: `quiet_spec_redirects_output_to_the_log_file` (pure builder test on the `cmd /C` line: `"<program>" args > "<log>" 2>&1`), `pid_of_a_spawned_sleep_is_alive_then_dead` (spawn `cmd /C timeout /T 30`-style child via the quiet launcher returning the PID; `is_alive(pid)` true; `kill_tree(pid)`; false — Windows-only test, gate with `#[cfg(windows)]`), `kill_tree_args_use_taskkill_with_tree_and_force`.
- [ ] **Step 2: Implement** in `spawn.rs`: `pub async fn launch_detached_quiet(spec: &SpawnSpec, log_path: &Path) -> Result<u32>` — `cmd /C` (not `/K`), `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS` (Windows; on other OSes `setsid` semantics via `process_group(0)`), stdout/stderr redirected to `log_path`, returns the child PID (the `cmd` wrapper's PID; the tree kill covers children). In `process.rs`: `pub async fn is_alive(pid: u32) -> bool` (`tasklist /FI "PID eq <pid>" /NH` contains the pid; non-Windows: `kill -0`), `pub async fn kill_tree(pid: u32) -> Result<()>` (`taskkill /PID <pid> /T /F`; leaves-first is what `/T` does), `pub fn write_pid_file(work_dir, pid)` / `read_pid_file`.
- [ ] **Step 3:** gates; commit `feat(launcher): quiet detached launch with log redirection; process liveness and tree kill`.

---

### Task 6: `TrainingAdapter` — installer, import probe, `RuntimeAdapter`

**Files:** Create `core/src/runtime/training/mod.rs`, `core/src/runtime/training/install.rs`; modify `core/src/runtime/mod.rs` (`RuntimeKind::Training`, `pub mod training`), `core/src/app.rs` (register + expose `app.training`).

- [ ] **Step 1: Compute the real archive hash** (never type one): in the scratchpad `curl -sSL -o ai-toolkit-e65c4d0.zip https://github.com/ostris/ai-toolkit/archive/e65c4d0fb69251e692390574c49873297dc4bae5.zip && sha256sum ai-toolkit-e65c4d0.zip && stat -c %s ai-toolkit-e65c4d0.zip`; paste into `PINNED_ARCHIVE: Archive`.
- [ ] **Step 2: Failing tests** (mirror `runtime/comfyui/install.rs`'s tests with the local-server `FetchSpec` substitution and a recording `CmdRunner`): `install_is_idempotent_via_markers`, `install_runs_uv_python_install_venv_torch_and_requirements_in_order` (recorded args: `uv python install 3.12`, `uv venv --python 3.12 <venv>`, `uv pip install --python <py> torch==2.13.0 torchvision==0.28.0 torchaudio==2.11.0 --index-url https://download.pytorch.org/whl/cu130`, `uv pip install --python <py> -r requirements.txt`), `probe_marks_env_broken_when_torch_import_fails` (fake runner returns non-zero for `python -c "import torch; print(torch.cuda.is_available(), torch.cuda.get_device_properties(0).total_memory)"`), `adapter_reports_one_pinned_loaded_model_while_a_run_is_alive`.
- [ ] **Step 3: Implement** `TrainingAdapter { runtimes_dir, state: Mutex<InstallState>, alive_run: Mutex<Option<(String /*run id*/, u64 /*vram_mb*/)>> }`: `install(offline)`, `is_installed()`, `is_installing()`, `install_state()` (same shape as ComfyUI's for the UI), `probe() -> Result<Probe { cuda: bool, vram_total_mb: u64, torch: String }>` + `env_broken` flag, `python_bin()`, `run_py()`; `impl RuntimeAdapter` (`id "training"`, `kind Training`, `spawn_spec None`, `health` = Healthy when installed & probe ok, `load_model`/`unload_model` = set/clear `alive_run`, `loaded_models` = `[LoadedModel { model_id: TRAINING_MODEL_ID, vram_mb }]` when alive). `App::load` registers it and calls `scheduler.pin(TRAINING_MODEL_ID)` when a run is alive (Task 7 recovery does the pin/unpin).
- [ ] **Step 4:** gates; commit `feat(runtime): TrainingAdapter — pinned ai-toolkit installer, import probe, GPU reservation`.

---

### Task 7: Runner — start / poll / pause / resume / cancel / recover / import

**Files:** Create `core/src/training/runner.rs`; modify `core/src/training/mod.rs`, `core/src/app.rs` (spawn the poller + recovery at startup, after the engine's own recover).

- [ ] **Step 1: Failing tests** (no GPU; use the `aiwm-fake-trainer` from Task 8 where a process is needed — write Task 8's fixture first if it makes these tests simpler; otherwise unit-test the pure parts): `start_writes_config_and_log_paths_and_moves_to_running`, `poll_updates_step_and_loss_from_the_log`, `poll_marks_interrupted_when_the_pid_is_gone_without_completion`, `completion_imports_the_latest_checkpoint_as_a_lora` (`ModelRepo` row: kind lora via `roles`/format `safetensors`, `family = profile.family`, `source = "training:<run_id>"`, `name = run.name`; `result_model_id` set; reservation released), `cancel_kills_the_tree_and_keeps_the_work_dir`, `resume_from_interrupted_relaunches_with_the_same_config_name`.
- [ ] **Step 2: Implement** `pub struct Runner { db, adapter: Arc<TrainingAdapter>, scheduler: Arc<dyn Scheduler>, engine hooks for GPU release }`: `start(run_id)`: preflight (installed + probe ok, base dir has `required_files`, dataset export dir exists with ≥ 1 pair, disk ≥ 20 GB free on `work_dir`'s drive, `min 1 prompt`), GPU release (unload chat model via the llama adapter, ComfyUI free-memory call if exposed), render YAML, `launch_detached_quiet` with `python run.py config.yaml -l train.log`, store pid, `adapter.load_model(TRAINING_MODEL_ID, profile.vram.reserve_mb)` + `scheduler.pin`, state `running`. `poll_once(run)`: tail log → progress/markers; scan work dir → `last_checkpoint_at`; PID dead → `Completed` marker seen ⇒ `finishing` → import → `completed`; `Error`/`OomAbort` marker ⇒ `failed` with text; else ⇒ `interrupted`. `pause` = kill tree → `paused`; `resume` = relaunch same config (ai-toolkit auto-resumes from the latest checkpoint) → `resuming` → `running` on first progress line; `cancel` = kill tree → `cancelled`, unpin/unload. `recover()` at startup: every `running|resuming` row → PID alive ⇒ re-pin and re-attach; else `interrupted`. Poller: `tokio::spawn` loop every 3 s over `list_alive()`.
- [ ] **Step 3:** gates; commit `feat(training): runner with detached start, file-based polling, pause/resume/cancel, recovery and result import`.

---

### Task 8: `aiwm-fake-trainer` fixture + lifecycle integration test

**Files:** Create the fixture binary following `aiwm-fake-comfy`'s location/pattern; create `core/tests/training_run.rs`.

- [ ] **Step 1: Fixture**: reads the YAML (`config.name`, `train.steps`, `save.save_every`, `training_folder`), prints a `Running 1 job` line, then every 100 ms one tqdm-shaped line (`\r`-terminated, exact appendix format) with rising step, writes `<name>_<step:09>.safetensors` every `save_every` steps and a sample PNG under `samples/`, resumes from the highest existing checkpoint (prints the `#### IMPORTANT RESUMING FROM … ####` line), exits 0 after printing the completion block; on `--oom-at N` prints the OOM abort marker and exits 1; ignores nothing on SIGTERM (Windows: taskkill kills it).
- [ ] **Step 2: Integration test** (App::load with `AppPaths::rooted`, `TrainingAdapter` pointed at the fixture via an env override `AIWM_TRAINER_CMD` — add that seam): create dataset + two exported pairs, create a run, `start` → poll until step > 0 → simulate "app closed" by dropping the poller → process still alive → `recover()` re-attaches → `kill_tree` → next poll ⇒ `interrupted` → `resume` → completes → `result_model_id` points at an imported LoRA row; meanwhile an image job submitted during `running` is `Blocked` with the pinned-model reason.
- [ ] **Step 3:** gates; commit `test(training): fake trainer fixture and full lifecycle integration test`.

---

### Task 9: API — DTOs, handlers, routes, Tauri, ipc, hooks, dev-mock

**Files:** `core/src/api/{dto.rs,handlers.rs,http.rs}`, `src-tauri/src/lib.rs`, `ui/src/lib/{ipc.ts,hooks.ts,dev-mock.ts}`.

- [ ] Routes: `GET /training/profiles` (profiles + per-profile `installed_base: bool` + library models trainable with them), `GET /training/status` (installed/installing/env_broken/probe), `POST /training/install`, `GET /training/runs`, `POST /training/runs` (create+start; body `{ name, target_model_id, dataset_id, trigger_word, preset, hyperparams, sample_prompts }` → 201), `GET /training/runs/{id}` (+ `latest_samples`, `log_tail`), `POST /training/runs/{id}/pause|resume|cancel`, `DELETE /training/runs/{id}` (only terminal; removes the row, keeps work_dir unless `?purge=true`), `GET /training/runs/{id}/samples/{n}` (image bytes, loopback). Tauri commands mirror them; `ipc.ts` types `TrainingProfile`, `TrainingRun`, `TrainerStatus`; hooks `useTrainerStatus (5 s)`, `useTrainingRuns (3 s)`, `useTrainingRun(id) (2 s)`, `useTrainingProfiles`; dev-mock simulates progress (step += 25 per tick, a checkpoint every 250, `interrupted` toggle via a hidden `__mock_interrupt` command for the UI check).
- [ ] Integration test `core/tests/training_api.rs`: create → 201, list, pause/resume transitions, cancel, 404s, install gate on offline → 400.
- [ ] Gates; commit `feat(api): training profiles, runs and trainer status over HTTP + Tauri`.

---

### Task 10: Training tab + Dataset-tab entry + "Test now"

**Files:** `ui/src/features/training/*`, `ui/src/features/dataset/ExportCard.tsx`, `ui/src/App.tsx`, `ui/src/features/image/Image.tsx` (accept `?lora=<id>&prompt=<trigger>` prefill via the existing navigation state pattern — check how Story Studio hands a prompt to Image).

- [ ] `NewRunForm`: target model (library filtered to families with a profile; fit label in plain text; profile-less greyed with "not trainable yet (family X)"), name + trigger (whitespace stripped, ≤ 30 chars, `tokenWarning`), preset radio + collapsible fine-tuning (rank/lr/resolution/steps), 1–3 sample prompts prefilled with the trigger; blocking preflight panel: trainer installed (else Install with download size), base weights present (else the exact `hf download … --local-dir <store>/training/<family>` command + "Register folder" like Colibri), disk, GPU release notice.
- [ ] `Training.tsx`: run list; per run progress bar, loss sparkline (last 200 points from the run's `loss_history` — add to the run detail DTO from the log tail), ETA, latest sample per prompt, collapsible log tail, Pause/Resume/Cancel; `interrupted` shown as "interrupted — resume?"; completed runs link to the imported LoRA + **Test now**.
- [ ] Dataset tab `ExportCard`: **Train LoRA** button after a successful export → opens the Training tab with the dataset preselected.
- [ ] Live verify against dev-mock (form, progress, interrupted → resume, test-now navigation); `pnpm typecheck/lint/build`; commit `feat(ui): Training tab, run form with preflight, dataset-tab entry and test-now`.

---

### Task 11: Base weights bridge, real 4B run, calibration, docs

**Files:** `core/src/model/catalog.rs` (a `TRAINING_BASES` list: repo, exclude patterns, expected files + sizes; hashes ONLY after the real download), `docs/TODO.md`, `docs/superpowers/specs/...` (calibration appendix).

- [ ] **Step 1:** download the 4B base once with the `hf` CLI into `<store>/training/flux2-klein-4b/` (exclude `*.jpg` and the single-file duplicate; ≈ 15.7 GB), compute per-file SHA-256 locally, cross-check against the HF API values in the appendix, register as a directory model with role `training_base_flux2_klein_4b`; pin the computed hashes in `TRAINING_BASES` and add a `verify_base_dir(dir, base) -> Result<()>` used by the preflight (size + hash spot-check of the largest file only, full check on demand).
- [ ] **Step 2: Real run** — FLUX.2 klein 4B, a small frames dataset from real material (user's anime frames if present, else the repo's own test PNGs are NOT enough — use ≥ 20 real images from `E:\Data` chosen by the user's folder structure; if none is available, mark the step as NEEDS_USER and stop), preset Fast; record VRAM peak (NVML via the app's telemetry), s/step, total time, the exact first three tqdm lines (pin one into Task 4's parser test), sample images checked; use the LoRA once in the Image tab. Then the 9B attempt with `low_vram + layer_offloading`; record the honest outcome (works / OOM) into the 9B profile's `license_note`/fit text.
- [ ] **Step 3:** `docs/TODO.md` "Teilsystem 2 — Trainings-Orchestrator ✅" block in the existing style + leftovers (Wan clips profile unverified, JoyCaption, Diagnostics GPU row); full gates; commit `feat(training): base-weight bridge; docs: record the trainer runtime and the first real 4B run`.
- [ ] **Step 4:** finishing-a-development-branch: final review, re-verify gates, merge `--no-ff`, push.

---

## Appendix — verified facts (2026-09-16, primary sources)

- ai-toolkit main commit `e65c4d0fb69251e692390574c49873297dc4bae5` (2026-09-16T19:46:39Z); archive `https://github.com/ostris/ai-toolkit/archive/<sha>.zip` (hash to be computed in Task 6).
- Install: python ≥ 3.10 (3.12 recommended); `torch==2.13.0 torchvision==0.28.0 torchaudio==2.11.0 --index-url https://download.pytorch.org/whl/cu130`; `pip install -r requirements.txt`. Headless: `python run.py <config.yaml> [-l <logfile>] [-n <name>] [-r]`.
- Arch ids/defaults (ui/src/app/jobs/new/options.tsx): `flux2_klein_4b` → `black-forest-labs/FLUX.2-klein-base-4B`, `flux2_klein_9b` → `…-9B` (gated page but weights not gated for 4B), `wan22_5b` → `Wan-AI/Wan2.2-TI2V-5B-Diffusers` (num_frames 121, fps 24, 768×1024, do_i2v), `sdxl` → `stabilityai/stable-diffusion-xl-base-1.0` (ddpm, guidance 6, no quantize); quantize/quantize_te/low_vram toggles, qtype `qfloat8`, sampler/noise_scheduler `flowmatch`, timestep_type weighted/sigmoid, layer_offloading.
- Output layout: `<training_folder>/<name>/<name>_<step:09>.safetensors` (keeps `max_step_saves_to_keep` newest), `samples/<time>_<step:09>_<count>.<ext>`; auto-resume by re-running the same config name (`#### IMPORTANT RESUMING FROM <path> ####`, `Found step N in metadata, starting from there`).
- Progress: `ToolkitProgressBar(total=steps, desc=job.name, leave=True, initial=step_num)`; `set_postfix_str(f"lr: {lr:.1e} loss: {loss:.3e}")` (loss_dict key `loss`). tqdm bar format: `<desc>: <pct>%|<bar>| <n>/<total> [<elapsed><<remaining>, <rate>, <postfix>]`, `\r`-updated.
- OOM: 1–3 consecutive → `# OOM during training step, skipping batch k/3 #`; >3 → `RuntimeError("OOM during training step 3 times in a row, aborting training")` → `Error running job: …`, non-zero exit. KeyboardInterrupt → `Job stopped`, exit 0, NO checkpoint written on interrupt. Success → `Result:` / ` - 1 completed job`. Checkpoint lines: `Saved checkpoint to <path>`.
- FLUX.2 klein base 4B (Apache-2.0, not gated): transformer 7,751,109,744 B (HF sha256 e109674697ffa1a3983126e32512f5428a9442bd8df59f9c95566ee90a473bb6), text_encoder shards 4,967,215,360 (8c0506e7f4936fa7e26183a4fd8da4e2bdbc5990ba64ae441f965d51228f36ea) + 3,077,766,632 (82f2bd839378541b0557bfabaf37c7d3d637071fdcb73302dedd7cf61162ce07), vae 168,120,878 (ca70d2202afe6415bdbcb8793ba8cd99fd159cfe6192381504d6c4d3036e0f04), tokenizer.json 11,422,654 (aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4); exclude `flux-2-klein-base-4b.safetensors` (single-file duplicate) and `*.jpg`. `hf` CLI 1.31.0 is installed on the machine; E: has ~747 GB free.
- AIWM hooks: `launcher::spawn::launch_detached` (cmd /K + CREATE_NEW_CONSOLE), `runtime::download::{download_verified, ensure_uv, extract_zip}` (`pub(crate)`), `CmdRunner` trait in `runtime/download.rs:202`, ComfyUI installer pattern in `runtime/comfyui/install.rs` (`Archive`, `FetchSpec`, `InstallPhase`, `__init__.py` markers), `HybridScheduler` reads every registered runtime's `loaded_models()` and never evicts pinned ids (`scheduler.pin`), `App::load` registers adapters via `runtimes.register(Arc<dyn RuntimeAdapter>)`, `model::directory::register_directory_model(db, dir, name, format, roles, ram_estimate_mb)`, `NewModel` fields (publisher, name, family, format, quant, arch, param_count, file_path, sha256, size_bytes, ctx_max, vram_estimate_mb, ram_estimate_mb, source, source_revision, n_layers, n_embd, n_heads, n_kv_heads, roles), migration 0015 = datasets ⇒ 0016 = training_runs.
