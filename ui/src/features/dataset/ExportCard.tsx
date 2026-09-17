import { useId } from "react";
import type { CaptionOrder } from "../../lib/ipc";
import { browseForDirectory } from "./browse";

export type ExportState =
  | { kind: "idle" }
  | { kind: "busy" }
  | { kind: "done"; exported: number; destDir: string }
  | { kind: "error"; message: string };

type Props = {
  destDir: string;
  onDestDirChange: (value: string) => void;
  captionOrder: CaptionOrder;
  onCaptionOrderChange: (value: CaptionOrder) => void;
  /** Items that would actually be written — also what the button counts. */
  keptCount: number;
  /** Clips mode writes trimmed videos, not stills — the button says so. */
  isClipMode: boolean;
  state: ExportState;
  onExport: () => void;
};

/** The export card at the foot of the result column: destination folder,
 *  caption order, the button and the run's summary line. */
export function ExportCard({
  destDir,
  onDestDirChange,
  captionOrder,
  onCaptionOrderChange,
  keptCount,
  isClipMode,
  state,
  onExport,
}: Props) {
  const destDirId = useId();
  const orderId = useId();

  return (
    <div className="card dataset__export">
      <h3>Export</h3>
      <label className="datasetform__field" htmlFor={destDirId}>
        <span>Destination folder</span>
        <div className="datasetform__row">
          <input
            id={destDirId}
            type="text"
            value={destDir}
            onChange={(e) => onDestDirChange(e.target.value)}
            placeholder="Where to write NNNN.png + NNNN.txt pairs"
          />
          <button
            type="button"
            className="chip"
            onClick={() => browseForDirectory(onDestDirChange)}
          >
            Browse…
          </button>
        </div>
      </label>
      <label className="datasetform__field datasetform__field--inline" htmlFor={orderId}>
        <span>Caption order</span>
        <select
          id={orderId}
          value={captionOrder}
          onChange={(e) => onCaptionOrderChange(e.target.value as CaptionOrder)}
        >
          <option value="prose_first">Prose first (FLUX.2)</option>
          <option value="tags_first">Tags first (Anime/SDXL)</option>
        </select>
      </label>
      <button
        type="button"
        className="datasetform__go"
        onClick={onExport}
        disabled={!destDir.trim() || state.kind === "busy" || keptCount === 0}
      >
        {state.kind === "busy"
          ? "Exporting…"
          : `Export ${keptCount} ${isClipMode ? "clip(s)" : "item(s)"}`}
      </button>
      {state.kind === "done" && (
        <p className="dataset__done">
          Exported {state.exported} item(s) to {state.destDir}.
        </p>
      )}
      {state.kind === "error" && <p className="dataset__err">{state.message}</p>}
    </div>
  );
}
