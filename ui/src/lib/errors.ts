/** Turning a rejected IPC call into something worth showing a person.
 *
 *  Tauri rejects with whatever the Rust side serialized — usually a plain
 *  string, sometimes an `Error`. Both reach `catch (e: unknown)`, and both need
 *  the same two steps before they belong on screen. */

/** The core tags a rejected request body with `configuration error: <what>` so
 *  the log line says which layer refused. On screen that prefix is noise: the
 *  reader is looking at the form that produced it. */
const NOISE = /^(Error:\s*)?configuration error:\s*/i;

/** The message to show for a caught IPC rejection: narrowed, de-prefixed, and
 *  never `[object Object]`. */
export function humanize(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e);
  return raw.replace(NOISE, "");
}
