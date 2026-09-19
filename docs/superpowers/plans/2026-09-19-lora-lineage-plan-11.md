# LoRA Overview & Continue Training Implementation Plan (Plan 11)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-19-lora-lineage-design.md`.

**Goal:** See every self-trained LoRA with its full training history (lineage of runs, datasets, images, steps) and continue an existing LoRA on another dataset as a new version.

**Architecture:** `training_runs.init_lora_model_id` + `image_count` (migration 0020), `pretrained_lora_path` in the rendered ai-toolkit config guarded by preflight (file, family, rank), lineage computed on read via `models.source = training:<id>` links, `/training/loras` routes, Training-tab "Your LoRAs" section and a continue-prefilled new-run form.

**Tech Stack:** Rust (axum, sqlx/SQLite, serde_yaml_ng), Tauri 2, React 19 + TS.

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits (`git commit -- <paths>`), never `git add -A`, never bare `git stash`, never `--no-verify`; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build` (in `ui/`); never edit an existing migration; never guess numbers.

---

### Task 1: Lineage data + continue-from config

- [ ] Migration `0020_training_run_lineage.sql`: `ALTER TABLE training_runs ADD COLUMN init_lora_model_id TEXT REFERENCES models(id) ON DELETE SET NULL; ALTER TABLE training_runs ADD COLUMN image_count INTEGER;` (`TrainingRun`/`NewTrainingRun` mirrors, repo setters, tests).
- [ ] `StartRequest.init_lora_model_id: Option<String>` (+ `StartRunDto`, TS `StartRunBody`); `image_count` recorded at start from the export/preflight frame count.
- [ ] `NetworkBlock.pretrained_lora_path: Option<String>` (`skip_serializing_if`), `linear_alpha = rank`; render test; `normalize_yaml_floats` invariant respected (string field single-line).
- [ ] Preflight `check_init_lora`: model exists + file exists; is a LoRA (store subdir); `library_family(profile_family) == model.family`; rank from safetensors header == requested rank (helper `lora_rank_from_header` in `core/src/model/safetensors.rs`, tested with a synthetic header); source LoRA's run not running/paused. Each refusal a clear message; tests for each; mutation-check.
- [ ] Commit `feat(training): continue a LoRA on another dataset (init_lora_model_id, pretrained_lora_path)`.

### Task 2: Overview API

- [ ] `core/src/training/lineage.rs`: `list_loras(db)` → for each `models` row with `source LIKE 'training:%'` (plus imported LoRAs by store subdir, flagged `imported: true`): name, family, rank (header), size, created, `runs`, `total_steps`, `total_images`; `lineage(db, model_id)` → ordered chain of runs (walk `init_lora_model_id` → `models.source` → run; cycle-safe; a missing link ends the chain) with dataset name/source_root, image_count, captioner (from dataset), trigger, profile, preset, hyperparams, steps, duration, state, samples (existing scan). Tests: 3-deep chain, cycle, broken link, imported LoRA.
- [ ] Routes `GET /training/loras`, `GET /training/loras/{model_id}` + Tauri + ipc + dev mock.
- [ ] Commit `feat(training): LoRA overview and lineage API`.

### Task 3: UI

- [ ] Training tab "Your LoRAs" section (above runs): list with aggregates and the data-amount guidance line; selecting opens the history panel (runs oldest-first, samples, "Test in Image tab", "Continue with another dataset").
- [ ] `NewRunForm`: "Start from" picker (none / a LoRA of the same family), rank pinned + explanation, the forgetting notice, `init_lora_model_id` sent only when set; server refusals shown.
- [ ] `RunCard`: show dataset, preset, rank/lr/resolution, started/finished, "continued from <LoRA>".
- [ ] Live-verify in the dev preview; commit `feat(ui): LoRA overview, history and continue training`.

### Task 4: Real run + docs

- [ ] Back up DB. Real daemon: pick an existing library LoRA + its family's base model + a dataset export (create a small one from the Plan 6 flow if needed); start a continue-run with the fast preset and the smallest step count the preset allows; verify YAML has `pretrained_lora_path`, trainer log shows it loading the file, result is a new library model, overview shows the 2-run lineage; record time/VRAM. If no suitable pair is installed, record exactly what is missing (no pretending).
- [ ] `docs/TODO.md` ✅ entry with measurements; full gates; commit `docs(training): LoRA overview + continue training shipped, measured`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.
