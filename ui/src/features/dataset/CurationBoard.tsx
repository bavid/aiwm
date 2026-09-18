import {
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type KeyboardEvent,
} from "react";
import {
  assignConcept,
  bulkUpdateDatasetFrames,
  deleteDatasetFrames,
  unassignConcept,
  updateDatasetFrame,
  type DatasetConcept,
  type DatasetFrame,
  type SkippedFile,
} from "../../lib/ipc";
import { ConfirmDialog } from "./ConfirmDialog";
import { CurationColumn, type CardHandlers, type ColumnHandlers } from "./CurationColumn";
import {
  ALL_DISCARDED,
  COLUMN_LABEL,
  columnOf,
  errorText,
  framesLabel,
  matchesDiscardFilter,
  otherColumn,
  type ColumnId,
} from "./curation";
import { formatBytes } from "./format";
import { RejectionChips } from "./RejectionChips";
import { SelectionToolbar, type AssignState } from "./SelectionToolbar";
import { SkippedFiles } from "./SkippedFiles";
import { useFrameSelection } from "./useFrameSelection";
import { useOptimisticFrames } from "./useOptimisticFrames";
import "./curation.css";

/** Cards rendered per column before "Show more". Select all still covers the
 *  whole column; only the DOM is bounded. */
const PAGE_SIZE = 120;
/** The drag payload type; a custom one, so a drop on a text field is inert. */
const DRAG_MIME = "application/x-aiwm-frames";

type Props = {
  datasetId: string;
  frames: readonly DatasetFrame[];
  isClipMode: boolean;
  imageUrlFor: (frame: DatasetFrame) => string;
  tokensByFrameId: Record<string, string[]>;
  concepts: DatasetConcept[];
  /** Refetch frames (and the disk usage) after a write. */
  onFramesChanged: () => void;
  onConceptsChanged: () => void;
};

type Notice = { text: string; skipped: SkippedFile[] };
type DeleteState = { ids: string[]; isBusy: boolean; error: string | null };
type DragInfo = { source: ColumnId; ids: string[] };

/** Typing in a field must not trigger the board's single-key shortcuts. */
function isTypingTarget(el: EventTarget): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.isContentEditable || el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
  if (el.tagName !== "INPUT") return false;
  const type = (el as HTMLInputElement).type;
  return type !== "checkbox" && type !== "radio" && type !== "button";
}

/** The curation board: Keep on the left, Discard on the right. Select with
 *  click / Ctrl / Shift / a rubber band / Select all; move by drag & drop,
 *  the toolbar or K / D; delete with Delete. One request per move. */
