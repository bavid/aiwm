import { useEffect, useId, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  useAbout,
  useCaptioners,
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
  type DatasetMode,
  type DatasetPrepParams,
  type JobDetail,
  type JobState,
} from "../../lib/ipc";
import { ConceptsPanel } from "./ConceptsPanel";
import { FrameCard } from "./FrameCard";
import { RejectionChips } from "./RejectionChips";
import { tokenWarning } from "./tokens";
import "./dataset.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 900;
const PAGE_SIZE = 60;

const DEFAULT_SAMPLE_FPS = 1.5;
const DEFAULT_BLUR_THRESHOLD = 100;
const DEFAULT_PHASH_MAX_DISTANCE = 6;
const DEFAULT_ESCALATE_EVERY_NTH = 20;
const DEFAULT_CONTEXT_OFFSET = 5;
const DEFAULT_MAX_FRAMES_PER_CLIP = 40;
const DEFAULT_MIN_CLIP_SECS = 2;

/** Prefer the dataset-keyed image route: an item outlives its prep job, so
 *  `job_id` can be `null`. The job-keyed URL stays as the fallback for rows
 *  written before datasets became objects. */
function frameImageUrl(coreApiPort: number, frame: DatasetFrame): string {
  if (frame.dataset_id) {
    return datasetFrameImageUrlByDataset(coreApiPort, frame.dataset_id, frame.id);
  }
  return frame.job_id ? datasetFrameImageUrl(coreApiPort, frame.job_id, frame.id) : "";
}

type ExportState =
  | { kind: "idle" }
  | { kind: "busy" }
  | { kind: "done"; exported: number; destDir: string }
  | { kind: "error"; message: string };

type AssignState = { kind: "idle" } | { kind: "done"; text: string } | { kind: "error"; text: string };

