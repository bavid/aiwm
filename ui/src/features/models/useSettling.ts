import { useCallback, useEffect, useRef, useState } from "react";

/** How long a just-finished stack may wait for the captioner registry to
 *  confirm it before a "not usable" verdict is shown (one registry poll plus
 *  slack). */
export const SETTLE_MS = 8000;

export interface Settling {
  /** Stack ids that just finished and await the registry's confirmation. */
  ids: ReadonlySet<string>;
  /** Start (or restart) the settle window for a stack. */
  settle: (id: string) => void;
  /** End it early — the registry confirmed. */
  clear: (id: string) => void;
}

function withoutId(set: ReadonlySet<string>, id: string): ReadonlySet<string> {
  return set.has(id) ? new Set([...set].filter((x) => x !== id)) : set;
}

/** A per-stack "just finished, registry not caught up yet" window, so a
 *  normal install never flashes "Files present but not usable". Its timers
 *  are cleared on unmount. */
export function useSettling(): Settling {
  const [ids, setIds] = useState<ReadonlySet<string>>(new Set());
  const timers = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  useEffect(() => {
    const pending = timers.current;
    return () => {
      for (const t of pending.values()) clearTimeout(t);
      pending.clear();
    };
  }, []);

  const clear = useCallback((id: string) => {
    const t = timers.current.get(id);
    if (t !== undefined) clearTimeout(t);
    timers.current.delete(id);
    setIds((prev) => withoutId(prev, id));
  }, []);

  const settle = useCallback(
    (id: string) => {
      const old = timers.current.get(id);
      if (old !== undefined) clearTimeout(old);
      timers.current.set(
        id,
        setTimeout(() => clear(id), SETTLE_MS),
      );
      setIds((prev) => (prev.has(id) ? prev : new Set(prev).add(id)));
    },
    [clear],
  );

  return { ids, settle, clear };
}
