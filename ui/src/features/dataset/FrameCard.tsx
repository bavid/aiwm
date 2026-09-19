import { memo, useEffect, useId, useState, type DragEvent, type MouseEvent } from "react";
import type { DatasetFrame } from "../../lib/ipc";
import { COLUMN_LABEL, otherColumn, type ColumnId } from "./curation";
import { boundField, fileName, formatDuration, parseBound } from "./format";
import { rejectionLabel } from "./rejection";
import type { ClickModifiers } from "./useFrameSelection";
import "./framecard.css";

type Props = {
  frame: DatasetFrame;
  /** 1-based position in the column, for `aria-rowindex`. */
  rowIndex: number;
  imageUrl: string;
  /** The board column the card sits in; its move button goes to the other. */
  column: ColumnId;
  isSelected: boolean;
  /** The column's roving tab stop: the one card Tab lands on. */
  isActive: boolean;
  /** Thumbnail-only, for fast sorting; the caption and trim fields show only
   *  in the detailed view. */
  isCompact: boolean;
  /** Concept tokens this item carries, for the chips under the thumbnail. */
  conceptTokens: string[];
  /** The dataset's mode: a clip card adds its length, source file and the
   *  in/out points the export trims to. */
  isClipMode: boolean;
  onSelectClick: (frame: DatasetFrame, mods: ClickModifiers) => void;
  onToggleSelected: (frame: DatasetFrame) => void;
  onFocusCard: (frame: DatasetFrame) => void;
  onDragStart: (frame: DatasetFrame, e: DragEvent<HTMLElement>) => void;
  onDragEnd: () => void;
  /** Moves just this card to the other column. */
  onMove: (frame: DatasetFrame) => void;
  onCaptionCommit: (frame: DatasetFrame, caption: string) => void;
  /** Both bounds at once; `null` is "the clip's natural start/end". */
  onClipBoundsCommit: (frame: DatasetFrame, start: number | null, end: number | null) => void;
};

const modifiersOf = (e: MouseEvent): ClickModifiers => ({
  toggle: e.ctrlKey || e.metaKey,
  range: e.shiftKey,
});

/** One item of the curation board: a grid row with one selectable cell. The
 *  thumbnail is the click target and the drag handle; the controls below it
 *  stay ordinary form fields. Memoised: the board hands it stable handlers
 *  (and a memoised token array), so a card only re-renders when its own row,
 *  selection, tab stop or tokens change. */
