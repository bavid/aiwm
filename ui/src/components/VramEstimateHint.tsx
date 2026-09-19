import type { GpuStatus } from "../lib/ipc";
import { formatGiB } from "../lib/units";
import "./vram-hint.css";

/** A quick "will this fit?" hint for the currently-selected model, shown
 *  right where the model is picked -- before the user hits Generate, not
 *  after a job comes back `blocked`. Client-side and deliberately rough
 *  (compares the model's own VRAM estimate against live free VRAM only,
 *  not the scheduler's real eviction logic), so it's worded as a heads-up,
 *  not a guarantee. */
export function VramEstimateHint({
  vramEstimateMb,
  gpu,
}: {
  vramEstimateMb: number | null | undefined;
  gpu: GpuStatus | undefined;
}) {
  if (!vramEstimateMb || gpu?.state !== "available") return null;

  const freeMb = gpu.vram_free_mb;
  const est = formatGiB(vramEstimateMb);
  const free = formatGiB(freeMb);

  let level: "ok" | "warn" | "crit";
  let text: string;
  if (vramEstimateMb <= freeMb * 0.9) {
    level = "ok";
    text = `≈${est} VRAM · fits in ${free} free`;
  } else if (vramEstimateMb <= freeMb) {
    level = "warn";
    text = `≈${est} VRAM · tight against ${free} free`;
  } else {
    level = "crit";
    text = `≈${est} VRAM · needs more than ${free} free right now — another model may be evicted`;
  }

  return (
    <p className="vram-hint" data-level={level}>
      {text}
    </p>
  );
}
