import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useAbout, useDatasetFrames, useJobs } from "../../lib/hooks";
import {
  cancelJob,
  datasetFrameImageUrl,
  datasetFrameImageUrlByDataset,
  exportDataset,
  jobDetail,
  submitJob,
  updateDatasetFrame,
  type DatasetFrame,
  type DatasetPrepParams,
  type Job,
  type JobDetail,
  type JobState,
} from "../../lib/ipc";
import "./dataset.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 900;
const PAGE_SIZE = 60;

const DEFAULT_SAMPLE_FPS = 1.5;
const DEFAULT_BLUR_THRESHOLD = 100;
const DEFAULT_PHASH_MAX_DISTANCE = 6;
const DEFAULT_ESCALATE_EVERY_NTH = 20;
const DEFAULT_CONTEXT_OFFSET = 5;

/** Prefer the dataset-keyed image route: an item outlives its prep job, so
 *  `job_id` can be `null`. The job-keyed URL stays as the fallback for rows
 *  written before datasets became objects. */
function frameImageUrl(coreApiPort: number, frame: DatasetFrame): string {
  if (frame.dataset_id) {
    return datasetFrameImageUrlByDataset(coreApiPort, frame.dataset_id, frame.id);
  }
  return frame.job_id ? datasetFrameImageUrl(coreApiPort, frame.job_id, frame.id) : "";
}

function datasetParamsOf(job: Job): Partial<DatasetPrepParams> {
  const p = job.params;
  return p && typeof p === "object" ? (p as Partial<DatasetPrepParams>) : {};
}

function rootOf(job: Job): string {
  return datasetParamsOf(job).root ?? "untitled dataset";
}

