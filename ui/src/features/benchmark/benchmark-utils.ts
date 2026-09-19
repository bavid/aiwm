/** Pure helpers behind the Benchmark tab — formatting, the "latest run per
 *  model" reduction the comparison table is built on, and the tolerant parse
 *  of the per-pass job event. No React, no IPC: everything here is a plain
 *  function over what `core::bench` already recorded. */

import {
  isBenchmarkable,
  isJobActive,
  type BenchPromptResult,
  type Benchmark,
  type BenchSuite,
  type Job,
  type Model,
} from "../../lib/ipc";
import { formatGiB } from "../../lib/units";

/** The newest `bench` job that has not finished, whoever started it — what the
 *  tab adopts when it is not already watching one. `null` when nothing is
 *  running. */
export function liveBenchJob(jobs: readonly Job[] | null): Job | null {
  return (jobs ?? [])
    .filter((j) => j.job_type === "bench" && isJobActive(j.state))
    .reduce<Job | null>(
      (newest, j) => (!newest || j.created_at > newest.created_at ? j : newest),
      null,
    );
}

/** The role a model must claim to be worth a chat/coding throughput test. */
const CHAT_ROLE = "chat";

/** What this tab offers in its picker: models the benchmark endpoint accepts
 *  at all ({@link isBenchmarkable}), narrowed to the ones the library calls
 *  chat models. A GGUF diffusion model (FLUX ships as one) passes the API's
 *  format check but has nothing to say about tokens per second. */
export function chatModelsToBenchmark(models: readonly Model[]): Model[] {
  return models.filter((m) => isBenchmarkable(m) && m.roles.includes(CHAT_ROLE));
}

/** What one `"{suite} · {title} — pass i/n: X tok/s"` event carries. */
export interface PassEvent {
  /** The prompt's title, or `null` when the line named no prompt (the
   *  suite-less quick test's `run i/n` lines). */
  promptTitle: string | null;
  pass: number;
  runs: number;
  tps: number;
}

/** Deliberately loose: it keys off the `i/n: X tok/s` tail rather than the
 *  exact prose or dash character around it, so a reworded opening in
 *  `core::bench` does not silently stop the live progress from advancing. */
const PASS_RE = /(?:pass|run)\s+(\d+)\s*\/\s*(\d+)\s*:\s*(\d+(?:\.\d+)?)\s*tok\/s/i;
/** Separates the suite id from the prompt title in a pass event. */
const LABEL_SEPARATOR = "·";
/** Trailing dashes/whitespace between the label and "pass i/n". */
const LABEL_TAIL = /[\s‐-―-]+$/u;

/** Reads one job event; `null` for every line that is not a pass result (the
 *  opening line, the closing score line, warnings). */
export function parsePassEvent(message: string): PassEvent | null {
  const m = PASS_RE.exec(message);
  if (!m) return null;
  const pass = Number(m[1]);
  const runs = Number(m[2]);
  const tps = Number(m[3]);
  if (!Number.isFinite(pass) || !Number.isFinite(runs) || !Number.isFinite(tps)) return null;

  // The *first* separator ends the suite id; a prompt title that contains one
  // itself keeps it (`lastIndexOf` would eat everything up to that one).
  const head = message.slice(0, m.index).replace(LABEL_TAIL, "");
  const dot = head.indexOf(LABEL_SEPARATOR);
  const title = (dot >= 0 ? head.slice(dot + LABEL_SEPARATOR.length) : head).trim();
  return { promptTitle: title === "" ? null : title, pass, runs, tps };
}

/** Where a parsed pass sits in the suite — `{ index, of }`, 1-based, or `null`
 *  when the line named no prompt this suite knows (an older suite version, or
 *  a suite-less quick test). Lets the live panel say "prompt 2/3".
 *
 *  Matched by *title*, not id, because the title is all the event line carries
 *  — `core::bench` writes "{suite} · {prompt title} — pass i/n", never the
 *  prompt id. A retitled prompt in a new suite version therefore stops
 *  matching, which is correct: that is a different prompt. */
export function promptPosition(
  suite: BenchSuite | null,
  title: string | null,
): { index: number; of: number } | null {
  if (!suite || title == null) return null;
  const at = suite.prompts.findIndex((p) => p.title === title);
  return at < 0 ? null : { index: at + 1, of: suite.prompts.length };
}

/** The passes-per-prompt the form offers. The core clamps to `1..=10`; five is
 *  as far as this tab goes before a run stops being a quick check. */
export const MIN_RUNS = 1;
export const MAX_RUNS = 5;
/** Matches `core::bench`'s own default for a suite run. */
export const DEFAULT_RUNS = 2;

/** The API's maximum page size for `GET /benchmarks/history`. The comparison
 *  asks for all of it: at the 50-row default a machine with a few models and a
 *  busy afternoon silently loses whole models off the chart. */
export const HISTORY_LIMIT = 200;

/** The passes field keeps raw text so clearing it does not snap to 1; this is
 *  what that text means. Blank or unparseable reads as the default, never as
 *  zero. */
export function parseRuns(text: string): number {
  const trimmed = text.trim();
  if (trimmed === "") return DEFAULT_RUNS;
  const n = Number(trimmed);
  if (!Number.isFinite(n)) return DEFAULT_RUNS;
  return Math.min(MAX_RUNS, Math.max(MIN_RUNS, Math.round(n)));
}

