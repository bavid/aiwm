# LoRA Overview & Continue Training Implementation Plan (Plan 11)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-19-lora-lineage-design.md`.

**Goal:** See every self-trained LoRA with its full training history (lineage of runs, datasets, images, steps) and continue an existing LoRA on another dataset as a new version.

**Architecture:** `training_runs.init_lora_model_id` + `image_count` (migration 0020), `pretrained_lora_path` in the rendered ai-toolkit config guarded by preflight (file, family, rank), lineage computed on read via `models.source = training:<id>` links, `/training/loras` routes, Training-tab "Your LoRAs" section and a continue-prefilled new-run form.

**Tech Stack:** Rust (axum, sqlx/SQLite, serde_yaml_ng), Tauri 2, React 19 + TS.

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits (`git commit -- <paths>`), never `git add -A`, never bare `git stash`, never `--no-verify`; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build` (in `ui/`); never edit an existing migration; never guess numbers.

---

### Task 1: Lineage data + continue-from config

- [x] Migration `0020_training_run_lineage.sql`: `ALTER TABLE training_runs ADD COLUMN init_lora_model_id TEXT REFERENCES models(id) ON DELETE SET NULL; ALTER TABLE training_runs ADD COLUMN image_count INTEGER;` (`TrainingRun`/`NewTrainingRun` mirrors, repo setters, tests).
- [x] `StartRequest.init_lora_model_id: Option<String>` (+ `StartRunDto`, TS `StartRunBody`); `image_count` recorded at start from the export/preflight frame count.
- [x] `NetworkBlock.pretrained_lora_path: Option<String>` (`skip_serializing_if`), `linear_alpha = rank`; render test; `normalize_yaml_floats` invariant respected (string field single-line).
- [x] Preflight `check_init_lora`: model exists + file exists; is a LoRA (store subdir); `library_family(profile_family) == model.family`; rank from safetensors header == requested rank (helper `lora_rank_from_header` in `core/src/model/safetensors.rs`, tested with a synthetic header); source LoRA's run not running/paused. Each refusal a clear message; tests for each; mutation-check.
- [x] Commit `feat(training): continue a LoRA on another dataset (init_lora_model_id, pretrained_lora_path)` — `8112352`, alpha check `b2b863d`.

### Task 2: Overview API

- [x] `core/src/training/lineage.rs`: `list_loras(db)` → for each `models` row with `source LIKE 'training:%'` (plus imported LoRAs by store subdir, flagged `imported: true`): name, family, rank (header), size, created, `runs`, `total_steps`, `total_images`; `lineage(db, model_id)` → ordered chain of runs (walk `init_lora_model_id` → `models.source` → run; cycle-safe; a missing link ends the chain) with dataset name/source_root, image_count, captioner (from dataset), trigger, profile, preset, hyperparams, steps, duration, state, samples (existing scan). Tests: 3-deep chain, cycle, broken link, imported LoRA.
- [x] Routes `GET /training/loras`, `GET /training/loras/{model_id}` + Tauri + ipc + dev mock.
- [x] Commit `feat(training): LoRA overview and lineage API` — `e590a08`, header/samples off the async thread `9013d47`.

### Task 3: UI

- [x] Training tab "Your LoRAs" section (above runs): list with aggregates and the data-amount guidance line; selecting opens the history panel (runs oldest-first, samples, "Test in Image tab", "Continue with another dataset").
- [x] `NewRunForm`: "Start from" picker (none / a LoRA of the same family), rank pinned + explanation, the forgetting notice, `init_lora_model_id` sent only when set; server refusals shown.
- [x] `RunCard`: show dataset, preset, rank/lr/resolution, started/finished, "continued from <LoRA>".
- [x] Live-verify in the dev preview; commit `feat(ui): LoRA overview, history and continue training` — `a6169ac`.

### Task 4: Real run + docs

- [x] Back up DB. Real daemon: pick an existing library LoRA + its family's base model + a dataset export (create a small one from the Plan 6 flow if needed); start a continue-run with the fast preset and the smallest step count the preset allows; verify YAML has `pretrained_lora_path`, trainer log shows it loading the file, result is a new library model, overview shows the 2-run lineage; record time/VRAM. If no suitable pair is installed, record exactly what is missing (no pretending). — see "Measured" below.
- [x] `docs/TODO.md` ✅ entry with measurements; full gates; commit `docs(training): LoRA overview + continue training shipped, measured`.
- [x] Hand back: whole-branch review (approved) → controller gates (1361 lib tests, UI clean) → merge `--no-ff` → push.

## Measured 2026-09-19

Real `aiwm-cored` built from this worktree (`cargo run -p aiwm-core --bin aiwm-cored`,
50.95 s build) with `AIWM_DATA_DIR=E:\AI\data`; `aiwm.db` + `-wal` + `-shm` backed up to
the session scratchpad first (SHA-256 of the copy = original); migration 0020 "training run
lineage" applied live on start. RTX 4080 SUPER 16 GB, idle 965 MiB; no other daemon on
port 48160, no other run alive (`alive_run_id` null).

**What was on the machine.** Exactly one self-trained LoRA: `myrender-v2`
(`01a0af1d-7f5d…`, flux2, rank 16 from the safetensors header — 160 `lora_A/lora_B`
tensors, no alpha tensors, PEFT format), `source training:01a0af0a-ec62…` (600 steps, fast,
FLUX.2 [klein] 4B, finished 2026-09-17, 1,217 s). Base model with a training profile:
`FLUX.2 [klein] 4B base (training)` (`01a0aee0-1517…`, `base_installed: true`). Dataset
`train-material` (`01a0aee2-a8c3…`, 54 frame rows, 50 kept, no captions) whose recorded
`export_dir` `E:\AI\data\training\datasets\myrenders-v1` — like run 1's `work_dir` — no
longer existed on disk (cleaned away since 2026-09-17; `E:\AI\data\training` held only an
empty `datasets` folder). Re-exported with `POST /datasets/{id}/export` to that same
recorded path: `{"exported":50}` in 104 ms, 50 PNG + 50 TXT (trigger word only),
80,277,647 B; the dataset row's `export_dir` is unchanged. No second exported dataset
exists, so the continue-run trained on the same export — the mechanism under test does
not depend on which dataset it is.

| Step | Result |
|---|---|
| `GET /training/loras` before | `[{"model_id":"01a0af1d…","name":"myrender-v2","family":"flux2","trained":true,"rank":16,"size_bytes":46223656,"runs":1,"total_steps":600,"total_images":null}]` |
| `POST /training/runs` — `preset fast`, `hyperparams {steps: 50, rank: 16}` (50 = `STEPS_RANGE` minimum), one sample prompt, `init_lora_model_id` = myrender-v2 | HTTP 201 immediately; run `01a0bab4-546e-7682-ad40-075035d2a2d3`, `state running`, `pid 26588`, `init_lora_model_id` recorded, **`image_count: 50`** (= the export's media count); daemon log `continuing from a library LoRA run=01a0bab4… lora=E:\AI\models\image/loras\myrender-v2.safetensors` |
| `work_dir/config.yaml` | `network: type: lora`, `linear: 16`, `linear_alpha: 16`, **`pretrained_lora_path: E:\AI\models\image/loras\myrender-v2.safetensors`** (byte-identical to `models.file_path`), `folder_path: E:\AI\data\training\datasets\myrenders-v1`, `steps: 50`, `save_every: 200`, `sample_every: 200` |
| `train.log` — ai-toolkit loading the LoRA | L319 `create LoRA network. base dim (rank): 16, alpha: 16` · L322 `create LoRA for U-Net: 80 modules.` · **L324 `Using pretrained lora path from config: E:\AI\models\image/loras\myrender-v2.safetensors`** · L325 `#### IMPORTANT RESUMING FROM E:\AI\models\image/loras\myrender-v2.safetensors ####` · L326 `Loading from E:\AI\models\image/loras\myrender-v2.safetensors` · **L327 `Missing keys: []`** · L328 `Dataset: E:\AI\data\training\datasets\myrenders-v1` · L370 `-  Found 50 images` |
| Timeline (local time) | 19:26:15 POST → 19:26:39 first trainer log line (+24 s python/accelerate start) → 19:27:25 model quantized + LoRA loaded (+70 s) → baseline sample → 19:27:50 step 4 → 19:29:05 step 50 (`last_checkpoint_at`; ≈1.6 s/step) → final sample + save → 19:29:26 settled and imported: **190.7 s wall**, `duration_secs 190`. Run 1 (600 steps) had taken 1,217 s |
| VRAM (`nvidia-smi` every 5 s, whole card) | load 5,169 → 7,542 → 9,834 MiB; training flat at 11,729–11,731 MiB, 100 % util; **peak 11,915 MiB** (final sample + save); back to 963 MiB 5 s after exit |
| Loss | `last_loss 1.061`; 0.57–1.06 across the 50 steps — noise at this length, not a quality statement |
| Result model | `myrender-v3` `01a0bab7-3d74-70b1-9312-fd5eefc028b0`, flux2, `source training:01a0bab4…`, `E:\AI\models\image/loras\myrender-v3.safetensors`, 46,223,656 B (same size as v2: same rank, same 80 modules); `myrender-v2.safetensors` untouched (mtime 2026-09-17 13:24:58) |
| `GET /training/loras` after | 2 entries: `myrender-v3` **`runs 2`, `total_steps 650`**, `total_images null` (run 1 predates 0020 — no partial sum by design); `myrender-v2` still `runs 1`, `total_steps 600` |
| `GET /training/loras/01a0bab7…` | `runs` oldest-first: [1] `myrender-v2` — completed, 600/600, `duration_secs 1217`, dataset `train-material`, `image_count null`, `init_lora_name null`, `samples []` (its work folder is gone); [2] `myrender-v3` — completed, 50/50, `duration_secs 190`, `image_count 50`, `hyperparams {steps 50, rank 16}`, **`init_lora_name "myrender-v2"`**, `samples ["0"]`. `GET /training/loras/01a0af1d…` (v2) unchanged: 1 run |
| Work folder of run 2 | 93,536,565 B: `output/myrender-v3/myrender-v3.safetensors` 46,223,656, `optimizer.pt` 47,115,531, samples step 0 + step 50 (60,158 / 55,572 B) + thumbs, `config.yaml`, `train.log` 38,209 B |
| Refused — HTTP 400, run count stayed 1, no folder created | base model as `init_lora_model_id`: `"FLUX.2 [klein] 4B base (training)" is not a LoRA — only a LoRA from the library can be continued`; VAE: `"flux2-vae" is not a LoRA — only a LoRA from the library can be continued`; `rank 32`: `"myrender-v2" has rank 16 — set the rank to 16 to continue it (this run asked for 32)`; unknown id: `the LoRA you chose to continue is no longer in the library`; `steps 49`: `steps 49 is outside the allowed range 50..=20000` |
| Not provable live | family mismatch — the library holds no LoRA of another family; alpha ≠ rank — v2 has no alpha tensor. Both covered by unit tests only (`runner/init_lora.rs`) |

**Left in place (nothing deleted):** run `01a0bab4…` and its 93.5 MB work folder (the second
link of the lineage the user wants to see), `myrender-v3` in the library, and the re-created
export folder (the dataset row references it). Daemon stopped afterwards: no `aiwm-cored`
process, 0 connections on 48160, `GET /about` times out; no ai-toolkit python left.

**Observed, pre-existing, not Plan 11:** every trainer line appears twice in `train.log`
(stdout and stderr both captured); `GET /training/profiles` lists the LoRAs themselves and
the flux2 VAEs under the klein-4b profile's `trainable_models`.
