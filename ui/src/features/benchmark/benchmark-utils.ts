/** Pure helpers behind the Benchmark tab — formatting, the "latest run per
 *  model" reduction the comparison table is built on, and the tolerant parse
 *  of the per-pass job event. No React, no IPC: everything here is a plain
 *  function over what `core::bench` already recorded. */

import {
  isBenchmarkable,
  type BenchPromptResult,
  type Benchmark,
  type BenchSuite,
  type Model,
} from "../../lib/ipc";

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

  const head = message.slice(0, m.index).replace(LABEL_TAIL, "");
  const dot = head.lastIndexOf(LABEL_SEPARATOR);
  const title = (dot >= 0 ? head.slice(dot + LABEL_SEPARATOR.length) : head).trim();
  return { promptTitle: title === "" ? null : title, pass, runs, tps };
}

/** The passes-per-prompt the form offers. The core clamps to `1..=10`; five is
 *  as far as this tab goes before a run stops being a quick check. */
export const MIN_RUNS = 1;
export const MAX_RUNS = 5;
/** Matches `core::bench`'s own default for a suite run. */
export const DEFAULT_RUNS = 2;

/** How many generation passes a suite run of `runs` passes per prompt means in
 *  total — the denominator the live panel counts towards. */
export function totalPasses(suite: BenchSuite | null, runs: number): number {
  return suite ? suite.prompts.length * Math.max(0, runs) : 0;
}

/** The same count for a job already in flight, read back from its own params
 *  rather than from the form — the form may have moved on since it started. */
export function jobPasses(params: unknown, suites: readonly BenchSuite[]): number {
  if (params == null || typeof params !== "object") return 0;
  const { suite: suiteId, runs } = params as { suite?: unknown; runs?: unknown };
  const suite = suites.find((s) => s.id === suiteId) ?? null;
  const asked = Number(runs);
  return totalPasses(suite, Number.isFinite(asked) ? asked : DEFAULT_RUNS);
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
    if (!held || row.created_at.localeCompare(held.created_at) > 0) newest.set(row.model_id, row);
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
  return mb == null ? DASH : `${(mb / 1024).toFixed(1)} GB`;
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
