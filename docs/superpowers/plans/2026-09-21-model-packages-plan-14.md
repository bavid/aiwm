# Model Packages Implementation Plan (Plan 14)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-21-model-packages-design.md` (approved 2026-09-21).

**Goal:** Picking any model — a Civitai LoRA, a checkpoint, a Hugging Face repo, or a file already in the library — shows the whole package it needs (base + companions), what is installed, what is missing and what cannot run here, and downloads the missing parts in one go.

**Architecture:** A base-family registry (`core/src/model/family.rs`) maps Civitai/HF base labels to AIWM families with their catalog stack and a runnable flag; family inference on read (download metadata → safetensors header → name); a pure resolver building `Package { item, needs, missing_bytes }`; routes `GET /packages/resolve`, `GET /packages/library`, `POST /models/{id}/family`; Discover "Get" dialog and a Models "Packages" section. SD 1.5 gains a curated stack.

**Tech Stack:** Rust (axum, sqlx/SQLite), Tauri 2, React 19 + TS.

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits (`git commit -- <paths>`), never `git add -A`, never bare `git stash`, never `--no-verify`; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build` (in `ui/`); never edit an existing migration; **never guess a sha256 or a size**; nothing written to the user's DB by merely reading.

---

### Task 1: Family registry + inference

- [ ] `core/src/model/family.rs`: `BaseFamily { id, label, civitai_labels, hf_base_models, arch_group, stack_id: Option, runnable: Runnable::Yes | No(reason) }`, the table (sdxl, sd15, flux1, flux2-klein-4b, flux2-klein-9b, wan22-5b, ltxv; Pony/Illustrious/NoobAI → own entries in the `sdxl` arch group, `stack_id: None`; Wan 2.2 14B/I2V-A14B/T2V-A14B → `No("does not fit 16 GB — this app runs the 5B")`; unknown → `None` family). `family_for_civitai(label)`, `family_for_hf(base_model)`, `works_with(a, b)` (same arch group). Tests over every label of the 2026-09-21 sample.
- [ ] Inference: `infer_family(model, header) -> Option<(family, FamilySource)>` — recorded metadata, safetensors header (checkpoint tensor-name patterns per architecture; LoRA key prefixes), file name last. Synthetic-header tests per architecture.
- [ ] Migration `0022_model_family_source.sql` (`ALTER TABLE models ADD COLUMN family_source TEXT`); repo read/write; `user` never overwritten.
- [ ] Commit `feat(models): base-family registry and family inference`.

### Task 2: Resolver + routes

- [ ] Downloads from Civitai/HF record `family` (+ `family_source = civitai|hf`) and `source` on import; a LoRA also records the base family it targets.
- [ ] `core/src/model/packages.rs`: `resolve_package(item, library, catalog) -> Package` with `Need { role, status: Installed | Catalog | Findable | NotRunnable }`, "made for" vs "works with"; `library_packages(library)` grouping by family with orphans. Pure, tested (all installed / base in catalog / findable / not runnable / made-for / unknown).
- [ ] `Findable`: Civitai checkpoint search by `baseModels=<label>`, top 3 by downloads, behind the offline gate.
- [ ] Routes `GET /packages/resolve?source=civitai&id=…|library&id=…`, `GET /packages/library`, `POST /models/{id}/family` (user choice; `POST /models/families` persists a previewed batch) + Tauri + ipc + dev mock.
- [ ] Commit `feat(models): resolve a model into the package it needs`.

### Task 3: SD 1.5

- [ ] Pick the SD 1.5 checkpoint from the official Hugging Face mirror, download to the session scratchpad, hash it, record real sha256 + size; catalog `KnownModel` + `ModelStack { id: "sd15" }`; delete the scratchpad copy afterwards.
- [ ] Verify the image pipeline handles an SD 1.5 checkpoint (512 px default size, sampler settings); tests. If it cannot run, stop and report — the family stays `No(reason)`.
- [ ] Commit `feat(models): SD 1.5 as a supported base`.

### Task 4: Discover

- [ ] Replace "Set import type" in Discover with **Get** → package dialog (✓/↓/?/✗, sizes, fit badge, licence, top-3 checkpoints for `Findable`); "Download … only" / "Download … + missing (N GB)"; a `NotRunnable` warning on the card before any download; the Downloads page groups a package's items.
- [ ] Live-verify in the dev preview; commit `feat(ui): get a model with everything it needs`.

### Task 5: Library "Packages"

- [ ] Models section "Packages": grouped by family (base, companions, LoRAs), "Download missing (N GB)", "What base is this?" picker, "Save detected families" with preview.
- [ ] Live-verify; commit `feat(ui): library packages view`.

### Task 6: Real run + docs

- [ ] DB backup; real daemon; resolve one LoRA per family the user has (SDXL, FLUX.2 klein), one Pony, one Wan 14B, one SD 1.5; `GET /packages/library` on the real library; record results and timings. **No download into the user's library without an explicit OK at that point.**
- [ ] `docs/TODO.md` ✅ entry; full gates; commit `docs(models): packages shipped, measured`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.
