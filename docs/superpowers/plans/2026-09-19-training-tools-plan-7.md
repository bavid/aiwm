# Training & Captioning Tools Implementation Plan (Plan 7)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-19-training-tools-design.md`.

**Goal:** One-click install of the dataset captioners (WD tagger recommended, Florence-2, optional Qwen2.5-VL) from a "Training & captioning" section and straight from the Dataset tab's hint.

**Architecture:** Catalog entries + model stacks following the Dia multi-file pattern (pinned revisions, real hashes), a directory-shaped model kind for Florence-2 and Qwen2.5-VL, captioner detection by role, API exposure, UI section and install buttons.

**Tech Stack:** Rust (catalog, import, download manager), React 19 + TS, Python sidecar (captioners already implemented).

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build`, sidecar `ruff`/`pytest`; never guess a sha256 or size; never edit an existing migration.

---

### Task 1: Catalog + model kinds + detection

- [ ] Verify on the Hub (network allowed for development): the exact file lists `Florence2`'s loader needs from `microsoft/Florence-2-large` and Qwen2.5-VL's from `Qwen/Qwen2.5-VL-7B-Instruct`, pick a commit revision for each, download every file to the session scratchpad, hash it; record licenses from the model cards.
- [ ] New directory-shaped kinds (pattern `DiaEngine`) with default roles `vision_florence2` / `vision_qwen2_5_vl`; `KnownModel` entries with pinned `/resolve/<commit>/` URLs and real sha256/size; `ModelStack`s for WD tagger, Florence-2, Qwen2.5-VL with a new media/category `training`; catalog invariant tests.
- [ ] Captioner detection works for an imported stack (directory kinds); tests.
- [ ] Commit `feat(models): one-click captioner stacks (WD tagger, Florence-2, Qwen2.5-VL) with pinned revisions`.

### Task 2: API + UI

- [ ] Expose the new section (existing catalog/stack routes if they carry media; otherwise extend them) + Tauri + ipc + dev mock.
- [ ] Models tab "Training & captioning" section; Dataset tab hint with "Install the recommended captioner" + progress + refresh; install buttons in the captioner picker; offline-gate message.
- [ ] Live-verify in the dev preview; commit `feat(ui): install captioners from the Models and Dataset tabs`.

### Task 3: Real install + run + docs

- [ ] Back up `E:\AI\data\aiwm.db`. Real `aiwm-cored`: install the WD tagger stack through the download path, run a dataset prep on `D:\Data\Test` (read-only, verify SHA-256 unchanged) with it; record captions, time, VRAM. Then Florence-2 the same way if feasible. Qwen2.5-VL: install only if disk and time allow; otherwise record why not.
- [ ] `docs/TODO.md` ✅ entry + measurements; full gates; commit `docs(models): captioner installs shipped, measured`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.
