import { open } from "@tauri-apps/plugin-dialog";

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
