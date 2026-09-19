# Training & Captioning Tools Implementation Plan (Plan 7)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-19-training-tools-design.md`.

**Goal:** One-click install of the dataset captioners (WD tagger recommended, Florence-2, optional Qwen2.5-VL) from a "Training & captioning" section and straight from the Dataset tab's hint.

**Architecture:** Catalog entries + model stacks following the Dia multi-file pattern (pinned revisions, real hashes), a directory-shaped model kind for Florence-2 and Qwen2.5-VL, captioner detection by role, API exposure, UI section and install buttons.

**Tech Stack:** Rust (catalog, import, download manager), React 19 + TS, Python sidecar (captioners already implemented).

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build`, sidecar `ruff`/`pytest`; never guess a sha256 or size; never edit an existing migration.

---

### Task 1: Catalog + model kinds + detection

- [x] Verify on the Hub (network allowed for development): the exact file lists `Florence2`'s loader needs from `microsoft/Florence-2-large` and Qwen2.5-VL's from `Qwen/Qwen2.5-VL-7B-Instruct`, pick a commit revision for each, download every file to the session scratchpad, hash it; record licenses from the model cards.
- [x] New directory-shaped kinds (pattern `DiaEngine`) with default roles `vision_florence2` / `vision_qwen2_5_vl`; `KnownModel` entries with pinned `/resolve/<commit>/` URLs and real sha256/size; `ModelStack`s for WD tagger, Florence-2, Qwen2.5-VL with a new media/category `training`; catalog invariant tests.
- [x] Captioner detection works for an imported stack (directory kinds); tests.
- [x] Commit `feat(models): one-click captioner stacks (WD tagger, Florence-2, Qwen2.5-VL) with pinned revisions`.

### Task 2: API + UI

- [x] Expose the new section (existing catalog/stack routes if they carry media; otherwise extend them) + Tauri + ipc + dev mock.
- [x] Models tab "Training & captioning" section; Dataset tab hint with "Install the recommended captioner" + progress + refresh; install buttons in the captioner picker; offline-gate message.
- [x] Live-verify in the dev preview; commit `feat(ui): install captioners from the Models and Dataset tabs`.

### Task 3: Real install + run + docs

- [x] Back up `E:\AI\data\aiwm.db`. Real `aiwm-cored`: install the WD tagger stack through the download path, run a dataset prep on `D:\Data\Test` (read-only, verify SHA-256 unchanged) with it; record captions, time, VRAM. Then Florence-2 the same way if feasible. Qwen2.5-VL: install only if disk and time allow; otherwise record why not.
- [x] `docs/TODO.md` ✅ entry + measurements; full gates; commit `docs(models): captioner installs shipped, measured`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.

---

## Measured 2026-09-19

Real `aiwm-cored` from this worktree (port 48160, offline off), DB backed up first; installs exactly as `useStackInstaller.ts` (one `POST /downloads` per stack member), preps as `PrepForm.tsx` (frames mode, defaults) on `D:\Data\Test` (one video, source SHA-256 `15F22FD0…9869BF28` unchanged before/after). RTX 4080 SUPER, idle 1,808–1,814 MiB.

| Step | Result |
|---|---|
| WD tagger install | 2 files, 1,260,744,467 B, 30.1 s, `installed: true` |
| Prep, WD tagger | 393.9 s (extract 19.9 s, filter 288.6 s, caption 84.0 s for 40 frames on CPU); 1,368 frames, 40 kept, **40/40 captioned** (tag lists, 23–31 tags), no error events; VRAM peak = idle |
| Florence-2 install | 10 files, 1,556,213,789 B, 40.2 s, `installed: true`, integrity check passes |
| Prep, Florence-2 (no escalation) | **failed** after 313.4 s: `captioning failed: 'Florence2LanguageConfig' object has no attribute 'forced_bos_token_id'` — pinned remote code (`configuration_florence2.py:265`) vs transformers 5.17.0; model never reached the GPU |
| Qwen2.5-VL | imported from the scratchpad via `POST /models` (`qwen_vl_engine`, 14 files, 16,595,961,188 B, 11.6 s); `GET /captioners/escalation` → usable; escalation run **not possible** (rides on Florence-2) |
| Cleanup | both run datasets deleted (1,173,444,536 B freed each); daemon stopped via Ctrl-Break, nothing left running, VRAM back to idle |

Follow-ups are in `docs/TODO.md` (Florence-2 on transformers 5 is PRIO 1).