export function DatasetStudio() {
  const about = useAbout();
  const { data: jobs, refetch: refetchJobs } = useJobs({ limit: 50 });
  const { data: captioners } = useCaptioners();
  const { data: datasets, refetch: refetchDatasets } = useDatasets();

  const datasetList = useMemo(() => datasets ?? [], [datasets]);
  const installed = useMemo(
    () => (captioners ?? []).filter((c) => c.installed),
    [captioners],
  );

  // --- form -------------------------------------------------------------
  const [root, setRoot] = useState("");
  const [mode, setMode] = useState<DatasetMode>("frames");
  const [sampleFps, setSampleFps] = useState(DEFAULT_SAMPLE_FPS);
  const [blurThreshold, setBlurThreshold] = useState(DEFAULT_BLUR_THRESHOLD);
  const [phashMaxDistance, setPhashMaxDistance] = useState(DEFAULT_PHASH_MAX_DISTANCE);
  const [maxFramesPerClip, setMaxFramesPerClip] = useState(DEFAULT_MAX_FRAMES_PER_CLIP);
  const [minClipSecs, setMinClipSecs] = useState(DEFAULT_MIN_CLIP_SECS);
  const [escalate, setEscalate] = useState(true);
  const [escalateEveryNth, setEscalateEveryNth] = useState(DEFAULT_ESCALATE_EVERY_NTH);
  const [contextOffset, setContextOffset] = useState(DEFAULT_CONTEXT_OFFSET);
  const [sendError, setSendError] = useState<string | null>(null);

  // Captioning is on by default, but only ever resolves to a captioner that is
  // actually installed -- with an empty library it stays off (and the checkbox
  // is disabled). Derived during render rather than synced by an effect, so
  // unchecking cannot be undone by the next captioner poll.
  const [captionWanted, setCaptionWanted] = useState(true);
  const [pickedCaptioner, setPickedCaptioner] = useState<string | null>(null);
  const captionerId = captionWanted
    ? (installed.find((c) => c.id === pickedCaptioner)?.id ?? installed[0]?.id ?? null)
    : null;
  const captionOn = captionerId !== null;
  const chosenCaptioner = installed.find((c) => c.id === captionerId) ?? null;

  // --- selection --------------------------------------------------------
  const [activeDatasetId, setActiveDatasetId] = useState<string | null>(null);
  /** The job just submitted, until its dataset row shows up in the list. */
  const [pendingJobId, setPendingJobId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  const [filter, setFilter] = useState("");
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [assignConceptId, setAssignConceptId] = useState("");
  const [assignState, setAssignState] = useState<AssignState>({ kind: "idle" });
  const [triggerDraft, setTriggerDraft] = useState<string | null>(null);

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
  const keptCount = frameList.filter((f) => f.rejection_reason === "" && !f.excluded).length;
  const shownFrames = frameList.filter((f) => f.rejection_reason === filter);

  // The live status of the run that produced (or is producing) this dataset.
  const liveJobId = activeDataset?.prep_job_id ?? pendingJobId;
  const activeJob =
    (detail?.job.id === liveJobId ? detail.job : null) ??
    (jobs ?? []).find((j) => j.id === liveJobId) ??
    null;
  const isRunning = !!activeJob && !DONE.includes(activeJob.state);

  const tokenByConceptId = useMemo(() => {
    const map: Record<string, string> = {};
    for (const c of conceptList) map[c.id] = c.token;
    return map;
  }, [conceptList]);

  const modeId = useId();
  const captionerSelectId = useId();
  const triggerId = useId();
  const orderId = useId();
  const assignSelectId = useId();

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

  const browseInto = async (set: (path: string) => void) => {
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === "string") set(picked);
    } catch {
      // Not running inside Tauri (e.g. the browser dev preview) -- no-op.
    }
  };

  const selectDataset = (id: string) => {
    setActiveDatasetId(id);
    setDetail(null);
    setVisibleCount(PAGE_SIZE);
    setFilter("");
    setSelectedIds([]);
    setAssignState({ kind: "idle" });
    setTriggerDraft(null);
    setExportState({ kind: "idle" });
  };

  const start = async () => {
    const path = root.trim();
    if (!path) return;
    setSendError(null);

    const params: DatasetPrepParams = {
      root: path,
      mode,
      captioner: captionerId,
      max_frames_per_clip: maxFramesPerClip,
      min_clip_secs: minClipSecs,
      sample_fps: sampleFps,
      blur_threshold: blurThreshold,
      phash_max_distance: phashMaxDistance,
      // Temporal escalation rides on top of a captioner that supports it.
      escalate: escalate && !!chosenCaptioner?.supports_escalation,
      escalate_every_nth: escalateEveryNth,
      context_offset: contextOffset,
    };

    try {
      const job = await submitJob({ job_type: "dataset_prep", params });
      setPendingJobId(job.id);
      setActiveDatasetId(null);
      setDetail(null);
      setVisibleCount(PAGE_SIZE);
      setFilter("");
      setSelectedIds([]);
      setExportState({ kind: "idle" });
      refetchJobs();
      refetchDatasets();
    } catch (e) {
      setSendError(String(e));
    }
  };

  const cancel = async () => {
    if (!liveJobId) return;
    await cancelJob(liveJobId);
  };

  const commitTrigger = async () => {
    if (!activeDataset || triggerDraft === null) return;
    const next = triggerDraft.trim();
    setTriggerDraft(null);
    if (next === activeDataset.trigger_word) return;
    try {
      await updateDataset(activeDataset.id, { trigger_word: next });
      refetchDatasets();
    } catch (e) {
      setSendError(String(e));
    }
  };

  const editCaption = async (frame: DatasetFrame, caption: string) => {
    if (caption === frame.caption) return;
    await updateDatasetFrame(frame.id, { caption });
    refetchFrames();
  };

  const toggleExcluded = async (frame: DatasetFrame) => {
    await updateDatasetFrame(frame.id, { excluded: !frame.excluded });
    refetchFrames();
  };

  const restore = async (frame: DatasetFrame) => {
    await updateDatasetFrame(frame.id, { restore: true });
    refetchFrames();
  };

  const toggleSelected = (frameId: string) => {
    setSelectedIds((cur) =>
      cur.includes(frameId) ? cur.filter((id) => id !== frameId) : [...cur, frameId],
    );
  };

  const runAssign = async (attach: boolean) => {
    if (!assignConceptId || selectedIds.length === 0) return;
    try {
      if (attach) {
        const summary = await assignConcept(assignConceptId, selectedIds);
        setAssignState({
          kind: "done",
          text: `${summary.attached} of ${summary.requested} assigned`,
        });
      } else {
        await unassignConcept(assignConceptId, selectedIds);
        setAssignState({ kind: "done", text: `${selectedIds.length} removed` });
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

        <div className="datasetform">
          <label className="datasetform__field">
            <span>Root folder</span>
            <div className="datasetform__row">
              <input
                type="text"
                value={root}
                onChange={(e) => setRoot(e.target.value)}
                placeholder="E:\Data\MyArtStyle"
              />
              <button type="button" className="chip" onClick={() => browseInto(setRoot)}>
                Browse…
              </button>
            </div>
          </label>

          <label className="datasetform__field" htmlFor={modeId}>
            <span>Mode</span>
            <select
              id={modeId}
              value={mode}
              onChange={(e) => setMode(e.target.value as DatasetMode)}
            >
              <option value="frames">Frames (stills from video + images)</option>
              <option value="clips">Clips (whole videos, for video models)</option>
            </select>
          </label>

          <div className="datasetform__grid">
            <label className="datasetform__field">
              <span>Sample rate (fps)</span>
              <input
                type="number"
                min={0.1}
                max={10}
                step={0.1}
                value={sampleFps}
                onChange={(e) => setSampleFps(Number(e.target.value))}
              />
            </label>
            <label className="datasetform__field">
              <span>Blur threshold</span>
              <input
                type="number"
                min={0}
                max={10000}
                step={5}
                value={blurThreshold}
                onChange={(e) => setBlurThreshold(Number(e.target.value))}
              />
            </label>
            <label className="datasetform__field">
              <span>Duplicate distance</span>
              <input
                type="number"
                min={0}
                max={64}
                step={1}
                value={phashMaxDistance}
                onChange={(e) => setPhashMaxDistance(Number(e.target.value))}
              />
            </label>
            <label className="datasetform__field">
              <span>Max frames per clip (0 = all)</span>
              <input
                type="number"
                min={0}
                max={500}
                step={1}
                value={maxFramesPerClip}
                onChange={(e) => setMaxFramesPerClip(Number(e.target.value))}
              />
            </label>
            {mode === "clips" && (
              <label className="datasetform__field">
                <span>Min clip length (s)</span>
                <input
                  type="number"
                  min={0}
                  max={600}
                  step={0.5}
                  value={minClipSecs}
                  onChange={(e) => setMinClipSecs(Number(e.target.value))}
                />
              </label>
            )}
          </div>

          <fieldset className="datasetform__captioning">
            <legend>Auto-caption</legend>
            <label className="datasetform__check">
              <input
                type="checkbox"
                checked={captionOn}
                disabled={installed.length === 0}
                onChange={(e) => setCaptionWanted(e.target.checked)}
              />
              <span>
                Describe every kept frame automatically.{" "}
                <em>
                  Recommended for style LoRAs: what is described stays controllable, what is not
                  becomes part of the style.
                </em>
              </span>
            </label>

            {installed.length === 0 && (
              <p className="datasetform__hint">
                No captioner installed — import Florence-2 or the WD tagger on the Models tab.
                Without one, everything recurring in your frames flows into the trigger word.
              </p>
            )}

            {captionOn && (
              <label
                className="datasetform__field datasetform__field--inline"
                htmlFor={captionerSelectId}
              >
                <span>Describe with</span>
                <select
                  id={captionerSelectId}
                  value={captionerId ?? ""}
                  onChange={(e) => setPickedCaptioner(e.target.value)}
                >
                  {installed.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name} {c.style === "tags" ? "· tags" : "· prose"}
                    </option>
                  ))}
                </select>
              </label>
            )}

            {captionOn && chosenCaptioner?.supports_escalation && (
              <>
                <label className="datasetform__check">
                  <input
                    type="checkbox"
                    checked={escalate}
                    onChange={(e) => setEscalate(e.target.checked)}
                  />
                  <span>
                    Escalate uncertain captions to Qwen2.5-VL with temporal context (frame vs. a
                    later frame)
                  </span>
                </label>
                {escalate && (
                  <>
                    <label className="datasetform__field datasetform__field--inline">
                      <span>Escalate every Nth frame too</span>
                      <input
                        type="number"
                        min={0}
                        max={500}
                        step={1}
                        value={escalateEveryNth}
                        onChange={(e) => setEscalateEveryNth(Number(e.target.value))}
                      />
                    </label>
                    <label className="datasetform__field datasetform__field--inline">
                      <span>Context offset (frames)</span>
                      <input
                        type="number"
                        min={1}
                        max={50}
                        step={1}
                        value={contextOffset}
                        onChange={(e) => setContextOffset(Number(e.target.value))}
                      />
                    </label>
                  </>
                )}
              </>
            )}
          </fieldset>

          <button
            type="button"
            className="datasetform__go"
            onClick={start}
            disabled={!root.trim() || isRunning}
          >
            {isRunning ? "Pipeline running…" : "Run pipeline"}
          </button>
          {sendError && <p className="dataset__err">{sendError}</p>}
        </div>

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
                  onChange={(e) => setTriggerDraft(e.target.value)}
                  onBlur={commitTrigger}
                  placeholder="ghibli_xy"
                />
              </label>
            )}
            {activeDataset && triggerWarn && <p className="dataset__warn">{triggerWarn}</p>}
          </div>
        ) : (
          <div className="card dataset__empty">
            Point the pipeline at a folder tree — each immediate subfolder becomes a tag applied
            to every frame extracted from the video/images inside it.
          </div>
        )}

        {activeDatasetId && (
          <ConceptsPanel
            datasetId={activeDatasetId}
            concepts={conceptList}
            onChanged={() => {
              refetchConcepts();
              refetchConceptMap();
            }}
          />
        )}

        {frameList.length > 0 && (
          <>
            <RejectionChips
              frames={frameList}
              active={filter}
              onSelect={(reason) => {
                setFilter(reason);
                setVisibleCount(PAGE_SIZE);
              }}
            />

            {selectedIds.length > 0 && (
              <div className="card dataset__toolbar">
                <span className="dataset__toolbar-count">{selectedIds.length} selected</span>
                <label className="datasetform__field--inline" htmlFor={assignSelectId}>
                  <span className="dataset__toolbar-label">Concept</span>
                  <select
                    id={assignSelectId}
                    value={assignConceptId}
                    onChange={(e) => setAssignConceptId(e.target.value)}
                  >
                    <option value="">Choose a concept…</option>
                    {conceptList.map((c) => (
                      <option key={c.id} value={c.id}>
                        {c.name} ({c.token})
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  type="button"
                  className="chip"
                  disabled={!assignConceptId}
                  onClick={() => runAssign(true)}
                >
                  Assign to concept
                </button>
                <button
                  type="button"
                  className="chip"
                  disabled={!assignConceptId}
                  onClick={() => runAssign(false)}
                >
                  Remove from concept
                </button>
                <button type="button" className="chip" onClick={() => setSelectedIds([])}>
                  Clear selection
                </button>
                {assignState.kind === "done" && (
                  <span className="dataset__toolbar-status">{assignState.text}</span>
                )}
                {assignState.kind === "error" && (
                  <span className="dataset__err">{assignState.text}</span>
                )}
              </div>
            )}

            <div className="dataset__grid">
              {shownFrames.slice(0, visibleCount).map((frame) => (
                <FrameCard
                  key={frame.id}
                  frame={frame}
                  imageUrl={about ? frameImageUrl(about.core_api_port, frame) : ""}
                  isSelected={selectedIds.includes(frame.id)}
                  conceptTokens={(conceptMap?.[frame.id] ?? [])
                    .map((id) => tokenByConceptId[id])
                    .filter((t): t is string => !!t)}
                  onToggleSelected={() => toggleSelected(frame.id)}
                  onCaptionCommit={(caption) => editCaption(frame, caption)}
                  onToggleExcluded={() => toggleExcluded(frame)}
                  onRestore={() => restore(frame)}
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

            <div className="card dataset__export">
              <h3>Export</h3>
              <div className="datasetform__row">
                <input
                  type="text"
                  value={destDir}
                  onChange={(e) => setDestDir(e.target.value)}
                  placeholder="Where to write NNNN.png + NNNN.txt pairs"
                />
                <button type="button" className="chip" onClick={() => browseInto(setDestDir)}>
                  Browse…
                </button>
              </div>
              <label className="datasetform__field datasetform__field--inline" htmlFor={orderId}>
                <span>Caption order</span>
                <select
                  id={orderId}
                  value={captionOrder}
                  onChange={(e) => setCaptionOrder(e.target.value as CaptionOrder)}
                >
                  <option value="prose_first">Prose first (FLUX.2)</option>
                  <option value="tags_first">Tags first (Anime/SDXL)</option>
                </select>
              </label>
              <button
                type="button"
                className="datasetform__go"
                onClick={runExport}
                disabled={!destDir.trim() || exportState.kind === "busy" || keptCount === 0}
              >
                {exportState.kind === "busy" ? "Exporting…" : `Export ${keptCount} item(s)`}
              </button>
              {exportState.kind === "done" && (
                <p className="dataset__done">
                  Exported {exportState.exported} item(s) to {exportState.destDir}.
                </p>
              )}
              {exportState.kind === "error" && <p className="dataset__err">{exportState.message}</p>}
            </div>
          </>
        )}
      </div>
    </section>
  );
}
