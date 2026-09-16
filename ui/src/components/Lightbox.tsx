import { useEffect } from "react";
import { createPortal } from "react-dom";
import "./lightbox.css";

/** A full-viewport image/video zoom overlay for a gallery — Esc/backdrop/×
 *  closes, ←/→ (or the on-screen arrows) move to the neighbouring item when
 *  the caller supplies `onPrev`/`onNext`. Rendered through a portal so a
 *  `position: fixed` overlay is never clipped by an ancestor's own layout. */
export function Lightbox({
  kind,
  src,
  caption,
  onClose,
  onPrev,
  onNext,
}: {
  kind: "image" | "video";
  src: string;
  caption?: string;
  onClose: () => void;
  onPrev?: () => void;
  onNext?: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowLeft" && onPrev) onPrev();
      else if (e.key === "ArrowRight" && onNext) onNext();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, onPrev, onNext]);

  return createPortal(
    <div className="lightbox" role="dialog" aria-modal="true" onClick={onClose}>
      <button type="button" className="lightbox__close" aria-label="Close" onClick={onClose}>
        ×
      </button>
      {onPrev && (
        <button
          type="button"
          className="lightbox__nav lightbox__nav--prev"
          aria-label="Previous"
          onClick={(e) => {
            e.stopPropagation();
            onPrev();
          }}
        >
          ‹
        </button>
      )}
      <figure className="lightbox__stage" onClick={(e) => e.stopPropagation()}>
        {kind === "image" ? (
          <img src={src} alt={caption ?? ""} />
        ) : (
          <video src={src} controls autoPlay playsInline />
        )}
        {caption && <figcaption className="lightbox__caption">{caption}</figcaption>}
      </figure>
      {onNext && (
        <button
          type="button"
          className="lightbox__nav lightbox__nav--next"
          aria-label="Next"
          onClick={(e) => {
            e.stopPropagation();
            onNext();
          }}
        >
          ›
        </button>
      )}
    </div>,
    document.body,
  );
}
