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

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** A byte count for people: `1.4 MB`, `26 GB`, `0 B` (binary steps, the way
 *  Windows Explorer counts). One decimal below 100, none above. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const exponent = Math.min(BYTE_UNITS.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  const value = bytes / 1024 ** exponent;
  const digits = exponent === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${BYTE_UNITS[exponent]}`;
}

/** The field text for a stored bound; `null` (natural bound) shows as blank. */
export function boundField(secs: number | null): string {
  return secs === null ? "" : String(secs);
}