export function CurationBoard({
  datasetId,
  frames: polledFrames,
  isClipMode,
  imageUrlFor,
  tokensByFrameId,
  concepts,
  onFramesChanged,
  onConceptsChanged,
}: Props) {
  const optimistic = useOptimisticFrames(polledFrames);
  const frames = optimistic.frames;
  const selection = useFrameSelection();
  const { selected } = selection;

  const [discardFilter, setDiscardFilter] = useState(ALL_DISCARDED);
  const [visible, setVisible] = useState<Record<ColumnId, number>>({
    keep: PAGE_SIZE,
    discard: PAGE_SIZE,
  });
  const [activeIds, setActiveIds] = useState<Record<ColumnId, string | null>>({
    keep: null,
    discard: null,
  });
  const [isCompact, setIsCompact] = useState(true);
  const [drag, setDrag] = useState<{ source: ColumnId; count: number } | null>(null);
  const [dropHover, setDropHover] = useState<ColumnId | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [announcement, setAnnouncement] = useState({ text: "", seq: 0 });
  const [deleteState, setDeleteState] = useState<DeleteState | null>(null);
  const [assignConceptId, setAssignConceptId] = useState("");
  const [assignState, setAssignState] = useState<AssignState>({ kind: "idle" });

  const boardRef = useRef<HTMLDivElement>(null);
  const ghostRef = useRef<HTMLDivElement>(null);
  const dragInfo = useRef<DragInfo | null>(null);
  const bandBase = useRef<ReadonlySet<string>>(new Set());
  const lastColumn = useRef<ColumnId>("keep");
  /** Where focus goes when a keyboard move takes the focused card away. */
  const refocus = useRef<{ column: ColumnId; index: number } | null>(null);

  // --- columns ------------------------------------------------------------
  const { keep, discardAll } = useMemo(() => {
    const k: DatasetFrame[] = [];
    const d: DatasetFrame[] = [];
    for (const f of frames) (columnOf(f) === "keep" ? k : d).push(f);
    return { keep: k, discardAll: d };
  }, [frames]);

  const discardShown = useMemo(() => {
    const filtered = discardAll.filter((f) => matchesDiscardFilter(f, discardFilter));
    // A filter emptied by a move falls back to the whole column.
    return filtered.length > 0 ? filtered : discardAll;
  }, [discardAll, discardFilter]);

  const columns = useMemo(
    () => ({ keep, discard: discardShown }) satisfies Record<ColumnId, DatasetFrame[]>,
    [keep, discardShown],
  );
  const orderedIds = useMemo(
    () => ({ keep: keep.map((f) => f.id), discard: discardShown.map((f) => f.id) }),
    [keep, discardShown],
  );
  const columnById = useMemo(() => {
    const map = new Map<string, ColumnId>();
    for (const f of frames) map.set(f.id, columnOf(f));
    return map;
  }, [frames]);

  /** Selected ids per column, in no particular order. */
  const selectedIn = useMemo(() => {
    const out: Record<ColumnId, string[]> = { keep: [], discard: [] };
    for (const id of selected) {
      const column = columnById.get(id);
      if (column) out[column].push(id);
    }
    return out;
  }, [selected, columnById]);
  const selectedTotal = selectedIn.keep.length + selectedIn.discard.length;

  const announce = useCallback(
    (text: string) => setAnnouncement((cur) => ({ text, seq: cur.seq + 1 })),
    [],
  );

  // --- writes -------------------------------------------------------------
  const move = useCallback(
    async (ids: string[], target: ColumnId) => {
      if (ids.length === 0) return;
      const excluded = target === "discard";
      const batch = optimistic.beginMove(ids, excluded);
      selection.remove(ids);
      setError(null);
      setNotice(null);
      announce(`${framesLabel(ids.length)} moved to ${COLUMN_LABEL[target]}`);
      try {
        const summary = await bulkUpdateDatasetFrames(datasetId, ids, excluded);
        optimistic.settleMove(batch);
        if (summary.updated < summary.requested) {
          setNotice({
            text: `${summary.updated} of ${summary.requested} frames moved — the rest no longer exist.`,
            skipped: [],
          });
        }
      } catch (e) {
        optimistic.rollbackMove(batch);
        const message = `Could not move ${framesLabel(ids.length)} to ${COLUMN_LABEL[target]}: ${errorText(e)}`;
        setError(message);
        announce(message);
      } finally {
        onFramesChanged();
      }
    },
    [datasetId, optimistic, selection, announce, onFramesChanged],
  );

  /** The ids a toolbar/keyboard move to `target` acts on: the selection in
   *  the other column, or else the focused card. */
  const idsToMove = (target: ColumnId): string[] => {
    const source = otherColumn(target);
    if (selectedTotal > 0) return selectedIn[source];
    const focused = document.activeElement;
    const id = focused instanceof HTMLElement ? focused.dataset.frameId : undefined;
    return id && columnById.get(id) === source ? [id] : [];
  };

  const moveFromKeyboard = (target: ColumnId) => {
    const ids = idsToMove(target);
    const focused = document.activeElement;
    const focusedId = focused instanceof HTMLElement ? focused.dataset.frameId : undefined;
    if (focusedId && ids.includes(focusedId)) {
      const column = otherColumn(target);
      refocus.current = { column, index: orderedIds[column].indexOf(focusedId) };
    }
    void move(ids, target);
  };

  // Put focus back on the column a keyboard move just emptied a slot in.
  useLayoutEffect(() => {
    const want = refocus.current;
    const board = boardRef.current;
    if (!want || !board) return;
    refocus.current = null;
    const cells = board.querySelectorAll<HTMLElement>(
      `[data-column="${want.column}"] [data-frame-id]`,
    );
    // The next card in the same column, else (the column is now empty) the
    // first card of the other one, else the board itself.
    const target =
      cells[Math.min(want.index, cells.length - 1)] ??
      board.querySelector<HTMLElement>(`[data-column="${otherColumn(want.column)}"] [data-frame-id]`) ??
      board;
    target.focus({ preventScroll: true });
  }, [frames]);

  const confirmDelete = async () => {
    if (!deleteState) return;
    const { ids } = deleteState;
    setDeleteState({ ...deleteState, isBusy: true, error: null });
    try {
      const summary = await deleteDatasetFrames(datasetId, ids);
      selection.remove(ids);
      setDeleteState(null);
      const text =
        `Deleted ${framesLabel(summary.deleted)} and ${summary.deleted_files.toLocaleString()} ` +
        `file(s) — ${formatBytes(summary.freed_bytes)} freed.`;
      setNotice({ text, skipped: summary.skipped_files });
      announce(text);
    } catch (e) {
      setDeleteState({ ids, isBusy: false, error: errorText(e) });
    } finally {
      onFramesChanged();
    }
  };

  const requestDelete = () => {
    const ids = selectedTotal > 0 ? [...selectedIn.keep, ...selectedIn.discard] : [];
    if (ids.length > 0) setDeleteState({ ids, isBusy: false, error: null });
  };

  const runAssign = async (attach: boolean) => {
    if (!assignConceptId || selectedTotal === 0) return;
    const ids = [...selected];
    try {
      if (attach) {
        const summary = await assignConcept(assignConceptId, ids);
        setAssignState({ kind: "done", text: `${summary.attached} of ${summary.requested} assigned` });
      } else {
        await unassignConcept(assignConceptId, ids);
        setAssignState({ kind: "done", text: `${ids.length} removed` });
      }
      onConceptsChanged();
    } catch (e) {
      setAssignState({ kind: "error", text: errorText(e) });
    }
  };

  // --- stable handlers (read the latest render through a ref) -------------
  const latest = useRef({ orderedIds, selectedIn, selected, move, columnById });
  useLayoutEffect(() => {
    latest.current = { orderedIds, selectedIn, selected, move, columnById };
  });

  const endDrag = useCallback(() => {
    dragInfo.current = null;
    setDrag(null);
    setDropHover(null);
  }, []);

  const cardHandlers = useMemo<CardHandlers>(
    () => ({
      onSelectClick: (frame, mods) => {
        const column = columnOf(frame);
        lastColumn.current = column;
        selection.click(frame.id, latest.current.orderedIds[column], mods);
      },
      onToggleSelected: (frame) => selection.toggle(frame.id),
      onFocusCard: (frame) => {
        const column = columnOf(frame);
        lastColumn.current = column;
        setActiveIds((cur) => (cur[column] === frame.id ? cur : { ...cur, [column]: frame.id }));
      },
      onDragStart: (frame, e: DragEvent<HTMLElement>) => {
        const source = columnOf(frame);
        const { selected: sel, selectedIn: inCol } = latest.current;
        const ids = sel.has(frame.id) ? inCol[source] : [frame.id];
        if (!sel.has(frame.id)) selection.click(frame.id, [], { toggle: false, range: false });
        dragInfo.current = { source, ids };
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData(DRAG_MIME, ids.join(","));
        const ghost = ghostRef.current;
        if (ghost) {
          ghost.textContent = `${framesLabel(ids.length)} → ${COLUMN_LABEL[otherColumn(source)]}`;
          e.dataTransfer.setDragImage(ghost, 16, 16);
        }
        setDrag({ source, count: ids.length });
      },
      onDragEnd: endDrag,
      onMove: (frame) => {
        const target = otherColumn(columnOf(frame));
        void latest.current.move([frame.id], target);
      },
      onCaptionCommit: async (frame, caption) => {
        if (caption === frame.caption) return;
        try {
          await updateDatasetFrame(frame.id, { caption });
          onFramesChanged();
        } catch (e) {
          setError(`Could not save the caption: ${errorText(e)}`);
        }
      },
      onClipBoundsCommit: async (frame, start, end) => {
        try {
          await updateDatasetFrame(frame.id, { clip_start_secs: start, clip_end_secs: end });
          onFramesChanged();
        } catch (e) {
          setError(`Could not save the clip bounds: ${errorText(e)}`);
        }
      },
    }),
    [selection, endDrag, onFramesChanged],
  );

  const columnHandlers = useMemo<ColumnHandlers>(
    () => ({
      onSelectAll: (column) => {
        lastColumn.current = column;
        selection.replace(latest.current.orderedIds[column]);
      },
      onSelectNone: (column) => selection.remove(latest.current.selectedIn[column]),
      onShowMore: (column) =>
        setVisible((cur) => ({ ...cur, [column]: cur[column] + PAGE_SIZE })),
      onBandStart: (column, additive) => {
        lastColumn.current = column;
        bandBase.current = additive ? latest.current.selected : new Set();
      },
      onBandChange: (_column, ids) => selection.extend(bandBase.current, ids),
      onEmptyClick: (column) => {
        lastColumn.current = column;
        selection.clear();
      },
      onDragOver: (column, e) => {
        const info = dragInfo.current;
        if (!info || info.source === column) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        setDropHover((cur) => (cur === column ? cur : column));
      },
      onDragLeave: (column, e) => {
        const next = e.relatedTarget;
        if (next instanceof Node && e.currentTarget.contains(next)) return;
        setDropHover((cur) => (cur === column ? null : cur));
      },
      onDrop: (column, e) => {
        const info = dragInfo.current;
        endDrag();
        if (!info || info.source === column) return;
        e.preventDefault();
        void latest.current.move(info.ids, column);
      },
    }),
    [selection, endDrag],
  );

  // --- keyboard -----------------------------------------------------------
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.defaultPrevented || isTypingTarget(e.target)) return;
    if (e.target instanceof Element && e.target.closest("dialog")) return;
    const key = e.key.toLowerCase();
    const isMod = e.ctrlKey || e.metaKey;
    if (isMod && key === "a") {
      const el = e.target instanceof Element ? e.target.closest<HTMLElement>("[data-column]") : null;
      const column = (el?.dataset.column as ColumnId | undefined) ?? lastColumn.current;
      e.preventDefault();
      columnHandlers.onSelectAll(column);
      announce(`${framesLabel(latest.current.orderedIds[column].length)} selected in ${COLUMN_LABEL[column]}`);
      return;
    }
    if (isMod || e.altKey) return;
    if (key === "k" || key === "d") {
      e.preventDefault();
      moveFromKeyboard(key === "k" ? "keep" : "discard");
    } else if (e.key === "Delete") {
      e.preventDefault();
      requestDelete();
    } else if (e.key === "Escape" && selectedTotal > 0) {
      e.preventDefault();
      selection.clear();
      announce("Selection cleared");
    }
  };

  /** A filter swaps the Discard cards out from under the selection, so a
   *  stale pick cannot linger there invisibly. */
  const selectDiscardFilter = (value: string) => {
    setDiscardFilter(value);
    setVisible((cur) => ({ ...cur, discard: PAGE_SIZE }));
    selection.remove(selectedIn.discard);
    setAssignState({ kind: "idle" });
  };

  const dropStateOf = (column: ColumnId) =>
    !drag || drag.source === column ? "none" : dropHover === column ? "hover" : "target";

  const deleteCount = deleteState?.ids.length ?? 0;

  return (
    <div className="curation" ref={boardRef} tabIndex={-1} onKeyDown={onKeyDown}>
      <div className="curation__bar card" role="toolbar" aria-label="Selection">
        <div className="curation__bar-main">
          <span className="curation__bar-count" aria-live="off">
            {selectedTotal > 0
              ? `${framesLabel(selectedTotal)} selected`
              : "Click, Ctrl-click, Shift-click or draw a box to select"}
          </span>
          <button
            type="button"
            className="chip curation__action"
            disabled={selectedIn.discard.length === 0}
            onClick={() => void move(selectedIn.discard, "keep")}
            aria-keyshortcuts="K"
          >
            ← Keep{selectedIn.discard.length > 0 ? ` ${selectedIn.discard.length}` : ""}
            <kbd>K</kbd>
          </button>
          <button
            type="button"
            className="chip curation__action"
            disabled={selectedIn.keep.length === 0}
            onClick={() => void move(selectedIn.keep, "discard")}
            aria-keyshortcuts="D"
          >
            Discard{selectedIn.keep.length > 0 ? ` ${selectedIn.keep.length}` : ""} →<kbd>D</kbd>
          </button>
          <button
            type="button"
            className="chip curation__action curation__action--danger"
            disabled={selectedTotal === 0}
            onClick={requestDelete}
            aria-keyshortcuts="Delete"
          >
            Delete…<kbd>Del</kbd>
          </button>
          <button
            type="button"
            className="chip"
            disabled={selectedTotal === 0}
            onClick={selection.clear}
            aria-keyshortcuts="Escape"
          >
            Clear
          </button>
          <div className="curation__density" role="group" aria-label="Card size">
            <button
              type="button"
              className="chip"
              aria-pressed={isCompact}
              onClick={() => setIsCompact(true)}
            >
              Thumbnails
            </button>
            <button
              type="button"
              className="chip"
              aria-pressed={!isCompact}
              onClick={() => setIsCompact(false)}
            >
              Details
            </button>
          </div>
        </div>
        {selectedTotal > 0 && (
          <SelectionToolbar
            concepts={concepts}
            conceptId={assignConceptId}
            onConceptIdChange={setAssignConceptId}
            onAssign={() => void runAssign(true)}
            onRemove={() => void runAssign(false)}
            state={assignState}
          />
        )}
      </div>

      {error && (
        <p className="dataset__err" role="alert">
          {error}
        </p>
      )}
      {notice && (
        <div className="curation__notice">
          <p className="dataset__done">{notice.text}</p>
          <SkippedFiles files={notice.skipped} />
        </div>
      )}

      <div className="curation__columns">
        {(["keep", "discard"] as const).map((column) => (
          <CurationColumn
            key={column}
            column={column}
            frames={columns[column]}
            selected={selected}
            selectedHere={selectedIn[column].length}
            activeId={activeIds[column]}
            visibleCount={visible[column]}
            isCompact={isCompact}
            isClipMode={isClipMode}
            imageUrlFor={imageUrlFor}
            tokensByFrameId={tokensByFrameId}
            dropState={dropStateOf(column)}
            dragCount={drag?.count ?? 0}
            cardHandlers={cardHandlers}
            columnHandlers={columnHandlers}
            filters={
              column === "discard" ? (
                <RejectionChips
                  frames={discardAll}
                  active={discardShown === discardAll ? ALL_DISCARDED : discardFilter}
                  onSelect={selectDiscardFilter}
                />
              ) : undefined
            }
          />
        ))}
      </div>

      <div ref={ghostRef} className="curation__ghost" aria-hidden="true" />
      <p className="visually-hidden" role="status" aria-live="polite">
        {announcement.text}
        {/* A trailing no-break space on every other message makes a
            repeated announcement ("1 frame moved…") count as new text. */}
        {announcement.seq % 2 === 1 ? String.fromCharCode(0xa0) : ""}
      </p>

      <ConfirmDialog
        isOpen={deleteState !== null}
        title={`Delete ${framesLabel(deleteCount)}?`}
        confirmLabel={`Delete ${framesLabel(deleteCount)}`}
        tone="danger"
        isBusy={deleteState?.isBusy ?? false}
        error={deleteState?.error ?? null}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleteState(null)}
      >
        <p>
          This cannot be undone: the frames' files are removed from disk; your source videos are not
          touched.
        </p>
        <p>To set frames aside without deleting them, move them to Discard instead.</p>
      </ConfirmDialog>
    </div>
  );
}
