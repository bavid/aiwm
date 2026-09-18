import type { ReactNode } from "react";
import { framesLabel } from "./curation";

type Props = {
  selectedTotal: number;
  /** Selected frames in the Discard column — what "Keep" would move. */
  toKeep: number;
  /** Selected frames in the Keep column — what "Discard" would move. */
  toDiscard: number;
  isCompact: boolean;
  onKeep: () => void;
  onDiscard: () => void;
  onDelete: () => void;
  onClear: () => void;
  onCompactChange: (isCompact: boolean) => void;
  /** The concept row, shown while something is selected. */
  children?: ReactNode;
};

/** The sticky bar above the board: selection count, the move / delete /
 *  clear actions with their shortcuts, and the card-size switch. */
export function SelectionBar({
  selectedTotal,
  toKeep,
  toDiscard,
  isCompact,
  onKeep,
  onDiscard,
  onDelete,
  onClear,
  onCompactChange,
  children,
}: Props) {
  return (
    <div className="curation__bar card" role="group" aria-label="Selection">
      <div className="curation__bar-main">
        <span className="curation__bar-count">
          {selectedTotal > 0
            ? `${framesLabel(selectedTotal)} selected`
            : "Click, Ctrl-click, Shift-click or draw a box to select"}
        </span>
        <button
          type="button"
          className="chip curation__action"
          disabled={toKeep === 0}
          onClick={onKeep}
          aria-keyshortcuts="K"
        >
          ← Keep{toKeep > 0 ? ` ${toKeep.toLocaleString()}` : ""}
          <kbd>K</kbd>
        </button>
        <button
          type="button"
          className="chip curation__action"
          disabled={toDiscard === 0}
          onClick={onDiscard}
          aria-keyshortcuts="D"
        >
          Discard{toDiscard > 0 ? ` ${toDiscard.toLocaleString()}` : ""} →<kbd>D</kbd>
        </button>
        <button
          type="button"
          className="chip curation__action curation__action--danger"
          disabled={selectedTotal === 0}
          onClick={onDelete}
          aria-keyshortcuts="Delete"
        >
          Delete…<kbd>Del</kbd>
        </button>
        <button
          type="button"
          className="chip"
          disabled={selectedTotal === 0}
          onClick={onClear}
          aria-keyshortcuts="Escape"
        >
          Clear
        </button>
        <div className="curation__density" role="group" aria-label="Card size">
          <button
            type="button"
            className="chip"
            aria-pressed={isCompact}
            onClick={() => onCompactChange(true)}
          >
            Thumbnails
          </button>
          <button
            type="button"
            className="chip"
            aria-pressed={!isCompact}
            onClick={() => onCompactChange(false)}
          >
            Details
          </button>
        </div>
      </div>
      {children}
    </div>
  );
}
