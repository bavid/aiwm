import { useCallback, useMemo, useRef, useState } from "react";

/** How a click on a card combines with the current selection — the Windows
 *  Explorer rules: plain click picks one, Ctrl/Cmd toggles, Shift picks the
 *  range from the anchor (Ctrl+Shift adds that range to what is selected). */
export type ClickModifiers = { toggle: boolean; range: boolean };

const EMPTY: ReadonlySet<string> = new Set();

function union(base: ReadonlySet<string>, ids: Iterable<string>): ReadonlySet<string> {
  const next = new Set(base);
  for (const id of ids) next.add(id);
  return next;
}

/** The ids between `from` and `to` (inclusive) in `ordered`, or `null` when
 *  either is not in the list (the anchor may sit in the other column). */
function rangeBetween(ordered: readonly string[], from: string, to: string): string[] | null {
  const a = ordered.indexOf(from);
  const b = ordered.indexOf(to);
  if (a < 0 || b < 0) return null;
  return ordered.slice(Math.min(a, b), Math.max(a, b) + 1);
}

/** The curation board's selection: one `Set` of frame ids shared by both
 *  columns (and by the concept toolbar), so `has` is O(1) per card and a
 *  selection change re-renders only the cards whose flag flipped. */
export function useFrameSelection() {
  const [selected, setSelected] = useState<ReadonlySet<string>>(EMPTY);
  /** Where a Shift-click range starts: the last plain or Ctrl click. */
  const anchor = useRef<string | null>(null);

  /** A click on `id`; `ordered` is the id order of the clicked card's column. */
  const click = useCallback((id: string, ordered: readonly string[], mods: ClickModifiers) => {
    const from = anchor.current;
    if (!mods.range || from === null) anchor.current = id;
    setSelected((cur) => {
      const range = mods.range && from !== null ? rangeBetween(ordered, from, id) : null;
      if (range) return union(mods.toggle ? cur : EMPTY, range);
      if (mods.toggle) {
        const next = new Set(cur);
        if (!next.delete(id)) next.add(id);
        return next;
      }
      return new Set([id]);
    });
  }, []);

  /** Flip one id (the card's checkbox, Space on a focused card). */
  const toggle = useCallback((id: string) => {
    anchor.current = id;
    setSelected((cur) => {
      const next = new Set(cur);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  }, []);

  /** Replace the selection (Select all, a rubber band without modifiers). */
  const replace = useCallback((ids: Iterable<string>) => setSelected(new Set(ids)), []);

  /** Union `ids` onto `base` — a rubber band drawn with Ctrl/Shift held. */
  const extend = useCallback(
    (base: ReadonlySet<string>, ids: Iterable<string>) => setSelected(union(base, ids)),
    [],
  );

  /** Drop `ids` from the selection (a column's Select none). */
  const remove = useCallback((ids: Iterable<string>) => {
    setSelected((cur) => {
      const next = new Set(cur);
      for (const id of ids) next.delete(id);
      return next.size === cur.size ? cur : next;
    });
  }, []);

  const clear = useCallback(() => {
    anchor.current = null;
    setSelected(EMPTY);
  }, []);

  return useMemo(
    () => ({ selected, click, toggle, replace, extend, remove, clear }),
    [selected, click, toggle, replace, extend, remove, clear],
  );
}

export type FrameSelection = ReturnType<typeof useFrameSelection>;
