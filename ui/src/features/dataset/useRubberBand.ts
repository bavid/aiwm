import { useCallback, useEffect, useRef, useState, type PointerEvent } from "react";

/** A rectangle in the container's own coordinates (px from its top-left). */
export type Box = { left: number; top: number; width: number; height: number };

type CardRect = { id: string; left: number; top: number; right: number; bottom: number };

type Drag = {
  pointerId: number;
  startX: number;
  startY: number;
  /** Card rectangles, measured once when the drag starts. */
  cards: CardRect[];
  additive: boolean;
  isActive: boolean;
  lastHit: string;
  frame: number;
  x: number;
  y: number;
};

type Handlers = {
  /** The band became a real drag (moved past the threshold). */
  onStart: (additive: boolean) => void;
  /** The ids of every card the band intersects, whenever that set changes. */
  onChange: (ids: string[]) => void;
  /** The band was released over these cards. */
  onEnd: (ids: string[]) => void;
  /** A plain click on empty space — Explorer clears the selection then. */
  onEmptyClick: () => void;
  /** What takes keyboard focus when a band starts (so the column's shortcuts
   *  keep working); the press itself is kept from focusing anything. */
  focusTarget: () => HTMLElement | null;
};

/** Pixels the pointer must travel before a press on empty space becomes a
 *  band, so a click stays a click. */
const THRESHOLD_PX = 4;

/** Anything a press should reach instead of starting a band: a card (its
 *  body is not empty space) and every control. */
const NOT_EMPTY =
  "[data-frame-id], input, textarea, select, button, label, a, [contenteditable='true']";

/** Card elements carry `data-frame-id`; their rects are taken relative to the
 *  container so a scroll of the page during the drag does not skew them. */
function measureCards(container: HTMLElement): CardRect[] {
  const origin = container.getBoundingClientRect();
  const cards: CardRect[] = [];
  for (const el of container.querySelectorAll<HTMLElement>("[data-frame-id]")) {
    const id = el.dataset.frameId;
    if (!id) continue;
    const r = el.getBoundingClientRect();
    cards.push({
      id,
      left: r.left - origin.left,
      top: r.top - origin.top,
      right: r.right - origin.left,
      bottom: r.bottom - origin.top,
    });
  }
  return cards;
}

function boxOf(drag: Drag): Box {
  return {
    left: Math.min(drag.startX, drag.x),
    top: Math.min(drag.startY, drag.y),
    width: Math.abs(drag.x - drag.startX),
    height: Math.abs(drag.y - drag.startY),
  };
}

function hitIds(cards: CardRect[], box: Box): string[] {
  const right = box.left + box.width;
  const bottom = box.top + box.height;
  return cards
    .filter((c) => c.left < right && c.right > box.left && c.top < bottom && c.bottom > box.top)
    .map((c) => c.id);
}

/** Rubber-band selection over one card container: press on empty space and
 *  draw a rectangle; every card it touches is reported. Card rects are
 *  measured once per drag, hit-testing runs at most once per animation frame,
 *  and the only layout read per frame is the container's own rect. */
export function useRubberBand({ onStart, onChange, onEnd, onEmptyClick, focusTarget }: Handlers) {
  const [box, setBox] = useState<Box | null>(null);
  const drag = useRef<Drag | null>(null);
  const handlers = useRef({ onStart, onChange, onEnd, onEmptyClick, focusTarget });
  useEffect(() => {
    handlers.current = { onStart, onChange, onEnd, onEmptyClick, focusTarget };
  }, [onStart, onChange, onEnd, onEmptyClick, focusTarget]);

  useEffect(
    () => () => {
      if (drag.current) cancelAnimationFrame(drag.current.frame);
    },
    [],
  );

  const onPointerDown = useCallback((e: PointerEvent<HTMLElement>) => {
    if (e.button !== 0 || !(e.target instanceof Element) || e.target.closest(NOT_EMPTY)) return;
    const container = e.currentTarget;
    const origin = container.getBoundingClientRect();
    const x = e.clientX - origin.left;
    const y = e.clientY - origin.top;
    drag.current = {
      pointerId: e.pointerId,
      startX: x,
      startY: y,
      x,
      y,
      cards: measureCards(container),
      additive: e.ctrlKey || e.metaKey || e.shiftKey,
      isActive: false,
      lastHit: "",
      frame: 0,
    };
    container.setPointerCapture(e.pointerId);
    // No text selection while drawing; keep keyboard shortcuts on this column.
    e.preventDefault();
    handlers.current.focusTarget()?.focus({ preventScroll: true });
  }, []);

  const onPointerMove = useCallback((e: PointerEvent<HTMLElement>) => {
    const d = drag.current;
    if (!d || d.pointerId !== e.pointerId) return;
    const origin = e.currentTarget.getBoundingClientRect();
    d.x = e.clientX - origin.left;
    d.y = e.clientY - origin.top;
    if (!d.isActive) {
      if (Math.hypot(d.x - d.startX, d.y - d.startY) < THRESHOLD_PX) return;
      d.isActive = true;
      handlers.current.onStart(d.additive);
    }
    if (d.frame) return;
    d.frame = requestAnimationFrame(() => {
      d.frame = 0;
      if (drag.current !== d) return;
      const next = boxOf(d);
      setBox(next);
      const ids = hitIds(d.cards, next);
      const key = ids.join(",");
      if (key === d.lastHit) return;
      d.lastHit = key;
      handlers.current.onChange(ids);
    });
  }, []);

  const finish = useCallback((e: PointerEvent<HTMLElement>, isCancel: boolean) => {
    const d = drag.current;
    if (!d || d.pointerId !== e.pointerId) return;
    cancelAnimationFrame(d.frame);
    drag.current = null;
    setBox(null);
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    if (isCancel) return;
    if (d.isActive) {
      // The last move may still be waiting for its frame -- settle it now.
      const ids = hitIds(d.cards, boxOf(d));
      handlers.current.onChange(ids);
      handlers.current.onEnd(ids);
    } else if (!d.additive) {
      handlers.current.onEmptyClick();
    }
  }, []);

  const onPointerUp = useCallback((e: PointerEvent<HTMLElement>) => finish(e, false), [finish]);
  const onPointerCancel = useCallback((e: PointerEvent<HTMLElement>) => finish(e, true), [finish]);

  return { box, onPointerDown, onPointerMove, onPointerUp, onPointerCancel };
}
