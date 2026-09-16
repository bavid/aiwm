import { memo, useEffect, useId, useState } from "react";
import type { DatasetFrame } from "../../lib/ipc";
import { boundField, fileName, formatDuration, parseBound } from "./format";
import { rejectionLabel } from "./rejection";

type Props = {
  frame: DatasetFrame;
  imageUrl: string;
  isSelected: boolean;
  /** Concept tokens this item carries, for the chips under the thumbnail. */
  conceptTokens: string[];
  /** The dataset's mode: a clip card adds its length, source file and the
   *  in/out points the export trims to. */
  isClipMode: boolean;
  onToggleSelected: (frame: DatasetFrame) => void;
  onCaptionCommit: (frame: DatasetFrame, caption: string) => void;
  onToggleExcluded: (frame: DatasetFrame) => void;
  onRestore: (frame: DatasetFrame) => void;
  /** Both bounds at once; `null` is "the clip's natural start/end". */
  onClipBoundsCommit: (frame: DatasetFrame, start: number | null, end: number | null) => void;
};

/** One item of the curation grid. Memoised: the container hands it stable
 *  handlers (and a memoised token array), so a card only re-renders when its
 *  own row, selection or tokens change -- not on every keystroke elsewhere. */
export const FrameCard = memo(function FrameCard({
  frame,
  imageUrl,
  isSelected,
  conceptTokens,
  isClipMode,
  onToggleSelected,
  onCaptionCommit,
  onToggleExcluded,
  onRestore,
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
  const canTrim = isClipMode && !isRejected;

  return (
    <div className="framecard" data-excluded={frame.excluded} data-rejected={isRejected}>
      <div className="framecard__thumb" data-empty={!hasThumb}>
        {/* The frame is the content here, not decoration -- name it with its
            caption, falling back to the folder tag before it is captioned. */}
        {hasThumb && <img src={imageUrl} alt={frame.caption || frame.tag} loading="lazy" />}
        <label className="framecard__select" title="Select for concept assignment">
          <input type="checkbox" checked={isSelected} onChange={() => onToggleSelected(frame)} />
          <span className="framecard__select-label">Select</span>
        </label>
        <span className="framecard__tag">{frame.tag}</span>
        {isRejected ? (
          <span className="framecard__badge framecard__badge--reject">
            {rejectionLabel(frame.rejection_reason)}
          </span>
        ) : (
          frame.caption_engine === "qwen2.5-vl" && (
            <span className="framecard__badge" title="Re-captioned with temporal context">
              Qwen
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

      {isRejected ? (
        <button type="button" className="chip framecard__restore" onClick={() => onRestore(frame)}>
          Restore
        </button>
      ) : (
        <label className="framecard__exclude">
          <input
            type="checkbox"
            checked={frame.excluded}
            onChange={() => onToggleExcluded(frame)}
          />
          <span>Exclude</span>
        </label>
      )}
    </div>
  );
});
