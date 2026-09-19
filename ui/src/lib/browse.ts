import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

const errorMessage = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Whether a Tauri IPC bridge is present — the app itself, or the dev mock
 *  (which installs one). A plain browser tab has none. */
function hasTauriBridge(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Opens the OS folder picker and hands the chosen path to `set`. Resolves to
 *  `null` when a folder was picked, the picker was cancelled, or there is no
 *  Tauri at all (a plain browser tab — the user types the path instead); to a
 *  sentence saying why when the picker itself failed, for the caller to show. */
export async function browseForDirectory(set: (path: string) => void): Promise<string | null> {
  if (!hasTauriBridge()) return null;
  try {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string") set(picked);
    return null;
  } catch (e) {
    return `Could not open the folder picker: ${errorMessage(e)}`;
  }
}

/** {@link browseForDirectory} with its failure kept as state: `pick()` opens
 *  the picker, `error` says why the last attempt failed (cleared on retry). */
export function useFolderPicker(onPicked: (path: string) => void): {
  pick: () => void;
  error: string | null;
} {
  const [error, setError] = useState<string | null>(null);
  const pick = () => {
    setError(null);
    void browseForDirectory(onPicked).then(setError);
  };
  return { pick, error };
}

/** Shows `path` in the system file manager. Resolves to `null` on success,
 *  or to a sentence saying why it could not be opened — callers show it
 *  rather than failing silently. */
export async function openFolder(path: string): Promise<string | null> {
  try {
    await revealItemInDir(path);
    return null;
  } catch (e) {
    return `Could not open ${path}: ${errorMessage(e)}`;
  }
}