/** How many generation passes a suite run of `runs` passes per prompt means in
 *  total — the denominator the live panel counts towards. */
export function totalPasses(suite: BenchSuite | null, runs: number): number {
  return suite ? suite.prompts.length * Math.max(0, runs) : 0;
}

/** The suite id a bench job was queued with — the run's own, which may differ
 *  from whatever the form shows now (a reload, or a job started elsewhere). */
export function jobSuiteId(params: unknown): string | null {
  if (params == null || typeof params !== "object") return null;
  const { suite } = params as { suite?: unknown };
  return typeof suite === "string" ? suite : null;
}

/** The suite a bench job is running, as far as this build still knows it. */
export function jobSuite(params: unknown, suites: readonly BenchSuite[]): BenchSuite | null {
  const id = jobSuiteId(params);
  return id == null ? null : (suites.find((s) => s.id === id) ?? null);
}

/** The core clamps a job's own `runs` to `1..=10` (`core::bench::MAX_RUNS`) —
 *  independent of this tab's own, tighter `MAX_RUNS` above, which only bounds
 *  what the *form* offers. A job adopted from elsewhere (the Model Library's
 *  own suite runs, or a future caller) can carry a value up to the core's
 *  limit, and reading it back unclamped would undercount or overcount the
 *  live panel's total. */
const CORE_MAX_RUNS = 10; // core::bench::MAX_RUNS

/** Total passes for a job already in flight, read back from its own params
 *  rather than from the form — the form may have moved on since it started. */
export function jobPasses(params: unknown, suites: readonly BenchSuite[]): number {
  if (params == null || typeof params !== "object") return 0;
  const { runs } = params as { runs?: unknown };
  const asked = Number(runs);
  const clamped = Number.isFinite(asked)
    ? Math.min(CORE_MAX_RUNS, Math.max(MIN_RUNS, Math.round(asked)))
    : DEFAULT_RUNS;
  return totalPasses(jobSuite(params, suites), clamped);
}

/** A model stopped generating before the token cap, so its tok/s covers a
 *  shorter run than the other prompts and is not comparable. Either the engine
 *  said so in `notes`, or a per-prompt result came in clearly short. */
const EARLY_TOKEN_RATIO = 0.9;
const EARLY_NOTE = "stopped early";

export function stoppedEarly(bench: Benchmark): boolean {
  if (bench.notes?.includes(EARLY_NOTE)) return true;
  return (bench.detail ?? []).some(isShort);
}

/** One per-prompt result that did not reach its own token cap. */
export function isShort(detail: BenchPromptResult): boolean {
  return detail.max_tokens > 0 && detail.tokens < EARLY_TOKEN_RATIO * detail.max_tokens;
}

/** One row per model — the newest run in `rows`, fastest first. `rows` is the
 *  history for a single suite; anything else would compare token caps that are
 *  not the same length. Rows without a `gen_tps` sort last. */
export function latestPerModel(rows: readonly Benchmark[]): Benchmark[] {
  const newest = new Map<string, Benchmark>();
  for (const row of rows) {
    const held = newest.get(row.model_id);
    // Plain `>`: these are ISO-8601 timestamps, where byte order *is*
    // chronological order. A locale collation would reorder them by rules that
    // have nothing to do with time.
    if (!held || row.created_at > held.created_at) newest.set(row.model_id, row);
  }
  return [...newest.values()].sort((a, b) => (b.gen_tps ?? -1) - (a.gen_tps ?? -1));
}

/** The fastest generation rate in a set of rows; `0` when none has one. */
export function fastestTps(rows: readonly Benchmark[]): number {
  return rows.reduce((best, r) => Math.max(best, r.gen_tps ?? 0), 0);
}

/** `0..1` — how long one comparison bar is relative to the fastest row. Used
 *  as a `scaleX` factor, never as a width. */
export function barScale(tps: number | null, fastest: number): number {
  if (tps == null || fastest <= 0) return 0;
  return Math.max(0, Math.min(1, tps / fastest));
}

const DASH = "—";

/** Generation/prefill rate. One decimal — the difference between 61.8 and 62
 *  tok/s is real and reproducible on the same machine. */
export function formatTps(tps: number | null): string {
  return tps == null ? DASH : tps.toFixed(1);
}

/** Prefill is hundreds of tokens a second; a decimal there is noise. */
export function formatRoundTps(tps: number | null): string {
  return tps == null ? DASH : Math.round(tps).toString();
}

/** A `null` load time is not missing data: the model was already resident, so
 *  this run paid no load cost at all. */
export function formatLoad(loadMs: number | null): string {
  return loadMs == null ? "already loaded" : `${(loadMs / 1000).toFixed(1)} s`;
}

export function formatVram(mb: number | null): string {
  return mb == null ? DASH : formatGiB(mb);
}

/** `stability_score` is 0..1; people read consistency as a percentage. */
export function formatStability(score: number): string {
  return `${Math.round(score * 100)}%`;
}

export function formatWhen(iso: string): string {
  const at = new Date(iso);
  return Number.isNaN(at.getTime()) ? iso : at.toLocaleString();
}

/** A prompt's own title, falling back to its id when the row was measured with
 *  a suite version this build no longer ships. */
export function promptTitle(suite: BenchSuite | null, promptId: string): string {
  return suite?.prompts.find((p) => p.id === promptId)?.title ?? promptId;
}
