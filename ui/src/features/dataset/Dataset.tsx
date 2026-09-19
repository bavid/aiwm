import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import {
  useAbout,
  useConcepts,
  useDatasetFramesForDataset,
  useDatasets,
  useDatasetUsage,
  useFrameConceptMap,
  useJobs,
} from "../../lib/hooks";
import {
  cancelJob,
  datasetFrameImageUrl,
  datasetFrameImageUrlByDataset,
  exportDatasetById,
  jobDetail,
  submitJob,
  updateDataset,
  type CaptionOrder,
  type DatasetDeleteSummary,
  type DatasetFrame,
  type DatasetPrepParams,
  type JobDetail,
  type JobState,
} from "../../lib/ipc";
import { ConceptsPanel } from "./ConceptsPanel";
import { CurationBoard } from "./CurationBoard";
import { errorText, framesLabel, isDiscarded } from "./curation";
import { ExportCard, type ExportState } from "./ExportCard";
import { formatBytes } from "./format";
import { HousekeepingPanel } from "./HousekeepingPanel";
import { LearnSets } from "./LearnSets";
import { PrepForm } from "./PrepForm";
import { SkippedFiles } from "./SkippedFiles";
import { tokenWarning } from "./tokens";
import "./dataset.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 900;

/** The curation board, or the guided set-by-set Learn mode over the same data. */
type ResultView = "grid" | "learn";

