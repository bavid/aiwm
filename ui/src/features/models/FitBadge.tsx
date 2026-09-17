import { useId, useState } from "react";
import type { FitVerdict } from "../../lib/ipc";
import { FIT_TIER_LABEL, fitReason, fitVramGb } from "./fit-utils";
import "./fit-badge.css";

/** The hardware-fit tier of one model/file as a labelled badge: a tier shape
 *  (circle / diamond / triangle / hollow ring, drawn in CSS so colour is
 *  never the only signal), the tier name, the VRAM estimate when the source
 *  gave one, and — for `yellow`/`red` — the core's own reason.
 *
 *  The reason is a real disclosure rather than a `title=` tooltip: a tooltip
 *  never reaches keyboard users, is announced unreliably by screen readers
 *  and does not exist at all on touch. Collapsed, the reason is `hidden` and
 *  reachable only as the button's `aria-describedby` (a description resolves
 *  hidden content), so assistive tech gets it once, without a click; open, it
 *  is on screen and the description is dropped — so it is never announced
 *  twice.
 *
 *  `subject` is what the verdict is about (a quant, a file name, a repo id).
 *  It goes into the accessible name, because a rotor full of bare "Tight" is
 *  useless.
 *
 *  Informational only, deliberately: a `Too big` verdict never disables a
 *  download. It is an estimate about *this* machine's current VRAM budget,
 *  and the user may well want the file anyway. */
export function FitBadge({
  fit,
  vramMb,
  subject,
}: {
  fit: FitVerdict;
  vramMb?: number | null;
  /** What this verdict is about — used for the accessible name. */
  subject?: string;
}) {
  const [open, setOpen] = useState(false);
  const reasonId = useId();

  const reason = fitReason(fit);
  const estimate = fitVramGb(vramMb);
  const label = FIT_TIER_LABEL[fit.level];
  const prefix = subject ? `Hardware fit, ${subject}: ` : "Hardware fit: ";
  const body = (
    <>
      <span className="fitbadge__mark" aria-hidden="true" />
      {label}
      {estimate && <span className="fitbadge__est numeric">{estimate}</span>}
    </>
  );

  if (!reason) {
    return (
      <span className="fitbadge">
        <span className={`fitbadge__pill fitbadge__pill--${fit.level}`}>
          {/* Real text, not `aria-label`: a generic <span> carries no role, so
              an accessible name would be ignored by most screen readers. */}
          <span className="fitbadge__sr">{prefix}</span>
          {body}
        </span>
      </span>
    );
  }

  return (
    <span className="fitbadge">
      <button
        type="button"
        className={`fitbadge__pill fitbadge__pill--${fit.level} fitbadge__pill--why`}
        aria-label={`${prefix}${label}${estimate ? `, ${estimate}` : ""}`}
        aria-expanded={open}
        aria-controls={reasonId}
        aria-describedby={open ? undefined : reasonId}
        onClick={() => setOpen((v) => !v)}
      >
        {body}
        <span className="fitbadge__hint" aria-hidden="true">
          ?
        </span>
      </button>
      <span id={reasonId} className="fitbadge__reason" hidden={!open}>
        {reason}
      </span>
    </span>
  );
}
