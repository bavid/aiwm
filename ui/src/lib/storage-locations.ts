import { useCallback, useEffect, useState } from "react";
import { storageLocations, type StorageLocation } from "./ipc";

/** A request body with the optional "store in" folder added only when the
 *  curator actually chose one. Blank (or whitespace) means "the default
 *  folder", which the core picks itself — so the field is left out rather
 *  than sent empty. */
export function withDataDir<T extends object>(body: T, draft: string): T & { data_dir?: string } {
  const dir = draft.trim();
  return dir === "" ? body : { ...body, data_dir: dir };
}

/** The drive letter of a Windows path (`"E:"`, upper-cased), or `null` for a
 *  UNC or relative path — whose volume the UI cannot tell from the text. */
export function driveOf(path: string): string | null {
  const m = /^([A-Za-z]):[\\/]/.exec(path.trim());
  return m ? `${m[1].toUpperCase()}:` : null;
}

export type DriveSpace = { drive: string; free: number; total: number | null };

/** Free space of the drive `path` lives on, read off any measured location on
 *  the same drive letter (every location reports its volume). `null` when no
 *  known location shares the drive — the core then checks at start. */
export function driveSpace(path: string, locations: StorageLocation[]): DriveSpace | null {
  const drive = driveOf(path);
  if (!drive) return null;
  const hit = locations.find(
    (l) => l.volume_free_bytes !== null && driveOf(l.path) === drive,
  );
  if (!hit || hit.volume_free_bytes === null) return null;
  return { drive, free: hit.volume_free_bytes, total: hit.volume_total_bytes };
}

/** How long a measurement is reused by the forms before walking again. The
 *  report is a recursive walk of every app folder, so opening the Dataset or
 *  Training tab must not trigger one each time. Settings' Refresh bypasses it. */
const REUSE_MS = 60_000;

let cached: { at: number; request: Promise<StorageLocation[]> } | null = null;

function loadLocations(fresh: boolean): Promise<StorageLocation[]> {
  if (!fresh && cached && Date.now() - cached.at < REUSE_MS) return cached.request;
  const request = storageLocations();
  cached = { at: Date.now(), request };
  // A failed walk must not be reused.
  request.catch(() => {
    if (cached?.request === request) cached = null;
  });
  return request;
}

export type StorageLocationsState = {
  data: StorageLocation[] | null;
  error: string | null;
  isLoading: boolean;
  /** Walk the folders again, ignoring any recent measurement. */
  refresh: () => void;
};

/** The storage report, fetched once on mount (a recent one is reused) and
 *  again on `refresh()` — never on a timer. */
export function useStorageLocations(): StorageLocationsState {
  const [data, setData] = useState<StorageLocation[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [generation, setGeneration] = useState(0);

  useEffect(() => {
    let isCurrent = true;
    setIsLoading(true);
    setError(null);
    loadLocations(generation > 0)
      .then((rows) => {
        if (isCurrent) setData(rows);
      })
      .catch((e: unknown) => {
        if (isCurrent) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (isCurrent) setIsLoading(false);
      });
    return () => {
      isCurrent = false;
    };
  }, [generation]);

  const refresh = useCallback(() => setGeneration((g) => g + 1), []);
  return { data, error, isLoading, refresh };
}
