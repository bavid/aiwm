# In-App Help & Tooltips Implementation Plan (Plan 12)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-19-help-and-tooltips-design.md`.

**Goal:** A searchable, offline Help tab that documents every feature, plus accessible `?` hints on every non-obvious setting — Datasets & Training first — all rendered from one content source so page and hints never drift.

**Architecture:** Typed content modules under `ui/src/help/` (`HelpTopic`/`HelpSetting`), a `HelpHint` disclosure component (FitBadge pattern), a `Help` tab with `SectionNav` + search + deep-link focus (`ModelsFocus` pattern), palette "Help" group, a node consistency script wired into `pnpm lint`. UI only — no core changes.

**Tech Stack:** React 19 + TS, existing CSS tokens; node script for the content check.

Rules: explicit-pathspec commits, never `git add -A`, never bare `git stash`, never `--no-verify`; gates `pnpm typecheck`, `pnpm lint` (incl. `lint:help`), `pnpm build`; numbers only from the measured tables in `docs/TODO.md` (cite the date); English UI text; no `title=`-only tooltips; no console.log; files < 400 lines where possible (content files split per area).

---

### Task 1: Content model + HelpHint + Help tab skeleton + Dataset/Training content

- [x] `ui/src/help/types.ts` (`HelpArea`, `HelpTopic`, `HelpSetting`, `HelpBlock`), `ui/src/help/index.ts` (registry, `findSetting(area, key)`, `searchHelp(query)`), `ui/src/help/dataset.ts`, `ui/src/help/training.ts` with complete content for every control listed in the spec (each `HelpSetting` has non-empty `what/why/effect/benefit`, pitfalls where known, `measured` with date where a number exists).
- [x] `ui/src/components/HelpHint.tsx` (+ css): `?` button, `aria-expanded`/`aria-controls`, `describes` prop → `aria-describedby` with the `what` line while collapsed, Escape closes, "More in Help" → `onOpenHelp(area, key)`; no hover-only behaviour.
- [x] `ui/src/features/help/Help.tsx` (+ css): tab `help` in `App.tsx` `TABS`; `SectionNav` per area; search box filtering topics/settings with highlighted matches; `helpFocus` lifted state in `App.tsx` (scroll + highlight target); `ShortcutsHelp` gets an "Open Help" button; `CommandPalette` gains a "Help" group and its `TAB_ENTRIES` completed.
- [x] Wire `HelpHint` into `PrepForm.tsx`, `Dataset.tsx` (trigger word), `HousekeepingPanel.tsx`, `ExportCard.tsx`, `RejectionChips`/`CurationColumn` (discard reasons), `ConceptsPanel`, `LearnSets`, `NewRunForm.tsx`, `FineTune.tsx`, `Preflight.tsx`, `RunCard.tsx`, `LoraList/LoraHistory`.
- [x] `ui/scripts/check-help.mjs` + `pnpm lint:help` wired into `pnpm lint`: every hint key resolves, every area has ≥1 topic, no empty fields.
- [x] Live-verify in the dev preview (keyboard-only hint open/close, `aria-describedby` toggling, search, deep link, palette). Commit `feat(ui): in-app Help tab and accessible hints for datasets and training`.

### Task 2: Remaining areas + `title=` cleanup

- [x] Content for Getting started, Dashboard, Chat/Personas, Image (Hi-Res-Fix, LoRA stack), Video, Upscale, Voice, Stories, Jobs, Agents, Models/Discover (fit badge, captioner installs), Benchmark, Diagnostics, Settings (data locations, retention, cleanup), Shortcuts — each with purpose, flow, settings, disk/GPU effects, limits, measured numbers where they exist.
- [x] Hints on the non-obvious controls of those tabs; replace `title=`-only icon buttons in Chat/Discover/Image/FrameCard/Dashboard/FileList/SelectionBar with accessible names (+ hint where meaning is non-obvious).
- [x] Live-verify; commit `feat(ui): help content and hints for every tab`.

### Task 3: Review + docs

- [x] Whole-branch review (react-reviewer + a11y pass); fix rounds.
- [x] `docs/TODO.md` ✅ entry (what shipped, what is deferred); commit `docs(help): in-app help shipped`.
- [x] Hand back: controller gates (UI typecheck/lint/build green; no core changes) → merge `--no-ff` → push.
