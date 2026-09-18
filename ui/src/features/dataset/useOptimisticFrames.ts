import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { DatasetFrame } from "../../lib/ipc";

/** A move the board shows before the server has confirmed it. */
type Override = {
  excluded: boolean;
  batch: number;
  /** The request succeeded; drop the override once the polled rows agree. */
  isSettled: boolean;
  /** Frame-list updates seen since settling — a poll that left before the
   *  write can land after it, so one disagreeing update is tolerated. */
  staleUpdates: number;
};

/** Polls that may still show the pre-move row after a successful write. */
const MAX_STALE_UPDATES = 2;

function agrees(frame: DatasetFrame, o: Override): boolean {
  // Keeping also clears the filter verdict (the core does that in one write).
  return o.excluded ? frame.excluded : !frame.excluded && frame.rejection_reason === "";
}

function applyOverride(frame: DatasetFrame, o: Override): DatasetFrame {
  return o.excluded
    ? { ...frame, excluded: true }
    : { ...frame, excluded: false, rejection_reason: "" };
}

/** The polled frame list with the curator's in-flight moves laid over it, so
 *  a move shows instantly and rolls back cleanly on failure. Untouched rows
 *  keep their identity, which keeps `FrameCard`'s memoisation intact. */
export function useOptimisticFrames(frames: readonly DatasetFrame[]) {
  const [overrides, setOverrides] = useState<ReadonlyMap<string, Override>>(() => new Map());
  const batchSeq = useRef(0);

  // Each new poll result retires the overrides it confirms (and those of
  // rows that are gone).
  useEffect(() => {
    const byId = new Map(frames.map((f) => [f.id, f]));
    setOverrides((cur) => {
      if (cur.size === 0) return cur;
      const next = new Map<string, Override>();
      for (const [id, o] of cur) {
        const frame = byId.get(id);
        if (!frame) continue;
        if (!o.isSettled) next.set(id, o);
        else if (!agrees(frame, o) && o.staleUpdates < MAX_STALE_UPDATES) {
          next.set(id, { ...o, staleUpdates: o.staleUpdates + 1 });
        }
      }
      return next;
    });
  }, [frames]);

  const effective = useMemo(() => {
    if (overrides.size === 0) return frames;
    return frames.map((f) => {
      const o = overrides.get(f.id);
      return o ? applyOverride(f, o) : f;
    });
  }, [frames, overrides]);

  /** Shows `ids` as moved; returns the batch to settle or roll back. */
  const beginMove = useCallback((ids: readonly string[], excluded: boolean): number => {
    batchSeq.current += 1;
    const batch = batchSeq.current;
    setOverrides((cur) => {
      const next = new Map(cur);
      for (const id of ids) next.set(id, { excluded, batch, isSettled: false, staleUpdates: 0 });
      return next;
    });
    return batch;
  }, []);

  const settleMove = useCallback((batch: number) => {
    setOverrides((cur) => {
      const next = new Map(cur);
      for (const [id, o] of cur) if (o.batch === batch) next.set(id, { ...o, isSettled: true });
      return next;
    });
  }, []);

  const rollbackMove = useCallback((batch: number) => {
    setOverrides((cur) => new Map([...cur].filter(([, o]) => o.batch !== batch)));
  }, []);

  return { frames: effective, beginMove, settleMove, rollbackMove };
}
