import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

/** Opens the OS folder picker and hands the chosen path to `set`. Outside
 *  Tauri (the browser dev preview) there is no dialog, so the call is a no-op
 *  and the curator types the path instead. */
export async function browseForDirectory(set: (path: string) => void): Promise<void> {
  try {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string") set(picked);
  } catch {
    // Not running inside Tauri (e.g. the browser dev preview) -- no-op.
  }
}

/** Shows `path` in the system file manager. Resolves to `null` on success,
 *  or to a sentence saying why it could not be opened — callers show it
 *  rather than failing silently. */
export async function openFolder(path: string): Promise<string | null> {
  try {
    await revealItemInDir(path);
    return null;
  } catch (e) {
    const why = e instanceof Error ? e.message : String(e);
    return `Could not open ${path}: ${why}`;
  }
}
