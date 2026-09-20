import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { findSetting, type HelpArea } from "../help/index.ts";
import { useOpenHelp } from "../help/HelpContext.ts";
import "./help-hint.css";

type Props = {
  area: HelpArea;
  /** A `HelpSetting.key` of that area; `scripts/check-help.mjs` verifies it. */
  setting: string;
  /** The DOM id of the control this hint explains. While the hint is
   *  collapsed, that control gets an `aria-describedby` pointing at a
   *  visually-hidden copy of the "what" line, so assistive tech hears the
   *  one-line answer without a click; open, the panel is on screen and the
   *  description is dropped, so nothing is announced twice. */
  describes?: string;
};

/** Gap between the button and the panel, and the panel's minimum distance
 *  from the window edges, in px. */
const GAP = 4;
const EDGE = 8;

/** Removes our description token from `el`'s `aria-describedby` and, when
 *  `add` is set, appends it. Attribute-only, so the host component's own
 *  `aria-describedby` (an error line, a help paragraph) is kept. */
function syncDescribedBy(el: Element, token: string, add: boolean) {
  const tokens = (el.getAttribute("aria-describedby") ?? "")
    .split(/\s+/)
    .filter((t) => t !== "" && t !== token);
  if (add) tokens.push(token);
  if (tokens.length > 0) el.setAttribute("aria-describedby", tokens.join(" "));
  else el.removeAttribute("aria-describedby");
}

/** Places `panel` (position: fixed) under `button`'s left edge, in the
 *  viewport: to the left of the button's right edge when it would run off
 *  the right side, above the button when it would run off the bottom. */
function placePanel(button: HTMLElement, panel: HTMLElement) {
  const r = button.getBoundingClientRect();
  const w = panel.offsetWidth;
  const h = panel.offsetHeight;
  let left = r.left;
  if (left + w > window.innerWidth - EDGE) left = Math.max(EDGE, r.right - w);
  let top = r.bottom + GAP;
  if (top + h > window.innerHeight - EDGE && r.top - GAP - h >= EDGE) top = r.top - GAP - h;
  panel.style.left = `${Math.round(left)}px`;
  panel.style.top = `${Math.round(top)}px`;
}

/** The `?` next to a setting: a real disclosure, not a `title=` tooltip — a
 *  tooltip never reaches keyboard users, is announced unreliably by screen
 *  readers and does not exist on touch (see `FitBadge` for the same
 *  argument). Enter/Space toggle (a plain `<button>`), Escape closes and
 *  returns focus to the button, a press outside closes, "More in Help" jumps
 *  to the setting on the Help tab. The text comes from `src/help/`, the same
 *  source the Help tab renders.
 *
 *  The panel is rendered through a portal into `document.body` and
 *  positioned from the button's rectangle: hosts include scrolling columns
 *  and `overflow: hidden` cards, which would clip an in-place panel. Focus
 *  stays on the button; the panel is tied to it by `aria-controls` and,
 *  while open, `aria-details`, so a screen reader can reach it although it
 *  is not adjacent in the DOM. Any scroll outside the panel closes it, and
 *  so does the button leaving view (its tab hidden, its column scrolled),
 *  so a fixed panel never floats away from its trigger. */
export function HelpHint({ area, setting, describes }: Props) {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const descId = useId();
  const buttonRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const openHelp = useOpenHelp();
  const found = findSetting(area, setting);

  // Position before paint on every open, and again when the window resizes.
  useLayoutEffect(() => {
    const button = buttonRef.current;
    const panel = panelRef.current;
    if (!open || !button || !panel) return;
    placePanel(button, panel);
    const onResize = () => placePanel(button, panel);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [open]);

  // A popover that stays open while the reader works elsewhere would sit on
  // top of that work: close it on any pointer press outside the button and
  // the panel, and on any scroll that is not the panel's own. (Escape and
  // the button itself handle the keyboard; focus is left where it is.)
  useEffect(() => {
    if (!open) return;
    const isOurs = (target: EventTarget | null) =>
      target instanceof Node &&
      (buttonRef.current?.contains(target) || panelRef.current?.contains(target));
    const onPointerDown = (e: PointerEvent) => {
      if (!isOurs(e.target)) setOpen(false);
    };
    const onScroll = (e: Event) => {
      if (!isOurs(e.target)) setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("scroll", onScroll, true);
    // The panel lives in <body>, so a `hidden` ancestor of the button (the
    // tab switching away, a collapsed section) does not hide it: close it
    // whenever the button itself is no longer visible.
    const observer = new IntersectionObserver(([entry]) => {
      if (entry && !entry.isIntersecting) setOpen(false);
    });
    if (buttonRef.current) observer.observe(buttonRef.current);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("scroll", onScroll, true);
      observer.disconnect();
    };
  }, [open]);

  // Keep our token in the described control's `aria-describedby` while
  // collapsed and out of it while open. Deliberately no dependency array:
  // the control belongs to the host component, which may rewrite the
  // attribute on any of its own renders (e.g. an error id appearing), and a
  // HelpHint re-renders whenever its host does — so re-asserting after every
  // render is what keeps the token from being dropped. Idempotent and cheap.
  useEffect(() => {
    if (!describes) return;
    const el = document.getElementById(describes);
    if (el) syncDescribedBy(el, descId, !open);
  });

  useEffect(() => {
    if (!describes) return;
    return () => {
      const el = document.getElementById(describes);
      if (el) syncDescribedBy(el, descId, false);
    };
  }, [describes, descId]);

  if (!found) return null;
  const { setting: s } = found;

  // Reaches this handler from the panel too: a portal's React events bubble
  // through the React tree, not the DOM.
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key !== "Escape" || !open) return;
    e.preventDefault();
    e.stopPropagation();
    setOpen(false);
    buttonRef.current?.focus();
  };

  const panel = (
    // The panel only catches Escape bubbling from its own "More in Help"
    // button; it is not itself a control (jsx-a11y's documented exception).
    // eslint-disable-next-line jsx-a11y/no-static-element-interactions
    <div
      ref={panelRef}
      id={panelId}
      className="helphint__panel"
      hidden={!open}
      onKeyDown={onKeyDown}
    >
      <p className="helphint__title">{s.label}</p>
      <dl className="helphint__qa">
        <dt>What does it do?</dt>
        <dd>{s.what}</dd>
        <dt>Why do I need it?</dt>
        <dd>{s.why}</dd>
        <dt>What happens when I change or start it?</dt>
        <dd>{s.effect}</dd>
        <dt>What do I gain?</dt>
        <dd>{s.benefit}</dd>
        {s.pitfalls && (
          <>
            <dt>Watch out</dt>
            <dd>{s.pitfalls}</dd>
          </>
        )}
        {s.measured && (
          <>
            <dt>Measured</dt>
            <dd className="numeric">{s.measured}</dd>
          </>
        )}
      </dl>
      {openHelp && (
        <button
          type="button"
          className="chip helphint__more"
          onClick={() => {
            setOpen(false);
            openHelp(area, setting);
          }}
        >
          More in Help
        </button>
      )}
    </div>
  );

  return (
    <span className="helphint">
      <button
        ref={buttonRef}
        type="button"
        className="helphint__btn"
        aria-label={`Help: ${s.label}`}
        aria-expanded={open}
        aria-controls={panelId}
        aria-details={open ? panelId : undefined}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={onKeyDown}
      >
        <span aria-hidden="true">?</span>
      </button>
      {!open && describes && (
        <span id={descId} className="visually-hidden">
          {s.what}
        </span>
      )}
      {createPortal(panel, document.body)}
    </span>
  );
}
