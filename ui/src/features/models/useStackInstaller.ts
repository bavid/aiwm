import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { humanize } from "../../lib/errors";
import { useDownloads, useModels } from "../../lib/hooks";
import { enqueueDownload, resumeDownload, type ModelStack } from "../../lib/ipc";
import {
  knownModelDownload,
  pendingMembers,
  stackProgress,
  type MemberProgress,
  type StackPhase,
  type StackProgress,
} from "./stack-install";

export interface StackInstaller {
  /** Progress per stack id; empty until the library and queue have loaded. */
  progress: ReadonlyMap<string, StackProgress>;
  /** Why the last Install click failed to start, per stack id. */
  errors: Readonly<Record<string, string>>;
  /** Why the download queue or the model library could not be read — shown
   *  instead of an endless "Checking…". `null` while both load fine. */
  loadError: string | null;
  /** Stack ids whose Install click has not yet shown up in a queue snapshot. */
  starting: ReadonlySet<string>;
  /** Start what is missing (queue) or stopped (resume); `false` when the
   *  start failed (the reason is in `errors`) or was a repeat click. */
  install: (stack: ModelStack) => Promise<boolean>;
  /** Fetch every file again, installed or not — for a stack whose files are
   *  all present but that the core does not accept as usable. */
  redownload: (stack: ModelStack) => Promise<boolean>;
}

const OFFLINE_REFUSAL = /offline mode is on/i;

/** The refusal as the person should read it: the core's offline gate gets a
 *  sentence that says what to do; anything else is shown as the core said it. */
function installErrorText(e: unknown): string {
  const text = humanize(e);
  return OFFLINE_REFUSAL.test(text)
    ? "Offline mode is on. Turn it off in Settings to install."
    : text;
}

function without<T>(record: Readonly<Record<string, T>>, key: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => k !== key));
}

function withoutId(set: ReadonlySet<string>, id: string): ReadonlySet<string> {
  return new Set([...set].filter((x) => x !== id));
}

/** One-click install for catalog stacks through the regular download queue
 *  (verify → import with the kind's default role), exactly like "Download
 *  entire stack" on the Models tab. Polls the queue and the library once for
 *  every stack it is given, and calls `onInstalled` when a stack this
 *  component watched go from not-installed to installed.
 *
 *  Never queues a file twice: a click is ignored while the same stack's
 *  previous click is still starting (synchronous ref, so a double click
 *  cannot slip between renders), the button stays replaced until a queue
 *  snapshot shows the files in flight, and the core hands back the active
 *  download for a file another tab already queued. */
export function useStackInstaller(
  stacks: readonly ModelStack[],
  onInstalled?: (stack: ModelStack) => void,
): StackInstaller {
  const { data: downloads, error: downloadsError, refetch: refetchDownloads } = useDownloads();
  const { data: models, error: modelsError, refetch: refetchModels } = useModels();
  const [errors, setErrors] = useState<Readonly<Record<string, string>>>({});
  const [starting, setStarting] = useState<ReadonlySet<string>>(new Set());
  const startingRef = useRef<Set<string>>(new Set());
  /** Per starting stack, the download ids its click created or resumed and
   *  the queue-snapshot count when the start resolved. */
  const awaited = useRef<Map<string, { ids: ReadonlySet<string>; afterSnapshot: number }>>(
    new Map(),
  );
  /** Counts queue snapshots (each poll result is a new array). */
  const snapshots = useRef(0);

  const endStarting = useCallback((id: string) => {
    startingRef.current.delete(id);
    awaited.current.delete(id);
    setStarting((prev) => withoutId(prev, id));
  }, []);

  const progress = useMemo(() => {
    const map = new Map<string, StackProgress>();
    if (!downloads || !models) return map;
    for (const s of stacks) map.set(s.id, stackProgress(s, downloads, models));
    return map;
  }, [stacks, downloads, models]);

  // A click stays "starting" until the first queue snapshot fetched after the
  // start resolved that contains every download it created or resumed —
  // whatever their state, so a file that already failed shows its failure
  // instead of "Starting…". A row can also be gone by then (cancelled,
  // cleared from the history), so the second snapshot after the start ends
  // it regardless.
  useEffect(() => {
    if (!downloads) return;
    snapshots.current += 1;
    const seen = new Set(downloads.map((d) => d.id));
    for (const [stackId, { ids, afterSnapshot }] of awaited.current) {
      const newer = snapshots.current - afterSnapshot;
      if (newer < 1) continue;
      if (newer >= 2 || [...ids].every((id) => seen.has(id))) endStarting(stackId);
    }
  }, [downloads, endStarting]);

  // The queue could not be read: no snapshot will confirm a start, so end
  // every "Starting…" and let the load error show instead.
  useEffect(() => {
    if (!downloadsError) return;
    for (const id of [...startingRef.current]) endStarting(id);
  }, [downloadsError, endStarting]);

  // Completion is observed, not assumed: only a transition seen by this
  // component counts, so a stack that was already installed on first load
  // never fires `onInstalled`.
  const lastPhase = useRef<ReadonlyMap<string, StackPhase>>(new Map());
  useEffect(() => {
    const next = new Map<string, StackPhase>();
    for (const s of stacks) {
      const p = progress.get(s.id);
      if (!p) continue;
      const before = lastPhase.current.get(s.id);
      if (before !== undefined && before !== "installed" && p.phase === "installed") {
        refetchModels();
        onInstalled?.(s);
      }
      next.set(s.id, p.phase);
    }
    if (next.size > 0) lastPhase.current = next;
  }, [progress, stacks, onInstalled, refetchModels]);

  const start = useCallback(
    async (stack: ModelStack, members: readonly MemberProgress[]): Promise<boolean> => {
      if (startingRef.current.has(stack.id)) return false;
      startingRef.current.add(stack.id);
      setErrors((prev) => without(prev, stack.id));
      setStarting((prev) => new Set(prev).add(stack.id));
      const ids = new Set<string>();
      try {
        for (const m of members) {
          if ((m.phase === "failed" || m.phase === "paused") && m.downloadId) {
            await resumeDownload(m.downloadId);
            ids.add(m.downloadId);
          } else {
            // The core returns the already-active download when another tab
            // queued this file first — that counts as "already downloading".
            const d = await enqueueDownload(knownModelDownload(m.member));
            ids.add(d.id);
          }
        }
        if (ids.size === 0) endStarting(stack.id);
        else awaited.current.set(stack.id, { ids, afterSnapshot: snapshots.current });
        return true;
      } catch (e) {
        endStarting(stack.id);
        setErrors((prev) => ({ ...prev, [stack.id]: installErrorText(e) }));
        return false;
      } finally {
        refetchDownloads();
      }
    },
    [endStarting, refetchDownloads],
  );

  const install = useCallback(
    async (stack: ModelStack) => {
      const p = progress.get(stack.id);
      return p ? start(stack, pendingMembers(p)) : false;
    },
    [progress, start],
  );

  const redownload = useCallback(
    async (stack: ModelStack) => {
      const p = progress.get(stack.id);
      // Every file, as a fresh download (installed ones included).
      const all = p?.members.map((m) => ({ ...m, phase: "missing" as const, downloadId: null }));
      return all ? start(stack, all) : false;
    },
    [progress, start],
  );

  const loadError = downloadsError ?? modelsError;
  return {
    progress,
    errors,
    loadError: loadError ? humanize(loadError) : null,
    starting,
    install,
    redownload,
  };
}
