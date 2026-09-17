# Benchmark Tab Implementation Plan (Plan 4)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-17-benchmark-tab-design.md`.

**Goal:** A Benchmark tab that runs a versioned chat/coding prompt suite against a GGUF model and compares tokens/sec across models and runs.

**Architecture:** Extend the existing `bench` job (`core/src/bench/mod.rs`) with embedded suites and per-prompt detail; persist via migration 0017; expose over HTTP/Tauri; new UI feature folder.

**Tech Stack:** Rust (tokio, sqlx/SQLite, axum), React 19 + TS, existing `aiwm-fake-llama` fixture.

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build`; without a suite the bench job must behave exactly as today.

---

### Task 1: Suites + request + report + DB

Files: create `core/src/bench/suites.rs`, `core/migrations/0017_benchmark_suites.sql`; modify `core/src/bench/mod.rs`, `core/src/db/bench.rs`, the engine's bench call site, `core/tests/bench.rs`.

- [x] `suites.rs`: `Suite`, `SuitePrompt`, `all()`, `find()`; `chat-v1` (explain a concept / summarise a given paragraph / rewrite a given email politely; max_tokens 256) and `coding-v1` (write a function from a spec / find and fix the bug in a given snippet / explain a given snippet and suggest tests; max_tokens 384). Self-written prompts, English. Tests: unique ids, `-vN` suffix, non-empty prompts, max_tokens within `MIN_MAX_TOKENS..=MAX_MAX_TOKENS`.
- [x] `BenchRequest.suite`, default runs 2 when a suite is set; unknown suite → `bench_err("unknown benchmark suite …")`.
- [x] `run`: with a suite, loop prompts × runs, one job event per pass, cancel checked between passes; `BenchReport.detail`; aggregate gen/prompt tps over all passes.
- [x] Migration 0017 (`suite`, `detail_json`), `NewBenchmark`/`Benchmark` fields, `list_all(suite: Option<&str>, limit)`; roundtrip tests.
- [x] `core/tests/bench.rs`: suite run via fake-llama writes a row with `suite = "chat-v1"` and 3 detail entries; unknown suite fails the job; legacy run unchanged.
- [x] Commit `feat(bench): versioned chat/coding suites with per-prompt detail`.

### Task 2: API + Tauri + ipc + dev-mock

Files: `core/src/api/{handlers,http}.rs`, `src-tauri` commands, `ui/src/lib/{ipc.ts,hooks.ts,dev-mock.ts}`, API tests.

- [x] `GET /bench/suites`, `POST /models/{id}/benchmark` optional JSON body `{suite, runs}` (empty body still works), `GET /benchmarks/history?suite=&limit=` (limit clamped 1..=200). Non-GGUF model → existing 400 message.
- [x] Tauri commands + `ipc.ts` types (`BenchSuite`, `Benchmark.suite/detail`) + hooks + dev-mock (fake suite run producing plausible rows).
- [x] Commit `feat(api): benchmark suites, suite runs and history`.

### Task 3: Benchmark tab UI

Files: create `ui/src/features/benchmark/{Benchmark.tsx,benchmark.css,…}`; modify `ui/src/App.tsx` (tab registration next to Models).

- [x] Form, live job state, result card, comparison table with bars, per-model history, empty states, the "speed, not quality" note. Follow existing tab conventions (tokens.css, `.chip`, `tabular-nums`), both themes, keyboard focus, no `any`.
- [x] Live-verify through the dev-mock; commit `feat(ui): Benchmark tab`.

### Task 4: Real run + docs

- [x] Real `aiwm-cored` run of `chat-v1` and `coding-v1` on one installed GGUF chat model (two if available); record numbers.
- [x] `docs/TODO.md`: replace the backlog block with ✅ + measured table + leftovers (correctness phase with sandbox, other runtimes, batch sweeps). Full gates. Commit `docs(bench): Benchmark tab shipped, measured numbers`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.

---

## Measured 2026-09-17

Real `aiwm-cored` from this worktree against `E:\AI\data` (migration 0017
applied to the real DB; backup taken first), real `llama-server b10855`,
RTX 4080 SUPER 16 GB, idle VRAM 705 MB.

| Modell | Suite | tok/s | Prefill tok/s | Ladezeit | VRAM-Spitze | Stabilität | Pässe | Tokens/Prompt | Anmerkung |
|---|---|---|---|---|---|---|---|---|---|
| Mistral-Small-3.2-24B-Instruct-2506 ultra-uncensored-heretic, IQ3_M (10,7 GB) | `chat-v1` | **56,35** | 2464 | 6090 ms (kalt) | 12406 MB | 0,9995 | 6 (3 × 2) | 256/256, 256/256, 256/256 | Wall 37 s inkl. Kaltstart |
| dito | `coding-v1` | **56,32** | 2369 | — (resident) | 12405 MB | 0,9997 | 6 (3 × 2) | 384/384, 384/384, 384/384 | Wall 45 s |
| dito | — (Quick-Test, leerer Body) | **56,66** | **79** | — (resident) | 12415 MB | 0,9991 | 3 | 128 (Cap), EOS-terminiert, kein Detail | Wall 8 s; Prefill bricht ein, weil der Quick-Test `cache_prompt` anlässt |
| Qwen2.5-7B-Instruct **F16** (15,2 GB) | `chat-v1`, `coding-v1`, Quick-Test | — | — | — | — | — | 0 | — | Vom VRAM-Planer abgelehnt (Wortlaut unten) |

**Key check — passed.** All twelve suite detail entries have
`tokens == max_tokens` (256 for `chat-v1`, 384 for `coding-v1`) and no `notes`
field contains "stopped early". The notes read
`"cold load; suite chat-v1, 6 pass(es) averaged"` and
`"model already resident (load time not measured); suite coding-v1, 6 pass(es)
averaged"`.

**Contrast with the quick test.** Same model, resident, same machine: the suite
measures 2369–2464 tok/s prefill, the quick test 79 tok/s (126 / 56 / 56 over
its three runs). That gap *is* `cache_prompt false` — the quick test re-uses the
server's cached prefill from run 2 on, so its prefill number is not a prefill
measurement at all. Generation rates agree to within 0.6 % (56,32–56,66), which
is the expected result: `ignore_eos` changes *what* is generated, not how fast.

**Qwen2.5 7B F16 refused, not worked around.** `jobs.error_text`, verbatim
(first attempt, GPU idle):

> not enough VRAM for Qwen2.5 7B Instruct: needs ~15.0 GB (weights 14.2 GB + KV
> cache 0.4 GB @ 8K ctx + 0.3 GB overhead) — 15329 MB needed, but this GPU only
> has 14840 MB usable in total — this model doesn't fit this card no matter what
> else is running. Try a smaller quant/model.. Free VRAM by closing the resident
> model, or import a smaller quant / lower the context.

A second attempt with the Mistral resident produced the same message with
`2733 MB usable` instead of `14840 MB`. The other two `llm/` entries
(`downloaded-7b{,-cb7e76e9}/qwen.Q4_K_M.gguf`) are 40 kB stubs from a download
test, not real models — hence one measured model, not two.

Minor observations, not fixed here:
- The refusal text contains a double period (`Try a smaller quant/model..`) — the
  scheduler's reason string is concatenated onto a sentence that already ends.
- A `blocked` bench job is re-evaluated every ~3 s indefinitely; on a model that
  can never fit this card that is a permanent log loop.
