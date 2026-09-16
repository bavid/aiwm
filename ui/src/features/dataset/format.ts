/** The last path segment of a Windows or POSIX path — the source file name
 *  under a card, and the set header in the Learn view. */
export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/** A clip length as `m:ss`. Seconds are floored, never rounded up: a 12.5 s
 *  clip reads `0:12`, so the label never claims more material than there is. */
export function formatDuration(secs: number): string {
  const total = Math.max(0, Math.floor(secs));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

/** Parses one trim-bound field: `""` is "the clip's natural bound" (`null`),
 *  a finite number is that bound, and anything else is `undefined` — rejected
 *  before it reaches the API. */
export function parseBound(raw: string): number | null | undefined {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : undefined;
}

/** The field text for a stored bound; `null` (natural bound) shows as blank. */
export function boundField(secs: number | null): string {
  return secs === null ? "" : String(secs);
}
