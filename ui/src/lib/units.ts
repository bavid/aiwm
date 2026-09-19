/** One unit convention for the whole UI.
 *
 *  - **Download and disk sizes** are decimal: 1 GB = 10^9 bytes. That is how
 *    Hugging Face, Civitai and every model card quote file sizes, so the
 *    number on an Install button matches the one on the model's page.
 *  - **Memory** (VRAM, RAM) comes from the core in MiB and is shown binary,
 *    labelled "GiB" so it is never mistaken for the decimal unit above. */

const DECIMAL_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** The number part of {@link formatGB}, for a meter that prints its unit
 *  separately: `toGB(1_260_744_467)` → `"1.3"`. */
export function toGB(bytes: number, digits = 1): string {
  return (bytes / 1e9).toFixed(digits);
}

/** Bytes as decimal gigabytes: `formatGB(1_260_744_467)` → `"1.3 GB"`. */
export function formatGB(bytes: number, digits = 1): string {
  return `${toGB(bytes, digits)} GB`;
}

/** Bytes in the best-fitting decimal unit: `1.4 MB`, `26 GB`, `0 B`. One
 *  decimal below 100, none above. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const exponent = Math.min(DECIMAL_UNITS.length - 1, Math.floor(Math.log10(bytes) / 3));
  const value = bytes / 1000 ** exponent;
  const digits = exponent === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${DECIMAL_UNITS[exponent]}`;
}

/** The number part of {@link formatGiB}: `toGiB(6144)` → `"6.0"`. */
export function toGiB(mib: number, digits = 1): string {
  return (mib / 1024).toFixed(digits);
}

/** A MiB memory figure (VRAM/RAM) as GiB: `formatGiB(6144)` → `"6.0 GiB"`. */
export function formatGiB(mib: number, digits = 1): string {
  return `${toGiB(mib, digits)} GiB`;
}
