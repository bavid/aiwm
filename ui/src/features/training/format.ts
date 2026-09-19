/** Display helpers the Training tab's cards and the LoRA history share. Pure
 *  string-in/string-out, so the components stay about layout. */

/** `"1h 05m"`, `"4m 09s"`, `"37s"`; `"—"` for nothing measurable. */
export function formatDuration(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds <= 0) return "—";
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}h ${String(m).padStart(2, "0")}m`;
  if (m > 0) return `${m}m ${String(s).padStart(2, "0")}s`;
  return `${s}s`;
}

/** A store timestamp as a short local date and time (`"17 Sep, 19:11"`) —
 *  enough to tell runs apart on one screen; `"—"` for `null` or garbage. */
export function formatWhen(iso: string | null): string {
  if (!iso) return "—";
  const ms = Date.parse(iso);
  if (!Number.isFinite(ms)) return "—";
  const d = new Date(ms);
  const day = d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return `${day}, ${time}`;
}

/** A count with thousands separators, or `"unknown"` — the overview's word
 *  for a run older than the column that records it (spec: migration 0020). */
export function formatCount(n: number | null): string {
  return n === null ? "unknown" : n.toLocaleString();
}
