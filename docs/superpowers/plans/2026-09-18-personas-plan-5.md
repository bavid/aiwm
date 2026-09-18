# Personas Implementation Plan (Plan 5)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-18-personas-design.md`.

**Goal:** Named presets (name, emoji icon, system prompt) that can be active globally and overridden per chat session; the chat job prepends the resolved persona's system prompt.

**Architecture:** New `personas` table + two session columns (migration 0018), server-side resolution at job start with self-healing for deleted personas, an optional system message in the llama.cpp chat body (default body byte-identical), CRUD over HTTP/Tauri, a persona chip + manage dialog in the Chat tab.

**Tech Stack:** Rust (tokio, sqlx/SQLite, axum), React 19 + TS, `aiwm-fake-llama` fixture with `/__test/last_request`.

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build`; never edit an existing migration file (sqlx checksums; 0017 is applied to the user's real DB); a chat without a persona must send exactly today's request.

---

### Task 1: Core — data, resolution, request, API

Files: create `core/migrations/0018_personas.sql`, `core/src/db/personas.rs`, `core/src/persona/mod.rs`; modify `core/src/db/{mod.rs,sessions.rs}`, `core/src/runtime/llamacpp/client.rs` (+ adapter), `core/src/capability/chat.rs`, `core/src/api/{dto,handlers,http}.rs`, `src-tauri/src/lib.rs`; tests in those modules, `core/tests/chat_job.rs`, new `core/tests/persona_api.rs`.

- [x] Migration 0018: `personas` table (STRICT, like the neighbours), `ALTER TABLE sessions ADD COLUMN persona_mode TEXT NOT NULL DEFAULT 'inherit'`, `ADD COLUMN persona_id TEXT`.
- [x] `persona::validate` (name 1–60 chars trimmed, icon non-empty ≤ 64 bytes, prompt 1–8000 chars) → `CoreError::Config`; the icon cap was widened from the planned 16 bytes in `b5f1a95`, because a single family/couple emoji (👨‍👩‍👧‍👦 is 25 bytes, 👩‍❤️‍👨 over 20) is one ZWJ sequence that already exceeds 16; repo CRUD with `updated_at`; deleting a persona resets sessions that point at it to `inherit` and clears `chat.active_persona_id` if it matches (one transaction).
- [x] `persona::resolve(db, session_id: Option<&str>) -> Result<Option<Persona>>`: session `persona` → it; `none` → None; else global active; orphaned id anywhere → heal + None. Tests for every branch.
- [x] Optional system message in the chat request; default body byte-identical (existing pinned test stays untouched and green); Colibri: support it if its client takes messages, otherwise leave it and report.
- [x] `capability::chat::run`: resolve, pass the system prompt, write `persona: {id,name,icon}` back into the job params; one info event "persona: <name>".
- [x] API + Tauri commands per the spec; `core/tests/persona_api.rs` (CRUD, 400 on invalid, 404 on unknown id, active get/put incl. null, session persona put incl. unknown session/persona); `core/tests/chat_job.rs`: active persona → system message first in `/__test/last_request` and `persona` in params; session `none` → no system message; deleted persona → chat still completes.
- [x] Commit `feat(chat): personas — presets with global default and per-session override`.

### Task 2: UI — ipc, dev-mock, Chat tab

Files: `ui/src/lib/{ipc.ts,hooks.ts,dev-mock.ts}`, create `ui/src/features/chat/personas/{PersonaChip.tsx,PersonaMenu.tsx,PersonaManager.tsx,persona-templates.ts,personas.css}`, modify `ui/src/features/chat/Chat.tsx` (keep it under 800 lines).

- [x] ipc types/functions/hooks; dev-mock with two seeded personas, CRUD, resolution mirrored, chat answers visibly prefixed by the persona so the preview shows the effect.
- [x] Chip (effective persona + origin), menu (global / this chat: inherit · none · pick), manager dialog (list, create from template, edit, delete with confirm), per-answer persona mark from job params, "Ungrouped" uses the global persona only. Accessible dialog (focus trap, Esc, labelled), both themes, no `any`.
- [x] Live-verify in the dev preview; commit `feat(ui): personas in the Chat tab`.

### Task 3: Real run + docs

- [x] Real `aiwm-cored` run with an installed GGUF chat model: same question without persona, with a global persona, and with a session override `none`; record the three answers' first lines as proof. Back up `E:\AI\data\aiwm.db` first (migration 0018 applies to the real DB).
- [x] `docs/TODO.md`: turn the Personas bullet into ✅ with what shipped and the leftovers; `docs/MODELS.md` or the route reference gets the new routes. Full gates. Commit `docs(chat): personas shipped`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.

---

## Verified 2026-09-18

Real `aiwm-cored` from this worktree against `E:\AI\data` (migration `0018`
applied to the real DB; `aiwm.db` backed up beforehand), model
**Mistral-Small-3.2-24B Instruct Uncensored IQ3_M** (GGUF, 12.4 GB estimate on a
16 GB RTX 4080 SUPER), one chat session, the same question three times: *"In one
sentence, what is a compiler?"*

1. **Global persona 🏴‍☠️ Pirate** — `GET /personas/effective` → `origin: "global"`;
   job params carried `persona: {id, name: "Pirate", icon: "🏴‍☠️"}` and the job
   event log `persona: 🏴‍☠️ Pirate`:
   > Arrr, a compiler be the scurvy dog that turns me code into machine language, savvy? Arr!
2. **Session override `none`** — `origin: "none"`, no `persona` in the params, no
   persona event:
   > A compiler is a program that translates code written in a high-level programming language into machine code for execution.
3. **No persona anywhere** (`PUT /personas/active {"id": null}`, session back to
   `inherit`) — `origin: "none"`, again no `persona` params and no event, and the
   answer came back **word for word identical** to run 2:
   > A compiler is a program that translates code written in a high-level programming language into machine code for execution.

Then the persona was deleted over HTTP while the session pointed at it with
`mode: "persona"`: `GET /sessions` showed `persona_mode: "inherit"`,
`persona_id: null` immediately, `GET /personas/active` was `{"id": null}`, and a
fourth chat completed normally with no persona and no error. `aiwm-cored` was
stopped with Ctrl-Break (`shutdown complete (ctrl-break)`); no `aiwm-cored` or
`llama-server` process was left behind and VRAM returned to its 719 MB idle
baseline.
