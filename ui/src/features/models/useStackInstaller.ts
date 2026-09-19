import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { humanize } from "../../lib/errors";
import { useDownloads, useModels } from "../../lib/hooks";
import { enqueueDownload, resumeDownload, type ModelStack } from "../../lib/ipc";
import { pendingMembers, stackProgress, type StackPhase, type StackProgress } from "./stack-install";

export interface StackInstaller {
  /** Progress per stack id; empty until the library and queue have loaded. */
  progress: ReadonlyMap<string, StackProgress>;
  /** Why the last Install click failed to start, per stack id — the core's
   *  own message (e.g. the offline-mode refusal), shown verbatim. */
  errors: Readonly<Record<string, string>>;
  /** Stack ids whose Install click is still queuing its files. */
  starting: ReadonlySet<string>;
  install: (stack: ModelStack) => Promise<void>;
}

function without<T>(record: Readonly<Record<string, T>>, key: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => k !== key));
}

/** One-click install for catalog stacks through the regular download queue
 *  (verify → import with the kind's default role), exactly like "Download
 *  entire stack" on the Models tab. Polls the queue and the library once for
 *  every stack it is given, and calls `onInstalled` when a stack this
 *  component watched go from not-installed to installed. */
export function useStackInstaller(
  stacks: readonly ModelStack[],
  onInstalled?: (stack: ModelStack) => void,
): StackInstaller {
  const { data: downloads, refetch: refetchDownloads } = useDownloads();
  const { data: models, refetch: refetchModels } = useModels();
  const [errors, setErrors] = useState<Readonly<Record<string, string>>>({});
  const [starting, setStarting] = useState<ReadonlySet<string>>(new Set());

  const progress = useMemo(() => {
    const map = new Map<string, StackProgress>();
    if (!downloads || !models) return map;
    for (const s of stacks) map.set(s.id, stackProgress(s, downloads, models));
    return map;
  }, [stacks, downloads, models]);

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
      if (!p) return;
      setErrors((prev) => without(prev, stack.id));
      setStarting((prev) => new Set(prev).add(stack.id));
      try {
        for (const m of pendingMembers(p)) {
          if (m.phase === "failed" && m.downloadId) {
            await resumeDownload(m.downloadId);
          } else {
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
        setErrors((prev) => ({ ...prev, [stack.id]: humanize(e) }));
      } finally {
        setStarting((prev) => new Set([...prev].filter((id) => id !== stack.id)));
        refetchDownloads();
      }
    },
    [progress, refetchDownloads],
  );

  return { progress, errors, starting, install };
}
