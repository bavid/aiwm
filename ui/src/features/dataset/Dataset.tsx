import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import {
  useAbout,
  useConcepts,
  useDatasetFramesForDataset,
  useDatasets,
  useFrameConceptMap,
  useJobs,
} from "../../lib/hooks";
import {
  assignConcept,
  cancelJob,
  datasetFrameImageUrl,
  datasetFrameImageUrlByDataset,
  exportDatasetById,
  jobDetail,
  submitJob,
  unassignConcept,
  updateDataset,
  updateDatasetFrame,
  type CaptionOrder,
  type DatasetFrame,
  type DatasetPrepParams,
  type JobDetail,
  type JobState,
} from "../../lib/ipc";
import { ConceptsPanel } from "./ConceptsPanel";
import { ExportCard, type ExportState } from "./ExportCard";
import { FrameCard } from "./FrameCard";
import { LearnSets } from "./LearnSets";
import { PrepForm } from "./PrepForm";
import { RejectionChips } from "./RejectionChips";
import { SelectionToolbar, type AssignState } from "./SelectionToolbar";
import { tokenWarning } from "./tokens";
import "./dataset.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 900;
const PAGE_SIZE = 60;

/** The curation grid, or the guided set-by-set Learn mode over the same data. */
type ResultView = "grid" | "learn";

/** One shared empty array, so a card without concepts keeps the same
 *  `conceptTokens` identity across renders and stays memoised. */
const NO_TOKENS: string[] = [];

/** Prefer the dataset-keyed image route: an item outlives its prep job, so
 *  `job_id` can be `null`. The job-keyed URL stays as the fallback for rows
 *  written before datasets became objects. */
function frameImageUrl(coreApiPort: number, frame: DatasetFrame): string {
  if (frame.dataset_id) {
    return datasetFrameImageUrlByDataset(coreApiPort, frame.dataset_id, frame.id);
  }
  return frame.job_id ? datasetFrameImageUrl(coreApiPort, frame.job_id, frame.id) : "";
}

type Props = {
  /** Hands the just-exported dataset to the Training tab (App owns the tab
   *  switch and the hand-over state). */
  onTrainLora: (datasetId: string) => void;
};

