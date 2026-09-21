import { useEffect, useId, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { HelpHint } from "../../components/HelpHint";
import { Lightbox } from "../../components/Lightbox";
import { LoraPicker } from "../../components/LoraPicker";
import { Meter } from "../../components/Meter";
import { NumField } from "../../components/NumField";
import { PromptAssistant } from "../../components/PromptAssistant";
import { PromptPresetPicker } from "../../components/PromptPresetPicker";
import { QueueList } from "../../components/QueueList";
import {
  SessionSidebar,
  type SessionSidebarLabels,
} from "../../components/SessionSidebar";
import { VramEstimateHint } from "../../components/VramEstimateHint";
import {
  useAbout,
  useJobProgress,
  useJobs,
  useModels,
  useRuntimes,
  useSessions,
  useTelemetry,
} from "../../lib/hooks";
import {
  cancelJob,
  deleteJob,
  downloadJobOutput,
  jobDetail,
  jobOutputUrl,
  submitJob,
  type Job,
  type JobDetail,
  type JobEvent,
  type JobProgress,
  type JobState,
  type LoraParam,
  type UpscaleParams,
  type VideoParams,
} from "../../lib/ipc";
import "./video.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 900;
const GALLERY_PAGE_SIZE = 24;

const MIN_DIM = 128;
const MAX_DIM = 1280;
const DIM_STEP = 16;
const MIN_FRAMES = 5;
const MAX_FRAMES = 121;
const MIN_FPS = 8;
const MAX_FPS = 30;
const MAX_STEPS = 60;
// Wan's comfort zone — bigger/longer is much slower and near the 16 GB edge.
const EASY_PIXELS = 832 * 480;
const EASY_FRAMES = 81;

/** The Video tab's wording on the shared session sidebar (the same list the
 *  Chat tab docks). */
const SESSION_LABELS: SessionSidebarLabels = {
  create: "+ New session",
  createdName: "New session",
  empty: "No sessions yet.",
  confirmDelete: (name) =>
    `Delete “${name}”?

Its clips stay in your history, just ungrouped.`,
  unsorted: "Ungrouped clips",
};

const PRESETS = [
  { label: "Landscape", w: 832, h: 480 },
  { label: "Portrait", w: 480, h: 832 },
  { label: "Square", w: 512, h: 512 },
  { label: "HD 720p", w: 1280, h: 720 },
];

const clampDim = (n: number) =>
  Math.min(MAX_DIM, Math.max(MIN_DIM, Math.round(n / DIM_STEP) * DIM_STEP));
/** Wan wants `(frames - 1) % 4 == 0`. Snap to the nearest `4k + 1`. */
const snapFrames = (n: number) =>
  Math.min(MAX_FRAMES, Math.max(MIN_FRAMES, 4 * Math.round((n - 1) / 4) + 1));
const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

function asVideoParams(p: unknown): Partial<VideoParams> {
  return p && typeof p === "object" ? (p as Partial<VideoParams>) : {};
}

function promptOf(job: Job): string {
  const p = job.params;
  return (p && typeof p === "object" && typeof (p as { prompt?: unknown }).prompt === "string"
    ? (p as { prompt: string }).prompt
    : "") || "untitled";
}

export function VideoStudio() {
  const about = useAbout();
  const { data: models } = useModels();
  const { data: runtimes } = useRuntimes();
  const { data: jobs } = useJobs();
  const { telemetry } = useTelemetry();
  const { data: sessions, refetch: refetchSessions } = useSessions("video");

  const videoModels = (models ?? []).filter((m) => m.roles.includes("base_video"));
  const hasEncoder = (models ?? []).some(
    (m) => m.roles.includes("text_encoder") && /umt5/i.test(m.name + m.file_path),
  );
  const hasVae = (models ?? []).some(
    (m) => m.roles.includes("vae") && /wan/i.test(m.name + m.file_path),
  );
  const modelNames = useMemo(
    () => new Map((models ?? []).map((m) => [m.id, m.name])),
    [models],
  );
  const comfy = (runtimes ?? []).find((r) => r.id === "comfyui");
  const comfyReady = !comfy || !(comfy.detail ?? "").includes("not installed");

  const imageJobs = (jobs ?? []).filter(
    (j) => j.job_type === "image" && j.state === "completed" && j.output_path,
  );

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [prompt, setPrompt] = useState("");
  const [negative, setNegative] = useState("");
  const [width, setWidth] = useState(832);
  const [height, setHeight] = useState(480);
  const [frames, setFrames] = useState(EASY_FRAMES);
  const [fps, setFps] = useState(24);
  const [steps, setSteps] = useState(30);
  const [cfg, setCfg] = useState(5);
  const [seed, setSeed] = useState("");
  const [modelId, setModelId] = useState("auto");
  const [loras, setLoras] = useState<LoraParam[]>([]);
  const [startJob, setStartJob] = useState("none");
  const [startPath, setStartPath] = useState("");
  const [sendError, setSendError] = useState<string | null>(null);
  const ids = useId();
  const startJobId = `${ids}-start`;
  const seedId = `${ids}-seed`;
  const modelPickId = `${ids}-model`;
  const estimateId = `${ids}-estimate`;
  const sidebarId = `${ids}-sessions`;

  const [pendingId, setPendingId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Real per-step progress straight from ComfyUI's own `/ws`, layered on top
  // of the `jobDetail` poll below (state/output/events) -- see
  // `core::progress` / `GET /ws/jobs/{id}`.
  const liveProgress = useJobProgress(about?.core_api_port ?? null, pendingId);

  useEffect(() => {
    if (!pendingId) return;
    let alive = true;
    const tick = async () => {
      let d: JobDetail | null;
      try {
        d = await jobDetail(pendingId);
      } catch {
        return;
      }
      if (!alive || !d) return;
      setDetail(d);
      if (DONE.includes(d.job.state)) setPendingId(null);
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [pendingId]);

  const gallery = (jobs ?? []).filter(
    (j) =>
      j.job_type === "video" &&
      j.state === "completed" &&
      j.output_path &&
      j.session_id === sessionId,
  );

  const [galleryPage, setGalleryPage] = useState(0);
  const [lightboxIndex, setLightboxIndex] = useState<number | null>(null);
  const pageCount = Math.max(1, Math.ceil(gallery.length / GALLERY_PAGE_SIZE));
  const clampedPage = Math.min(galleryPage, pageCount - 1);
  const pagedGallery = gallery.slice(
    clampedPage * GALLERY_PAGE_SIZE,
    clampedPage * GALLERY_PAGE_SIZE + GALLERY_PAGE_SIZE,
  );
  useEffect(() => {
    setGalleryPage(0);
    setLightboxIndex(null);
  }, [sessionId]);

  const selected =
    (detail?.job.id === selectedId ? detail.job : null) ??
    (jobs ?? []).find((j) => j.id === selectedId) ??
    null;
  const selectedEvents = detail?.job.id === selected?.id ? (detail?.events ?? []) : [];

  const w = clampDim(width);
  const h = clampDim(height);
  const f = snapFrames(frames);
  const rate = clamp(Math.round(fps), MIN_FPS, MAX_FPS);
  const clipSecs = rate > 0 ? f / rate : 0;
  const heavy = w * h > EASY_PIXELS || f > EASY_FRAMES;
  const estUnits = (f / EASY_FRAMES) * (steps / 25) * ((w * h) / EASY_PIXELS);
  const estLo = Math.max(1, Math.round(estUnits * 2));
  const estHi = Math.max(estLo + 1, Math.round(estUnits * 7));

  const startImage = startPath.trim() || (startJob === "none" ? "" : startJob);

  const browseForImage = async () => {
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }],
      });
      if (typeof picked === "string") setStartPath(picked);
    } catch {
      // Not running inside Tauri (e.g. the browser dev preview) — no-op.
    }
  };

  // A `blocked` job (not enough VRAM right now) isn't actively running -- it's
  // just waiting for room, and may sit there indefinitely if none frees up.
  // Don't lock the form forever: let the user start a fresh (e.g. smaller)
  // request instead of being stuck until they cancel or switch tabs.
  const stuck = detail?.job.state === "blocked";

  const generate = async () => {
    const text = prompt.trim();
    if (!text || (pendingId && !stuck)) return;
    setSendError(null);

    const params: Record<string, unknown> = {
      prompt: text,
      negative: negative.trim(),
      width: w,
      height: h,
      length: f,
      fps: rate,
      steps: clamp(Math.round(steps), 1, MAX_STEPS),
      cfg,
    };
    const s = Number(seed);
    if (seed.trim() !== "" && Number.isFinite(s) && s >= 0) params.seed = Math.floor(s);
    if (startImage) params.init_image = startImage;
    if (loras.length > 0) params.loras = loras;

    try {
      const job = await submitJob({
        job_type: "video",
        model_id: modelId === "auto" ? undefined : modelId,
        session_id: sessionId ?? undefined,
        params,
      });
      setPendingId(job.id);
      setSelectedId(job.id);
      setDetail({ job, events: [] });
    } catch (err) {
      setSendError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteJob(id);
      if (selectedId === id) {
        setSelectedId(null);
        setDetail(null);
      }
    } catch (err) {
      setSendError(err instanceof Error ? err.message : String(err));
    }
  };

  /** A real scheduled ComfyUI job (VRAM, queueing), not a synchronous
   *  in-place pass — submit it and switch the Result panel over to watch it,
   *  exactly like `generate()` does for a fresh render. The Rust side
   *  defaults everything but `source` (2x scale, ULTRA quality), so this is
   *  a genuine one click. */
  const handleUpscale = async (id: string) => {
    setSendError(null);
    const params: UpscaleParams = { source: id };
    try {
      const job = await submitJob({
        job_type: "upscale",
        session_id: sessionId ?? undefined,
        params,
      });
      setPendingId(job.id);
      setSelectedId(job.id);
      setDetail({ job, events: [] });
    } catch (err) {
      setSendError(err instanceof Error ? err.message : String(err));
    }
  };

  /** "Erneut versuchen" — resubmit a failed job's own `params` as a brand new
   *  job (never mutates the failed one). Mirrors `generate()`/`handleUpscale`. */
  const handleRetry = async (job: Job) => {
    setSendError(null);
    try {
      const fresh = await submitJob({
        job_type: job.job_type,
        model_id: job.model_id ?? undefined,
        session_id: job.session_id ?? sessionId ?? undefined,
        params: job.params,
      });
      setPendingId(fresh.id);
      setSelectedId(fresh.id);
      setDetail({ job: fresh, events: [] });
    } catch (err) {
      setSendError(err instanceof Error ? err.message : String(err));
    }
  };

  const canGenerate = !!prompt.trim() && (!pendingId || stuck) && comfyReady;

  return (
    <div className="session-page">
      <div className="session-page__sidebar" id={sidebarId}>
        <SessionSidebar
          capability="video"
          labels={SESSION_LABELS}
          activeId={sessionId}
          onChange={setSessionId}
          sessions={sessions}
          onRefetch={refetchSessions}
          nameOnCreate
        />
        <p className="session-page__sidebar-hint">
          <HelpHint area="video" setting="sessions" describes={sidebarId} />
        </p>
      </div>
      <div className="image">
        <section className="card image__form">
          <header className="card__head">
            <h2>Generate a video</h2>
            {modelId === "auto" && videoModels.length > 0 && (
              <span className="card__sub">Auto · {videoModels.length} model(s)</span>
            )}
          </header>

          <fieldset className="startframe">
            <legend>Start from an image (optional)</legend>
            <div className="imgform__field">
              <span>
                <label htmlFor={startJobId}>A finished image</label>
                <HelpHint area="video" setting="start-frame" describes={startJobId} />
              </span>
              <select id={startJobId} value={startJob} onChange={(e) => setStartJob(e.target.value)}>
                <option value="none">None — text to video</option>
                {imageJobs.map((j) => (
                  <option key={j.id} value={j.id}>
                    {promptOf(j).slice(0, 48)}
                  </option>
                ))}
              </select>
            </div>
            <label className="imgform__field">
              <span>…or an image file</span>
              <div className="pathpick">
                <input
                  type="text"
                  value={startPath}
                  onChange={(e) => setStartPath(e.target.value)}
                  placeholder="E:\\shots\\frame_01.png"
                  spellCheck={false}
                />
                <button type="button" className="chip" onClick={browseForImage}>
                  Browse…
                </button>
              </div>
            </label>
            {startImage && about && startPath.trim() === "" && (
              <img
                className="startframe__thumb"
                src={jobOutputUrl(about.core_api_port, startImage)}
                alt="start frame"
                loading="lazy"
              />
            )}
          </fieldset>

          <PromptAssistant
            kind="video"
            sessionId={sessionId}
            onApplyPrompt={(text) => setPrompt((p) => (p.trim() ? `${p.trim()}, ${text}` : text))}
            onApplyNegative={(text) => setNegative((n) => (n.trim() ? `${n.trim()}, ${text}` : text))}
          />


          {!comfyReady && (
            <p className="muted">ComfyUI is not set up yet — open Diagnostics to install it.</p>
          )}
          {comfyReady && models && videoModels.length === 0 && (
            <p className="muted">
              No video model yet — import <strong>Wan 2.2 TI2V-5B</strong> as a “Video model” on the
              Models tab (it also needs the umt5 text encoder and the Wan VAE).
            </p>
          )}
          {comfyReady && videoModels.length > 0 && (!hasEncoder || !hasVae) && (
            <p className="muted">
              Wan also needs {!hasEncoder && <>the <code>umt5</code> text encoder</>}
              {!hasEncoder && !hasVae && " and "}
              {!hasVae && <>its VAE (<code>wan2.2_vae</code>)</>} — import{" "}
              {!hasEncoder && !hasVae ? "them" : "it"} on the Models tab.
            </p>
          )}

          <form
            className="imgform"
            onSubmit={(e) => {
              e.preventDefault();
              generate();
            }}
          >
            <label className="imgform__field">
              <span>Prompt</span>
              <textarea
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                rows={3}
                spellCheck
                placeholder="a paper boat drifting down a rain-soaked street, slow dolly shot"
              />
            </label>
            <PromptPresetPicker
              kind="positive"
              onApply={(text) => setPrompt((p) => (p.trim() ? `${p.trim()}, ${text}` : text))}
            />

            <label className="imgform__field">
              <span>Negative prompt</span>
              <textarea
                value={negative}
                onChange={(e) => setNegative(e.target.value)}
                rows={2}
                spellCheck
                placeholder="blurry, jitter, warped faces, low quality"
              />
            </label>
            <PromptPresetPicker
              kind="negative"
              onApply={(text) => setNegative((n) => (n.trim() ? `${n.trim()}, ${text}` : text))}
            />

            <div className="imgform__presets">
              {PRESETS.map((p) => (
                <button
                  key={p.label}
                  type="button"
                  className="chip"
                  aria-pressed={width === p.w && height === p.h}
                  onClick={() => {
                    setWidth(p.w);
                    setHeight(p.h);
                  }}
                >
                  {p.label}
                </button>
              ))}
            </div>

            <div className="imgform__grid">
              <NumField
                label="Width"
                value={width}
                step={DIM_STEP}
                min={MIN_DIM}
                max={MAX_DIM}
                onChange={setWidth}
                hint={(id) => <HelpHint area="video" setting="size" describes={id} />}
              />
              <NumField label="Height" value={height} step={DIM_STEP} min={MIN_DIM} max={MAX_DIM} onChange={setHeight} />
              <NumField
                label="Frames"
                value={frames}
                step={4}
                min={MIN_FRAMES}
                max={MAX_FRAMES}
                onChange={setFrames}
                hint={(id) => <HelpHint area="video" setting="frames" describes={id} />}
              />
              <NumField
                label="FPS"
                value={fps}
                step={1}
                min={MIN_FPS}
                max={MAX_FPS}
                onChange={setFps}
                hint={(id) => <HelpHint area="video" setting="fps" describes={id} />}
              />
              <NumField
                label="Steps"
                value={steps}
                step={1}
                min={1}
                max={MAX_STEPS}
                onChange={setSteps}
                hint={(id) => <HelpHint area="video" setting="steps" describes={id} />}
              />
              <NumField
                label="CFG"
                value={cfg}
                step={0.5}
                min={1}
                max={15}
                onChange={setCfg}
                hint={(id) => <HelpHint area="video" setting="cfg" describes={id} />}
              />
            </div>

            <div className="imgform__grid">
              <div className="imgform__field">
                <span>
                  <label htmlFor={seedId}>Seed</label>
                  <HelpHint area="video" setting="seed" describes={seedId} />
                </span>
                <input
                  id={seedId}
                  type="text"
                  inputMode="numeric"
                  value={seed}
                  onChange={(e) => setSeed(e.target.value.replace(/[^\d]/g, ""))}
                  placeholder="random"
                  spellCheck={false}
                />
              </div>
              <div className="imgform__field imgform__field--wide">
                <span>
                  <label htmlFor={modelPickId}>Model</label>
                  <HelpHint area="video" setting="model" describes={modelPickId} />
                </span>
                <select id={modelPickId} value={modelId} onChange={(e) => setModelId(e.target.value)}>
                  <option value="auto">Auto (most-recently-used)</option>
                  {videoModels.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name}
                    </option>
                  ))}
                </select>
              </div>
            </div>
            <VramEstimateHint
              vramEstimateMb={videoModels.find((m) => m.id === modelId)?.vram_estimate_mb}
              gpu={telemetry?.gpu}
            />
            <LoraPicker
              models={models ?? []}
              family={videoModels.find((m) => m.id === modelId)?.family}
              base={videoModels.find((m) => m.id === modelId)}
              selected={loras}
              onChange={setLoras}
            />

            <p id={estimateId} className={heavy ? "video__note video__note--warn" : "video__note"}>
              ~{clipSecs.toFixed(1)}s clip · very rough guess {estLo}–{estHi} min on a 16 GB card
              (not yet calibrated). Video is slow — minutes, not seconds. The window stays usable
              while it renders.
              {heavy && " Above 480p / 81 frames is much slower and can run out of VRAM."}{" "}
              <HelpHint area="video" setting="time-estimate" />
            </p>

            <button type="submit" className="imgform__go" disabled={!canGenerate}>
              {pendingId && !stuck ? "Rendering…" : "Generate"}
            </button>
          </form>
          {sendError && <p className="image__err">{sendError}</p>}
        </section>

        <div className="image__result">
          <QueueList
            jobType="video"
            jobs={jobs ?? []}
            modelNames={modelNames}
            selectedId={selectedId}
            onSelect={(id) => {
              setSelectedId(id);
              setDetail(null);
            }}
            onCancel={cancelJob}
            promptOf={promptOf}
          />
          <section className="card">
            <header className="card__head">
              <h2>Result</h2>
              {selected && <span className="card__sub">{selected.state}</span>}
            </header>
            <Result
              job={selected}
              events={selectedEvents}
              port={about?.core_api_port ?? null}
              modelNames={modelNames}
              progress={liveProgress}
              onCancel={selected ? () => cancelJob(selected.id) : undefined}
              onDelete={selected ? () => handleDelete(selected.id) : undefined}
              onRetry={
                selected && selected.state === "failed" ? () => handleRetry(selected) : undefined
              }
              onUpscale={
                selected && selected.state === "completed" && selected.job_type !== "upscale"
                  ? () => handleUpscale(selected.id)
                  : undefined
              }
              onReuseSeed={setSeed}
            />
          </section>
        </div>

        <section className="card card--wide">
          <header className="card__head">
            <h2>Gallery</h2>
            <span className="card__sub numeric">{gallery.length}</span>
          </header>
          {gallery.length === 0 ? (
            <p className="muted">Rendered clips show up here.</p>
          ) : (
            <>
              <div className="gallery">
                {pagedGallery.map((j, i) => (
                  <div
                    key={j.id}
                    className={
                      j.id === selectedId
                        ? "gallery__item video-item gallery__item--selected"
                        : "gallery__item video-item"
                    }
                  >
                    <button
                      type="button"
                      className="gallery__item-select"
                      onClick={() => {
                        setSelectedId(j.id);
                        setDetail(null);
                      }}
                    >
                      {about && (
                        <span className="video-item__frame">
                          <video
                            src={jobOutputUrl(about.core_api_port, j.id)}
                            muted
                            preload="metadata"
                            playsInline
                          />
                          <span className="video-item__play" aria-hidden="true">
                            ▶
                          </span>
                        </span>
                      )}
                      <span className="gallery__cap">{asVideoParams(j.params).prompt ?? "video"}</span>
                    </button>
                    <button
                      type="button"
                      className="gallery__zoom"
                      aria-label="Zoom this video"
                      onClick={(e) => {
                        e.stopPropagation();
                        setLightboxIndex(i);
                      }}
                    >
                      ⤢
                    </button>
                    <button
                      type="button"
                      className="gallery__delete"
                      aria-label="Delete this video"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDelete(j.id);
                      }}
                    >
                      ×
                    </button>
                  </div>
                ))}
              </div>
              {pageCount > 1 && (
                <div className="gallery__pager">
                  <button
                    type="button"
                    className="chip"
                    disabled={clampedPage === 0}
                    onClick={() => setGalleryPage((p) => Math.max(0, p - 1))}
                  >
                    ← Prev
                  </button>
                  <span className="muted numeric">
                    Page {clampedPage + 1} / {pageCount}
                  </span>
                  <button
                    type="button"
                    className="chip"
                    disabled={clampedPage >= pageCount - 1}
                    onClick={() => setGalleryPage((p) => Math.min(pageCount - 1, p + 1))}
                  >
                    Next →
                  </button>
                </div>
              )}
            </>
          )}
        </section>

        {lightboxIndex != null && about && pagedGallery[lightboxIndex] && (
          <Lightbox
            kind="video"
            src={jobOutputUrl(about.core_api_port, pagedGallery[lightboxIndex].id)}
            caption={asVideoParams(pagedGallery[lightboxIndex].params).prompt}
            onClose={() => setLightboxIndex(null)}
            onPrev={lightboxIndex > 0 ? () => setLightboxIndex(lightboxIndex - 1) : undefined}
            onNext={
              lightboxIndex < pagedGallery.length - 1
                ? () => setLightboxIndex(lightboxIndex + 1)
                : undefined
            }
          />
        )}
      </div>
    </div>
  );
}

