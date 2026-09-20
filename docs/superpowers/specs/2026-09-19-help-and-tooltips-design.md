# In-App Help & Tooltips — Design (Plan 12)

Status: written 2026-09-19 from the backlog section "In-App-Dokumentation & Tooltips" in
`docs/TODO.md` (user wish 2026-09-18): a page inside the app where **every** feature is explained
(what it is for, step-by-step flow, what each setting does, what happens on disk/GPU, limits and
pitfalls), searchable, offline; tooltips wherever a setting is not self-explanatory answering
*What does it do? Why do I need it? What happens when I change/start it? What do I gain?*; keyboard and
screen-reader usable (not `title=`); focus first on Datasets & Training.

## What exists (verified in code 2026-09-19)

- Navigation: one `Tab` union + `TABS` list in `ui/src/App.tsx:21-207`; every tab stays mounted
  (`hidden` toggling). Intra-page table of contents: `components/SectionNav.tsx` (used by Settings,
  Models). Cross-tab "jump to a section" precedent: `ModelsFocus` (`features/models/Models.tsx`).
- Accessible disclosure pattern to copy: `features/models/FitBadge.tsx:1-25,66-81` — a `?` button with
  `aria-expanded`/`aria-controls`, text `hidden` when collapsed and reachable via `aria-describedby`;
  the doc comment explains why `title=` tooltips are rejected. `aria-describedby` + `useId` help text
  exists on several fields (`StorageDirField`, `HousekeepingPanel`, `PrepForm`, `CurationColumn`).
- **No shared Tooltip/Disclosure component**, no markdown renderer, no i18n, no About/changelog page.
  Static text lives inline in JSX; "content as a data module" precedents:
  `features/chat/personas/persona-templates.ts`, `lib/prompt-presets.ts`.
- Search: `components/CommandPalette.tsx` (Ctrl+K, in-memory substring filter, groups Go to/Model/Job).
  Its `TAB_ENTRIES` is stale (omits voice, stories, dataset, training). `?` key opens
  `components/ShortcutsHelp.tsx` (dialog, 4 shortcuts).
- Measured numbers to quote live in `docs/TODO.md` (dataset prep 298.6 s / 1,368 frames / 40 kept,
  cleanup 0.28 s; FLUX.2 klein 4B 1.69 s/step, VRAM peak 12,340 MB; captioner VRAM 1,820 → 11,867 →
  2,145 MiB; Florence-2 ~2,187 MiB; filter ~200 ms/frame; storage-location run 296.6 s).

## Decisions

| Question | Decision |
|---|---|
| Content source | **One data module** `ui/src/help/content.ts` (+ per-area files `help/dataset.ts`, `help/training.ts`, … each < 400 lines): typed `HelpTopic { id, area, title, summary, body: HelpBlock[], settings?: HelpSetting[] }` and `HelpSetting { key, label, what, why, effect, benefit, pitfalls?, measured? }`. Tooltips and the Help page render from the same objects, so they cannot drift. Plain TSX/structured text — no markdown dependency. English, like the rest of the UI. Numbers copied from the measured tables in `docs/TODO.md` with the date they were measured; never estimated. |
| Help page | New tab `help` ("Help") at the end of `TABS`; `SectionNav` on the left with one section per area (Getting started, Dashboard, Chat, Image, Video, Voice, Stories, Dataset, Training, Jobs, Agents, Models & Discover, Benchmark, Diagnostics, Settings, Storage & cleanup, Shortcuts); each area: purpose, typical flow (ordered steps), settings table (from `HelpSetting`), what happens on disk/GPU, limits & pitfalls, measured numbers. A search box at the top filters topics and settings by substring (title, summary, setting labels, body text) and highlights matches; Ctrl+K palette gains a "Help" group listing topics (and its `TAB_ENTRIES` is completed with the missing tabs). Deep link: `helpFocus: { topic, setting? }` lifted state in `App.tsx` like `ModelsFocus`; the page scrolls to and highlights the target. |
| Tooltip component | New `components/HelpHint.tsx`: a `?` button (same visual as `FitBadge`'s hint) that toggles an inline disclosure panel with the four answers (What / Why / What happens / Benefit) + optional pitfalls + a "More in Help" link (`onOpenHelp(topic, setting)`). Keyboard: button is focusable, Enter/Space toggles, Escape closes; `aria-expanded` + `aria-controls`; when collapsed the *summary line* (`what`) is exposed via `aria-describedby` on the field it explains (prop `describes: id`). Touch-friendly (it's a button, not hover). No `title=`. |
| Where hints go first | Dataset: root folder (subfolder = tag; files directly in the root are tagged with the folder name), mode, fps, blur threshold, duplicate distance, max frames per clip, min clip length, auto-caption + captioner choice (Florence-2 prose vs WD tags vs Qwen escalation, VRAM and speed from measurements), escalation + Nth + context offset, trigger word (and why everything recurring flows into it without captions), store frames in, curation columns and discard reasons (per-reason chip explanations), concepts, learn sets, housekeeping (duplicate threshold, clean up, delete dataset — what is never touched), export (caption order). Training: target model (profiles and what each is for), dataset (why only exported), run name, trigger word, presets (steps/rank/lr/resolution in plain words), fine-tune fields, sample prompts, store run in, preflight rows, run actions (pause/resume/cancel/delete+purge/test), continue-from LoRA (Plan 11), forgetting caveat. Then, in the same plan but lower priority: Image (Hi-Res-Fix, LoRA stack), Video, Upscale, Voice, Chat/Personas, Agents, Stories, Models/Discover (fit badge), Benchmark, Settings (data locations, retention). Existing `title=`-only icon buttons in Chat/Discover/Image/FrameCard get an accessible name and, where meaning is non-obvious, a hint. |
| Existing `?` key | Keeps opening the shortcuts dialog; the dialog gets a "Open Help" button. |
| Consistency | An ESLint-free sanity test (vitest is not installed — add a tiny node script `ui/scripts/check-help.mjs` run by `pnpm lint:help`, wired into `pnpm lint`): every `HelpSetting.key` referenced by a `HelpHint` exists, every area has a topic, no empty `what/why/effect/benefit`. |

## Tests / proof

`check-help` script green; typecheck/lint/build; live preview: Help tab renders every area, search
filters and highlights, a `HelpHint` on the prep form opens with keyboard only and is announced
(`aria-describedby` present when collapsed, absent when open), "More in Help" lands on the right
setting, palette "Help" group navigates. Screenshot of the Dataset area and one open hint for the
record. No real daemon needed.

## Not in this plan

Translations; a docs export; editing help text inside the app; auto-generated screenshots.
