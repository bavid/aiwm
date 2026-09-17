import { useId, useState } from "react";
import type { FitVerdict } from "../../lib/ipc";
import { FIT_TIER_LABEL, fitReason, fitVramGb } from "./fit-utils";

/** The hardware-fit tier of one model/file as a labelled badge: a tier shape
 *  (circle / diamond / triangle / hollow ring, drawn in CSS so colour is
 *  never the only signal), the tier name, the VRAM estimate when the source
 *  gave one, and — for `yellow`/`red` — the core's own reason.
 *
 *  The reason is a real, focusable disclosure rather than a `title=` tooltip:
 *  a tooltip never reaches keyboard users, is announced unreliably by screen
 *  readers and does not exist at all on touch. Collapsed, the reason stays in
 *  the DOM as visually-hidden text wired up via `aria-describedby`, so a
 *  screen reader gets it without anyone having to click; clicking (or Enter /
 *  Space) reveals the same single copy on screen.
 *
 *  Informational only, deliberately: a `Too big` verdict never disables a
 *  download. It is an estimate about *this* machine's current VRAM budget,
 *  and the user may well want the file anyway. */
export function FitBadge({ fit, vramMb }: { fit: FitVerdict; vramMb?: number | null }) {
  const [open, setOpen] = useState(false);
  const reasonId = useId();

  const reason = fitReason(fit);
  const estimate = fitVramGb(vramMb);
  const body = (
    <>
      <span className="fit__mark" aria-hidden="true" />
      {FIT_TIER_LABEL[fit.level]}
      {estimate && <span className="fit__est numeric">{estimate}</span>}
    </>
  );

  if (!reason) {
    return <span className={`fit fit--${fit.level}`}>{body}</span>;
  }

  return (
    <>
      <button
        type="button"
        className={`fit fit--${fit.level} fit--why`}
        aria-expanded={open}
        aria-describedby={reasonId}
        onClick={() => setOpen((v) => !v)}
      >
        {body}
        <span className="fit__hint" aria-hidden="true">
          ?
        </span>
      </button>
      <span id={reasonId} className={`fit__reason${open ? "" : " fit__reason--sr"}`}>
        {reason}
      </span>
    </>
  );
}
