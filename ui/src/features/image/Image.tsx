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
import { SessionSwitcher } from "../../components/SessionSwitcher";
import { VramEstimateHint } from "../../components/VramEstimateHint";
import {
  useAbout,
  useJobProgress,
  useJobs,
  useModels,
  useRuntimes,
  useTelemetry,
} from "../../lib/hooks";
import {
  cancelJob,
  deleteJob,
  downloadJobOutput,
  imageOutputUrl,
  jobDetail,
  submitJob,
  type ImageParams,
  type Job,
  type JobDetail,
  type JobProgress,
  type JobState,
  type LoraParam,
  type UpscaleParams,
} from "../../lib/ipc";
import { HiresFixField } from "./HiresFixField";
import {
  DEFAULT_HIRES,
  hiresAutoSteps,
  hiresFinalDim,
  toHiresParams,
  type HiresFixSettings,
} from "./hires-fix";
import "./image.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 700;
const MIN_DIM = 512;
const MAX_DIM = 2048;
const DIM_STEP = 64;
const GALLERY_PAGE_SIZE = 24;
/** The first pass's step count a fresh form starts at, and the fallback when
 *  a job's own params don't carry one. */
const DEFAULT_STEPS = 25;

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

/** Same middle-ground strength `LoraPicker` uses when a LoRA is ticked by
 *  hand — a first test render should look like the picker's own default. */
const PREFILL_LORA_STRENGTH = 0.8;

/** A hand-over from another tab: a LoRA to stack and a prompt to start from.
 *  Today the Training tab's "Test now" is the only source. */
export type ImagePrefill = {
  loraModelId: string;
  prompt: string;
};

type Props = {
  /** Applied once, then handed back via {@link Props.onPrefillConsumed}. */
  prefill?: ImagePrefill | null;
  onPrefillConsumed?: () => void;
};

