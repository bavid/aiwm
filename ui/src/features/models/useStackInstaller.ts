import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { humanize } from "../../lib/errors";
import { useDownloads, useModels } from "../../lib/hooks";
import { enqueueDownload, resumeDownload, type ModelStack } from "../../lib/ipc";
import {
  isUnderway,
  pendingMembers,
  stackProgress,
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
  install: (stack: ModelStack) => Promise<void>;
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

  const progress = useMemo(() => {
    const map = new Map<string, StackProgress>();
    if (!downloads || !models) return map;
    for (const s of stacks) map.set(s.id, stackProgress(s, downloads, models));
    return map;
  }, [stacks, downloads, models]);

  // A click stays "starting" until a snapshot shows it took effect.
  useEffect(() => {
    for (const id of startingRef.current) {
      const p = progress.get(id);
      if (p && isUnderway(p)) {
        startingRef.current.delete(id);
        setStarting((prev) => withoutId(prev, id));
      }
    }
  }, [progress]);

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

  const install = useCallback(
    async (stack: ModelStack) => {
      const p = progress.get(stack.id);
      if (!p || startingRef.current.has(stack.id)) return;
      startingRef.current.add(stack.id);
      setErrors((prev) => without(prev, stack.id));
      setStarting((prev) => new Set(prev).add(stack.id));
      try {
        for (const m of pendingMembers(p)) {
          if ((m.phase === "failed" || m.phase === "paused") && m.downloadId) {
            await resumeDownload(m.downloadId);
          } else {
            // The core returns the already-active download when another tab
            // queued this file first — that counts as "already downloading".
            await enqueueDownload({
              url: m.member.url,
              filename: m.member.file,
              model_type: m.member.kind,
              sha256: m.member.sha256,
              size_bytes: m.member.size_bytes,
            });
          }
        }
      } catch (e) {
        startingRef.current.delete(stack.id);
        setStarting((prev) => withoutId(prev, stack.id));
        setErrors((prev) => ({ ...prev, [stack.id]: installErrorText(e) }));
      } finally {
        refetchDownloads();
      }
    },
    [progress, refetchDownloads],
  );

  const loadError = downloadsError ?? modelsError;
  return { progress, errors, loadError: loadError ? humanize(loadError) : null, starting, install };
}
