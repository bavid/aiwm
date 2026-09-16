import { useEffect, useId, useState } from "react";
import type { DatasetFrame } from "../../lib/ipc";
import { rejectionLabel } from "./rejection";

type Props = {
  frame: DatasetFrame;
  imageUrl: string;
  isSelected: boolean;
  /** Concept tokens this item carries, for the chips under the thumbnail. */
  conceptTokens: string[];
  onToggleSelected: () => void;
  onCaptionCommit: (caption: string) => void;
  onToggleExcluded: () => void;
  onRestore: () => void;
};

export function FrameCard({
  frame,
  imageUrl,
  isSelected,
  conceptTokens,
  onToggleSelected,
  onCaptionCommit,
  onToggleExcluded,
  onRestore,
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

  const commitCaption = () => {
    setIsDirty(false);
    onCaptionCommit(caption);
  };

  const captionId = useId();
  const isRejected = frame.rejection_reason !== "";

  return (
    <div className="framecard" data-excluded={frame.excluded} data-rejected={isRejected}>
      <div className="framecard__thumb">
        {/* The frame is the content here, not decoration -- name it with its
            caption, falling back to the folder tag before it is captioned. */}
        {imageUrl && <img src={imageUrl} alt={frame.caption || frame.tag} loading="lazy" />}
        <label className="framecard__select" title="Select for concept assignment">
          <input type="checkbox" checked={isSelected} onChange={onToggleSelected} />
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
          <span className="framecard__duration">{frame.duration_secs.toFixed(1)}s</span>
        )}
      </div>

      {conceptTokens.length > 0 && (
        <div className="framecard__concepts">
          {conceptTokens.map((token) => (
            <span key={token} className="framecard__concept">
              {token}
            </span>
          ))}
        </div>
      )}

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
        <button type="button" className="chip framecard__restore" onClick={onRestore}>
          Restore
        </button>
      ) : (
        <label className="framecard__exclude">
          <input type="checkbox" checked={frame.excluded} onChange={onToggleExcluded} />
          <span>Exclude</span>
        </label>
      )}
    </div>
  );
}
