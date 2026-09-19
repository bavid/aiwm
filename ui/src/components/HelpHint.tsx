import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
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

/** The `?` next to a setting: a real disclosure, not a `title=` tooltip — a
 *  tooltip never reaches keyboard users, is announced unreliably by screen
 *  readers and does not exist on touch (see `FitBadge` for the same
 *  argument). Enter/Space toggle (a plain `<button>`), Escape closes and
 *  returns focus to the button, a press outside closes, "More in Help" jumps
 *  to the setting on the Help tab. The panel is a popover in the DOM right
 *  after the button (`aria-controls`), so reading order and focus order are
 *  the same. The text comes from `src/help/`, the same source the Help tab
 *  renders. */
export function HelpHint({ area, setting, describes }: Props) {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const descId = useId();
  const buttonRef = useRef<HTMLButtonElement>(null);
  const wrapRef = useRef<HTMLSpanElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const openHelp = useOpenHelp();
  const found = findSetting(area, setting);

  // The panel is a popover anchored at the button's left edge; near the
  // right edge of the window it would run off screen, so anchor it at the
  // right edge instead. Measured before paint, on every open.
  useLayoutEffect(() => {
    const panel = panelRef.current;
    if (!open || !panel) return;
    panel.removeAttribute("data-align");
    const overflows = panel.getBoundingClientRect().right > window.innerWidth - 8;
    if (overflows) panel.setAttribute("data-align", "end");
  }, [open]);

  // A popover that stays open while the reader works elsewhere would sit on
  // top of that work: close it on any pointer press outside it. (Escape and
  // the button itself handle the keyboard; focus is left where it is.)
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (e.target instanceof Node && wrapRef.current?.contains(e.target)) return;
      setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  // The described control belongs to the host component, which may set its
  // own `aria-describedby` (an error line, a help paragraph). Keep our token
  // in that list while collapsed and out of it while open, re-asserted after
  // every render so a host re-render that rewrote the attribute cannot drop
  // it. Attribute-only, so React's own reconciliation is not disturbed.
  useEffect(() => {
    if (!describes) return;
    const el = document.getElementById(describes);
    if (!el) return;
    const tokens = (el.getAttribute("aria-describedby") ?? "")
      .split(/\s+/)
      .filter((t) => t !== "" && t !== descId);
    if (!open) tokens.push(descId);
    if (tokens.length > 0) el.setAttribute("aria-describedby", tokens.join(" "));
    else el.removeAttribute("aria-describedby");
  });

  useEffect(() => {
    if (!describes) return;
    return () => {
      const el = document.getElementById(describes);
      if (!el) return;
      const tokens = (el.getAttribute("aria-describedby") ?? "")
        .split(/\s+/)
        .filter((t) => t !== "" && t !== descId);
      if (tokens.length > 0) el.setAttribute("aria-describedby", tokens.join(" "));
      else el.removeAttribute("aria-describedby");
    };
  }, [describes, descId]);

  if (!found) return null;
  const { setting: s } = found;

  const onKeyDown = (e: KeyboardEvent<HTMLSpanElement>) => {
    if (e.key !== "Escape" || !open) return;
    e.preventDefault();
    e.stopPropagation();
    setOpen(false);
    buttonRef.current?.focus();
  };

  return (
    <span ref={wrapRef} className="helphint" onKeyDown={onKeyDown}>
      <button
        ref={buttonRef}
        type="button"
        className="helphint__btn"
        aria-label={`Help: ${s.label}`}
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((v) => !v)}
      >
        <span aria-hidden="true">?</span>
      </button>
      {!open && describes && (
        <span id={descId} className="visually-hidden">
          {s.what}
        </span>
      )}
      <div ref={panelRef} id={panelId} className="helphint__panel" hidden={!open}>
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
            onClick={() => openHelp(area, setting)}
          >
            More in Help
          </button>
        )}
      </div>
    </span>
  );
}
