import { useEffect, useMemo, useState } from "react";
import { LoraPicker } from "../../components/LoraPicker";
import { NumField } from "../../components/NumField";
import { QueueList } from "../../components/QueueList";
import { SessionSwitcher } from "../../components/SessionSwitcher";
import { VramEstimateHint } from "../../components/VramEstimateHint";
import { useAbout, useJobs, useModels, useRuntimes, useTelemetry } from "../../lib/hooks";
import {
  cancelJob,
  deleteJob,
  imageOutputUrl,
  jobDetail,
  submitJob,
  type ImageParams,
  type Job,
  type JobDetail,
  type JobState,
  type LoraParam,
} from "../../lib/ipc";
import "./image.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 700;
const MIN_DIM = 512;
const MAX_DIM = 2048;
const DIM_STEP = 64;

const PRESETS = [
  { label: "Square", w: 1024, h: 1024 },
  { label: "Portrait", w: 832, h: 1216 },
  { label: "Landscape", w: 1216, h: 832 },
];

const clampDim = (n: number) =>
  Math.min(MAX_DIM, Math.max(MIN_DIM, Math.round(n / DIM_STEP) * DIM_STEP));

function asImageParams(p: unknown): Partial<ImageParams> {
  return p && typeof p === "object" ? (p as Partial<ImageParams>) : {};
}

function promptOf(job: Job): string {
  return asImageParams(job.params).prompt ?? "";
}

