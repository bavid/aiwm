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
  MAX_FRAME_IDS,
  unassignConcept,
  updateDatasetFrame,
  type DatasetConcept,
  type DatasetFrame,
  type SkippedFile,
} from "../../lib/ipc";
import { CurationColumn, type CardHandlers, type ColumnHandlers } from "./CurationColumn";
import {
  ALL_DISCARDED,
  chunk,
  COLUMN_LABEL,
  columnOf,
  errorText,
  filesLabel,
  framesLabel,
  isColumnId,
  withoutFinalStop,
  matchesDiscardFilter,
  otherColumn,
  type ColumnId,
} from "./curation";
import { DeleteFramesDialog, type DeleteRequest } from "./DeleteFramesDialog";
import { formatBytes } from "./format";
import { RejectionChips } from "./RejectionChips";
import { SelectionBar } from "./SelectionBar";
import { SelectionToolbar, type AssignState } from "./SelectionToolbar";
import { SkippedFiles } from "../../components/SkippedFiles";
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
  /** The prep run is still writing frames: deleting waits for it (the core
   *  refuses too), like the housekeeping panel. */
  isPrepRunning: boolean;
  imageUrlFor: (frame: DatasetFrame) => string;
  tokensByFrameId: Record<string, string[]>;
  concepts: DatasetConcept[];
  /** Refetch the frame list after a write. */
  onFramesChanged: () => void;
  /** Frame files were deleted: the disk usage changed too. */
  onFilesDeleted: () => void;
  onConceptsChanged: () => void;
};

type Notice = { text: string; skipped: SkippedFile[] };
type DragInfo = { source: ColumnId; ids: string[] };
/** Where focus goes once a move or delete takes cards away: this card, or
 *  (`null`) the column's grid. */
type Refocus = { column: ColumnId; id: string | null };

/** Typing in a field must not trigger the board's single-key shortcuts. */
function isTypingTarget(el: EventTarget): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.isContentEditable || el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLSelectElement) return true;
  if (!(el instanceof HTMLInputElement)) return false;
  return el.type !== "checkbox" && el.type !== "radio" && el.type !== "button";
}

function focusedFrameId(): string | undefined {
  const el = document.activeElement;
  return el instanceof HTMLElement ? el.dataset.frameId : undefined;
}

/** The curation board: Keep on the left, Discard on the right. Select with
 *  click / Ctrl / Shift / a rubber band / Select all; move by drag & drop,
 *  the toolbar or K / D; delete with Delete. One request per move. */
