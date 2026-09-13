import type { GpuStatus } from "../lib/ipc";
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
  const estGb = (vramEstimateMb / 1024).toFixed(1);
  const freeGb = (freeMb / 1024).toFixed(1);

  let level: "ok" | "warn" | "crit";
  let text: string;
  if (vramEstimateMb <= freeMb * 0.9) {
    level = "ok";
    text = `≈${estGb} GB VRAM · fits in ${freeGb} GB free`;
  } else if (vramEstimateMb <= freeMb) {
    level = "warn";
    text = `≈${estGb} GB VRAM · tight against ${freeGb} GB free`;
  } else {
    level = "crit";
    text = `≈${estGb} GB VRAM · needs more than ${freeGb} GB free right now — another model may be evicted`;
  }

  return (
    <p className="vram-hint" data-level={level}>
      {text}
    </p>
  );
}