export function ImageStudio() {
  const about = useAbout();
  const { data: models } = useModels();
  const { data: runtimes } = useRuntimes();
  const { data: jobs } = useJobs();
  const { telemetry } = useTelemetry();

  const checkpoints = (models ?? []).filter((m) => m.roles.includes("base_diffusion"));
  const modelNames = useMemo(
    () => new Map((models ?? []).map((m) => [m.id, m.name])),
    [models],
  );
  const comfy = (runtimes ?? []).find((r) => r.id === "comfyui");
  const comfyReady = !comfy || !(comfy.detail ?? "").includes("not installed");

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [prompt, setPrompt] = useState("");
  const [negative, setNegative] = useState("");
  const [width, setWidth] = useState(1024);
  const [height, setHeight] = useState(1024);
  const [steps, setSteps] = useState(25);
  const [cfg, setCfg] = useState(7);
  const [seed, setSeed] = useState("");
  const [modelId, setModelId] = useState("auto");
  const [loras, setLoras] = useState<LoraParam[]>([]);
  const [sendError, setSendError] = useState<string | null>(null);

  const selectedCheckpoint = checkpoints.find((m) => m.id === modelId);
  const isFlux = modelId !== "auto" && selectedCheckpoint?.family === "flux";
  const isFlux2 = modelId !== "auto" && selectedCheckpoint?.family === "flux2";

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
    // `jobs` (2 s poll) refreshes the gallery once the job completes.
  }, [pendingId]);

  const gallery = (jobs ?? []).filter(
    (j) =>
      j.job_type === "image" &&
      j.state === "completed" &&
      j.output_path &&
      j.session_id === sessionId,
  );

  const selected =
    (detail?.job.id === selectedId ? detail.job : null) ??
    (jobs ?? []).find((j) => j.id === selectedId) ??
    null;

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
      width: clampDim(width),
      height: clampDim(height),
      steps,
      cfg,
    };
    const s = Number(seed);
    if (seed.trim() !== "" && Number.isFinite(s) && s >= 0) params.seed = Math.floor(s);
    if (loras.length > 0) params.loras = loras;

    try {
      const job = await submitJob({
        job_type: "image",
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

  /** One-click recipe: FLUX.2 [klein] 9B + its realistic-detail LoRA, low CFG,
   *  a lighting prompt suffix — a saved "config" for the realistic
   *  smartphone-photo look, rather than hand-tuning every field each time. */
  const applySmartphonePreset = () => {
    const flux2Model = (models ?? []).find(
      (m) => m.roles.includes("base_diffusion") && m.family === "flux2",
    );
    if (!flux2Model) {
      setSendError(
        "Import the FLUX.2 [klein] 9B stack first — Models tab → Discover.",
      );
      return;
    }
    const flux2Lora = (models ?? []).find(
      (m) => m.roles.includes("lora") && m.family === "flux2",
    );
    setModelId(flux2Model.id);
    setCfg(1.5);
    setSteps(8);
    setLoras(flux2Lora ? [{ model_id: flux2Lora.id, strength: 0.8 }] : []);
    setPrompt((p) => {
      const suffix = "natural window light, soft shadows, candid framing, shot on iphone, high detail skin";
      return p.trim() ? `${p.trim()}, ${suffix}` : `a candid photo, ${suffix}`;
    });
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
          <h2>Generate an image</h2>
          {modelId === "auto" && checkpoints.length > 0 && (
            <span className="card__sub">Auto · {checkpoints.length} checkpoint(s)</span>
          )}
        </header>
        <SessionSwitcher capability="image" activeId={sessionId} onChange={setSessionId} />


        {!comfyReady && (
          <p className="muted">
            ComfyUI is not set up yet — open Diagnostics to install it.
          </p>
        )}
        {comfyReady && models && checkpoints.length === 0 && (
          <p className="muted">
            No image checkpoint yet — import an SDXL <code>.safetensors</code> on the Models tab.
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
              placeholder="a red fox in the snow, cinematic lighting, highly detailed"
            />
          </label>

          <label className="imgform__field">
            <span>Negative prompt</span>
            <textarea
              value={negative}
              onChange={(e) => setNegative(e.target.value)}
              rows={2}
              spellCheck
              placeholder="blurry, low quality, watermark"
            />
          </label>

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
            <button
              type="button"
              className="chip"
              title="Swap width and height"
              onClick={() => {
                setWidth(height);
                setHeight(width);
              }}
            >
              ↔
            </button>
          </div>

          <div className="imgform__grid">
            <NumField label="Width" value={width} step={DIM_STEP} min={MIN_DIM} max={MAX_DIM} onChange={setWidth} />
            <NumField label="Height" value={height} step={DIM_STEP} min={MIN_DIM} max={MAX_DIM} onChange={setHeight} />
            <NumField label="Steps" value={steps} step={1} min={1} max={60} onChange={setSteps} />
            <NumField
              label={isFlux ? "Guidance" : "CFG"}
              value={cfg}
              step={0.5}
              min={1}
              max={isFlux || isFlux2 ? 10 : 15}
              onChange={setCfg}
            />
          </div>
          {isFlux && (
            <p className="muted">Flux runs at CFG 1 — this sets FluxGuidance (≈ 3–4 is typical).</p>
          )}
          {isFlux2 && (
            <p className="muted">
              FLUX.2 Klein is fast/distilled — low CFG (≈1.5–2) and few steps (≈8) is typical.
            </p>
          )}

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
                {checkpoints.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <div className="imgform__presets">
            <button
              type="button"
              className="chip"
              title="FLUX.2 [klein] 9B + realistic-detail LoRA, low CFG, a lighting prompt suffix"
              onClick={applySmartphonePreset}
            >
              Smartphone photo preset
            </button>
          </div>
          <LoraPicker
            models={models ?? []}
            family={selectedCheckpoint?.family}
            selected={loras}
            onChange={setLoras}
          />
          <VramEstimateHint vramEstimateMb={selectedCheckpoint?.vram_estimate_mb} gpu={telemetry?.gpu} />

          <button type="submit" className="imgform__go" disabled={!canGenerate}>
            {pendingId && !stuck ? "Generating…" : "Generate"}
          </button>
        </form>
        {sendError && <p className="image__err">{sendError}</p>}
      </section>

      <div className="image__result">
        <QueueList
          jobType="image"
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
          <p className="muted">Generated images show up here.</p>
        ) : (
          <div className="gallery">
            {gallery.map((j) => (
              <div
                key={j.id}
                className={
                  j.id === selectedId ? "gallery__item gallery__item--selected" : "gallery__item"
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
                    <img src={imageOutputUrl(about.core_api_port, j.id)} alt="" loading="lazy" />
                  )}
                  <span className="gallery__cap">
                    {asImageParams(j.params).prompt ?? "image"}
                  </span>
                </button>
                <button
                  type="button"
                  className="gallery__delete"
                  title="Delete"
                  aria-label="Delete this image"
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

function Result({
  job,
  port,
  modelNames,
  onCancel,
  onDelete,
  onReuseSeed,
}: {
  job: Job | null;
  port: number | null;
  modelNames: Map<string, string>;
  onCancel?: () => void;
  onDelete?: () => void;
  onReuseSeed: (seed: string) => void;
}) {
  if (!job) return <p className="muted">Fill in a prompt and hit Generate.</p>;

  const p = asImageParams(job.params);
  const running = !DONE.includes(job.state);
  const modelName = job.model_id ? (modelNames.get(job.model_id) ?? job.model_id) : "—";

  return (
    <div className="result">
      <div className="result__canvas" data-state={job.state}>
        {job.state === "completed" && port != null ? (
          <img src={imageOutputUrl(port, job.id)} alt={p.prompt ?? "generated image"} />
        ) : job.state === "failed" ? (
          <span className="result__err">{job.error_text ?? "generation failed"}</span>
        ) : job.state === "blocked" ? (
          <span className="result__err">
            {job.error_text ?? "not enough VRAM free right now"}
          </span>
        ) : job.state === "cancelled" ? (
          <span className="muted">cancelled</span>
        ) : (
          <span className="result__spin">{job.state}…</span>
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
        <dt>Model</dt>
        <dd>{modelName}</dd>
        {p.width && p.height && (
          <>
            <dt>Size</dt>
            <dd className="numeric">
              {p.width}×{p.height} · {p.steps ?? "?"} steps · cfg {p.cfg ?? "?"}
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