export const FrameCard = memo(function FrameCard({
  frame,
  rowIndex,
  imageUrl,
  column,
  isSelected,
  isActive,
  isCompact,
  conceptTokens,
  isClipMode,
  onSelectClick,
  onToggleSelected,
  onFocusCard,
  onDragStart,
  onDragEnd,
  onMove,
  onCaptionCommit,
  onClipBoundsCommit,
}: Props) {
  const [caption, setCaption] = useState(frame.caption);
  // The frame list is polled every 3s. Re-syncing the textarea from the server
  // copy unconditionally would wipe whatever the curator is typing the moment a
  // tick lands, so an unsaved draft wins until it is committed on blur.
  const [isDirty, setIsDirty] = useState(false);
  useEffect(() => {
    if (isDirty) return;
    setCaption(frame.caption);
  }, [frame.caption, isDirty]);

  // Same deal for the two trim fields, which are edited as a pair.
  const [startDraft, setStartDraft] = useState(() => boundField(frame.clip_start_secs));
  const [endDraft, setEndDraft] = useState(() => boundField(frame.clip_end_secs));
  const [areBoundsDirty, setAreBoundsDirty] = useState(false);
  const [boundsError, setBoundsError] = useState<string | null>(null);
  useEffect(() => {
    if (areBoundsDirty) return;
    setStartDraft(boundField(frame.clip_start_secs));
    setEndDraft(boundField(frame.clip_end_secs));
  }, [frame.clip_start_secs, frame.clip_end_secs, areBoundsDirty]);

  const commitCaption = () => {
    setIsDirty(false);
    onCaptionCommit(frame, caption);
  };

  /** Validates the pair and only then writes — an invalid range never leaves
   *  the card, so the curator sees why instead of an API error. */
  const commitBounds = () => {
    if (!areBoundsDirty) return;
    const start = parseBound(startDraft);
    const end = parseBound(endDraft);
    if (start === undefined || end === undefined) {
      setBoundsError("Start and End must be numbers (leave blank for the whole clip).");
      return;
    }
    if ((start !== null && start < 0) || (end !== null && end < 0)) {
      setBoundsError("Start and End cannot be negative.");
      return;
    }
    const length = frame.duration_secs;
    if (length !== null && ((start !== null && start > length) || (end !== null && end > length))) {
      setBoundsError(`The clip is only ${length}s long.`);
      return;
    }
    if (start !== null && end !== null && start >= end) {
      setBoundsError("Start must be before End.");
      return;
    }
    setBoundsError(null);
    setAreBoundsDirty(false);
    onClipBoundsCommit(frame, start, end);
  };

  const captionId = useId();
  const startId = useId();
  const endId = useId();
  const isRejected = frame.rejection_reason !== "";
  // An unusable clip never got a preview still, and there is nothing to trim.
  const hasThumb = imageUrl !== "" && (!isClipMode || frame.frame_path !== "");
  const canTrim = !isCompact && isClipMode && !isRejected;
  const target = COLUMN_LABEL[otherColumn(column)];

  return (
    <div role="row" className="framecard__row" aria-rowindex={rowIndex}>
      <div
        role="gridcell"
        className="framecard"
        aria-selected={isSelected}
        tabIndex={isActive ? 0 : -1}
        data-frame-id={frame.id}
        data-compact={isCompact}
        data-excluded={frame.excluded}
        data-rejected={isRejected}
        onFocus={(e) => {
          if (e.target === e.currentTarget) onFocusCard(frame);
        }}
      >
        <div
          className="framecard__thumb"
          data-empty={!hasThumb}
          data-drag-handle
          draggable
          onClick={(e) => onSelectClick(frame, modifiersOf(e))}
          onDragStart={(e) => onDragStart(frame, e)}
          onDragEnd={onDragEnd}
        >
          {/* The frame is the content here, not decoration -- name it with its
              caption, falling back to the folder tag before it is captioned. */}
          {hasThumb && (
            <img src={imageUrl} alt={frame.caption || frame.tag} loading="lazy" draggable={false} />
          )}
          <label className="framecard__select" onClick={(e) => e.stopPropagation()}>
            <input
              type="checkbox"
              checked={isSelected}
              // Not a tab stop: Space on the focused card does the same.
              tabIndex={-1}
              onChange={() => onToggleSelected(frame)}
              aria-label={`Select ${frame.caption || frame.tag}`}
            />
          </label>
          <span className="framecard__tag">{frame.tag}</span>
          {isRejected ? (
            <span className="framecard__badge framecard__badge--reject">
              {rejectionLabel(frame.rejection_reason)}
            </span>
          ) : frame.excluded ? (
            <span className="framecard__badge framecard__badge--excluded">Excluded</span>
          ) : (
            frame.caption_engine === "qwen2.5-vl" && (
              <span className="framecard__badge">
                Qwen
                <span className="visually-hidden"> — re-captioned with temporal context</span>
              </span>
            )
          )}
          {frame.duration_secs !== null && (
            <span className="framecard__duration">{formatDuration(frame.duration_secs)}</span>
          )}
        </div>

        {isClipMode && <p className="framecard__source">{fileName(frame.source_path)}</p>}

        {conceptTokens.length > 0 && (
          <div className="framecard__concepts">
            {conceptTokens.map((token) => (
              <span key={token} className="framecard__concept">
                {token}
              </span>
            ))}
          </div>
        )}

        {canTrim && (
          <div className="framecard__trim">
            <label className="framecard__trim-field" htmlFor={startId}>
              <span>Start (s)</span>
              <input
                id={startId}
                type="number"
                min={0}
                step={0.1}
                value={startDraft}
                placeholder="0"
                onChange={(e) => {
                  setAreBoundsDirty(true);
                  setStartDraft(e.target.value);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitBounds();
                  }
                }}
                onBlur={commitBounds}
              />
            </label>
            <label className="framecard__trim-field" htmlFor={endId}>
              <span>End (s)</span>
              <input
                id={endId}
                type="number"
                min={0}
                step={0.1}
                value={endDraft}
                placeholder={frame.duration_secs === null ? "end" : String(frame.duration_secs)}
                onChange={(e) => {
                  setAreBoundsDirty(true);
                  setEndDraft(e.target.value);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitBounds();
                  }
                }}
                onBlur={commitBounds}
              />
            </label>
          </div>
        )}
        {boundsError && <p className="framecard__trim-err">{boundsError}</p>}

        {!isCompact && (
          <>
            <label className="framecard__caption-label" htmlFor={captionId}>
              Caption
            </label>
            <textarea
              id={captionId}
              value={caption}
              onChange={(e) => {
                setIsDirty(true);
                setCaption(e.target.value);
              }}
              onBlur={commitCaption}
              rows={2}
            />
            <button type="button" className="chip framecard__move" onClick={() => onMove(frame)}>
              Move to {target}
            </button>
          </>
        )}
      </div>
    </div>
  );
});
