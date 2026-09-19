import {
  memo,
  useId,
  useRef,
  type DragEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import type { DatasetFrame } from "../../lib/ipc";
import { COLUMN_LABEL, framesLabel, otherColumn, type ColumnId } from "./curation";
import { FrameCard } from "./FrameCard";
import type { ClickModifiers } from "./useFrameSelection";
import { useRubberBand } from "./useRubberBand";

/** Stable per-card callbacks, shared by every card of both columns. */
export type CardHandlers = {
  /** `fallbackAnchor`: where a Shift range starts when the anchor is not in
   *  this column (the focused card, for Shift+arrow keys). */
  onSelectClick: (frame: DatasetFrame, mods: ClickModifiers, fallbackAnchor?: string) => void;
  onToggleSelected: (frame: DatasetFrame) => void;
  onFocusCard: (frame: DatasetFrame) => void;
  onDragStart: (frame: DatasetFrame, e: DragEvent<HTMLElement>) => void;
  onDragEnd: () => void;
  onMove: (frame: DatasetFrame) => void;
  onCaptionCommit: (frame: DatasetFrame, caption: string) => void;
  onClipBoundsCommit: (frame: DatasetFrame, start: number | null, end: number | null) => void;
};

/** Stable per-column callbacks; each takes the column it came from. */
export type ColumnHandlers = {
  onSelectAll: (column: ColumnId) => void;
  onSelectNone: (column: ColumnId) => void;
  onShowMore: (column: ColumnId) => void;
  onBandStart: (column: ColumnId, additive: boolean) => void;
  onBandChange: (column: ColumnId, ids: string[]) => void;
  onBandEnd: (column: ColumnId, ids: string[]) => void;
  onEmptyClick: (column: ColumnId) => void;
  onDragOver: (column: ColumnId, e: DragEvent<HTMLElement>) => void;
  onDragLeave: (column: ColumnId, e: DragEvent<HTMLElement>) => void;
  onDrop: (column: ColumnId, e: DragEvent<HTMLElement>) => void;
};

/** `target`: a drag from the other column is under way; `hover`: it is over
 *  this column right now. */
export type DropState = "none" | "target" | "hover";

type Props = {
  column: ColumnId;
  /** The column's frames in display order (after the Discard filter). */
  frames: readonly DatasetFrame[];
  /** The column before the Discard filter, for "12 of 500". */
  totalCount: number;
  selected: ReadonlySet<string>;
  selectedHere: number;
  activeId: string | null;
  visibleCount: number;
  isCompact: boolean;
  isClipMode: boolean;
  imageUrlFor: (frame: DatasetFrame) => string;
  tokensByFrameId: Record<string, string[]>;
  dropState: DropState;
  /** Frames being dragged, for the drop hint. */
  dragCount: number;
  cardHandlers: CardHandlers;
  columnHandlers: ColumnHandlers;
  /** The Discard column's reason filter. */
  filters?: ReactNode;
};

const NO_TOKENS: string[] = [];

const EMPTY_HINT: Record<ColumnId, string> = {
  keep: "Nothing kept yet — drag frames here, or select them and press K.",
  discard: "Nothing discarded — drag frames here, or select them and press D.",
};

/** How many cards sit in the first visual row, read from the laid-out cells
 *  (only on an Up/Down key press). */
function cardsPerRow(cells: HTMLElement[]): number {
  if (cells.length === 0) return 1;
  const top = cells[0].offsetTop;
  const perRow = cells.findIndex((c) => c.offsetTop !== top);
  return perRow <= 0 ? cells.length : perRow;
}

/** One column of the curation board: header with counts and Select all /
 *  none, then the card grid — a multi-selectable ARIA grid with one roving
 *  tab stop, arrow-key focus movement and rubber-band selection. */
export const CurationColumn = memo(function CurationColumn({
  column,
  frames,
  totalCount,
  selected,
  selectedHere,
  activeId,
  visibleCount,
  isCompact,
  isClipMode,
  imageUrlFor,
  tokensByFrameId,
  dropState,
  dragCount,
  cardHandlers,
  columnHandlers,
  filters,
}: Props) {
  const headingId = useId();
  const helpId = useId();
  const gridRef = useRef<HTMLDivElement>(null);
  const shown = frames.slice(0, visibleCount);
  const tabStop = activeId !== null && shown.some((f) => f.id === activeId) ? activeId : shown[0]?.id;

  // The whole column is the band's canvas (header gaps, padding, the space
  // between cards), like whitespace in Explorer.
  const band = useRubberBand({
    onStart: (additive) => columnHandlers.onBandStart(column, additive),
    onChange: (ids) => columnHandlers.onBandChange(column, ids),
    onEnd: (ids) => columnHandlers.onBandEnd(column, ids),
    onEmptyClick: () => columnHandlers.onEmptyClick(column),
    focusTarget: () => gridRef.current,
  });

  const onGridKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const cell = e.target instanceof HTMLElement ? e.target : null;
    const id = cell?.dataset.frameId;
    const grid = gridRef.current;
    if (!id || !grid || e.altKey) return;
    const cells = [...grid.querySelectorAll<HTMLElement>("[data-frame-id]")];
    const index = cells.findIndex((c) => c.dataset.frameId === id);
    const frame = shown[index];
    if (index < 0 || !frame) return;
    if (e.key === " " && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      cardHandlers.onToggleSelected(frame);
      return;
    }
    const step: Record<string, number> = {
      ArrowLeft: -1,
      ArrowRight: 1,
      ArrowUp: -cardsPerRow(cells),
      ArrowDown: cardsPerRow(cells),
      Home: -index,
      End: cells.length - 1 - index,
    };
    if (!(e.key in step)) return;
    e.preventDefault();
    const next = Math.max(0, Math.min(cells.length - 1, index + step[e.key]));
    cells[next].focus();
    if (e.shiftKey) cardHandlers.onSelectClick(shown[next], { toggle: false, range: true }, id);
  };

  const label = COLUMN_LABEL[column];
  const isFiltered = totalCount !== frames.length;
  const count = isFiltered
    ? `${frames.length.toLocaleString()} of ${totalCount.toLocaleString()}`
    : frames.length.toLocaleString();
  return (
    <section
      className="curation__column"
      data-column={column}
      data-drop={dropState}
      aria-labelledby={headingId}
      onDragOver={(e) => columnHandlers.onDragOver(column, e)}
      onDragLeave={(e) => columnHandlers.onDragLeave(column, e)}
      onDrop={(e) => columnHandlers.onDrop(column, e)}
      onPointerDown={band.onPointerDown}
      onPointerMove={band.onPointerMove}
      onPointerUp={band.onPointerUp}
      onPointerCancel={band.onPointerCancel}
    >
      <header className="curation__head">
        <h3 id={headingId} className="curation__title">
          {label} <span className="curation__count">{count}</span>
        </h3>
        <div className="curation__head-actions">
          {selectedHere > 0 && (
            <span className="curation__selected">{selectedHere.toLocaleString()} selected</span>
          )}
          <button
            type="button"
            className="chip"
            disabled={frames.length === 0 || selectedHere === frames.length}
            onClick={() => columnHandlers.onSelectAll(column)}
          >
            Select all
          </button>
          <button
            type="button"
            className="chip"
            disabled={selectedHere === 0}
            onClick={() => columnHandlers.onSelectNone(column)}
          >
            Select none
          </button>
        </div>
      </header>
      {filters}

      {dropState !== "none" && (
        <p className="curation__drop-hint" aria-hidden="true">
          Drop to move {framesLabel(dragCount)} to {label}
        </p>
      )}

      {frames.length === 0 ? (
        <p className="curation__empty">{EMPTY_HINT[column]}</p>
      ) : (
        <div
          ref={gridRef}
          role="grid"
          aria-multiselectable="true"
          aria-labelledby={headingId}
          aria-rowcount={frames.length}
          aria-describedby={helpId}
          tabIndex={-1}
          className="curation__grid"
          data-compact={isCompact}
          onKeyDown={onGridKeyDown}
        >
          {shown.map((frame, index) => (
            <FrameCard
              key={frame.id}
              frame={frame}
              rowIndex={index + 1}
              imageUrl={imageUrlFor(frame)}
              column={column}
              isSelected={selected.has(frame.id)}
              isActive={frame.id === tabStop}
              isCompact={isCompact}
              conceptTokens={tokensByFrameId[frame.id] ?? NO_TOKENS}
              isClipMode={isClipMode}
              {...cardHandlers}
            />
          ))}
        </div>
      )}
      <p id={helpId} className="visually-hidden">
        Arrow keys move between frames, Space selects, Shift plus arrows extends the selection,
        Control plus A selects the whole column. Press {column === "keep" ? "D" : "K"} to move the
        selection to {COLUMN_LABEL[otherColumn(column)]}, Delete to delete it.
      </p>

      {band.box && (
        <div
          className="curation__band"
          aria-hidden="true"
          style={{
            transform: `translate(${band.box.left}px, ${band.box.top}px)`,
            width: band.box.width,
            height: band.box.height,
          }}
        />
      )}

      {visibleCount < frames.length && (
        <button
          type="button"
          className="chip curation__more"
          onClick={() => columnHandlers.onShowMore(column)}
        >
          Show more ({(frames.length - visibleCount).toLocaleString()} remaining)
        </button>
      )}
    </section>
  );
});