export function CurationBoard({
  datasetId,
  frames: polledFrames,
  isClipMode,
  isPrepRunning,
  imageUrlFor,
  tokensByFrameId,
  concepts,
  onFramesChanged,
  onFilesDeleted,
  onConceptsChanged,
}: Props) {
  const { frames, beginMove, settleMove, rollbackMove } = useOptimisticFrames(polledFrames);
  const { selected, click, toggle, replace, extend, remove, clear } = useFrameSelection();

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
  const [deleteRequest, setDeleteRequest] = useState<DeleteRequest | null>(null);
  const [assignConceptId, setAssignConceptId] = useState("");
  const [assignState, setAssignState] = useState<AssignState>({ kind: "idle" });

  const boardRef = useRef<HTMLDivElement>(null);
  const ghostRef = useRef<HTMLDivElement>(null);
  const dragInfo = useRef<DragInfo | null>(null);
  const bandBase = useRef<ReadonlySet<string>>(new Set());
  const lastColumn = useRef<ColumnId>("keep");
  const refocus = useRef<Refocus | null>(null);

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

  const columns: Record<ColumnId, DatasetFrame[]> = { keep, discard: discardShown };
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

  // The latest render, for the stable handlers below.
  const latest = useRef({ orderedIds, selectedIn, selected });
  useLayoutEffect(() => {
    latest.current = { orderedIds, selectedIn, selected };
  });

  /** Focus should land on the card after the first one taken away from
   *  `column` (else the one before it, else the column's grid). */
  const planRefocus = useCallback((ids: readonly string[], column: ColumnId) => {
    const ordered = latest.current.orderedIds[column];
    const gone = new Set(ids);
    const first = ordered.findIndex((id) => gone.has(id));
    const remaining = (list: string[]) => list.find((id) => !gone.has(id));
    const id =
      first < 0
        ? null
        : (remaining(ordered.slice(first)) ?? remaining(ordered.slice(0, first).reverse()) ?? null);
    refocus.current = { column, id };
  }, []);

  // Apply a planned refocus once no dialog holds focus (the dialog closes in
  // its own layout effect, which runs before this one).
  useLayoutEffect(() => {
    const want = refocus.current;
    const board = boardRef.current;
    if (!want || !board || deleteRequest) return;
    refocus.current = null;
    const column = board.querySelector(`[data-column="${want.column}"]`);
    const card = want.id
      ? column?.querySelector<HTMLElement>(`[data-frame-id="${CSS.escape(want.id)}"]`)
      : null;
    const target = card ?? column?.querySelector<HTMLElement>('[role="grid"]') ?? board;
    target.focus({ preventScroll: true });
  });

  // A delete that empties the dataset unmounts the card focus went to: land
  // on the empty state instead of the page body.
  const emptyRef = useRef<HTMLParagraphElement>(null);
  const isEmpty = frames.length === 0;
  useLayoutEffect(() => {
    const active = document.activeElement;
    if (isEmpty && (active === null || active === document.body)) emptyRef.current?.focus();
  }, [isEmpty]);

  // --- writes -------------------------------------------------------------
  /** One optimistic batch per request-sized chunk, sent one after another; a
   *  failure rolls back its own chunk and every unsent one, and says how many
   *  frames were already moved. */
  const move = useCallback(
    async (ids: string[], target: ColumnId) => {
      if (ids.length === 0) return;
      const excluded = target === "discard";
      const label = COLUMN_LABEL[target];
      planRefocus(ids, otherColumn(target));
      const parts = chunk(ids, MAX_FRAME_IDS).map((part) => ({
        part,
        batch: beginMove(part, excluded),
      }));
      remove(ids);
      setError(null);
      setNotice(null);
      announce(`${framesLabel(ids.length)} moved to ${label}`);
      let requested = 0;
      let updated = 0;
      try {
        for (const [index, { part, batch }] of parts.entries()) {
          try {
            const summary = await bulkUpdateDatasetFrames(datasetId, part, excluded);
            settleMove(batch);
            requested += summary.requested;
            updated += summary.updated;
          } catch (e) {
            const unmoved = parts.slice(index);
            for (const rest of unmoved) rollbackMove(rest.batch);
            // The frames that stayed put are selected again, ready to retry.
            extend(
              latest.current.selected,
              unmoved.flatMap((rest) => rest.part),
            );
            const before = updated > 0 ? ` ${framesLabel(updated)} were moved before it.` : "";
            // The alert below announces it; no second announcement.
            setError(
              `Could not move ${framesLabel(ids.length - requested)} to ${label}: ` +
                `${withoutFinalStop(errorText(e))}.${before}`,
            );
            return;
          }
        }
        if (updated < requested) {
          setNotice({
            text: `${updated} of ${requested} frames moved — the rest no longer exist.`,
            skipped: [],
          });
        }
      } finally {
        onFramesChanged();
      }
    },
    [
      datasetId,
      planRefocus,
      beginMove,
      settleMove,
      rollbackMove,
      remove,
      extend,
      announce,
      onFramesChanged,
    ],
  );

  /** A toolbar/keyboard move to `target` acts on the selection in the other
   *  column, or else on the focused card. */
  const moveSelection = (target: ColumnId) => {
    const source = otherColumn(target);
    const focused = focusedFrameId();
    const ids =
      selectedTotal > 0
        ? selectedIn[source]
        : focused && columnById.get(focused) === source
          ? [focused]
          : [];
    void move(ids, target);
  };

  /** Counts for the delete confirm: per column, and how many of `ids` are
   *  not rendered right now (paged out or filtered away). */
  const describeDelete = (ids: string[]) => {
    const onScreen = new Set(
      (["keep", "discard"] as const).flatMap((c) => orderedIds[c].slice(0, visible[c])),
    );
    return {
      ids,
      fromKeep: ids.filter((id) => columnById.get(id) === "keep").length,
      fromDiscard: ids.filter((id) => columnById.get(id) === "discard").length,
      offscreen: ids.filter((id) => !onScreen.has(id)).length,
    };
  };

  const requestDelete = () => {
    if (isPrepRunning) return;
    const focused = focusedFrameId();
    const ids =
      selectedTotal > 0
        ? [...selectedIn.keep, ...selectedIn.discard]
        : focused && columnById.has(focused)
          ? [focused]
          : [];
    if (ids.length === 0) return;
    setDeleteRequest({ ...describeDelete(ids), isBusy: false, error: null });
  };

  /** Deletes in request-sized chunks, one after another. A failure midway
   *  keeps the dialog open with what was already deleted; only the ids that
   *  really went leave the selection and the request. */
  const confirmDelete = async () => {
    if (!deleteRequest) return;
    const { ids } = deleteRequest;
    setDeleteRequest({ ...deleteRequest, isBusy: true, error: null });
    const focused = focusedFrameId();
    const column = (focused && columnById.get(focused)) || lastColumn.current;
    const gone: string[] = [];
    const skipped: SkippedFile[] = [];
    let deleted = 0;
    let files = 0;
    let bytes = 0;
    const summaryText = () =>
      `Deleted ${framesLabel(deleted)} and ${filesLabel(files)} — ` +
      `${formatBytes(bytes)} freed.`;
    try {
      for (const part of chunk(ids, MAX_FRAME_IDS)) {
        const summary = await deleteDatasetFrames(datasetId, part);
        gone.push(...part);
        deleted += summary.deleted;
        files += summary.deleted_files;
        bytes += summary.freed_bytes;
        skipped.push(...summary.skipped_files);
      }
      planRefocus(ids, column);
      remove(ids);
      setDeleteRequest(null);
      setNotice({ text: summaryText(), skipped });
      announce(summaryText());
    } catch (e) {
      remove(gone);
      const done = new Set(gone);
      const rest = ids.filter((id) => !done.has(id));
      if (gone.length > 0) setNotice({ text: summaryText(), skipped });
      const before =
        gone.length > 0
          ? ` — before it: ${summaryText()} The ${framesLabel(rest.length)} below were not deleted.`
          : "";
      setDeleteRequest({
        ...describeDelete(rest),
        isBusy: false,
        error: `${errorText(e)}${before}`,
      });
    } finally {
      onFramesChanged();
      if (gone.length > 0) onFilesDeleted();
    }
  };

  /** Concept writes go in request-sized chunks too, one after another. */
  const runAssign = async (attach: boolean) => {
    if (!assignConceptId || selectedTotal === 0) return;
    const ids = [...selected];
    /** Frames of `ids` already sent in chunks that succeeded. */
    let done = 0;
    try {
      if (attach) {
        let attached = 0;
        for (const part of chunk(ids, MAX_FRAME_IDS)) {
          attached += (await assignConcept(assignConceptId, part)).attached;
          done += part.length;
        }
        setAssignState({ kind: "done", text: `${attached} of ${ids.length} assigned` });
      } else {
        for (const part of chunk(ids, MAX_FRAME_IDS)) {
          await unassignConcept(assignConceptId, part);
          done += part.length;
        }
        setAssignState({ kind: "done", text: `${ids.length} removed` });
      }
    } catch (e) {
      const before =
        done > 0
          ? ` — ${framesLabel(done)} of ${ids.length.toLocaleString()} were already ` +
            `${attach ? "assigned" : "removed"}`
          : "";
      setAssignState({ kind: "error", text: `${withoutFinalStop(errorText(e))}${before}` });
    } finally {
      if (done > 0) onConceptsChanged();
    }
  };

  // --- stable handlers: none depends on the selection itself --------------
  const latestMove = useRef(move);
  useLayoutEffect(() => {
    latestMove.current = move;
  });

  const endDrag = useCallback(() => {
    dragInfo.current = null;
    setDrag(null);
    setDropHover(null);
  }, []);

  const cardHandlers = useMemo<CardHandlers>(
    () => ({
      onSelectClick: (frame, mods, fallbackAnchor) => {
        const column = columnOf(frame);
        lastColumn.current = column;
        click(frame.id, latest.current.orderedIds[column], mods, fallbackAnchor);
      },
      onToggleSelected: (frame) => toggle(frame.id),
      onFocusCard: (frame) => {
        const column = columnOf(frame);
        lastColumn.current = column;
        setActiveIds((cur) => (cur[column] === frame.id ? cur : { ...cur, [column]: frame.id }));
      },
      onDragStart: (frame, e: DragEvent<HTMLElement>) => {
        const source = columnOf(frame);
        const { selected: sel, selectedIn: inCol } = latest.current;
        const ids = sel.has(frame.id) ? inCol[source] : [frame.id];
        if (!sel.has(frame.id)) click(frame.id, [], { toggle: false, range: false });
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
      onMove: (frame) => void latestMove.current([frame.id], otherColumn(columnOf(frame))),
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
    [click, toggle, endDrag, onFramesChanged],
  );

  const columnHandlers = useMemo<ColumnHandlers>(
    () => ({
      onSelectAll: (column) => {
        lastColumn.current = column;
        replace(latest.current.orderedIds[column]);
      },
      onSelectNone: (column) => remove(latest.current.selectedIn[column]),
      onShowMore: (column) =>
        setVisible((cur) => ({ ...cur, [column]: cur[column] + PAGE_SIZE })),
      onBandStart: (column, additive) => {
        lastColumn.current = column;
        bandBase.current = additive ? latest.current.selected : new Set();
      },
      onBandChange: (_column, ids) => extend(bandBase.current, ids),
      onBandEnd: (column, ids) => {
        // Put focus on the first card the band caught, so arrows go on
        // from there.
        const first = latest.current.orderedIds[column].find((id) => ids.includes(id));
        const card = first
          ? boardRef.current?.querySelector<HTMLElement>(`[data-frame-id="${CSS.escape(first)}"]`)
          : null;
        card?.focus({ preventScroll: true });
      },
      onEmptyClick: (column) => {
        lastColumn.current = column;
        clear();
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
        void latestMove.current(info.ids, column);
      },
    }),
    [replace, remove, extend, clear, endDrag],
  );

  // --- keyboard -----------------------------------------------------------
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.defaultPrevented || isTypingTarget(e.target)) return;
    if (e.target instanceof Element && e.target.closest("dialog")) return;
    const key = e.key.toLowerCase();
    const isMod = e.ctrlKey || e.metaKey;
    if (isMod && key === "a") {
      const el = e.target instanceof Element ? e.target.closest<HTMLElement>("[data-column]") : null;
      const named = el?.dataset.column;
      const column = named && isColumnId(named) ? named : lastColumn.current;
      e.preventDefault();
      columnHandlers.onSelectAll(column);
      announce(`${framesLabel(orderedIds[column].length)} selected in ${COLUMN_LABEL[column]}`);
      return;
    }
    if (isMod || e.altKey) return;
    if (key === "k" || key === "d") {
      e.preventDefault();
      moveSelection(key === "k" ? "keep" : "discard");
    } else if (e.key === "Delete") {
      e.preventDefault();
      requestDelete();
    } else if (e.key === "Escape" && selectedTotal > 0) {
      e.preventDefault();
      clear();
      announce("Selection cleared");
    }
  };

  /** A filter swaps the Discard cards out from under the selection, so a
   *  stale pick cannot linger there invisibly. */
  const selectDiscardFilter = (value: string) => {
    setDiscardFilter(value);
    setVisible((cur) => ({ ...cur, discard: PAGE_SIZE }));
    remove(selectedIn.discard);
    setAssignState({ kind: "idle" });
  };

  const dropStateOf = (column: ColumnId) =>
    !drag || drag.source === column ? "none" : dropHover === column ? "hover" : "target";

  return (
    // The board catches the K / D / Delete / Escape shortcuts bubbling up
    // from its focusable cards; every one of them is also a real button in
    // the selection bar (jsx-a11y's documented container exception).
    // eslint-disable-next-line jsx-a11y/no-static-element-interactions
    <div className="curation" ref={boardRef} tabIndex={-1} onKeyDown={onKeyDown}>
      {frames.length > 0 && (
        <SelectionBar
          selectedTotal={selectedTotal}
          toKeep={selectedIn.discard.length}
          toDiscard={selectedIn.keep.length}
          isCompact={isCompact}
          onKeep={() => void move(selectedIn.discard, "keep")}
          onDiscard={() => void move(selectedIn.keep, "discard")}
          canDelete={!isPrepRunning}
          onDelete={requestDelete}
          onClear={clear}
          onCompactChange={setIsCompact}
        >
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
        </SelectionBar>
      )}

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

      {frames.length === 0 ? (
        <p className="curation__empty" ref={emptyRef} tabIndex={-1}>
          {isPrepRunning
            ? "No frames yet — the prep run is still working."
            : "No frames left in this dataset."}
        </p>
      ) : (
        <div className="curation__columns">
          {(["keep", "discard"] as const).map((column) => (
            <CurationColumn
              key={column}
              column={column}
              frames={columns[column]}
              totalCount={column === "discard" ? discardAll.length : keep.length}
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
      )}

      <div ref={ghostRef} className="curation__ghost" aria-hidden="true" />
      <p className="visually-hidden" role="status" aria-live="polite">
        {announcement.text}
        {/* A trailing no-break space on every other message makes a
            repeated announcement ("1 frame moved…") count as new text. */}
        {announcement.seq % 2 === 1 ? String.fromCharCode(0xa0) : ""}
      </p>

      <DeleteFramesDialog
        request={deleteRequest}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleteRequest(null)}
      />
    </div>
  );
}