export function DatasetStudio() {
  const about = useAbout();
  const { data: jobs, refetch: refetchJobs } = useJobs({ limit: 50 });
  const datasetJobs = useMemo(
    () => (jobs ?? []).filter((j) => j.job_type === "dataset_prep"),
    [jobs],
  );

  const [root, setRoot] = useState("");
  const [sampleFps, setSampleFps] = useState(DEFAULT_SAMPLE_FPS);
  const [blurThreshold, setBlurThreshold] = useState(DEFAULT_BLUR_THRESHOLD);
  const [phashMaxDistance, setPhashMaxDistance] = useState(DEFAULT_PHASH_MAX_DISTANCE);
  const [escalate, setEscalate] = useState(true);
  const [escalateEveryNth, setEscalateEveryNth] = useState(DEFAULT_ESCALATE_EVERY_NTH);
  const [contextOffset, setContextOffset] = useState(DEFAULT_CONTEXT_OFFSET);
  const [sendError, setSendError] = useState<string | null>(null);

  const [activeJobId, setActiveJobId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  const [destDir, setDestDir] = useState("");
  const [exportState, setExportState] = useState<
    { kind: "idle" } | { kind: "busy" } | { kind: "done"; exported: number } | { kind: "error"; message: string }
  >({ kind: "idle" });

  const activeJob =
    (detail?.job.id === activeJobId ? detail.job : null) ??
    datasetJobs.find((j) => j.id === activeJobId) ??
    null;
  const isRunning = !!activeJob && !DONE.includes(activeJob.state);

  const { data: frames } = useDatasetFrames(activeJobId);
  const frameList = frames ?? [];
  const keptCount = frameList.filter((f) => !f.excluded).length;

  // Default to the most recent dataset job once the list loads, so returning
  // to this tab lands on the last pipeline run instead of a blank state.
  useEffect(() => {
    if (activeJobId === null && datasetJobs.length > 0) {
      setActiveJobId(datasetJobs[0].id);
    }
    // Only auto-select on first load -- once the curator has picked a job (or
    // a new one was just submitted), later job-list refreshes must not steal
    // focus away from it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [datasetJobs.length]);

  useEffect(() => {
    if (!activeJobId || !isRunning) return;
    let alive = true;
    const tick = async () => {
      try {
        const d = await jobDetail(activeJobId);
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
  }, [activeJobId, isRunning]);

  const browseForRoot = async () => {
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === "string") setRoot(picked);
    } catch {
      // Not running inside Tauri (e.g. the browser dev preview) -- no-op.
    }
  };

  const browseForDest = async () => {
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === "string") setDestDir(picked);
    } catch {
      // no-op outside Tauri
    }
  };

  const start = async () => {
    const path = root.trim();
    if (!path) return;
    setSendError(null);

    const params: DatasetPrepParams = {
      root: path,
      sample_fps: sampleFps,
      blur_threshold: blurThreshold,
      phash_max_distance: phashMaxDistance,
      escalate,
      escalate_every_nth: escalateEveryNth,
      context_offset: contextOffset,
    };

    try {
      const job = await submitJob({ job_type: "dataset_prep", params });
      setActiveJobId(job.id);
      setDetail(null);
      setVisibleCount(PAGE_SIZE);
      setExportState({ kind: "idle" });
      refetchJobs();
    } catch (e) {
      setSendError(String(e));
    }
  };

  const cancel = async () => {
    if (!activeJobId) return;
    await cancelJob(activeJobId);
  };

  const editCaption = async (frame: DatasetFrame, caption: string) => {
    if (caption === frame.caption) return;
    await updateDatasetFrame(frame.id, { caption });
  };

  const toggleExcluded = async (frame: DatasetFrame) => {
    await updateDatasetFrame(frame.id, { excluded: !frame.excluded });
  };

  const runExport = async () => {
    if (!activeJobId || !destDir.trim()) return;
    setExportState({ kind: "busy" });
    try {
      const summary = await exportDataset(activeJobId, destDir.trim());
      setExportState({ kind: "done", exported: summary.exported });
    } catch (e) {
      setExportState({ kind: "error", message: String(e) });
    }
  };

  const events = detail?.job.id === activeJob?.id ? (detail?.events ?? []) : [];
  const lastEvent = events.length > 0 ? events[events.length - 1] : null;

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
              <button type="button" className="chip" onClick={browseForRoot}>
                Browse…
              </button>
            </div>
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
              <span>Context offset (frames)</span>
              <input
                type="number"
                min={1}
                max={50}
                step={1}
                value={contextOffset}
                onChange={(e) => setContextOffset(Number(e.target.value))}
                disabled={!escalate}
              />
            </label>
          </div>

          <label className="datasetform__check">
            <input type="checkbox" checked={escalate} onChange={(e) => setEscalate(e.target.checked)} />
            <span>
              Escalate uncertain captions to Qwen2.5-VL with temporal context (frame vs. a later
              frame)
            </span>
          </label>

          {escalate && (
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
          )}

          <button
            type="button"
            className="datasetform__go"
            onClick={start}
            disabled={!root.trim() || (isRunning && !!activeJobId)}
          >
            {isRunning ? "Pipeline running…" : "Run pipeline"}
          </button>
          {sendError && <p className="dataset__err">{sendError}</p>}
        </div>

        {datasetJobs.length > 0 && (
          <div className="dataset__history">
            <span className="dataset__history-label">Recent runs</span>
            <ul>
              {datasetJobs.slice(0, 8).map((j) => (
                <li key={j.id}>
                  <button
                    type="button"
                    className="chip"
                    aria-pressed={j.id === activeJobId}
                    onClick={() => {
                      setActiveJobId(j.id);
                      setDetail(null);
                      setVisibleCount(PAGE_SIZE);
                      setExportState({ kind: "idle" });
                    }}
                  >
                    {rootOf(j)} — {j.state}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>

      <div className="dataset__result">
        {activeJob ? (
          <div className="card">
            <div className="dataset__status">
              <div>
                <strong>{rootOf(activeJob)}</strong>
                <span className="dataset__state" data-state={activeJob.state}>
                  {activeJob.state}
                </span>
              </div>
              {isRunning && (
                <button type="button" className="chip" onClick={cancel}>
                  Cancel
                </button>
              )}
            </div>
            {lastEvent && <p className="dataset__event">{lastEvent.message}</p>}
            {activeJob.state === "failed" && activeJob.error_text && (
              <p className="dataset__err">{activeJob.error_text}</p>
            )}
            <p className="dataset__counts">
              {frameList.length} frame(s) so far — {keptCount} kept for export
            </p>
          </div>
        ) : (
          <div className="card dataset__empty">
            Point the pipeline at a folder tree — each immediate subfolder becomes a tag applied
            to every frame extracted from the video/images inside it.
          </div>
        )}

        {frameList.length > 0 && (
          <>
            <div className="dataset__grid">
              {frameList.slice(0, visibleCount).map((frame) => (
                <FrameCard
                  key={frame.id}
                  frame={frame}
                  imageUrl={about ? frameImageUrl(about.core_api_port, frame) : ""}
                  onCaptionCommit={(caption) => editCaption(frame, caption)}
                  onToggleExcluded={() => toggleExcluded(frame)}
                />
              ))}
            </div>
            {visibleCount < frameList.length && (
              <button
                type="button"
                className="chip dataset__more"
                onClick={() => setVisibleCount((n) => n + PAGE_SIZE)}
              >
                Show more ({frameList.length - visibleCount} remaining)
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
                <button type="button" className="chip" onClick={browseForDest}>
                  Browse…
                </button>
              </div>
              <button
                type="button"
                className="datasetform__go"
                onClick={runExport}
                disabled={!destDir.trim() || exportState.kind === "busy" || keptCount === 0}
              >
                {exportState.kind === "busy" ? "Exporting…" : `Export ${keptCount} frame(s)`}
              </button>
              {exportState.kind === "done" && (
                <p className="dataset__done">Exported {exportState.exported} frame(s) to {destDir}.</p>
              )}
              {exportState.kind === "error" && <p className="dataset__err">{exportState.message}</p>}
            </div>
          </>
        )}
      </div>
    </section>
  );
}

function FrameCard({
  frame,
  imageUrl,
  onCaptionCommit,
  onToggleExcluded,
}: {
  frame: DatasetFrame;
  imageUrl: string;
  onCaptionCommit: (caption: string) => void;
  onToggleExcluded: () => void;
}) {
  const [caption, setCaption] = useState(frame.caption);
  useEffect(() => setCaption(frame.caption), [frame.caption]);

  return (
    <div className="framecard" data-excluded={frame.excluded}>
      <div className="framecard__thumb">
        {imageUrl && <img src={imageUrl} alt="" loading="lazy" />}
        <span className="framecard__tag">{frame.tag}</span>
        {frame.caption_engine === "qwen2.5-vl" && (
          <span className="framecard__badge" title="Re-captioned with temporal context">
            Qwen
          </span>
        )}
      </div>
      <textarea
        value={caption}
        onChange={(e) => setCaption(e.target.value)}
        onBlur={() => onCaptionCommit(caption)}
        rows={2}
      />
      <label className="framecard__exclude">
        <input type="checkbox" checked={frame.excluded} onChange={onToggleExcluded} />
        <span>Exclude</span>
      </label>
    </div>
  );
}