export function ImageStudio({ prefill = null, onPrefillConsumed }: Props = {}) {
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
  const [steps, setSteps] = useState(DEFAULT_STEPS);
  const [cfg, setCfg] = useState(7);
  const [seed, setSeed] = useState("");
  const [modelId, setModelId] = useState("auto");
  const [loras, setLoras] = useState<LoraParam[]>([]);
  const [hires, setHires] = useState<HiresFixSettings>(DEFAULT_HIRES);
  const [sendError, setSendError] = useState<string | null>(null);
  const [sourceJob, setSourceJob] = useState("none");
  const [sourcePath, setSourcePath] = useState("");
  const ids = useId();
  const sourceJobId = `${ids}-source`;
  const negativeId = `${ids}-negative`;
  const swapId = `${ids}-swap`;
  const seedId = `${ids}-seed`;
  const modelPickId = `${ids}-model`;
  const presetId = `${ids}-preset`;
  const sessionHintId = `${ids}-session`;

  const selectedCheckpoint = checkpoints.find((m) => m.id === modelId);
  const isFlux = modelId !== "auto" && selectedCheckpoint?.family === "flux";
  const isFlux2 = modelId !== "auto" && selectedCheckpoint?.family === "flux2";

  // Editing an existing image instead of generating one from scratch --
  // a finished image job's id, or a path to a file on disk.
  const sourceImage = sourcePath.trim() || (sourceJob === "none" ? "" : sourceJob);
  const editing = !!sourceImage;
  const priorImages = (jobs ?? []).filter(
    (j) => j.job_type === "image" && j.state === "completed" && j.output_path,
  );

  const browseForSourceImage = async () => {
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }],
      });
      if (typeof picked === "string") setSourcePath(picked);
    } catch {
      // Not running inside Tauri (e.g. the browser dev preview) — no-op.
    }
  };

  const [pendingId, setPendingId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Real per-step progress straight from ComfyUI's own `/ws`, layered on top
  // of the `jobDetail` poll below (which still carries state/output/events) —
  // see `core::progress` / `GET /ws/jobs/{id}`.
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
    // `jobs` (2 s poll) refreshes the gallery once the job completes.
  }, [pendingId]);

  const gallery = (jobs ?? []).filter(
    (j) =>
      j.job_type === "image" &&
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

  // A `blocked` job (not enough VRAM right now) isn't actively running -- it's
  // just waiting for room, and may sit there indefinitely if none frees up.
  // Don't lock the form forever: let the user start a fresh (e.g. smaller)
  // request instead of being stuck until they cancel or switch tabs.
  const stuck = detail?.job.state === "blocked";

  const generate = async () => {
    const text = prompt.trim();
    if (!text || (pendingId && !stuck)) return;
    setSendError(null);

    const params: Record<string, unknown> = editing
      ? { prompt: text, source_image: sourceImage, steps, cfg }
      : {
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
    // Text-to-image only — an edit keeps the source image's own size.
    if (!editing && hires.enabled) params.hires = toHiresParams(hires);

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

  // A hand-over from another tab (Training's "Test now"): stack the freshly
  // trained LoRA and put its trigger word in the prompt, then clear the
  // hand-over so switching back here later does not overwrite a new draft.
  useEffect(() => {
    if (!prefill) return;
    setLoras((cur) =>
      cur.some((l) => l.model_id === prefill.loraModelId)
        ? cur
        : [...cur, { model_id: prefill.loraModelId, strength: PREFILL_LORA_STRENGTH }],
    );
    setPrompt(prefill.prompt);
    onPrefillConsumed?.();
  }, [prefill, onPrefillConsumed]);

  const appendPrompt = (text: string) =>
    setPrompt((p) => (p.trim() ? `${p.trim()}, ${text}` : text));
  const appendNegative = (text: string) =>
    setNegative((n) => (n.trim() ? `${n.trim()}, ${text}` : text));

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
   *  job (never mutates the failed one). Mirrors `generate()`/`handleUpscale`:
   *  submit, then switch the Result panel over to watch the new job. */
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

  /** "Use as base" — send a finished result straight into the "Edit an
   *  existing image" field instead of requiring a fresh file browse, so the
   *  next prompt re-uses it as the img2img source. */
  const handleUseAsBase = (jobId: string) => {
    setSourcePath("");
    setSourceJob(jobId);
  };

  const canGenerate = !!prompt.trim() && (!pendingId || stuck) && comfyReady;

  return (
    <div className="image">
      <section className="card image__form">
        <header className="card__head">
          <h2>{editing ? "Edit an image" : "Generate an image"}</h2>
          {!editing && modelId === "auto" && checkpoints.length > 0 && (
            <span className="card__sub">Auto · {checkpoints.length} checkpoint(s)</span>
          )}
        </header>
        <div className="imgform__session" id={sessionHintId}>
          <SessionSwitcher capability="image" activeId={sessionId} onChange={setSessionId} />
          <HelpHint area="image" setting="sessions" describes={sessionHintId} />
        </div>

        <fieldset className="startframe">
          <legend>Edit an existing image (optional)</legend>
          <div className="imgform__field">
            <span>
              <label htmlFor={sourceJobId}>A finished image</label>
              <HelpHint area="image" setting="edit-source" describes={sourceJobId} />
            </span>
            <select id={sourceJobId} value={sourceJob} onChange={(e) => setSourceJob(e.target.value)}>
              <option value="none">None — generate from a prompt</option>
              {priorImages.map((j) => (
                <option key={j.id} value={j.id}>
                  {promptOf(j).slice(0, 48) || j.id}
                </option>
              ))}
            </select>
          </div>
          <label className="imgform__field">
            <span>…or an image file</span>
            <div className="pathpick">
              <input
                type="text"
                value={sourcePath}
                onChange={(e) => setSourcePath(e.target.value)}
                placeholder="E:\\photos\\me.jpg"
                spellCheck={false}
              />
              <button type="button" className="chip" onClick={browseForSourceImage}>
                Browse…
              </button>
            </div>
          </label>
          {sourceImage && about && sourcePath.trim() === "" && (
            <img
              className="startframe__thumb"
              src={imageOutputUrl(about.core_api_port, sourceImage)}
              alt="The source this prompt will edit"
              loading="lazy"
            />
          )}
          {editing && !isFlux2 && (
            <p className="muted">Editing needs the FLUX.2 [klein] 9B stack — pick it below.</p>
          )}
        </fieldset>

        <PromptAssistant
          kind={editing ? "edit" : "image"}
          sessionId={sessionId}
          onApplyPrompt={appendPrompt}
          onApplyNegative={editing ? undefined : appendNegative}
        />


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
            <span>{editing ? "Edit instruction" : "Prompt"}</span>
            <textarea
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              rows={3}
              spellCheck
              placeholder={
                editing
                  ? "remove the blisters, make the hair blonde, add me to a train platform…"
                  : "a red fox in the snow, cinematic lighting, highly detailed"
              }
            />
          </label>
          <PromptPresetPicker kind="positive" onApply={appendPrompt} />

          {!editing && (
            <>
              <div className="imgform__field">
                <span>
                  <label htmlFor={negativeId}>Negative prompt</label>
                  <HelpHint area="image" setting="negative-prompt" describes={negativeId} />
                </span>
                <textarea
                  id={negativeId}
                  value={negative}
                  onChange={(e) => setNegative(e.target.value)}
                  rows={2}
                  spellCheck
                  placeholder="blurry, low quality, watermark"
                />
              </div>
              <PromptPresetPicker kind="negative" onApply={appendNegative} />
            </>
          )}

          {!editing && (
            <>
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
                  id={swapId}
                  type="button"
                  className="chip"
                  aria-label="Swap width and height"
                  onClick={() => {
                    setWidth(height);
                    setHeight(width);
                  }}
                >
                  <span aria-hidden="true">↔</span>
                </button>
              </div>

              <div className="imgform__grid">
                <NumField
                  label="Width"
                  value={width}
                  step={DIM_STEP}
                  min={MIN_DIM}
                  max={MAX_DIM}
                  onChange={setWidth}
                  hint={(id) => <HelpHint area="image" setting="size" describes={id} />}
                />
                <NumField label="Height" value={height} step={DIM_STEP} min={MIN_DIM} max={MAX_DIM} onChange={setHeight} />
              </div>
            </>
          )}
          {editing && (
            <p className="muted">The edited image keeps the source image's own size.</p>
          )}

          <div className="imgform__grid">
            <NumField
              label="Steps"
              value={steps}
              step={1}
              min={1}
              max={60}
              onChange={setSteps}
              hint={(id) => <HelpHint area="image" setting="steps" describes={id} />}
            />
            <NumField
              label={isFlux ? "Guidance" : "CFG"}
              value={cfg}
              step={0.5}
              min={1}
              max={isFlux || isFlux2 ? 10 : 15}
              onChange={setCfg}
              hint={(id) => <HelpHint area="image" setting="cfg" describes={id} />}
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

          {!editing && (
            <HiresFixField
              value={hires}
              onChange={setHires}
              width={clampDim(width)}
              height={clampDim(height)}
              steps={steps}
            />
          )}

          <div className="imgform__grid">
            <div className="imgform__field">
              <span>
                <label htmlFor={seedId}>Seed</label>
                <HelpHint area="image" setting="seed" describes={seedId} />
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
                <HelpHint area="image" setting="model" describes={modelPickId} />
              </span>
              <select id={modelPickId} value={modelId} onChange={(e) => setModelId(e.target.value)}>
                <option value="auto">Auto (most-recently-used)</option>
                {checkpoints.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <div className="imgform__presets">
            <button id={presetId} type="button" className="chip" onClick={applySmartphonePreset}>
              Smartphone photo preset
            </button>
            <HelpHint area="image" setting="smartphone-preset" describes={presetId} />
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
            onUseAsBase={
              selected && selected.state === "completed" && selected.job_type === "image"
                ? () => handleUseAsBase(selected.id)
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
          <p className="muted">Generated images show up here.</p>
        ) : (
          <>
            <div className="gallery">
              {pagedGallery.map((j, i) => (
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
                    className="gallery__zoom"
                    aria-label="Zoom this image"
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
          kind="image"
          src={imageOutputUrl(about.core_api_port, pagedGallery[lightboxIndex].id)}
          caption={asImageParams(pagedGallery[lightboxIndex].params).prompt}
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
  );
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
  port,
  modelNames,
  progress,
  onCancel,
  onDelete,
  onRetry,
  onUpscale,
  onUseAsBase,
  onReuseSeed,
}: {
  job: Job | null;
  port: number | null;
  modelNames: Map<string, string>;
  /** Real per-step progress from ComfyUI's own `/ws`, when this job is the
   *  one currently streaming it (`null` otherwise — an earlier phase with no
   *  node executing yet, a connection that didn't come up, or this just
   *  isn't the in-flight job). */
  progress?: JobProgress | null;
  onCancel?: () => void;
  onDelete?: () => void;
  /** Resubmit this failed job's own params as a fresh job. Only offered for
   *  `state === "failed"`. */
  onRetry?: () => void;
  onUpscale?: () => void;
  /** Send this finished image back into the "Edit an existing image" field —
   *  removes the extra file-browse round trip to iterate on a result. */
  onUseAsBase?: () => void;
  onReuseSeed: (seed: string) => void;
}) {
  const ids = useId();
  const upscaleId = `${ids}-upscale`;
  const useAsBaseId = `${ids}-base`;
  if (!job) return <p className="muted">Fill in a prompt and hit Generate.</p>;

  const isUpscale = job.job_type === "upscale";
  const p = asImageParams(job.params);
  const up = asUpscaleParams(job.params);
  const running = !DONE.includes(job.state);
  const modelName = job.model_id ? (modelNames.get(job.model_id) ?? job.model_id) : "—";
  const live = progress && progress.job_id === job.id ? progress : null;

  // The engine writes `output_*` and the resolved second-pass steps back only
  // once the job actually runs, so predict them the same way for a job that is
  // still queued or blocked — otherwise it would read "1024×1024 → 1024×1024
  // · ? steps". Both rows below use these, so they can never disagree.
  const finalWidth =
    p.output_width ?? (p.hires && p.width ? hiresFinalDim(p.width, p.hires.scale_by) : p.width);
  const finalHeight =
    p.output_height ?? (p.hires && p.height ? hiresFinalDim(p.height, p.hires.scale_by) : p.height);
  const hiresSteps = p.hires?.steps ?? hiresAutoSteps(p.steps ?? DEFAULT_STEPS);

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
          <span className="result__spin">
            {isUpscale ? "upscaling…" : live?.node ? `${live.node}…` : `${job.state}…`}
          </span>
        )}
      </div>
      {running && live?.steps_total != null && (
        <Meter
          label="Rendering"
          value={live.step ?? 0}
          max={live.steps_total}
          unit="steps"
        />
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
                <dt>{p.source_image ? "Edit instruction" : "Prompt"}</dt>
                <dd>{p.prompt}</dd>
              </>
            )}
            {p.source_image && (
              <>
                <dt>Edited from</dt>
                <dd className="numeric">{p.source_image}</dd>
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
                  {/* Hi-Res-Fix finishes bigger than it was asked for. */}
                  {finalWidth}×{finalHeight} · {p.steps ?? "?"} steps · cfg {p.cfg ?? "?"}
                </dd>
              </>
            )}
            {p.hires && p.width && p.height && (
              <>
                <dt>Hi-res fix</dt>
                <dd className="numeric">
                  {p.width}×{p.height} → {finalWidth}×{finalHeight} · denoise{" "}
                  {p.hires.denoise.toFixed(2)} · {hiresSteps} steps ·{" "}
                  {p.hires.upscale_method ?? "nearest-exact"}
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
        {job.state === "completed" && onUseAsBase && (
          <>
            <button id={useAsBaseId} type="button" className="result__cancel" onClick={onUseAsBase}>
              Use as base
            </button>
            <HelpHint area="image" setting="use-as-base" describes={useAsBaseId} />
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
