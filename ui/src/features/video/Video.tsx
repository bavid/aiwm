import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { LoraPicker } from "../../components/LoraPicker";
import { NumField } from "../../components/NumField";
import { PromptAssistant } from "../../components/PromptAssistant";
import { PromptPresetPicker } from "../../components/PromptPresetPicker";
import { QueueList } from "../../components/QueueList";
import { SessionSwitcher } from "../../components/SessionSwitcher";
import { VramEstimateHint } from "../../components/VramEstimateHint";
import { useAbout, useJobs, useModels, useRuntimes, useTelemetry } from "../../lib/hooks";
import {
  cancelJob,
  deleteJob,
  jobDetail,
  jobOutputUrl,
  submitJob,
  type Job,
  type JobDetail,
  type JobEvent,
  type JobState,
  type LoraParam,
  type VideoParams,
} from "../../lib/ipc";
import "./video.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 900;

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

  const [pendingId, setPendingId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);

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

  const canGenerate = !!prompt.trim() && (!pendingId || stuck) && comfyReady;

  return (
    <div className="image">
      <section className="card image__form">
        <header className="card__head">
          <h2>Generate a video</h2>
          {modelId === "auto" && videoModels.length > 0 && (
            <span className="card__sub">Auto · {videoModels.length} model(s)</span>
          )}
        </header>
        <SessionSwitcher capability="video" activeId={sessionId} onChange={setSessionId} />

        <fieldset className="startframe">
          <legend>Start from an image (optional)</legend>
          <label className="imgform__field">
            <span>A finished image</span>
            <select value={startJob} onChange={(e) => setStartJob(e.target.value)}>
              <option value="none">None — text to video</option>
              {imageJobs.map((j) => (
                <option key={j.id} value={j.id}>
                  {promptOf(j).slice(0, 48)}
                </option>
              ))}
            </select>
          </label>
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
            <NumField label="Width" value={width} step={DIM_STEP} min={MIN_DIM} max={MAX_DIM} onChange={setWidth} />
            <NumField label="Height" value={height} step={DIM_STEP} min={MIN_DIM} max={MAX_DIM} onChange={setHeight} />
            <NumField label="Frames" value={frames} step={4} min={MIN_FRAMES} max={MAX_FRAMES} onChange={setFrames} />
            <NumField label="FPS" value={fps} step={1} min={MIN_FPS} max={MAX_FPS} onChange={setFps} />
            <NumField label="Steps" value={steps} step={1} min={1} max={MAX_STEPS} onChange={setSteps} />
            <NumField label="CFG" value={cfg} step={0.5} min={1} max={15} onChange={setCfg} />
          </div>

          <div className="imgform__grid">
            <label className="imgform__field">
              <span>Seed</span>
              <input
                type="text"
                inputMode="numeric"
                value={seed}
                onChange={(e) => setSeed(e.target.value.replace(/[^\d]/g, ""))}
                placeholder="random"
                spellCheck={false}
              />
            </label>
            <label className="imgform__field imgform__field--wide">
              <span>Model</span>
              <select value={modelId} onChange={(e) => setModelId(e.target.value)}>
                <option value="auto">Auto (most-recently-used)</option>
                {videoModels.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <VramEstimateHint
            vramEstimateMb={videoModels.find((m) => m.id === modelId)?.vram_estimate_mb}
            gpu={telemetry?.gpu}
          />
          <LoraPicker
            models={models ?? []}
            family={videoModels.find((m) => m.id === modelId)?.family}
            selected={loras}
            onChange={setLoras}
          />

          <p className={heavy ? "video__note video__note--warn" : "video__note"}>
            ~{clipSecs.toFixed(1)}s clip · very rough guess {estLo}–{estHi} min on a 16 GB card
            (not yet calibrated). Video is slow — minutes, not seconds. The window stays usable
            while it renders.
            {heavy && " Above 480p / 81 frames is much slower and can run out of VRAM."}
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
            onCancel={selected ? () => cancelJob(selected.id) : undefined}
            onDelete={selected ? () => handleDelete(selected.id) : undefined}
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
          <div className="gallery">
            {gallery.map((j) => (
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
                  className="gallery__delete"
                  title="Delete"
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
        )}
      </section>
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

function Result({
  job,
  events,
  port,
  modelNames,
  onCancel,
  onDelete,
  onReuseSeed,
}: {
  job: Job | null;
  events: JobEvent[];
  port: number | null;
  modelNames: Map<string, string>;
  onCancel?: () => void;
  onDelete?: () => void;
  onReuseSeed: (seed: string) => void;
}) {
  if (!job) return <p className="muted">Fill in a prompt and hit Generate.</p>;

  const p = asVideoParams(job.params);
  const running = !DONE.includes(job.state);
  const modelName = job.model_id ? (modelNames.get(job.model_id) ?? job.model_id) : "—";
  const progress = latestProgress(events);
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
        ) : (
          <span className="result__spin">
            {progress ?? `${job.state}…`}
            <br />
            <span className="muted">This can take several minutes.</span>
          </span>
        )}
      </div>
      <dl className="result__meta">
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
      </dl>
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
  );
}
