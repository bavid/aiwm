# Benchmark Tab Implementation Plan (Plan 4)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Spec: `docs/superpowers/specs/2026-09-17-benchmark-tab-design.md`.

**Goal:** A Benchmark tab that runs a versioned chat/coding prompt suite against a GGUF model and compares tokens/sec across models and runs.

**Architecture:** Extend the existing `bench` job (`core/src/bench/mod.rs`) with embedded suites and per-prompt detail; persist via migration 0017; expose over HTTP/Tauri; new UI feature folder.

**Tech Stack:** Rust (tokio, sqlx/SQLite, axum), React 19 + TS, existing `aiwm-fake-llama` fixture.

Rules for every task: TDD with observed RED; no `unsafe`; no `unwrap/expect` in production; explicit-pathspec commits; rustfmt edited files immediately; gates `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build`; without a suite the bench job must behave exactly as today.

---

### Task 1: Suites + request + report + DB

Files: create `core/src/bench/suites.rs`, `core/migrations/0017_benchmark_suites.sql`; modify `core/src/bench/mod.rs`, `core/src/db/bench.rs`, the engine's bench call site, `core/tests/bench.rs`.

- [ ] `suites.rs`: `Suite`, `SuitePrompt`, `all()`, `find()`; `chat-v1` (explain a concept / summarise a given paragraph / rewrite a given email politely; max_tokens 256) and `coding-v1` (write a function from a spec / find and fix the bug in a given snippet / explain a given snippet and suggest tests; max_tokens 384). Self-written prompts, English. Tests: unique ids, `-vN` suffix, non-empty prompts, max_tokens within `MIN_MAX_TOKENS..=MAX_MAX_TOKENS`.
- [ ] `BenchRequest.suite`, default runs 2 when a suite is set; unknown suite → `bench_err("unknown benchmark suite …")`.
- [ ] `run`: with a suite, loop prompts × runs, one job event per pass, cancel checked between passes; `BenchReport.detail`; aggregate gen/prompt tps over all passes.
- [ ] Migration 0017 (`suite`, `detail_json`), `NewBenchmark`/`Benchmark` fields, `list_all(suite: Option<&str>, limit)`; roundtrip tests.
- [ ] `core/tests/bench.rs`: suite run via fake-llama writes a row with `suite = "chat-v1"` and 3 detail entries; unknown suite fails the job; legacy run unchanged.
- [ ] Commit `feat(bench): versioned chat/coding suites with per-prompt detail`.

### Task 2: API + Tauri + ipc + dev-mock

Files: `core/src/api/{handlers,http}.rs`, `src-tauri` commands, `ui/src/lib/{ipc.ts,hooks.ts,dev-mock.ts}`, API tests.

- [ ] `GET /bench/suites`, `POST /models/{id}/benchmark` optional JSON body `{suite, runs}` (empty body still works), `GET /benchmarks/history?suite=&limit=` (limit clamped 1..=200). Non-GGUF model → existing 400 message.
- [ ] Tauri commands + `ipc.ts` types (`BenchSuite`, `Benchmark.suite/detail`) + hooks + dev-mock (fake suite run producing plausible rows).
- [ ] Commit `feat(api): benchmark suites, suite runs and history`.

### Task 3: Benchmark tab UI

Files: create `ui/src/features/benchmark/{Benchmark.tsx,benchmark.css,…}`; modify `ui/src/App.tsx` (tab registration next to Models).

- [ ] Form, live job state, result card, comparison table with bars, per-model history, empty states, the "speed, not quality" note. Follow existing tab conventions (tokens.css, `.chip`, `tabular-nums`), both themes, keyboard focus, no `any`.
- [ ] Live-verify through the dev-mock; commit `feat(ui): Benchmark tab`.

### Task 4: Real run + docs

- [ ] Real `aiwm-cored` run of `chat-v1` and `coding-v1` on one installed GGUF chat model (two if available); record numbers.
- [ ] `docs/TODO.md`: replace the backlog block with ✅ + measured table + leftovers (correctness phase with sandbox, other runtimes, batch sweeps). Full gates. Commit `docs(bench): Benchmark tab shipped, measured numbers`.
- [ ] Hand back: whole-branch review → controller gates → merge `--no-ff` → push.