/** What deleting a dataset did, shown once the tab has left it. */
type DeletedNotice = { name: string; summary: DatasetDeleteSummary };

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
  const [view, setView] = useState<ResultView>("grid");
  const [triggerDraft, setTriggerDraft] = useState<string | null>(null);
  /** Failures from Cancel and the trigger word, shown in the status card. */
  const [statusError, setStatusError] = useState<string | null>(null);
  const [deletedNotice, setDeletedNotice] = useState<DeletedNotice | null>(null);
  // The trigger word commits on Enter and on blur (click-away-to-save); this
  // guards against both firing for the same edit -- Enter moves focus off the
  // input, which would otherwise blur-commit a second time.
  const triggerSettled = useRef(false);
  /** The tab lands on the newest dataset once; after a deletion it stays on
   *  the list instead of silently opening another dataset. */
  const hasLanded = useRef(false);
  const deletedNoticeRef = useRef<HTMLDivElement>(null);

  // --- export -----------------------------------------------------------
  const [destDir, setDestDir] = useState("");
  const [captionOrder, setCaptionOrder] = useState<CaptionOrder>("prose_first");
  const [exportState, setExportState] = useState<ExportState>({ kind: "idle" });

  const activeDataset = datasetList.find((d) => d.id === activeDatasetId) ?? null;
  const { data: frames, refetch: refetchFrames } = useDatasetFramesForDataset(activeDatasetId);
  const { data: concepts, refetch: refetchConcepts } = useConcepts(activeDatasetId);
  const { data: conceptMap, refetch: refetchConceptMap } = useFrameConceptMap(activeDatasetId);
  const {
    usage,
    isLoading: isUsageLoading,
    refresh: refreshUsage,
  } = useDatasetUsage(activeDatasetId);

  const frameList = useMemo(() => frames ?? [], [frames]);
  const conceptList = useMemo(() => concepts ?? [], [concepts]);
  const keptCount = useMemo(() => frameList.filter((f) => !isDiscarded(f)).length, [frameList]);

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

  /** After a housekeeping action (dedup, cleanup, frame deletion): the frame
   *  list and the disk use change. Moves only touch the list — the usage is
   *  fetched on demand, since the core walks the folders for it. */
  const refetchAfterHousekeeping = useCallback(() => {
    refetchFrames();
    refreshUsage();
  }, [refetchFrames, refreshUsage]);

  const triggerId = useId();

  // Land on the newest dataset when the tab opens, unless a freshly submitted
  // run is still waiting for its own dataset row.
  useEffect(() => {
    if (hasLanded.current || activeDatasetId !== null || pendingJobId !== null) return;
    if (datasetList.length === 0) return;
    hasLanded.current = true;
    setActiveDatasetId(datasetList[0].id);
  }, [activeDatasetId, pendingJobId, datasetList]);

  // Back on the list after a deletion: focus the summary, so keyboard users
  // are not left on a button that no longer exists.
  useEffect(() => {
    if (deletedNotice) deletedNoticeRef.current?.focus();
  }, [deletedNotice]);

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

  /** Everything that belongs to one dataset starts fresh on a switch; the
   *  board resets itself (it is keyed by dataset). */
  const resetDatasetState = () => {
    setDetail(null);
    setTriggerDraft(null);
    triggerSettled.current = false;
    setStatusError(null);
    setExportState({ kind: "idle" });
  };

  const selectDataset = (id: string) => {
    hasLanded.current = true;
    setActiveDatasetId(id);
    setDeletedNotice(null);
    resetDatasetState();
  };

  const start = async (params: DatasetPrepParams) => {
    setSendError(null);
    try {
      const job = await submitJob({ job_type: "dataset_prep", params });
      hasLanded.current = true;
      setPendingJobId(job.id);
      setActiveDatasetId(null);
      setDeletedNotice(null);
      resetDatasetState();
      refetchJobs();
      refetchDatasets();
    } catch (e) {
      setSendError(errorText(e));
    }
  };

  const onDatasetDeleted = (summary: DatasetDeleteSummary) => {
    setDeletedNotice({ name: activeDataset?.name ?? "Dataset", summary });
    setActiveDatasetId(null);
    resetDatasetState();
    refetchDatasets();
    refetchJobs();
  };

  const cancel = async () => {
    if (!liveJobId) return;
    setStatusError(null);
    try {
      await cancelJob(liveJobId);
    } catch (e) {
      setStatusError(`Could not cancel the run: ${errorText(e)}`);
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
      setStatusError(`Could not save the trigger word: ${errorText(e)}`);
    }
  };

  const runExport = async () => {
    if (!activeDatasetId || !destDir.trim()) return;
    setExportState({ kind: "busy" });
    try {
      const summary = await exportDatasetById(activeDatasetId, destDir.trim(), captionOrder);
      setExportState({ kind: "done", exported: summary.exported, destDir: summary.dest_dir });
      refetchDatasets();
      refreshUsage();
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

        {deletedNotice && (
          <div className="dataset__deleted" role="status" ref={deletedNoticeRef} tabIndex={-1}>
            <p className="dataset__done">
              Deleted “{deletedNotice.name}”: {framesLabel(deletedNotice.summary.frames)},{" "}
              {deletedNotice.summary.deleted_files.toLocaleString()} files,{" "}
              {formatBytes(deletedNotice.summary.freed_bytes)} freed.
              {deletedNotice.summary.export_dir_kept &&
                ` Your export folder was kept: ${deletedNotice.summary.export_dir_kept}`}
            </p>
            <SkippedFiles files={deletedNotice.summary.skipped_files} />
          </div>
        )}

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
            {statusError && <p className="dataset__err">{statusError}</p>}
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

        {activeDataset && (
          <HousekeepingPanel
            key={`housekeeping-${activeDataset.id}`}
            dataset={activeDataset}
            usage={usage}
            isUsageLoading={isUsageLoading}
            onRefreshUsage={refreshUsage}
            isRunning={isRunning}
            onChanged={refetchAfterHousekeeping}
            onDeleted={onDatasetDeleted}
          />
        )}

        {activeDatasetId && (
          <div className="dataset__views" role="group" aria-label="Curation view">
            <button
              type="button"
              className="chip"
              aria-pressed={view === "grid"}
              onClick={() => setView("grid")}
            >
              Sort
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

        {/* Hidden rather than unmounted while the board is up: the set index,
            selection, grouping and the concept draft survive a Sort<->Learn
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

        {/* Hidden, not unmounted, in the Learn view: the selection, card size,
            Discard filter and paging survive a Sort<->Learn round-trip. */}
        {/* Stays mounted when a delete empties the dataset, so its result
            notice survives; the board shows its own empty state then. */}
        {activeDatasetId && frames !== null && (
          <div hidden={view !== "grid"}>
            <CurationBoard
              key={`board-${activeDatasetId}`}
              datasetId={activeDatasetId}
              frames={frameList}
              isClipMode={isClipMode}
              isPrepRunning={isRunning}
              imageUrlFor={imageUrlFor}
              tokensByFrameId={tokensByFrameId}
              concepts={conceptList}
              onFramesChanged={refetchFrames}
              onFilesDeleted={refreshUsage}
              onConceptsChanged={refetchConceptData}
            />
          </div>
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