export function DatasetStudio({ onTrainLora }: Props) {
  const about = useAbout();
  const { data: jobs, refetch: refetchJobs } = useJobs({ limit: 50 });
  const { data: datasets, refetch: refetchDatasets } = useDatasets();

  const datasetList = useMemo(() => datasets ?? [], [datasets]);

  const [sendError, setSendError] = useState<string | null>(null);

  // --- selection --------------------------------------------------------
  const [activeDatasetId, setActiveDatasetId] = useState<string | null>(null);
  /** The job just submitted, until its dataset row shows up in the list. */
  const [pendingJobId, setPendingJobId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  const [view, setView] = useState<ResultView>("grid");
  const [filter, setFilter] = useState("");
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(() => new Set());
  const [assignConceptId, setAssignConceptId] = useState("");
  const [assignState, setAssignState] = useState<AssignState>({ kind: "idle" });
  const [triggerDraft, setTriggerDraft] = useState<string | null>(null);
  /** Failures from the per-card actions (restore / exclude / caption) and from
   *  Cancel, shown above the grid -- these used to reject silently. */
  const [gridError, setGridError] = useState<string | null>(null);
  // The trigger word commits on Enter and on blur (click-away-to-save); this
  // guards against both firing for the same edit -- Enter moves focus off the
  // input, which would otherwise blur-commit a second time.
  const triggerSettled = useRef(false);

  // --- export -----------------------------------------------------------
  const [destDir, setDestDir] = useState("");
  const [captionOrder, setCaptionOrder] = useState<CaptionOrder>("prose_first");
  const [exportState, setExportState] = useState<ExportState>({ kind: "idle" });

  const activeDataset = datasetList.find((d) => d.id === activeDatasetId) ?? null;
  const { data: frames, refetch: refetchFrames } = useDatasetFramesForDataset(activeDatasetId);
  const { data: concepts, refetch: refetchConcepts } = useConcepts(activeDatasetId);
  const { data: conceptMap, refetch: refetchConceptMap } = useFrameConceptMap(activeDatasetId);

  const frameList = useMemo(() => frames ?? [], [frames]);
  const conceptList = useMemo(() => concepts ?? [], [concepts]);
  const keptCount = useMemo(
    () => frameList.filter((f) => f.rejection_reason === "" && !f.excluded).length,
    [frameList],
  );
  const shownFrames = useMemo(
    () => frameList.filter((f) => f.rejection_reason === filter),
    [frameList, filter],
  );

  // The live status of the run that produced (or is producing) this dataset.
  const liveJobId = activeDataset?.prep_job_id ?? pendingJobId;
  const activeJob =
    (detail?.job.id === liveJobId ? detail.job : null) ??
    (jobs ?? []).find((j) => j.id === liveJobId) ??
    null;
  const isRunning = !!activeJob && !DONE.includes(activeJob.state);
  /** Clips mode changes what a card is: a whole video with a length and
   *  curator-set in/out points, not a still. */
  const isClipMode = activeDataset?.mode === "clips";

  const tokenByConceptId = useMemo(() => {
    const map: Record<string, string> = {};
    for (const c of conceptList) map[c.id] = c.token;
    return map;
  }, [conceptList]);

  /** frame id -> its concept tokens, resolved once per poll so each card gets
   *  a stable array instead of a fresh one on every container render. */
  const tokensByFrameId = useMemo(() => {
    const map: Record<string, string[]> = {};
    for (const [frameId, ids] of Object.entries(conceptMap ?? {})) {
      const tokens = ids.map((id) => tokenByConceptId[id]).filter((t): t is string => !!t);
      if (tokens.length > 0) map[frameId] = tokens;
    }
    return map;
  }, [conceptMap, tokenByConceptId]);

  const imageUrlFor = useCallback(
    (frame: DatasetFrame) => (about ? frameImageUrl(about.core_api_port, frame) : ""),
    [about],
  );

  const refetchConceptData = useCallback(() => {
    refetchConcepts();
    refetchConceptMap();
  }, [refetchConcepts, refetchConceptMap]);

  const triggerId = useId();

  // Land on the newest dataset when the tab opens, unless a freshly submitted
  // run is still waiting for its own dataset row.
  useEffect(() => {
    if (activeDatasetId !== null || pendingJobId !== null) return;
    if (datasetList.length > 0) setActiveDatasetId(datasetList[0].id);
  }, [activeDatasetId, pendingJobId, datasetList]);

  // A submitted run's dataset appears a moment later -- adopt it then.
  useEffect(() => {
    if (pendingJobId === null) return;
    const produced = datasetList.find((d) => d.prep_job_id === pendingJobId);
    if (!produced) return;
    setActiveDatasetId(produced.id);
    setPendingJobId(null);
  }, [pendingJobId, datasetList]);

  useEffect(() => {
    if (!liveJobId || !isRunning) return;
    let alive = true;
    const tick = async () => {
      try {
        const d = await jobDetail(liveJobId);
        if (alive) setDetail(d);
      } catch {
        /* transient -- next tick retries */
      }
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [liveJobId, isRunning]);

  const selectDataset = (id: string) => {
    setActiveDatasetId(id);
    setDetail(null);
    setVisibleCount(PAGE_SIZE);
    setFilter("");
    setSelectedIds(new Set());
    setAssignState({ kind: "idle" });
    setTriggerDraft(null);
    triggerSettled.current = false;
    setGridError(null);
    setExportState({ kind: "idle" });
  };

  /** Switching the verdict filter swaps the visible cards out from under the
   *  selection, so a stale pick cannot linger invisibly in the toolbar. */
  const selectFilter = (reason: string) => {
    setFilter(reason);
    setVisibleCount(PAGE_SIZE);
    setSelectedIds(new Set());
    setAssignState({ kind: "idle" });
  };

  const start = async (params: DatasetPrepParams) => {
    setSendError(null);
    try {
      const job = await submitJob({ job_type: "dataset_prep", params });
      setPendingJobId(job.id);
      setActiveDatasetId(null);
      setDetail(null);
      setVisibleCount(PAGE_SIZE);
      setFilter("");
      setSelectedIds(new Set());
      setGridError(null);
      setExportState({ kind: "idle" });
      refetchJobs();
      refetchDatasets();
    } catch (e) {
      setSendError(String(e));
    }
  };

  const cancel = async () => {
    if (!liveJobId) return;
    setGridError(null);
    try {
      await cancelJob(liveJobId);
    } catch (e) {
      setGridError(`Could not cancel the run: ${e}`);
    }
  };

  /** Commits on Enter and on blur; `triggerSettled` keeps the pair from
   *  writing twice. The draft is only dropped once the write succeeded, so a
   *  failure leaves the curator's text in the field to retry. */
  const commitTrigger = async () => {
    if (triggerSettled.current) return;
    triggerSettled.current = true;
    if (!activeDataset || triggerDraft === null) return;
    const next = triggerDraft.trim();
    if (next === activeDataset.trigger_word) {
      setTriggerDraft(null);
      return;
    }
    try {
      await updateDataset(activeDataset.id, { trigger_word: next });
      setTriggerDraft(null);
      refetchDatasets();
    } catch (e) {
      setGridError(`Could not save the trigger word: ${e}`);
    }
  };

  // The grid's per-card handlers are stable so `FrameCard`'s memoisation
  // holds: each takes the frame it acts on instead of closing over it.
  const editCaption = useCallback(
    async (frame: DatasetFrame, caption: string) => {
      if (caption === frame.caption) return;
      try {
        await updateDatasetFrame(frame.id, { caption });
        refetchFrames();
      } catch (e) {
        setGridError(`Could not save the caption: ${e}`);
      }
    },
    [refetchFrames],
  );

  const toggleExcluded = useCallback(
    async (frame: DatasetFrame) => {
      try {
        await updateDatasetFrame(frame.id, { excluded: !frame.excluded });
        refetchFrames();
      } catch (e) {
        setGridError(`Could not change the exclude flag: ${e}`);
      }
    },
    [refetchFrames],
  );

  const restore = useCallback(
    async (frame: DatasetFrame) => {
      try {
        await updateDatasetFrame(frame.id, { restore: true });
        refetchFrames();
      } catch (e) {
        setGridError(`Could not restore the frame: ${e}`);
      }
    },
    [refetchFrames],
  );

  /** Both bounds go in one write: the card validates them as a pair, and an
   *  explicit `null` clears one back to the clip's natural start/end. */
  const commitClipBounds = useCallback(
    async (frame: DatasetFrame, start: number | null, end: number | null) => {
      try {
        await updateDatasetFrame(frame.id, { clip_start_secs: start, clip_end_secs: end });
        refetchFrames();
      } catch (e) {
        setGridError(`Could not save the clip bounds: ${e}`);
      }
    },
    [refetchFrames],
  );

  const toggleSelected = useCallback((frame: DatasetFrame) => {
    setSelectedIds((cur) => {
      const next = new Set(cur);
      if (!next.delete(frame.id)) next.add(frame.id);
      return next;
    });
  }, []);

  const clearSelection = useCallback(() => setSelectedIds(new Set()), []);

  const runAssign = async (attach: boolean) => {
    if (!assignConceptId || selectedIds.size === 0) return;
    const ids = [...selectedIds];
    try {
      if (attach) {
        const summary = await assignConcept(assignConceptId, ids);
        setAssignState({
          kind: "done",
          text: `${summary.attached} of ${summary.requested} assigned`,
        });
      } else {
        await unassignConcept(assignConceptId, ids);
        setAssignState({ kind: "done", text: `${ids.length} removed` });
      }
      refetchConcepts();
      refetchConceptMap();
    } catch (e) {
      setAssignState({ kind: "error", text: String(e) });
    }
  };

  const runExport = async () => {
    if (!activeDatasetId || !destDir.trim()) return;
    setExportState({ kind: "busy" });
    try {
      const summary = await exportDatasetById(activeDatasetId, destDir.trim(), captionOrder);
      setExportState({ kind: "done", exported: summary.exported, destDir: summary.dest_dir });
      refetchDatasets();
    } catch (e) {
      setExportState({ kind: "error", message: String(e) });
    }
  };

  const events = detail?.job.id === activeJob?.id ? (detail?.events ?? []) : [];
  const lastEvent = events.length > 0 ? events[events.length - 1] : null;
  const triggerValue = triggerDraft ?? activeDataset?.trigger_word ?? "";
  const triggerWarn = triggerValue.trim() === "" ? null : tokenWarning(triggerValue);

  return (
    <section className="dataset">
      <div className="card dataset__form-card">
        <h2>Dataset prep</h2>
        <p className="dataset__intro">
          Turn a folder tree of video/images into a curated, captioned dataset — one folder per
          tag/style, ready for any external LoRA trainer. This does not train anything itself.
        </p>

        <PrepForm isRunning={isRunning} error={sendError} onStart={start} />

        {datasetList.length > 0 && (
          <div className="dataset__history">
            <span className="dataset__history-label">Datasets</span>
            <ul>
              {datasetList.slice(0, 12).map((d) => (
                <li key={d.id}>
                  <button
                    type="button"
                    className="chip"
                    aria-pressed={d.id === activeDatasetId}
                    onClick={() => selectDataset(d.id)}
                  >
                    {d.name} · {d.mode}
                    {d.id === activeDatasetId ? ` · ${frameList.length} frames` : ""}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>

      <div className="dataset__result">
        {activeDataset || activeJob ? (
          <div className="card">
            <div className="dataset__status">
              <div>
                <strong>{activeDataset?.name ?? "New run"}</strong>
                {activeJob && (
                  <span className="dataset__state" data-state={activeJob.state}>
                    {activeJob.state}
                  </span>
                )}
              </div>
              {isRunning && (
                <button type="button" className="chip" onClick={cancel}>
                  Cancel
                </button>
              )}
            </div>
            {lastEvent && <p className="dataset__event">{lastEvent.message}</p>}
            {activeJob?.state === "failed" && activeJob.error_text && (
              <p className="dataset__err">{activeJob.error_text}</p>
            )}
            <p className="dataset__counts">
              {frameList.length} item(s) so far — {keptCount} kept for export
            </p>

            {activeDataset && (
              <label className="datasetform__field datasetform__field--inline" htmlFor={triggerId}>
                <span>Trigger word</span>
                <input
                  id={triggerId}
                  type="text"
                  value={triggerValue}
                  onChange={(e) => {
                    triggerSettled.current = false;
                    setTriggerDraft(e.target.value);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      commitTrigger();
                      e.currentTarget.blur();
                    }
                  }}
                  onBlur={commitTrigger}
                  placeholder="ghibli_xy"
                />
              </label>
            )}
            {activeDataset && triggerWarn && <p className="dataset__warn">{triggerWarn}</p>}
          </div>
        ) : (
          <div className="card dataset__empty">
            Point the pipeline at a folder of videos/images (.mp4, .png, .jpg, .jpeg, .webp).
            Files directly in the folder are tagged with the folder's name; for several styles,
            use subfolders — each immediate subfolder becomes a tag applied to every frame
            extracted from the videos/images inside it.
          </div>
        )}

        {activeDatasetId && (
          <div className="dataset__views" role="group" aria-label="Curation view">
            <button
              type="button"
              className="chip"
              aria-pressed={view === "grid"}
              onClick={() => setView("grid")}
            >
              Grid
            </button>
            <button
              type="button"
              className="chip"
              aria-pressed={view === "learn"}
              onClick={() => setView("learn")}
            >
              Learn
            </button>
          </div>
        )}

        {activeDatasetId && (
          <div hidden={view !== "grid"}>
            <ConceptsPanel
              datasetId={activeDatasetId}
              concepts={conceptList}
              onChanged={refetchConceptData}
            />
          </div>
        )}

        {/* Hidden rather than unmounted while the grid is up: the set index,
            selection, grouping and the concept draft survive a Grid<->Learn
            round-trip. Keyed by dataset, so switching datasets still resets
            all of that. */}
        {activeDatasetId && (
          <div hidden={view !== "learn"}>
            <LearnSets
              key={activeDatasetId}
              datasetId={activeDatasetId}
              frames={frameList}
              concepts={conceptList}
              conceptMap={conceptMap ?? {}}
              imageUrlFor={imageUrlFor}
              onChanged={refetchConceptData}
            />
          </div>
        )}

        {frameList.length > 0 && view === "grid" && (
          <>
            <RejectionChips frames={frameList} active={filter} onSelect={selectFilter} />

            {gridError && <p className="dataset__err">{gridError}</p>}

            {selectedIds.size > 0 && (
              <SelectionToolbar
                selectedCount={selectedIds.size}
                concepts={conceptList}
                conceptId={assignConceptId}
                onConceptIdChange={setAssignConceptId}
                onAssign={() => runAssign(true)}
                onRemove={() => runAssign(false)}
                onClear={clearSelection}
                state={assignState}
              />
            )}

            <div className="dataset__grid">
              {shownFrames.slice(0, visibleCount).map((frame) => (
                <FrameCard
                  key={frame.id}
                  frame={frame}
                  imageUrl={imageUrlFor(frame)}
                  isSelected={selectedIds.has(frame.id)}
                  conceptTokens={tokensByFrameId[frame.id] ?? NO_TOKENS}
                  isClipMode={isClipMode}
                  onToggleSelected={toggleSelected}
                  onCaptionCommit={editCaption}
                  onToggleExcluded={toggleExcluded}
                  onRestore={restore}
                  onClipBoundsCommit={commitClipBounds}
                />
              ))}
            </div>
            {visibleCount < shownFrames.length && (
              <button
                type="button"
                className="chip dataset__more"
                onClick={() => setVisibleCount((n) => n + PAGE_SIZE)}
              >
                Show more ({shownFrames.length - visibleCount} remaining)
              </button>
            )}

          </>
        )}

        {frameList.length > 0 && (
          <ExportCard
            destDir={destDir}
            onDestDirChange={setDestDir}
            captionOrder={captionOrder}
            onCaptionOrderChange={setCaptionOrder}
            keptCount={keptCount}
            isClipMode={isClipMode}
            state={exportState}
            onExport={runExport}
            onTrainLora={() => activeDatasetId && onTrainLora(activeDatasetId)}
          />
        )}
      </div>
    </section>
  );
}