function latestProgress(events: JobEvent[]): string | null {
  for (let i = events.length - 1; i >= 0; i--) {
    const m = events[i].message;
    if (m.startsWith("rendering ") || m.includes("takes several minutes") || m.startsWith("image→video")) {
      return m;
    }
  }
  return null;
}

function asUpscaleParams(p: unknown): Partial<UpscaleParams> {
  return p && typeof p === "object" ? (p as Partial<UpscaleParams>) : {};
}

function upscaleSummary(p: Partial<UpscaleParams>): string {
  const resize =
    p.resize_mode === "dimensions" && p.width && p.height
      ? `${p.width}×${p.height}`
      : `${(p.scale ?? 2).toFixed(2)}×`;
  return `${resize} · ${p.quality ?? "ULTRA"} quality`;
}

function Result({
  job,
  events,
  port,
  modelNames,
  progress,
  onCancel,
  onDelete,
  onRetry,
  onUpscale,
  onReuseSeed,
}: {
  job: Job | null;
  events: JobEvent[];
  port: number | null;
  modelNames: Map<string, string>;
  /** Real per-step progress from ComfyUI's own `/ws`, when this job is the
   *  one currently streaming it. */
  progress?: JobProgress | null;
  onCancel?: () => void;
  onDelete?: () => void;
  /** Resubmit this failed job's own params as a fresh job. Only offered for
   *  `state === "failed"`. */
  onRetry?: () => void;
  onUpscale?: () => void;
  onReuseSeed: (seed: string) => void;
}) {
  const upscaleId = useId();
  if (!job) return <p className="muted">Fill in a prompt and hit Generate.</p>;

  const isUpscale = job.job_type === "upscale";
  const p = asVideoParams(job.params);
  const up = asUpscaleParams(job.params);
  const running = !DONE.includes(job.state);
  const modelName = job.model_id ? (modelNames.get(job.model_id) ?? job.model_id) : "—";
  const logLine = latestProgress(events);
  const live = progress && progress.job_id === job.id ? progress : null;
  const secs = p.length && p.fps ? (p.length / p.fps).toFixed(1) : null;

  return (
    <div className="result">
      <div className="result__canvas" data-state={job.state}>
        {job.state === "completed" && port != null ? (
          <video src={jobOutputUrl(port, job.id)} controls preload="metadata" playsInline />
        ) : job.state === "failed" ? (
          <span className="result__err">{job.error_text ?? "generation failed"}</span>
        ) : job.state === "blocked" ? (
          <span className="result__err">
            {job.error_text ?? "not enough VRAM free right now"}
          </span>
        ) : job.state === "cancelled" ? (
          <span className="muted">cancelled</span>
        ) : isUpscale ? (
          <span className="result__spin">
            upscaling…
            <br />
            <span className="muted">This can take a while for a longer clip.</span>
          </span>
        ) : (
          <span className="result__spin">
            {live?.node ? `${live.node}…` : (logLine ?? `${job.state}…`)}
            <br />
            <span className="muted">This can take several minutes.</span>
          </span>
        )}
      </div>
      {running && live?.steps_total != null && (
        <Meter label="Rendering" value={live.step ?? 0} max={live.steps_total} unit="steps" />
      )}
      <dl className="result__meta">
        {isUpscale ? (
          <>
            <dt>Upscaled from</dt>
            <dd className="numeric">{up.source ?? "—"}</dd>
            <dt>
              RTX Video Super Resolution <HelpHint area="upscale" setting="factor" />
            </dt>
            <dd className="numeric">{upscaleSummary(up)}</dd>
          </>
        ) : (
          <>
            {p.prompt && (
              <>
                <dt>Prompt</dt>
                <dd>{p.prompt}</dd>
              </>
            )}
            {p.negative && (
              <>
                <dt>Negative</dt>
                <dd>{p.negative}</dd>
              </>
            )}
            {p.init_image && (
              <>
                <dt>Start frame</dt>
                <dd className="numeric">{p.init_image}</dd>
              </>
            )}
            <dt>Model</dt>
            <dd>{modelName}</dd>
            {p.width && p.height && (
              <>
                <dt>Clip</dt>
                <dd className="numeric">
                  {p.width}×{p.height} · {p.length ?? "?"}f @ {p.fps ?? "?"}fps
                  {secs && ` (~${secs}s)`} · {p.steps ?? "?"} steps · cfg {p.cfg ?? "?"}
                </dd>
              </>
            )}
            {p.seed != null && (
              <>
                <dt>Seed</dt>
                <dd className="numeric">
                  {p.seed}
                  <button
                    type="button"
                    className="result__reuse"
                    onClick={() => onReuseSeed(String(p.seed))}
                  >
                    reuse
                  </button>
                </dd>
              </>
            )}
          </>
        )}
      </dl>
      <div className="result__actions">
        {job.state === "completed" && onUpscale && (
          <>
            <button id={upscaleId} type="button" className="result__cancel" onClick={onUpscale}>
              Upscale
            </button>
            <HelpHint area="upscale" setting="upscale" describes={upscaleId} />
          </>
        )}
        {job.state === "completed" && (
          <button
            type="button"
            className="result__cancel"
            onClick={() => downloadJobOutput(job)}
          >
            Download
          </button>
        )}
        {job.state === "failed" && onRetry && (
          <button type="button" className="result__cancel" onClick={onRetry}>
            Erneut versuchen
          </button>
        )}
        {running && onCancel && (
          <button type="button" className="result__cancel" onClick={onCancel}>
            Stop
          </button>
        )}
        {!running && onDelete && (
          <button type="button" className="result__cancel" onClick={onDelete}>
            Delete
          </button>
        )}
      </div>
    </div>
  );
}
