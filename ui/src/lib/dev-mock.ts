/** A canned Tauri IPC bridge for running the UI in a plain browser (`pnpm dev`
 *  outside the Tauri shell). Dev-only: `main.tsx` loads this via a dynamic
 *  import that is dead code in a production build. It exists so the studios can
 *  be eyeballed without booting the whole stack; it is not a test double for
 *  logic. Media URLs (`/jobs/{id}/output`) will not resolve here — the layout
 *  and controls are what this is for. */
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { HIRES_UPSCALE_METHODS } from "../features/image/hires-fix";

type AnyRecord = Record<string, unknown>;

const now = () => new Date().toISOString();

/** Removes every matching element from `arr` in place (mock-only helper —
 *  used to fake the real backend's cascade deletes, e.g. deleting a Story
 *  also drops its Characters/Npcs/Locations/Scenes). */
function removeWhere<T>(arr: T[], predicate: (item: T) => boolean): void {
  for (let i = arr.length - 1; i >= 0; i--) {
    if (predicate(arr[i])) arr.splice(i, 1);
  }
}

const MODELS: AnyRecord[] = [
  mkModel("m-wan", "wan2.2_ti2v_5B_fp16", { family: "wan", roles: ["base_video"], runtimes: ["comfyui"], vram_estimate_mb: 11800 }),
  mkModel("m-umt5", "umt5_xxl_fp8_e4m3fn_scaled", { roles: ["text_encoder"], runtimes: ["comfyui"] }),
  mkModel("m-wanvae", "wan2.2_vae", { family: "wan", roles: ["vae"], runtimes: ["comfyui"] }),
  mkModel("m-lora-wan-motion", "Wan Motion Boost", { family: "wan", roles: ["lora"], runtimes: ["comfyui"], size_bytes: 128_000_000 }),
  mkModel("m-sdxl", "SDXL Base 1.0", { family: "sdxl", roles: ["base_diffusion"], runtimes: ["comfyui"], vram_estimate_mb: 8200 }),
  mkModel("m-lora-detail", "Add Detail XL", { family: "sdxl", roles: ["lora"], runtimes: ["comfyui"], size_bytes: 220_000_000 }),
  mkModel("m-lora-flux-style", "Ink Wash Style (Flux)", { family: "flux", roles: ["lora"], runtimes: ["comfyui"], size_bytes: 340_000_000 }),
  mkModel("m-flux2-klein", "FLUX.2 [klein] 9B — Q4_K_M (GGUF)", { family: "flux2", format: "gguf", roles: ["base_diffusion"], runtimes: ["comfyui"], vram_estimate_mb: 5900 }),
  mkModel("m-lora-flux2-detail", "Realistic Detail LoRA (FLUX.2 Klein 9B)", { family: "flux2", roles: ["lora"], runtimes: ["comfyui"], size_bytes: 165_704_488 }),
  mkModel("m-qwen", "Qwen2.5 7B Instruct", { family: "qwen2", format: "gguf", quant: "Q5_K_M", param_count: 7_615_616_512, ctx_max: 32_768, roles: ["chat"], runtimes: ["llamacpp"], vram_estimate_mb: 6400 }),
  mkModel("m-hermes", "Hermes-3-Llama-3.1-8B", { family: "llama3", format: "gguf", quant: "Q5_K_M", param_count: 8_030_000_000, ctx_max: 131_072, roles: ["chat", "coding"], runtimes: ["llamacpp"], vram_estimate_mb: 5700 }),
  mkModel("m-kokoro", "Kokoro 82M — int8", { family: "kokoro", format: "onnx", roles: ["voice_model"], runtimes: [], size_bytes: 114_119_327 }),
  mkModel("m-kokoro-voices", "Kokoro voices (54 English voices)", { family: "kokoro", format: "bin", roles: ["voice_data"], runtimes: [], size_bytes: 28_214_398 }),
  // Dia's 9+3 files each import as their own row in the real app; one
  // representative row per role is enough to drive the dev-preview's
  // roles-based gate (`hasDiaEngine`/`hasDiaCodec` in Voice.tsx never look
  // past `.some(...)`).
  mkModel("m-dia-engine", "Dia 1.6B — model config", { family: "dia", format: "json", roles: ["dia_engine"], runtimes: [], size_bytes: 1396 }),
  mkModel("m-dia-codec", "DAC 44kHz codec — config", { format: "json", roles: ["dia_codec"], runtimes: [], size_bytes: 541 }),
];

const JOBS: AnyRecord[] = [
  mkJob("j-vid-1", "video", "completed", {
    model_id: "m-wan",
    output_path: "E:\\AI\\data\\outputs\\j-vid-1.mp4",
    params: { prompt: "a paper boat drifting down a rain-soaked street", negative: "blurry", width: 832, height: 480, length: 81, fps: 24, steps: 30, cfg: 5, seed: 4212981 },
  }),
  mkJob("j-img-1", "image", "completed", {
    model_id: "m-sdxl",
    output_path: "E:\\AI\\data\\outputs\\j-img-1.png",
    params: { prompt: "a bay at dawn, wide angle", negative: "", width: 1024, height: 1024, steps: 25, cfg: 7, seed: 99 },
  }),
];

function mkModel(id: string, name: string, over: AnyRecord): AnyRecord {
  return {
    id, name, publisher: null, family: null, format: "safetensors", quant: null, arch: null,
    param_count: null, file_path: `E:\\AI\\models\\${name}`, sha256: null, size_bytes: 6_000_000_000,
    ctx_max: null, vram_estimate_mb: null, ram_estimate_mb: null, source: "manual",
    imported_at: now(), last_used_at: null, use_count: 0,
    n_layers: null, n_embd: null, n_heads: null, n_kv_heads: null, roles: [], runtimes: [],
    ...over,
  };
}

function mkJob(id: string, jobType: string, state: string, over: AnyRecord): AnyRecord {
  return {
    id, job_type: jobType, capability: null, state, params: {}, runtime_id: "comfyui",
    model_id: null, created_at: now(), started_at: now(), finished_at: now(),
    error_text: null, output_path: null, result: null, session_id: null, ...over,
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/** Mirror of `ImageRequest`'s Hi-Res-Fix parsing (`core/src/capability/
 *  image.rs`): the same clamps and defaults, so the dev preview shows the
 *  numbers the real engine would pin back. Absent/null stays absent. */
function resolveHires(hires: AnyRecord | null, firstPassSteps: number): AnyRecord | null {
  if (!hires || typeof hires !== "object") return null;
  const method = String(hires.upscale_method ?? "").trim().toLowerCase();
  return {
    scale_by: clamp(Number(hires.scale_by ?? 1.5), 1.25, 2),
    denoise: clamp(Number(hires.denoise ?? 0.45), 0.2, 0.7),
    steps: clamp(Math.trunc(Number(hires.steps ?? Math.floor(firstPassSteps / 2))), 4, 60),
    upscale_method: HIRES_UPSCALE_METHODS.some((m) => m === method) ? method : "nearest-exact",
  };
}

/** `LatentUpscaleBy` rounds in *latent* units (8 px each), so the decoded
 *  size is `round(px / 8 * scale_by) * 8` -- 1000 px x 1.5 is 1504, not 1500.
 *  Same arithmetic as `ImageRequest::final_size`. */
function upscaledDim(dim: number, scaleBy: number): number {
  return Math.round(Math.trunc(dim / 8) * scaleBy) * 8;
}

/** What the engine writes back over a submitted job's params. Only the image
 *  path has anything to resolve here today: Hi-Res-Fix's clamped knobs plus
 *  the finished output size (absent when the render is single-pass). */
function resolveJobParams(jobType: string, params: AnyRecord): AnyRecord {
  if (jobType !== "image") return params;
  const hires = resolveHires((params.hires ?? null) as AnyRecord | null, Number(params.steps ?? 25));
  if (!hires) return { ...params, hires: null };
  const scaleBy = Number(hires.scale_by);
  return {
    ...params,
    hires,
    output_width: upscaledDim(Number(params.width ?? 1024), scaleBy),
    output_height: upscaledDim(Number(params.height ?? 1024), scaleBy),
  };
}

function mkSession(id: string, capability: string, name: string): AnyRecord {
  return {
    id,
    capability,
    name,
    created_at: now(),
    archived_at: null,
    persona_mode: "inherit",
    persona_id: null,
  };
}

/** Two seeded presets so the chip, the menu and the manage dialog all have
 *  something to show on a fresh preview. The real app seeds none (the spec
 *  keeps templates a UI affordance, not database rows) -- these stand in for
 *  "you have used this feature before". */
const PERSONAS: AnyRecord[] = [
  {
    id: "persona-concise",
    name: "Concise & direct",
    icon: "🎯",
    system_prompt:
      "Answer in as few words as the question honestly allows. No preamble, no summary of the question, no offers of further help.",
    created_at: now(),
    updated_at: now(),
  },
  {
    id: "persona-tutor",
    name: "Patient tutor",
    icon: "🧑‍🏫",
    system_prompt:
      "Explain things step by step, starting from what the reader already knows. Use one concrete example per idea and check understanding before moving on.",
    created_at: now(),
    updated_at: now(),
  },
];

/** The globally active persona (`chat.active_persona_id` in the real store). */
let activePersonaId: string | null = null;

/** Unicode's control category -- what Rust's `char::is_control` matches. */
const CONTROL_CHARS = /\p{Cc}/u;

/** Mirror of `core::persona::validate` -- the same limits, in the same order,
 *  with the same messages, so the manage dialog's "server said" line reads
 *  here exactly as it does against the real core. Returns the trimmed fields
 *  to store. */
function validatePersona(body: AnyRecord): AnyRecord {
  const name = String(body.name ?? "").trim();
  const icon = String(body.icon ?? "").trim();
  const systemPrompt = String(body.system_prompt ?? "").trim();

  if (!name) throw new Error("persona name must not be empty");
  const nameChars = [...name].length;
  if (nameChars > 60) {
    throw new Error(`persona name must be at most 60 characters (${nameChars} given)`);
  }
  if (CONTROL_CHARS.test(name)) {
    throw new Error("persona name must not contain control characters");
  }
  if (!icon) throw new Error("persona icon must not be empty");
  if (CONTROL_CHARS.test(icon)) {
    throw new Error("persona icon must not contain control characters");
  }
  const iconBytes = new TextEncoder().encode(icon).length;
  if (iconBytes > 64) {
    throw new Error(`persona icon too long: at most 64 bytes, ${iconBytes} given`);
  }
  if (!systemPrompt) throw new Error("persona system prompt must not be empty");
  const promptChars = [...systemPrompt].length;
  if (promptChars > 8000) {
    throw new Error(
      `persona system prompt must be at most 8000 characters (${promptChars} given)`,
    );
  }
  return { name, icon, system_prompt: systemPrompt };
}

/** Mirror of `core::persona::active`: the globally active persona, healing the
 *  key when it names one that no longer exists. */
function activeMockPersona(): AnyRecord | null {
  if (!activePersonaId) return null;
  const persona = PERSONAS.find((p) => p.id === activePersonaId);
  if (persona) return persona;
  activePersonaId = null;
  return null;
}

/** Mirror of `core::persona::resolve_effective`: a session's own override
 *  wins, `inherit` (and an unknown session id, and the ungrouped case) falls
 *  through to the globally active persona, and a dangling id anywhere heals
 *  itself on the spot rather than failing the chat. */
function resolvePersona(sessionId: string | null): AnyRecord {
  const session = sessionId ? SESSIONS.find((s) => s.id === sessionId) : undefined;
  if (session) {
    const mode = String(session.persona_mode ?? "inherit");
    if (mode === "none") return { persona: null, origin: "none" };
    if (mode === "persona") {
      const picked = PERSONAS.find((p) => p.id === session.persona_id);
      if (picked) return { persona: { ...picked }, origin: "session" };
      session.persona_mode = "inherit";
      session.persona_id = null;
    }
  }
  const global = activeMockPersona();
  return global ? { persona: { ...global }, origin: "global" } : { persona: null, origin: "none" };
}

/** `{ id, name, icon }` for a chat job's params, or nothing when this chat
 *  runs without a persona -- the write-back `Persona::apply_to` does. */
function personaParams(sessionId: string | null): AnyRecord {
  const { persona } = resolvePersona(sessionId) as { persona: AnyRecord | null };
  if (!persona) return {};
  return { persona: { id: persona.id, name: persona.name, icon: persona.icon } };
}

const SESSIONS: AnyRecord[] = [
  mkSession("sess-chat-1", "chat", "General"),
  mkSession("sess-img-1", "image", "Anime pics"),
];

const DOCUMENTS: AnyRecord[] = [];

function mkVoiceIdentity(id: string, name: string, transcript: string): AnyRecord {
  return {
    id,
    name,
    reference_audio_path: `E:\\AI\\data\\voice-identities\\${id}\\reference.wav`,
    reference_transcript: transcript,
    created_at: now(),
  };
}

const VOICE_IDENTITIES: AnyRecord[] = [
  mkVoiceIdentity(
    "vi-gareth",
    "Old Man Gareth",
    "Well now, I've seen stranger things wash up on this shore, believe you me.",
  ),
];

// --- Story Studio (Phase 1) --------------------------------------------

const STORIES: AnyRecord[] = [
  {
    id: "story-dev-1",
    name: "The Salt Road",
    setting: "bronze-age Mediterranean coast",
    art_style: "linocut woodblock",
    premise: "a caravan carries something it shouldn't",
    created_at: now(),
  },
];

const CHARACTERS: AnyRecord[] = [
  {
    id: "char-dev-1", story_id: "story-dev-1", name: "Nera",
    traits: "sharp-eyed, stubborn", backstory: "grew up on the docks, trusts no one who hasn't earned it",
    alignment: "human, wary, resourceful", portrait_job_id: "j-img-1",
    inventory: ["rope", "dagger"], created_at: now(),
  },
  {
    id: "char-dev-2", story_id: "story-dev-1", name: "Kesh",
    traits: "gentle giant, slow to anger", backstory: "exiled from his clan for refusing a fight",
    alignment: "orc, friendly, strong", portrait_job_id: null,
    inventory: [], created_at: now(),
  },
];

const CHARACTER_RELATIONSHIPS: AnyRecord[] = [
  {
    id: "rel-dev-1", character_id: "char-dev-1", related_character_id: "char-dev-2",
    note: "trusts him completely, even when she won't admit it", created_at: now(),
  },
];

const CHARACTER_LOGS: AnyRecord[] = [
  { id: "log-dev-1", character_id: "char-dev-1", scene_id: "scene-dev-1", created_at: now(), text: "Appeared in a scene: a strange sail on the horizon" },
  { id: "log-dev-2", character_id: "char-dev-2", scene_id: "scene-dev-1", created_at: now(), text: "Appeared in a scene: a strange sail on the horizon" },
];

const NPCS: AnyRecord[] = [
  {
    id: "npc-dev-1", story_id: "story-dev-1", name: "Old Tom", role: "harbourmaster",
    location_id: "loc-dev-1", description: "knows every ship that's ever docked here",
    created_at: now(),
  },
];

const LOCATIONS: AnyRecord[] = [
  {
    id: "loc-dev-1", story_id: "story-dev-1", name: "The Salt Docks",
    description: "a sprawling harbor thick with brine and gossip",
    reference_job_id: "j-img-1", created_at: now(),
  },
];

const SCENES: AnyRecord[] = [
  {
    id: "scene-dev-1", story_id: "story-dev-1", location_id: "loc-dev-1",
    narrative: "Nera spots the sail before anyone else, and Kesh doesn't like what it means.",
    redline: "a strange sail on the horizon", position: 0, created_at: now(),
    participant_ids: ["char-dev-1", "char-dev-2"],
    dialogue: [
      { id: "dl-dev-1", scene_id: "scene-dev-1", character_id: "char-dev-1", position: 0, text: "There — see it?" },
      { id: "dl-dev-2", scene_id: "scene-dev-1", character_id: "char-dev-2", position: 1, text: "That's not one of ours." },
    ],
    images: [
      { id: "si-dev-1", scene_id: "scene-dev-1", job_id: "j-img-1", is_canonical: true, created_at: now() },
    ],
  },
];

const DATASET_FRAMES: AnyRecord[] = [];
const DATASETS: AnyRecord[] = [];
const CONCEPTS: AnyRecord[] = [];
/** `{ frame_id, concept_id }` pairs — the join table's mock stand-in. */
const FRAME_CONCEPTS: { frame_id: string; concept_id: string }[] = [];
const DATASET_TAGS = ["Ghibli", "Cyberpunk"];
const DATASET_MOCK_TOTAL_FRAMES = 18;

/** Clips mode produces one row per source video, not per still. These stand
 *  in for what `ffprobe` + the preview-still extraction would report: a mix of
 *  lengths, and one clip the pipeline could not read at all (`unusable` — no
 *  duration, no preview still, nothing to trim). */
const DATASET_MOCK_CLIPS: { file: string; duration: number | null; rejection: string }[] = [
  { file: "opening_pan.mp4", duration: 12.5, rejection: "" },
  { file: "market_walk.mp4", duration: 48, rejection: "" },
  { file: "rooftop_cut.mp4", duration: 3, rejection: "" },
  { file: "corrupt_take.mp4", duration: null, rejection: "unusable" },
  { file: "night_drive.mp4", duration: 96.4, rejection: "" },
  { file: "closing_shot.mp4", duration: 7.2, rejection: "" },
];

/** Loosely mirrors `capability::dataset::compose::COMMON_WORDS` — enough for
 *  the dev preview to show the inline "pick a made-up token" warning. */
const COMMON_TOKEN_WORDS = [
  "anime", "manga", "photo", "picture", "image", "art", "style", "character",
  "person", "man", "woman", "boy", "girl", "foot", "hand", "face", "city",
];

function mockTokenWarning(token: string): string | null {
  const t = String(token).trim();
  if (!t) return "Token is empty — use something like 'kenji_xy'.";
  if (/\s/.test(t)) {
    return `Token contains spaces — use one word, e.g. '${t.split(/\s+/).join("_").toLowerCase()}_xy'.`;
  }
  const lower = t.toLowerCase();
  return COMMON_TOKEN_WORDS.includes(lower)
    ? `'${t}' is an ordinary word the base model already knows — use a made-up token like '${lower}_xy'.`
    : null;
}

function mkDatasetFrame(id: string, jobId: string, tag: string, over: AnyRecord): AnyRecord {
  return {
    id,
    job_id: jobId,
    dataset_id: null,
    tag,
    source_path: `E:\\Data\\Demo\\${tag}\\clip.mp4`,
    frame_path: `E:\\Data\\Demo\\${tag}\\clip_0001.png`,
    timestamp_secs: 0,
    caption: "",
    caption_engine: "",
    excluded: false,
    rejection_reason: "",
    duration_secs: null,
    clip_start_secs: null,
    clip_end_secs: null,
    created_at: now(),
    ...over,
  };
}

/** The dataset row a running `dataset_prep` job produces, created on the
 *  job's first tick so the Dataset tab has something to select. */
function datasetForJob(job: AnyRecord): AnyRecord {
  const existing = DATASETS.find((d) => d.prep_job_id === job.id);
  if (existing) return existing;
  const params = (job.params ?? {}) as AnyRecord;
  const root = String(params.root ?? "E:\\Data\\Demo");
  const dataset: AnyRecord = {
    id: `ds-${String(job.id)}`,
    name: root.split(/[\\/]/).filter(Boolean).pop() ?? "Dataset",
    mode: params.mode === "clips" ? "clips" : "frames",
    source_root: root,
    trigger_word: "",
    prep_job_id: job.id,
    export_dir: null,
    created_at: now(),
  };
  DATASETS.push(dataset);
  return dataset;
}

/** Grows a running `dataset_prep` job's curation set by one frame per tick
 *  (mirrors how the real pipeline lands rows incrementally, per source, as
 *  ffmpeg/captioning works through the tree), then completes the job once
 *  the mock target is reached. No real image bytes exist here -- the
 *  curation grid's `<img>` tags will 404 in the dev preview, same caveat as
 *  every other job type's `output_path` (see the module doc). */
function progressDatasetJobs(): void {
  for (const j of JOBS) {
    if (j.job_type !== "dataset_prep" || j.state !== "running") continue;
    const jobId = String(j.id);
    const dataset = datasetForJob(j);
    const clips = dataset.mode === "clips";
    const target = clips ? DATASET_MOCK_CLIPS.length : DATASET_MOCK_TOTAL_FRAMES;
    const existing = DATASET_FRAMES.filter((f) => f.job_id === jobId);
    if (existing.length >= target) {
      j.state = "completed";
      j.finished_at = now();
      continue;
    }
    const idx = existing.length + 1;
    const tag = DATASET_TAGS[existing.length % DATASET_TAGS.length];

    if (clips) {
      const spec = DATASET_MOCK_CLIPS[existing.length];
      const unusable = spec.rejection === "unusable";
      DATASET_FRAMES.push(
        mkDatasetFrame(`${jobId}-c${idx}`, jobId, tag, {
          dataset_id: dataset.id,
          source_path: `E:\\Data\\Demo\\${tag}\\${spec.file}`,
          // No readable stream means no preview still was ever written.
          frame_path: unusable ? "" : `E:\\Data\\Demo\\${tag}\\${spec.file}.preview.png`,
          timestamp_secs: null,
          caption: unusable ? "" : `${tag} clip, dev-mock: ${spec.file}`,
          caption_engine: unusable ? "" : "florence2",
          rejection_reason: spec.rejection,
          duration_secs: spec.duration,
          clip_start_secs: null,
          clip_end_secs: null,
        }),
      );
      continue;
    }

    const escalated = idx % 6 === 0;
    // A slice of every run is auto-rejected, so the "verworfen" filter and the
    // "doch behalten" button have something to act on in the dev preview.
    const rejection = idx % 7 === 0 ? "blur" : idx % 11 === 0 ? "cap" : "";
    DATASET_FRAMES.push(
      mkDatasetFrame(`${jobId}-f${idx}`, jobId, tag, {
        dataset_id: dataset.id,
        source_path: `E:\\Data\\Demo\\${tag}\\clip.mp4`,
        frame_path: `E:\\Data\\Demo\\${tag}\\clip_${String(idx).padStart(4, "0")}.png`,
        timestamp_secs: idx * 0.7,
        caption: escalated
          ? `${tag} scene, dev-mock: the figure turns and walks toward the doorway`
          : `${tag} scene, dev-mock caption ${idx}`,
        caption_engine: escalated ? "qwen2.5-vl" : "florence2",
        rejection_reason: rejection,
        duration_secs: null,
      }),
    );
  }
}

// --- training orchestrator ----------------------------------------------

/** Steps one mock tick advances a running run by. Small enough that the
 *  progress bar visibly crawls, large enough to reach a checkpoint in a few
 *  polls of the 3 s `useTrainingRuns` interval. */
const TRAINING_STEPS_PER_TICK = 25;

/** Checkpoint (and sample) interval — the real `save_every`/`sample_every` of
 *  the balanced presets. */
const TRAINING_CHECKPOINT_EVERY = 250;

const TRAINING_RUNS: AnyRecord[] = [
  mkTrainingRun("tr-done", "Ghibli Look v1", {
    state: "completed",
    step: 1500,
    total_steps: 1500,
    last_loss: 0.0412,
    last_checkpoint_at: now(),
    result_model_id: "m-lora-flux-style",
    finished_at: now(),
  }),
  mkTrainingRun("tr-live", "Kenji Character v2", {
    state: "running",
    step: 325,
    total_steps: 1500,
    last_loss: 0.1183,
    last_checkpoint_at: now(),
    pid: 24680,
    started_at: now(),
  }),
];

function mkTrainingRun(id: string, name: string, over: AnyRecord): AnyRecord {
  return {
    id,
    name,
    profile_family: "flux2-klein-4b",
    target_model_id: "m-flux2-klein",
    dataset_id: DATASETS[0]?.id ?? null,
    data_kind: "frames",
    trigger_word: "ghibli_xy",
    preset: "balanced",
    hyperparams_json: "{}",
    sample_prompts_json: JSON.stringify([`${name} — a portrait, soft light`]),
    state: "preparing",
    step: 0,
    total_steps: 1500,
    last_loss: null,
    last_checkpoint_at: null,
    pid: null,
    work_dir: `E:\\AI\\data\\training\\${id}`,
    result_model_id: null,
    error_text: null,
    created_at: now(),
    started_at: null,
    finished_at: null,
    ...over,
  };
}

/** Advances every `running` mock run by one tick: steps climb, the loss
 *  decays with a little noise, a checkpoint lands every
 *  {@link TRAINING_CHECKPOINT_EVERY} steps, and reaching `total_steps`
 *  completes the run and imports a LoRA row the Image tab can then pick —
 *  the same end-to-end shape the real runner produces, minus the GPU. */
function progressTrainingRuns(): void {
  for (const r of TRAINING_RUNS) {
    if (r.state !== "running" && r.state !== "resuming") continue;
    r.state = "running";
    const total = Number(r.total_steps);
    const step = Math.min(total, Number(r.step) + TRAINING_STEPS_PER_TICK);
    const previous = Number(r.step);
    r.step = step;
    // A decaying loss with a touch of jitter -- the chart has to look alive
    // without ever suggesting real numbers.
    r.last_loss = Number((0.35 * Math.exp(-step / 600) + Math.random() * 0.01).toFixed(4));
    if (
      Math.floor(step / TRAINING_CHECKPOINT_EVERY) >
      Math.floor(previous / TRAINING_CHECKPOINT_EVERY)
    ) {
      r.last_checkpoint_at = now();
    }
    if (step < total) continue;

    r.state = "completed";
    r.finished_at = now();
    r.pid = null;
    r.last_checkpoint_at = now();
    const model = mkModel(`m-lora-${String(r.id)}`, `${String(r.name)} (LoRA)`, {
      family: "flux2",
      roles: ["lora"],
      runtimes: ["comfyui"],
      size_bytes: 168_000_000,
      source: `training:${String(r.id)}`,
    });
    MODELS.push(model);
    r.result_model_id = model.id;
  }
}

/** The four seeded profiles. Only the 4B base is staged, so the dev preview
 *  shows both halves of the "Basisgewichte fehlen" branch at once. */
const TRAINING_PROFILES: AnyRecord[] = [
  mkTrainingProfile("flux2-klein-4b", "FLUX.2 [klein] 4B", "flux2_klein_4b", {
    fit: "comfortable", fit_label: "fits comfortably", reserve_mb: 12288,
    base_repo: "black-forest-labs/FLUX.2-klein-base-4B",
    base_role: "training_base_flux2_klein_4b", base_approx_gb: 16, base_installed: true,
    license_note: "Apache-2.0 base weights",
    trainable_models: [{ id: "m-flux2-klein", name: "FLUX.2 [klein] 4B (dev-mock)", family: "flux2" }],
  }),
  mkTrainingProfile("flux2-klein-9b", "FLUX.2 [klein] 9B", "flux2_klein_9b", {
    fit: "at_the_edge", fit_label: "at the edge", reserve_mb: 15000,
    base_repo: "black-forest-labs/FLUX.2-klein-base-9B",
    base_role: "training_base_flux2_klein_9b", base_approx_gb: 30, base_installed: false,
    license_note: "FLUX non-commercial licence; needs fp8 + layer offloading on 16 GB",
  }),
  mkTrainingProfile("sdxl", "SDXL", "sdxl", {
    fit: "comfortable", fit_label: "fits comfortably", reserve_mb: 10240,
    base_repo: "stabilityai/stable-diffusion-xl-base-1.0",
    base_role: "training_base_sdxl", base_approx_gb: 7, base_installed: false,
    caption_order: "tags_first", license_note: "CreativeML Open RAIL++-M",
    trainable_models: [{ id: "m-sdxl", name: "SDXL Base 1.0", family: "sdxl" }],
  }),
  mkTrainingProfile("wan", "Wan 2.2 TI2V 5B", "wan22_5b", {
    data_kind: "both", fit: "at_the_edge", fit_label: "at the edge", reserve_mb: 15000,
    base_repo: "Wan-AI/Wan2.2-TI2V-5B-Diffusers",
    base_role: "training_base_wan22_5b", base_approx_gb: 20, base_installed: false,
    license_note: "Apache-2.0; the 16 GB setting is unverified until a real attempt",
    trainable_models: [{ id: "m-wan", name: "wan2.2_ti2v_5B_fp16", family: "wan" }],
  }),
];

function mkTrainingProfile(
  family: string,
  label: string,
  arch: string,
  over: AnyRecord,
): AnyRecord {
  return {
    family, label, arch, data_kind: "frames", fit: "comfortable",
    fit_label: "fits comfortably", reserve_mb: 12288, base_repo: "", base_role: "",
    base_required_files: ["model_index.json"], base_approx_gb: 16, base_installed: false,
    // The real core builds this from `training::bases`; the shape is what the
    // preflight panel shows and copies verbatim, so the mock spells out a
    // plausible one rather than leaving the copy block empty.
    base_download_command:
      `hf download ${String(over.base_repo ?? "")} ` +
      `--local-dir E:\\AI\\models\\training\\${family} --exclude "*.jpg"`,
    caption_order: "prose_first", license_note: "",
    presets: {
      fast: { steps: 600, lr: 1e-4, rank: 16, resolution: 768, save_every: 200, sample_every: 200 },
      balanced: { steps: 1500, lr: 1e-4, rank: 16, resolution: 1024, save_every: 250, sample_every: 250 },
      thorough: { steps: 3000, lr: 8e-5, rank: 32, resolution: 1024, save_every: 250, sample_every: 250 },
    },
    trainable_models: [],
    ...over,
  };
}

const RUNTIMES: AnyRecord[] = [
  {
    id: "llamacpp", kind: "llama_cpp", health: "healthy", vram_used_mb: 6400,
    detail: "serving Qwen2.5 7B Instruct on :8080",
    loaded_models: [{ model_id: "m-qwen", vram_mb: 6400 }],
  },
  {
    id: "comfyui", kind: "comfy_ui", health: "healthy", vram_used_mb: 0,
    detail: "running on :48096 · ComfyUI 0.34.0 · idle",
    loaded_models: [],
  },
  {
    id: "colibri", kind: "colibri", health: "unknown", vram_used_mb: 0,
    detail: "not installed",
    loaded_models: [],
  },
  {
    id: "tts", kind: "tts", health: "unknown", vram_used_mb: 0,
    detail: "not started yet",
    loaded_models: [],
  },
];

/** One example of each `ToolUpdateStatus` state, so the dev preview exercises
 *  every row style the Diagnostics card can render. */
const TOOL_VERSIONS: AnyRecord[] = [
  { id: "comfyui", current: "v0.34.0", status: { state: "up_to_date" } },
  { id: "llamacpp", current: "b10855", status: { state: "update_available", latest: "b10900" } },
  { id: "colibri", current: "v1.10.2", status: { state: "up_to_date" } },
  { id: "hermes", current: "0.19.0", status: { state: "update_available", latest: "0.20.0" } },
  { id: "opencode", current: null, status: { state: "unmanaged", latest: "v0.5.0" } },
];

const COLIBRI_MODELS: AnyRecord[] = [
  {
    id: "qwen3.6-35b-a3b-colibri", label: "Qwen3.6-35B-A3B (Colibri)",
    repo: "Kreuzzelg/qwen36-35b-a3b-colibri-i4-gs64", ram_estimate_mb: 24_576,
    disk_estimate_bytes: 20 * 1024 * 1024 * 1024, license: "Apache-2.0",
    note: "A 35B-parameter MoE (3B active) too large for GGUF/llama.cpp on a 16 GB card — runs CPU-only via Colibri instead. Needs ~24 GB RAM resident; tight on a 32 GB machine.",
  },
];

const AGENTS: AnyRecord[] = [
  {
    id: "a-coder", name: "Repo coder", adapter: "opencode", model_id: null,
    workspace_path: "E:\\AI", allowed_paths: [], toolset: null, created_at: now(),
  },
];

type DevSession = {
  id: string;
  agent_id: string;
  state: string;
  answered: boolean;
  stopped: boolean;
  firstMessage?: string | null;
};
let DEV_SESSION: DevSession | null = null;
let HERMES_INSTALLED = false;
let DEV_LAUNCH: AnyRecord | null = null;

const DOWNLOADS: AnyRecord[] = [];
const TAGS: Record<string, string[]> = { "m-qwen": ["coding", "favourite"] };
let HF_TOKEN = "";
let CIVITAI_TOKEN = "";
let LOCAL_API_TOKEN = "";

/** The built-in suites, mirroring `core/src/bench/suites.rs`. Ids, titles,
 *  descriptions, `max_tokens` and prompt titles are copied verbatim; the
 *  prompt *texts* are shortened here — the dev preview only ever previews
 *  them, and the real ones are several lines each. */
const BENCH_SUITES: AnyRecord[] = [
  {
    id: "chat-v1",
    title: "Chat (v1)",
    description:
      "Three everyday assistant prompts — explain, summarise, rewrite. Measures generation speed on prose, not answer quality.",
    max_tokens: 256,
    prompts: [
      { id: "chat-v1-explain", title: "Explain a concept", text: "Explain what a write-ahead log is, why databases keep one… (shortened in dev-mock)" },
      { id: "chat-v1-summarise", title: "Summarise a paragraph", text: "Summarise the paragraph below in two sentences… (shortened in dev-mock)" },
      { id: "chat-v1-rewrite", title: "Rewrite an email politely", text: "Rewrite the email below so that it stays firm about the deadline… (shortened in dev-mock)" },
    ],
  },
  {
    id: "coding-v1",
    title: "Coding (v1)",
    description:
      "Three programming prompts — write, debug, explain. Measures generation speed on code-shaped output; the code is never run or checked for correctness.",
    max_tokens: 384,
    prompts: [
      { id: "coding-v1-write", title: "Write a function from a spec", text: "Write a Rust function with the signature `fn merge_ranges(…)`… (shortened in dev-mock)" },
      { id: "coding-v1-fix", title: "Find and fix a bug", text: "The function below should return the index of the first element greater than `target`… (shortened in dev-mock)" },
      { id: "coding-v1-explain", title: "Explain code and propose tests", text: "Explain what the function below does and propose five test cases… (shortened in dev-mock)" },
    ],
  },
];

/** How long one fake generation pass "takes" — slow enough that the event
 *  trail is readable as it grows, quick enough to not stall the preview. */
const BENCH_PASS_MS = 900;
/** Extra time after the last pass, standing in for the scoring + store write. */
const BENCH_SETTLE_MS = 600;
/** `core::bench::BenchRequest::from_params` clamps the requested passes to
 *  this range; the API deliberately passes the raw number through, so the
 *  clamp belongs on the job side here too. */
const BENCH_RUNS_MIN = 1;
const BENCH_RUNS_MAX = 10;
/** `GET /benchmarks/history` defaults to 50 rows and clamps to 1..=200. */
const BENCH_HISTORY_LIMIT_DEFAULT = 50;
const BENCH_HISTORY_LIMIT_MAX = 200;

function benchSuite(id: string | null): AnyRecord | null {
  return BENCH_SUITES.find((s) => s.id === id) ?? null;
}

/** The passes-per-prompt a job actually runs: the params value clamped the way
 *  the core clamps it, with anything unparseable falling back to the default. */
function benchRuns(raw: unknown, fallback: number): number {
  const n = Number(raw ?? fallback);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(BENCH_RUNS_MAX, Math.max(BENCH_RUNS_MIN, Math.floor(n)));
}

/** The per-pass tok/s a bench job was born with — fixed at submit time so the
 *  event trail does not jitter between polls. */
function benchPassRates(j: AnyRecord): number[] {
  return (j.dev_pass_tps as number[] | undefined) ?? [];
}

/** One `{suite, prompt title, pass}` triple per element of `benchPassRates`,
 *  in the order `core::bench` runs them: every pass of a prompt, then the
 *  next prompt. */
function benchPassLabels(j: AnyRecord): { label: string | null; runs: number; pass: number; promptId: string | null }[] {
  const params = (j.params ?? {}) as AnyRecord;
  const suite = benchSuite(params.suite ? String(params.suite) : null);
  const runs = benchRuns(params.runs, suite ? 2 : 3);
  if (!suite) {
    return Array.from({ length: runs }, (_, i) => ({ label: null, runs, pass: i + 1, promptId: null }));
  }
  const prompts = suite.prompts as AnyRecord[];
  return prompts.flatMap((p) =>
    Array.from({ length: runs }, (_, i) => ({
      label: `${suite.id} · ${p.title}`,
      runs,
      pass: i + 1,
      promptId: String(p.id),
    })),
  );
}

/** The event trail `core::bench` would have appended by now — the opening
 *  line, one line per finished pass, and the closing score line. Wording and
 *  number formatting match `core/src/bench/mod.rs`. */
function benchEvents(j: AnyRecord): AnyRecord[] {
  const params = (j.params ?? {}) as AnyRecord;
  const suite = benchSuite(params.suite ? String(params.suite) : null);
  const model = MODELS.find((m) => m.id === j.model_id);
  const rates = benchPassRates(j);
  const labels = benchPassLabels(j);
  const runs = labels[0]?.runs ?? 3;
  const tokens = Number(suite?.max_tokens ?? 128);
  const events: AnyRecord[] = [
    {
      ts: j.started_at ?? j.created_at,
      level: "info",
      message: suite
        ? `benchmarking “${model?.name ?? j.model_id}” with suite ${suite.id} — ${(suite.prompts as AnyRecord[]).length} prompt(s) × ${runs} run(s), ${tokens} tokens each`
        : `benchmarking “${model?.name ?? j.model_id}” — ${runs} run(s), ${tokens} tokens each`,
    },
  ];
  const startedAt = Date.parse(String(j.started_at ?? j.created_at));
  const elapsed = Date.now() - startedAt;
  const done = Math.min(rates.length, Math.floor(elapsed / BENCH_PASS_MS));
  for (let i = 0; i < done; i++) {
    const step = labels[i];
    const tps = rates[i];
    events.push({
      // When that pass *finished*, not when this poll happened — a wall-clock
      // `now()` here would make every already-logged line's timestamp jump on
      // every poll, which is not how an append-only event trail behaves.
      ts: new Date(startedAt + (i + 1) * BENCH_PASS_MS).toISOString(),
      level: "info",
      message: step.label
        ? `${step.label} — pass ${step.pass}/${step.runs}: ${tps.toFixed(1)} tok/s`
        : `run ${step.pass}/${step.runs}: ${tps.toFixed(1)} tok/s generation, ${Math.round(tps * 6)} tok/s prompt`,
    });
  }
  const row = BENCHMARKS.find((b) => b.job_id === j.id);
  if (row) {
    events.push({
      ts: j.finished_at,
      level: "info",
      message: `score ${row.overall_score} — ${Number(row.gen_tps).toFixed(1)} tok/s gen, ${Math.round(Number(row.prompt_tps))} tok/s prompt, stability ${Number(row.stability_score).toFixed(2)}`,
    });
  }
  return events;
}

/** Flip a running `bench` job to `completed` once its fake passes have all
 *  "run", and drop a benchmark row — the dev-mock stand-in for `core::bench`.
 *  A suite run carries the per-prompt `detail` the real one records. */
function progressBenchJobs(): void {
  for (const j of JOBS) {
    if (j.job_type !== "bench" || j.state !== "running") continue;
    const rates = benchPassRates(j);
    const started = Date.parse(String(j.started_at ?? j.created_at));
    if (Date.now() - started < rates.length * BENCH_PASS_MS + BENCH_SETTLE_MS) continue;
    j.state = "completed";
    j.finished_at = now();
    const m = MODELS.find((x) => x.id === j.model_id);
    const labels = benchPassLabels(j);
    const suiteId = ((j.params ?? {}) as AnyRecord).suite;
    const suite = benchSuite(suiteId ? String(suiteId) : null);
    const gen = rates.reduce((a, b) => a + b, 0) / Math.max(1, rates.length);
    BENCHMARKS.unshift({
      id: `bench-${seq++}`,
      model_id: j.model_id,
      job_id: j.id,
      kind: "llm",
      runs: rates.length,
      prompt_tps: 260 + Math.round(Math.random() * 220),
      gen_tps: Math.round(gen * 10) / 10,
      load_ms: 1500 + Math.round(Math.random() * 2200),
      vram_peak_mb: Number(m?.vram_estimate_mb ?? 6000),
      ram_peak_mb: 14000 + Math.round(Math.random() * 2000),
      stability_score: 0.9 + Math.random() * 0.09,
      overall_score: Math.min(100, Math.round(gen * 1.15)),
      notes: "dev-mock",
      suite: suite ? String(suite.id) : null,
      detail: suite ? benchDetail(labels, rates, Number(suite.max_tokens ?? 256)) : null,
      created_at: now(),
    });
  }
}

/** Collapse the per-pass rates into one `PromptResult` per prompt, the way
 *  `core::bench` averages a suite's passes. */
function benchDetail(
  labels: { promptId: string | null }[],
  rates: number[],
  maxTokens: number,
): AnyRecord[] {
  const byPrompt = new Map<string, number[]>();
  labels.forEach((step, i) => {
    if (!step.promptId || rates[i] == null) return;
    byPrompt.set(step.promptId, [...(byPrompt.get(step.promptId) ?? []), rates[i]]);
  });
  return [...byPrompt].map(([prompt_id, tps]) => ({
    prompt_id,
    tokens: maxTokens,
    max_tokens: maxTokens,
    gen_tps: Math.round((tps.reduce((a, b) => a + b, 0) / tps.length) * 10) / 10,
    prompt_tps: 300 + Math.round(Math.random() * 180),
  }));
}

function mkBenchmark(
  modelId: string,
  suiteId: string | null,
  gen: number,
  hoursAgo: number,
): AnyRecord {
  const suite = benchSuite(suiteId);
  const prompts = (suite?.prompts as AnyRecord[] | undefined) ?? [];
  return {
    id: `bench-seed-${modelId}-${suiteId ?? "quick"}`,
    model_id: modelId,
    job_id: null,
    kind: "llm",
    runs: suite ? prompts.length * 2 : 3,
    prompt_tps: 380,
    gen_tps: gen,
    load_ms: 2100,
    vram_peak_mb: Number(MODELS.find((m) => m.id === modelId)?.vram_estimate_mb ?? 6000),
    ram_peak_mb: 15200,
    stability_score: 0.94,
    overall_score: Math.min(100, Math.round(gen * 1.15)),
    notes: "dev-mock",
    suite: suiteId,
    detail: suite
      ? prompts.map((p, i) => ({
          prompt_id: String(p.id),
          tokens: Number(suite?.max_tokens ?? 256),
          max_tokens: Number(suite?.max_tokens ?? 256),
          gen_tps: Math.round((gen + (i - 1) * 2.4) * 10) / 10,
          prompt_tps: 360 + i * 25,
        }))
      : null,
    created_at: new Date(Date.now() - hoursAgo * 3_600_000).toISOString(),
  };
}

/** Seeded history so the Benchmark tab has something to compare on first
 *  paint: both suites across the two GGUF chat models, plus one suite-less
 *  quick test. Newest first, like `list_all`. */
const BENCHMARKS: AnyRecord[] = [
  mkBenchmark("m-hermes", "coding-v1", 52.4, 2),
  mkBenchmark("m-qwen", "coding-v1", 61.8, 5),
  mkBenchmark("m-hermes", "chat-v1", 57.1, 26),
  mkBenchmark("m-qwen", "chat-v1", 66.3, 29),
  mkBenchmark("m-qwen", null, 64.9, 50),
];

/** Flip a running `chat`/`colibri` job to `completed` with a canned reply a
 *  moment in -- the dev-mock stand-in for a real llama.cpp answer. Recognizes
 *  the Prompt Assistant's own transcript format (see `buildTranscriptPrompt`)
 *  and answers with a PROMPT:/NEGATIVE: suggestion so that flow can be
 *  eyeballed here too, not just a real chat message. */
function progressChatJobs(): void {
  for (const j of JOBS) {
    if (!["chat", "colibri"].includes(String(j.job_type)) || j.state !== "running") continue;
    const started = Date.parse(String(j.started_at ?? j.created_at));
    const elapsed = Date.now() - started;
    const prompt = String((j.params as AnyRecord)?.prompt ?? "");
    const isEditAssistant = /want an existing photo edited/i.test(prompt);
    const isAssistant = isEditAssistant || /prompt-writing assistant/i.test(prompt);
    if (elapsed < 1200) {
      j.result = isAssistant ? "Thinking about a good prompt…" : "…";
      continue;
    }
    j.state = "completed";
    j.finished_at = now();
    // A real run prepends the persona's system prompt and the answer changes
    // shape; nothing here talks to a model, so the persona's *name* is put in
    // front of the canned reply instead -- enough to see in the preview that
    // the resolution actually reached the job.
    // Assistant jobs need no exception here: `submit_job` never stamps a
    // persona onto them in the first place, just like the core.
    const persona = (j.params as AnyRecord)?.persona as AnyRecord | undefined;
    const as = persona ? `[as ${String(persona.name)}] ` : "";
    j.result = isEditAssistant
      ? "Got it — that's a clear edit.\n\nPROMPT: give the subject blonde hair, keep everything else the same"
      : isAssistant
        ? "Got it — that's enough to work with.\n\nPROMPT: a moody portrait of an old lighthouse keeper, dramatic side lighting, weathered skin, oil painting texture\nNEGATIVE: blurry, cartoon, low detail"
        : `${as}This is a mocked reply — dev-mock has no real model attached.`;
  }
}

/** Flip a running `tts` job to `completed` a moment in -- images/video have
 *  no equivalent here either (dev-mock never had a reason to finish those),
 *  but Voice's "completed" UI (Result panel, History rows, Clean audio) is
 *  otherwise impossible to eyeball in the dev preview at all. */
function progressTtsJobs(): void {
  for (const j of JOBS) {
    if (j.job_type !== "tts" || j.state !== "running") continue;
    const started = Date.parse(String(j.started_at ?? j.created_at));
    if (Date.now() - started < 900) continue;
    j.state = "completed";
    j.finished_at = now();
    j.output_path = `/dev-mock/${j.id}.wav`;
  }
}

/** Flip a running `upscale` job to `completed` a moment in -- mirrors
 *  `progressTtsJobs`: without this, Image/Video's "Upscale" button has no way
 *  to ever finish in the dev preview. Output extension follows the source
 *  job's own kind (an image source produces a `.png`, a video source a
 *  `.mp4`) so the Result panel's `<img>`/`<video>` tag picks the right one. */
function progressUpscaleJobs(): void {
  for (const j of JOBS) {
    if (j.job_type !== "upscale" || j.state !== "running") continue;
    const started = Date.parse(String(j.started_at ?? j.created_at));
    if (Date.now() - started < 900) continue;
    const source = String((j.params as AnyRecord)?.source ?? "");
    const sourceJob = JOBS.find((s) => s.id === source);
    const ext = sourceJob?.job_type === "video" ? "mp4" : "png";
    j.state = "completed";
    j.finished_at = now();
    j.output_path = `/dev-mock/${j.id}.${ext}`;
  }
}

/** Flip a running `recommend` job to `completed` with a canned report. */
function progressRecommendJobs(): void {
  for (const j of JOBS) {
    if (j.job_type !== "recommend" || j.state !== "running") continue;
    const started = Date.parse(String(j.started_at ?? j.created_at));
    if (Date.now() - started < 1200) continue;
    const query = String((j.params as AnyRecord)?.query ?? "");
    j.state = "completed";
    j.finished_at = now();
    j.result = JSON.stringify({
      query,
      note: "Ranked by your local model against what you described. Quality and content claims (e.g. “uncensored”) are the publisher's own — not locally verified.",
      freshness: { kind: "live" },
      candidates: [
        {
          id: "ArliAI/GLM-4.6-Derestricted-v3",
          why: "tagged uncensored and roleplay, matches the ask, active community",
          downloads: 18_442,
          likes: 214,
          last_modified: "2026-05-02",
          param_count: 106_000_000_000,
          format: "gguf",
          gated: false,
          tags: ["uncensored", "roleplay", "not-for-all-audiences"],
          fit: { level: "yellow", reason: "needs offloading to system RAM — tight on 16 GB" },
          llm_ranked: true,
        },
        {
          id: "SicariusSicariiStuff/Assistant_Pepe_32B",
          why: "smaller, uncensored, fits comfortably",
          downloads: 6_120,
          likes: 91,
          last_modified: "2026-03-11",
          param_count: 32_000_000_000,
          format: "gguf",
          gated: false,
          tags: ["uncensored", "chat"],
          fit: { level: "green" },
          llm_ranked: true,
        },
      ],
    });
  }
}

/** Flip a running `upgrade_check` job to `completed` with a canned report. */
function progressUpgradeJobs(): void {
  for (const j of JOBS) {
    if (j.job_type !== "upgrade_check" || j.state !== "running") continue;
    const started = Date.parse(String(j.started_at ?? j.created_at));
    if (Date.now() - started < 3000) continue;
    const m = MODELS.find((x) => x.id === (j.params as AnyRecord)?.target_model_id);
    j.state = "completed";
    j.finished_at = now();
    j.result = JSON.stringify({
      target: String(m?.name ?? "model"),
      query: String(m?.family ?? "qwen2"),
      note: "Ranked by your local model over the objective signals (release date, downloads, size, fit). Quality is not locally verifiable.",
      freshness: { kind: "live" },
      candidates: [
        {
          id: "Qwen/Qwen2.5-Coder-14B-Instruct-GGUF",
          why: "same family, ~2× the parameters, still fits your 16 GB budget, 400k downloads",
          downloads: 402113,
          likes: 88,
          last_modified: "2026-04-18",
          param_count: 14_770_000_000,
          format: "gguf",
          gated: false,
          fit: { level: "yellow", reason: "needs ~9 GB — little head-room" },
          installed: false,
          llm_ranked: true,
        },
        {
          id: "bartowski/Qwen2.5-7B-Instruct-GGUF",
          why: "same size, newer quant set, more actively maintained",
          downloads: 1_780_676,
          likes: 436,
          last_modified: "2026-05-02",
          param_count: 7_615_616_512,
          format: "gguf",
          gated: false,
          fit: { level: "green" },
          installed: false,
          llm_ranked: true,
        },
      ],
    });
  }
}

const DISCOVER_MODELS: AnyRecord[] = [
  {
    id: "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF", author: "Qwen", downloads: 1_780_676, likes: 436,
    trending_score: 12, created_at: now(), last_modified: now(), pipeline_tag: "text-generation",
    library_name: "transformers", gated: "no", license: "apache-2.0",
    base_model: "Qwen/Qwen2.5-Coder-7B-Instruct", tags: ["gguf", "code", "text-generation-inference"],
    param_count: 7_615_616_512, arch: "qwen2", ctx_max: 131_072, precision: null, format: "gguf",
  },
  {
    id: "bartowski/Qwen2.5-Coder-14B-Instruct-GGUF", author: "bartowski", downloads: 402_113, likes: 88,
    trending_score: 5, created_at: now(), last_modified: now(), pipeline_tag: "text-generation",
    library_name: null, gated: "no", license: "apache-2.0",
    base_model: "Qwen/Qwen2.5-Coder-14B-Instruct", tags: ["gguf"],
    param_count: 14_770_000_000, arch: "qwen2", ctx_max: 131_072, precision: null, format: "gguf",
  },
  {
    id: "ArliAI/GLM-4.6-Derestricted-v3", author: "ArliAI", downloads: 18_442, likes: 214,
    trending_score: 9, created_at: now(), last_modified: now(), pipeline_tag: "text-generation",
    library_name: null, gated: "no", license: "mit",
    base_model: "zai-org/GLM-4.6", tags: ["gguf", "uncensored", "roleplay", "not-for-all-audiences"],
    param_count: 106_000_000_000, arch: "glm4", ctx_max: 131_072, precision: null, format: "gguf",
  },
];

/** Civitai's own field shape (verified against a real, unauthenticated
 *  request — see `registry::civitai`'s module doc). One clean checkpoint,
 *  one LoRA, one flagged with a non-"Success" pickle scan (the "don't hide
 *  it" fixture), one NSFW-tagged model that the default search excludes. */
const CIVITAI_DISCOVER_MODELS: AnyRecord[] = [
  {
    id: "257749", name: "Pony Diffusion V6 XL", author: "AstraliteHeart",
    downloads: 1_200_000, likes: 34_000, trending_score: null, created_at: null,
    last_modified: "2023-07-18T00:00:00.000Z", pipeline_tag: null, library_name: null,
    gated: "no", license: null, base_model: null,
    tags: ["western art", "base model", "anime"], param_count: null, arch: null, ctx_max: null,
    precision: null, format: "safetensors", nsfw: false,
    preview_image_url: "https://placehold.co/144x144/2a2540/e8e3ff?text=Pony+V6",
    allow_commercial_use: ["Image", "RentCivit"], model_kind_hint: "Checkpoint",
    base_model_family: "Pony, SD 1.5",
  },
  {
    id: "99263", name: "Add More Detail (detail enhancer LoRA)", author: "Lykon",
    downloads: 500_000, likes: 9_000, trending_score: null, created_at: null,
    last_modified: "2023-08-07T00:00:00.000Z", pipeline_tag: null, library_name: null,
    gated: "no", license: null, base_model: null,
    tags: ["detailed", "enhancer"], param_count: null, arch: null, ctx_max: null,
    precision: null, format: "safetensors", nsfw: false,
    preview_image_url: "https://placehold.co/144x144/1f2a24/d8ffe8?text=Detail",
    allow_commercial_use: ["Image"], model_kind_hint: "LORA", base_model_family: "SD 1.5",
  },
  {
    id: "424242", name: "Suspicious Upload", author: "rando",
    downloads: 12, likes: 0, trending_score: null, created_at: null,
    last_modified: "2024-01-01T00:00:00.000Z", pipeline_tag: null, library_name: null,
    gated: "no", license: null, base_model: null,
    tags: [], param_count: null, arch: null, ctx_max: null,
    precision: null, format: "safetensors", nsfw: false, preview_image_url: null,
    allow_commercial_use: [], model_kind_hint: "Checkpoint", base_model_family: "SD 1.5",
  },
  {
    id: "777777", name: "NSFW Only Model", author: "rando2",
    downloads: 5, likes: 0, trending_score: null, created_at: null,
    last_modified: "2024-01-01T00:00:00.000Z", pipeline_tag: null, library_name: null,
    gated: "no", license: null, base_model: null,
    tags: ["nsfw"], param_count: null, arch: null, ctx_max: null,
    precision: null, format: "safetensors", nsfw: true, preview_image_url: null,
    allow_commercial_use: [], model_kind_hint: "Checkpoint", base_model_family: "SD 1.5",
  },
];

const CIVITAI_FILES: Record<string, AnyRecord[]> = {
  "257749": [{
    path: "ponyDiffusionV6XL_v6StartWithThisOne.safetensors", size_bytes: 6_775_982_592,
    sha256: "67ab2fd8ec439a89b3fedb15cc65f54336af163c7eb5e4f2acc98f090a29b0b3", quant: "F16",
    shard: null, download_url: "https://civitai.com/api/download/models/290640",
    vram_estimate_mb: 8200, fit: { level: "green" },
    pickle_scan_result: "Success", virus_scan_result: "Success",
  }],
  "99263": [{
    path: "add-detail-xl.safetensors", size_bytes: 228_452_331,
    sha256: "0d9bd1b873a7863e128b4672e3e245838858f71469a3cec58123c16c06f83bd7", quant: null,
    shard: null, download_url: "https://civitai.com/api/download/models/135867",
    vram_estimate_mb: null, fit: { level: "unknown" },
    pickle_scan_result: "Success", virus_scan_result: "Success",
  }],
  "424242": [{
    path: "suspicious.safetensors", size_bytes: 102_400_000,
    sha256: "aa".repeat(32), quant: null,
    shard: null, download_url: "https://civitai.com/api/download/models/424242",
    vram_estimate_mb: null, fit: { level: "unknown" },
    pickle_scan_result: "Danger", virus_scan_result: "Success",
  }],
};

function devSession(): AnyRecord {
  const s = DEV_SESSION!;
  return {
    id: s.id, agent_id: s.agent_id, adapter_session_id: "ses_dev", state: s.state,
    error_text: null, checkpoint_path: null, started_at: now(),
    ended_at: s.stopped ? now() : null,
  };
}

function devEvents(): AnyRecord[] {
  const s = DEV_SESSION!;
  const events: AnyRecord[] = [
    { ts: now(), kind: "text", payload: { type: "text", text: "I'll read the README to get oriented." } },
    { ts: now(), kind: "tool", payload: { type: "tool", id: "c1", name: "bash", status: s.answered ? "done" : "running", input: { command: "cat README.md" }, output: s.answered ? "AI Workstation Manager — a local control plane…\n" : null } },
  ];
  if (!s.answered) {
    events.push({ ts: now(), kind: "permission", payload: { type: "permission", id: "per_1", kind: "bash", summary: "cat README.md", always_pattern: "cat *" } });
  } else {
    events.push({ ts: now(), kind: "text", payload: { type: "text", text: "The README says it's an offline-first orchestrator for local AI runtimes." } });
    events.push({ ts: now(), kind: "idle", payload: { type: "idle" } });
  }
  return events;
}

const ABOUT: AnyRecord = {
  core_version: "0.0.1-dev", data_dir: "E:\\AI\\data", store_path: "E:\\AI\\models",
  outputs_dir: "E:\\AI\\data\\outputs", outputs_bytes: 4_812_300_000,
  runtimes_dir: "E:\\AI\\data\\runtimes", cache_dir: "E:\\AI\\data\\cache",
  core_api_port: 48096, vram_budget_mb: 14848, offline_mode: false,
};

const CONFIG: AnyRecord = {
  store_path: "E:\\AI\\models", core_api_port: 48096, offline_mode: false, log_filter: "info",
  vram_budget_mb: 0,
  llama: {
    gpu_layers: 999, ctx_size: 0, flash_attention: true,
    jinja: true, chat_template: "", load_timeout_secs: 180,
  },
  comfyui: { vram_mode: "auto", reserve_vram_mb: 0, extra_args: "" },
  models: { auto_preference: "balanced" },
  paths: { outputs_path: null, runtimes_path: null, cache_path: null },
  retention: { max_age_days: 0, max_total_mb: 0 },
};

const KNOWN_MOCK: AnyRecord[] = [
  {
    id: "sdxl-base-1.0", name: "Stable Diffusion XL 1.0 (base)", kind: "checkpoint",
    family: "sdxl", publisher: "Stability AI", repo: "stabilityai/stable-diffusion-xl-base-1.0",
    file: "sd_xl_base_1.0.safetensors",
    url: "https://huggingface.co/stabilityai/stable-diffusion-xl-base-1.0/resolve/main/sd_xl_base_1.0.safetensors",
    sha256: "3".repeat(64), size_bytes: 6_938_078_334,
    license: "CreativeML Open RAIL++-M (commercial use allowed)",
    note: "The default. Fits comfortably in 16 GB, huge LoRA/ControlNet ecosystem.",
    is_default: true, media: "image", fit: { level: "green" },
  },
  {
    id: "flux1-dev-q8", name: "FLUX.1-dev — Q8_0 (GGUF)", kind: "diffusion_model",
    family: "flux", publisher: "Black Forest Labs / city96", repo: "city96/FLUX.1-dev-gguf",
    file: "flux1-dev-Q8_0.gguf",
    url: "https://huggingface.co/city96/FLUX.1-dev-gguf/resolve/main/flux1-dev-Q8_0.gguf",
    sha256: "1".repeat(64), size_bytes: 12_708_281_504,
    license: "FLUX.1 [dev] Non-Commercial License",
    note: "Best prompt fidelity + in-image text. Needs the T5, CLIP-L and VAE below.",
    is_default: false, media: "image",
    fit: { level: "yellow", reason: "needs ~13.4 GB of your ~14.5 GB VRAM budget — little head-room for a longer context or a second resident model" },
  },
  {
    id: "t5xxl-fp8", name: "T5-XXL — fp8 (Flux text encoder)", kind: "text_encoder",
    family: null, publisher: "Comfy-Org", repo: "comfyanonymous/flux_text_encoders",
    file: "t5xxl_fp8_e4m3fn.safetensors",
    url: "https://huggingface.co/comfyanonymous/flux_text_encoders/resolve/main/t5xxl_fp8_e4m3fn.safetensors",
    sha256: "5".repeat(64), size_bytes: 4_893_934_904,
    license: "Apache-2.0",
    note: "Flux prompt encoder. ComfyUI offloads it after encoding, so it is not resident during sampling.",
    is_default: false, media: "image", fit: { level: "green" },
  },
  {
    id: "clip-l", name: "CLIP-L (Flux text encoder)", kind: "text_encoder",
    family: null, publisher: "Comfy-Org", repo: "comfyanonymous/flux_text_encoders",
    file: "clip_l.safetensors",
    url: "https://huggingface.co/comfyanonymous/flux_text_encoders/resolve/main/clip_l.safetensors",
    sha256: "6".repeat(64), size_bytes: 246_144_152,
    license: "MIT", note: "The second Flux encoder. Small.",
    is_default: false, media: "image", fit: { level: "green" },
  },
  {
    id: "flux-vae", name: "FLUX.1 VAE (ae)", kind: "vae",
    family: "flux", publisher: "Black Forest Labs", repo: "second-state/FLUX.1-dev-GGUF",
    file: "ae.safetensors",
    url: "https://huggingface.co/second-state/FLUX.1-dev-GGUF/resolve/main/ae.safetensors",
    sha256: "8".repeat(64), size_bytes: 335_304_388,
    license: "FLUX.1 [dev] Non-Commercial License",
    note: "The Flux autoencoder. Byte-identical across every Flux re-upload.",
    is_default: false, media: "image", fit: { level: "green" },
  },
  {
    id: "wan22-ti2v-5b", name: "Wan 2.2 TI2V-5B — fp16 (video, default)", kind: "video",
    family: "wan", publisher: "Alibaba / Comfy-Org", repo: "Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
    file: "wan2.2_ti2v_5B_fp16.safetensors",
    url: "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/diffusion_models/wan2.2_ti2v_5B_fp16.safetensors",
    sha256: "4".repeat(64), size_bytes: 9_999_658_848,
    license: "Apache-2.0 (commercial use allowed)",
    note: "The default video model. One model for text→video and image→video. Needs the umt5 encoder and the Wan VAE.",
    is_default: true, media: "video", fit: { level: "green" },
  },
  {
    id: "wan-umt5-xxl-fp8", name: "umt5-XXL — fp8 (Wan text encoder)", kind: "text_encoder",
    family: null, publisher: "Comfy-Org", repo: "Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
    file: "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
    url: "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors",
    sha256: "9".repeat(64), size_bytes: 6_735_906_897,
    license: "Apache-2.0",
    note: "Wan's prompt encoder (multilingual T5). Offloaded to the CPU during sampling.",
    is_default: false, media: "video", fit: { level: "green" },
  },
  {
    id: "wan22-vae", name: "Wan 2.2 VAE", kind: "vae",
    family: "wan", publisher: "Alibaba / Comfy-Org", repo: "Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
    file: "wan2.2_vae.safetensors",
    url: "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged/resolve/main/split_files/vae/wan2.2_vae.safetensors",
    sha256: "a".repeat(64), size_bytes: 1_409_400_960,
    license: "Apache-2.0", note: "The Wan 2.2 autoencoder. Import as VAE.",
    is_default: false, media: "video", fit: { level: "green" },
  },
  {
    id: "ltx-video-2b-095", name: "LTX-Video 2B v0.9.5 (video, second template)", kind: "video",
    family: "ltx", publisher: "Lightricks", repo: "Lightricks/LTX-Video",
    file: "ltx-video-2b-v0.9.5.safetensors",
    url: "https://huggingface.co/Lightricks/LTX-Video/resolve/main/ltx-video-2b-v0.9.5.safetensors",
    sha256: "7".repeat(64), size_bytes: 6_340_729_500,
    license: "LTXV License (OpenRAIL-M-style; check the repo for commercial terms)",
    note: "Fast, light. One .safetensors carries model + VAE — only needs a t5xxl encoder.",
    is_default: false, media: "video", fit: { level: "green" },
  },
];

/** Mirrors `core::model::MODEL_STACKS` — base model first, then companions. */
const STACK_MEMBER_IDS: Record<string, string[]> = {
  sdxl: ["sdxl-base-1.0"],
  flux: ["flux1-dev-q8", "t5xxl-fp8", "clip-l", "flux-vae"],
  wan22: ["wan22-ti2v-5b", "wan-umt5-xxl-fp8", "wan22-vae"],
  ltx: ["ltx-video-2b-095", "t5xxl-fp8"],
};
const STACK_META: Record<string, AnyRecord> = {
  sdxl: {
    label: "Stable Diffusion XL", media: "image", is_default: true,
    note: "One file — the checkpoint carries its own VAE and text encoder.",
    fit: { level: "green" },
  },
  flux: {
    label: "FLUX.1-dev", media: "image", is_default: false,
    note: "Best prompt fidelity + in-image text. Four files: the diffusion model plus its T5 and CLIP-L text encoders and its VAE.",
    // All four members individually look green/yellow, but a real render
    // needs them all resident at once (~17 GB combined) — the whole-stack
    // verdict is what actually answers "can I run this", and on a 16 GB
    // card it's red even though no single file looks that bad alone.
    fit: { level: "red", reason: "needs ~17 GB combined (model + T5 + CLIP-L + VAE) of your ~16 GB VRAM budget" },
  },
  wan22: {
    label: "Wan 2.2 TI2V-5B", media: "video", is_default: true,
    note: "The default video setup. Three files: the model, its umt5 text encoder, and its VAE.",
    fit: { level: "green" },
  },
  ltx: {
    label: "LTX-Video 0.9.5 (2B)", media: "video", is_default: false,
    note: "Fast and light. Two files: model + VAE bundled in one, plus a shared T5 text encoder.",
    fit: { level: "green" },
  },
};
const STACKS_MOCK: AnyRecord[] = Object.entries(STACK_MEMBER_IDS).map(([id, memberIds]) => ({
  id,
  ...STACK_META[id],
  members: memberIds.map((mid) => KNOWN_MOCK.find((m) => m.id === mid)),
}));

const FEATURED_MOCK: AnyRecord[] = [
  {
    id: "qwen2.5-7b-instruct", role: "chat", label: "Qwen2.5-7B-Instruct",
    repo: "bartowski/Qwen2.5-7B-Instruct-GGUF", quant_hint: "Q5_K_M", typical_vram_mb: 8704,
    license: "Apache-2.0", note: "Strong general-purpose chat model with native tool-calling.",
    import_roles: ["chat"], is_default: true, fit: { level: "green" },
  },
  {
    id: "qwen2.5-14b-instruct", role: "chat", label: "Qwen2.5-14B-Instruct",
    repo: "bartowski/Qwen2.5-14B-Instruct-GGUF", quant_hint: "Q4_K_M", typical_vram_mb: 12800,
    license: "Apache-2.0", note: "Bigger and sharper if you can spare the VRAM — fills most of a 16 GB card.",
    import_roles: ["chat"], is_default: false,
    fit: { level: "yellow", reason: "needs ~12.5 GB of your ~14.5 GB VRAM budget — little head-room for a longer context or a second resident model" },
  },
  {
    id: "qwen2.5-coder-7b-instruct", role: "coding", label: "Qwen2.5-Coder-7B-Instruct",
    repo: "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF", quant_hint: "Q5_K_M", typical_vram_mb: 8704,
    license: "Apache-2.0", note: "Best tool-calling/VRAM ratio for agent sessions (OpenCode) on 16 GB.",
    import_roles: ["chat", "coding"], is_default: true, fit: { level: "green" },
  },
  {
    id: "hermes-3-llama-3.1-8b", role: "coding", label: "Hermes-3-Llama-3.1-8B",
    repo: "NousResearch/Hermes-3-Llama-3.1-8B-GGUF", quant_hint: "Q5_K_M", typical_vram_mb: 9216,
    license: "Llama-3.1 Community License",
    note: "Nous Research's own agent-tuned model — the natural pick for the Hermes runtime.",
    import_roles: ["chat", "coding"], is_default: false, fit: { level: "green" },
  },
];

const TELEMETRY: AnyRecord = {
  captured_at_ms: Date.now(),
  gpu: { state: "available", name: "NVIDIA RTX 4080 SUPER", vram_total_mb: 16376, vram_used_mb: 2100, vram_free_mb: 14276, utilization_pct: 3, temperature_c: 41, processes: [] },
  host: { ram_total_mb: 32000, ram_used_mb: 14200, cpu_total_pct: 8, cpu_per_core_pct: [] },
};

let seq = 100;

export function installDevMock(): void {
  mockIPC(async (cmd, args): Promise<unknown> => {
    const a = (args ?? {}) as AnyRecord;
    switch (cmd) {
      case "about":
        return ABOUT;
      case "get_telemetry":
        return TELEMETRY;
      case "get_runtimes":
        return RUNTIMES;
      case "check_tool_versions":
        return TOOL_VERSIONS;
      case "list_models":
        // Fresh array — `usePolled` needs a changed reference to re-render
        // (e.g. after delete_model splices MODELS).
        return MODELS.map((m) => ({ ...m }));
      case "list_known_models":
        return KNOWN_MOCK;
      case "list_model_stacks":
        return STACKS_MOCK;
      case "list_featured_models":
        return FEATURED_MOCK;
      case "list_colibri_models":
        return COLIBRI_MODELS;
      case "register_colibri_model": {
        const body = (a.body ?? {}) as AnyRecord;
        const catalog = COLIBRI_MODELS.find((m) => m.id === body.catalog_id);
        const model = mkModel(`m-colibri-${seq++}`, String(catalog?.label ?? "Colibri model"), {
          format: "colibri", file_path: String(body.dir ?? ""),
          size_bytes: Number(catalog?.disk_estimate_bytes ?? 0),
          ram_estimate_mb: Number(catalog?.ram_estimate_mb ?? 0),
          roles: ["chat"],
        });
        MODELS.push(model);
        return model;
      }
      case "install_colibri": {
        const rt = RUNTIMES.find((r) => r.id === "colibri");
        if (rt) rt.detail = "installed · idle";
        return "started";
      }
      case "list_jobs":
        progressBenchJobs();
        progressUpgradeJobs();
        progressChatJobs();
        progressRecommendJobs();
        progressTtsJobs();
        progressUpscaleJobs();
        progressDatasetJobs();
        // Fresh array — `usePolled` needs a changed reference to re-render.
        return JOBS.map((j) => ({ ...j }));
      case "get_config":
        return CONFIG;
      case "save_config": {
        const u = (a.update ?? {}) as AnyRecord;
        Object.assign(CONFIG, u);
        return CONFIG;
      }
      case "get_settings":
        return {};
      case "get_recent_logs":
        return ["dev-mock: no real logs"];
      case "job_detail": {
        const job = JOBS.find((j) => j.id === a.id);
        if (!job) return null;
        if (job.job_type === "bench") {
          progressBenchJobs();
          return { job, events: benchEvents(job) };
        }
        const message =
          job.job_type === "dataset_prep"
            ? job.state === "completed"
              ? "dataset ready"
              : `found 2 tag folder(s), 2 source file(s) — captioning frames`
            : "rendering 832×480 video, 81 frames @ 24 fps (~3.4s), 30 steps, cfg 5, seed 4212981 — wan2.2_ti2v_5B_fp16";
        return { job, events: [{ ts: now(), level: "info", message }] };
      }
      case "submit_job": {
        const body = (a.body ?? {}) as AnyRecord;
        const jobType = String(body.job_type ?? "video");
        // Auto's real per-role pick isn't mocked -- just default sanely per
        // job type instead of always falling back to a video model.
        const autoModel: Record<string, string> = {
          video: "m-wan",
          image: "m-sdxl",
          chat: "m-qwen",
          colibri: "m-qwen",
          recommend: "m-qwen",
          tts: "m-kokoro",
          // No library model backs this (a custom-node install, not a
          // checkpoint) -- matches the Rust engine's own synthetic id so it
          // reads sensibly wherever a raw model_id falls back to display.
          upscale: "rtx-video-super-resolution",
        };
        const sessionId = (body.session_id as string) ?? null;
        // The core resolves the persona when the job starts and writes
        // `persona: {id,name,icon}` back over its params; the mock has no
        // separate start, so it happens here. Prompt Assistant completions
        // (`assistant_for`) are skipped, exactly as `capability::chat` skips
        // them -- their reply is parsed for PROMPT:/NEGATIVE: markers, so no
        // persona may reach them.
        const isAssistantJob = !!(body.params as AnyRecord | undefined)?.assistant_for;
        const persona =
          (jobType === "chat" || jobType === "colibri") && !isAssistantJob
            ? personaParams(sessionId)
            : {};
        const job = mkJob(`j-dev-${seq++}`, jobType, "running", {
          params: {
            ...resolveJobParams(jobType, (body.params ?? {}) as AnyRecord),
            ...persona,
          },
          model_id: (body.model_id as string) ?? autoModel[jobType] ?? "m-wan",
          runtime_id: (body.runtime_id as string) ?? "comfyui",
          output_path: null,
          session_id: sessionId,
        });
        JOBS.unshift(job);
        return job;
      }
      case "cancel_job": {
        const job = JOBS.find((j) => j.id === a.id);
        if (!job) return null;
        if (!["queued", "scheduled", "blocked", "preparing", "running"].includes(String(job.state))) {
          return false;
        }
        job.state = "cancelled";
        job.finished_at = now();
        return true;
      }
      case "delete_job": {
        const i = JOBS.findIndex((j) => j.id === a.id);
        if (i < 0) throw new Error(`no such job ${a.id}`);
        const terminal = ["completed", "failed", "cancelled"];
        if (!terminal.includes(String(JOBS[i].state))) {
          throw new Error("this job is still running — cancel it first");
        }
        JOBS.splice(i, 1);
        return null;
      }
      case "save_job_output":
        // No real filesystem in the browser dev preview -- the save dialog
        // itself already resolves to `null` here (mocked "plugin:" command),
        // so `downloadJobOutput` never actually calls this; kept as a no-op
        // for completeness / future test harnesses that do mock the dialog.
        return null;
      case "clean_audio": {
        const job = JOBS.find((j) => j.id === a.id);
        if (!job) throw new Error(`no such job ${a.id}`);
        if (job.job_type !== "tts") throw new Error("only narration clips can be cleaned");
        return 2.5;
      }
      case "list_sessions": {
        const capability = String(a.capability ?? "");
        return SESSIONS.filter((s) => s.capability === capability).map((s) => ({ ...s }));
      }
      case "create_session": {
        const body = (a.body ?? {}) as AnyRecord;
        const session = mkSession(
          `sess-dev-${seq++}`,
          String(body.capability ?? "chat"),
          String(body.name ?? "Untitled"),
        );
        SESSIONS.unshift(session);
        return session;
      }
      case "rename_session": {
        const s = SESSIONS.find((x) => x.id === a.id);
        if (s) s.name = String(a.name ?? s.name);
        return null;
      }
      case "set_session_archived": {
        const s = SESSIONS.find((x) => x.id === a.id);
        if (s) s.archived_at = a.archived ? now() : null;
        return null;
      }
      case "delete_session": {
        const i = SESSIONS.findIndex((x) => x.id === a.id);
        if (i >= 0) SESSIONS.splice(i, 1);
        return null;
      }
      case "list_personas":
        // Fresh objects -- `usePolled` needs a changed reference to re-render.
        return PERSONAS.map((p) => ({ ...p }));
      case "create_persona": {
        const fields = validatePersona((a.body ?? {}) as AnyRecord);
        const persona = { id: `persona-dev-${seq++}`, ...fields, created_at: now(), updated_at: now() };
        PERSONAS.push(persona);
        return { ...persona };
      }
      case "update_persona": {
        // Validation first, like the core: a malformed body is a 400 whether
        // or not the id happens to exist.
        const fields = validatePersona((a.body ?? {}) as AnyRecord);
        const persona = PERSONAS.find((p) => p.id === a.id);
        if (!persona) return null;
        Object.assign(persona, fields, { updated_at: now() });
        return { ...persona };
      }
      case "delete_persona": {
        const i = PERSONAS.findIndex((p) => p.id === a.id);
        if (i < 0) return false;
        PERSONAS.splice(i, 1);
        // Same self-healing as the core's delete transaction: chats that used
        // it go back to inheriting, and the global default is cleared if it
        // was this one.
        for (const s of SESSIONS) {
          if (s.persona_id === a.id) {
            s.persona_mode = "inherit";
            s.persona_id = null;
          }
        }
        if (activePersonaId === a.id) activePersonaId = null;
        return true;
      }
      case "active_persona":
        // Reading heals a dangling id, exactly as `persona::active` does.
        return { id: activeMockPersona()?.id ?? null };
      case "set_active_persona": {
        // `null` is the one spelling of "clear"; an empty string names no
        // persona and is refused like any other unknown id.
        const id = (a.id as string | null | undefined) ?? null;
        if (id !== null && !PERSONAS.some((p) => p.id === id)) return false;
        activePersonaId = id;
        return true;
      }
      case "set_session_persona": {
        const body = (a.body ?? {}) as AnyRecord;
        const mode = String(body.mode ?? "");
        if (!["inherit", "none", "persona"].includes(mode)) {
          // The real core rejects this while deserializing the body; the
          // wording differs, the 400 does not.
          throw new Error(`unknown persona mode "${mode}"`);
        }
        if (mode === "persona" && body.persona_id == null) {
          throw new Error('persona mode "persona" needs a persona_id');
        }
        // The persona is checked before the session, same as the core.
        const personaId = mode === "persona" ? String(body.persona_id) : null;
        if (personaId !== null && !PERSONAS.some((p) => p.id === personaId)) {
          return "unknown_persona";
        }
        const session = SESSIONS.find((s) => s.id === a.id);
        if (!session) return "unknown_session";
        if (personaId !== null) {
          session.persona_mode = "persona";
          session.persona_id = personaId;
        } else {
          session.persona_mode = mode;
          session.persona_id = null;
        }
        return "stored";
      }
      case "effective_persona":
        return resolvePersona((a.sessionId as string | null) ?? null);
      case "list_documents":
        return DOCUMENTS.filter((d) => d.session_id === a.sessionId).map((d) => ({ ...d }));
      case "attach_document": {
        const path = String(a.path ?? "");
        const name = path.split(/[\\/]/).pop() ?? path;
        const ext = name.includes(".") ? (name.split(".").pop() ?? "").toLowerCase() : "";
        if (!["txt", "md"].includes(ext)) {
          throw new Error(`only .txt and .md documents are supported right now — got ${path}`);
        }
        const doc = {
          id: `doc-dev-${seq++}`,
          session_id: String(a.sessionId ?? ""),
          name,
          source_path: path,
          format: ext,
          created_at: now(),
        };
        DOCUMENTS.push(doc);
        return doc;
      }
      case "delete_document": {
        const i = DOCUMENTS.findIndex((d) => d.id === a.id);
        if (i >= 0) DOCUMENTS.splice(i, 1);
        return null;
      }
      case "list_voice_identities":
        return VOICE_IDENTITIES.map((v) => ({ ...v }));
      case "create_voice_identity": {
        const body = (a.body ?? {}) as AnyRecord;
        const name = String(body.name ?? "").trim();
        const transcript = String(body.reference_transcript ?? "").trim();
        if (!name) throw new Error("voice identity name must not be empty");
        if (!transcript) throw new Error("reference transcript must not be empty");
        const identity = mkVoiceIdentity(`vi-dev-${seq++}`, name, transcript);
        VOICE_IDENTITIES.unshift(identity);
        return identity;
      }
      case "delete_voice_identity": {
        const i = VOICE_IDENTITIES.findIndex((v) => v.id === a.id);
        if (i >= 0) VOICE_IDENTITIES.splice(i, 1);
        return null;
      }
      case "cleanup_outputs": {
        // Stand-in for `cleanup::outputs::sweep` -- no real filesystem to
        // scan here, so just report a plausible result reflecting whether a
        // policy is actually configured (mirrors the real no-op-until-saved
        // behavior for the "clean up now" button).
        const retention = (CONFIG.retention ?? {}) as AnyRecord;
        const active = Number(retention.max_age_days ?? 0) > 0 || Number(retention.max_total_mb ?? 0) > 0;
        return active
          ? { deleted_files: 2, freed_bytes: 734_003_200, errors: [] }
          : { deleted_files: 0, freed_bytes: 0, errors: [] };
      }
      case "list_dataset_frames":
        progressDatasetJobs();
        return DATASET_FRAMES.filter((f) => f.job_id === a.jobId).map((f) => ({ ...f }));
      case "update_dataset_frame": {
        const frame = DATASET_FRAMES.find((f) => f.id === a.frameId);
        if (!frame) throw new Error(`no such dataset frame ${a.frameId}`);
        const body = (a.body ?? {}) as AnyRecord;
        if (typeof body.excluded === "boolean") frame.excluded = body.excluded;
        if (typeof body.caption === "string") {
          frame.caption = body.caption;
          frame.caption_engine = "";
        }
        if (body.restore === true) frame.rejection_reason = "";
        // Three-valued, like the Rust DTO: an absent key keeps the stored
        // bound, an explicit `null` clears it.
        if ("clip_start_secs" in body) frame.clip_start_secs = body.clip_start_secs ?? null;
        if ("clip_end_secs" in body) frame.clip_end_secs = body.clip_end_secs ?? null;
        return { ...frame };
      }
      case "export_dataset": {
        const kept = DATASET_FRAMES.filter((f) => f.job_id === a.jobId && !f.excluded);
        if (kept.length === 0) {
          throw new Error("nothing to export — every frame is excluded from this dataset");
        }
        return { exported: kept.length, dest_dir: String(a.destDir ?? "") };
      }
      case "list_captioners":
        return [
          {
            id: "florence2", name: "Florence-2 (prose)", style: "prose", role: "vision_florence2",
            vram_mb: 2600, license: "MIT", supports_escalation: true, required_files: [],
            installed: false,
          },
          {
            id: "wd-eva02-tagger-v3", name: "WD EVA02 Tagger v3 (Danbooru tags)", style: "tags",
            role: "vision_wd_tagger", vram_mb: 0, license: "Apache-2.0",
            supports_escalation: false, required_files: ["model.onnx", "selected_tags.csv"],
            installed: true,
          },
        ];
      case "list_datasets":
        progressDatasetJobs();
        return DATASETS.map((d) => ({ ...d }));
      case "get_dataset": {
        const dataset = DATASETS.find((d) => d.id === a.id);
        return dataset ? { ...dataset } : null;
      }
      case "update_dataset": {
        const dataset = DATASETS.find((d) => d.id === a.id);
        if (!dataset) throw new Error(`no such dataset ${a.id}`);
        const body = (a.body ?? {}) as AnyRecord;
        if (typeof body.trigger_word === "string") dataset.trigger_word = body.trigger_word.trim();
        return { ...dataset };
      }
      case "delete_dataset": {
        const dropped = CONCEPTS.filter((c) => c.dataset_id === a.id).map((c) => c.id);
        removeWhere(FRAME_CONCEPTS, (fc) => dropped.includes(fc.concept_id));
        removeWhere(CONCEPTS, (c) => c.dataset_id === a.id);
        removeWhere(DATASET_FRAMES, (f) => f.dataset_id === a.id);
        removeWhere(DATASETS, (d) => d.id === a.id);
        return null;
      }
      case "list_dataset_frames_for_dataset":
        progressDatasetJobs();
        return DATASET_FRAMES.filter((f) => f.dataset_id === a.datasetId).map((f) => ({ ...f }));
      case "frame_concept_map": {
        const ids = CONCEPTS.filter((c) => c.dataset_id === a.datasetId).map((c) => c.id);
        const map: Record<string, string[]> = {};
        for (const fc of FRAME_CONCEPTS) {
          if (!ids.includes(fc.concept_id)) continue;
          (map[fc.frame_id] ??= []).push(fc.concept_id);
        }
        return map;
      }
      case "list_concepts":
        return CONCEPTS.filter((c) => c.dataset_id === a.datasetId).map((c) => ({
          ...c,
          frame_count: FRAME_CONCEPTS.filter((fc) => fc.concept_id === c.id).length,
          token_warning: mockTokenWarning(String(c.token)),
        }));
      case "create_concept": {
        const body = (a.body ?? {}) as AnyRecord;
        const name = String(body.name ?? "").trim();
        const token = String(body.token ?? "").trim();
        if (!name || !token) throw new Error("concept name and token must not be empty");
        const clash = CONCEPTS.some((c) => c.dataset_id === a.datasetId && c.token === token);
        if (clash) {
          throw new Error(`token "${token}" is already used by another concept in this dataset`);
        }
        const concept: AnyRecord = {
          id: `concept-${seq++}`,
          dataset_id: a.datasetId,
          name,
          token,
          description: String(body.description ?? "").trim(),
          created_at: now(),
        };
        CONCEPTS.push(concept);
        // The real command answers with the stored row only -- `frame_count`
        // and `token_warning` come from `list_concepts`.
        return { ...concept };
      }
      case "update_concept": {
        const concept = CONCEPTS.find((c) => c.id === a.id);
        if (!concept) throw new Error(`no such concept ${a.id}`);
        const body = (a.body ?? {}) as AnyRecord;
        const name = String(body.name ?? "").trim();
        const token = String(body.token ?? "").trim();
        if (!name || !token) throw new Error("concept name and token must not be empty");
        const clash = CONCEPTS.some(
          (c) => c.dataset_id === concept.dataset_id && c.token === token && c.id !== concept.id,
        );
        if (clash) {
          throw new Error(`token "${token}" is already used by another concept in this dataset`);
        }
        concept.name = name;
        concept.token = token;
        concept.description = String(body.description ?? "").trim();
        return null;
      }
      case "delete_concept": {
        removeWhere(FRAME_CONCEPTS, (fc) => fc.concept_id === a.id);
        removeWhere(CONCEPTS, (c) => c.id === a.id);
        return null;
      }
      case "assign_concept": {
        const conceptId = String(a.conceptId);
        const concept = CONCEPTS.find((c) => c.id === conceptId);
        const body = (a.body ?? {}) as AnyRecord;
        const frameIds = (body.frame_ids as string[]) ?? [];
        let attached = 0;
        for (const frameId of frameIds) {
          const frame = DATASET_FRAMES.find((f) => f.id === frameId);
          // Same guard as the real INSERT ... WHERE EXISTS: a frame from
          // another dataset is skipped, not an error.
          if (!concept || !frame || frame.dataset_id !== concept.dataset_id) continue;
          if (FRAME_CONCEPTS.some((fc) => fc.frame_id === frameId && fc.concept_id === conceptId)) {
            continue;
          }
          FRAME_CONCEPTS.push({ frame_id: frameId, concept_id: conceptId });
          attached += 1;
        }
        return { requested: frameIds.length, attached };
      }
      case "unassign_concept": {
        const body = (a.body ?? {}) as AnyRecord;
        const frameIds = (body.frame_ids as string[]) ?? [];
        removeWhere(
          FRAME_CONCEPTS,
          (fc) => fc.concept_id === a.conceptId && frameIds.includes(fc.frame_id),
        );
        return null;
      }
      case "export_dataset_by_id": {
        const body = (a.body ?? {}) as AnyRecord;
        const kept = DATASET_FRAMES.filter(
          (f) => f.dataset_id === a.datasetId && !f.excluded && !f.rejection_reason,
        );
        if (kept.length === 0) {
          throw new Error("nothing to export — every item is excluded from this dataset");
        }
        const destDir = String(body.dest_dir ?? "");
        const dataset = DATASETS.find((d) => d.id === a.datasetId);
        if (dataset) dataset.export_dir = destDir;
        return { exported: kept.length, dest_dir: destDir };
      }
      // --- training orchestrator ---
      case "training_status":
        return {
          installed: true, installing: false, env_broken: false,
          install_state: { state: "idle" },
          detail: "installed · idle",
          alive_run_id: TRAINING_RUNS.find((r) => r.state === "running")?.id ?? null,
        };
      case "install_trainer":
        return "already_installed";
      case "probe_trainer":
        return { torch_version: "2.13.0+cu130", cuda: true, vram_total_mb: 16376 };
      case "list_training_profiles":
        return TRAINING_PROFILES.map((p) => ({ ...p }));
      case "list_training_runs":
        progressTrainingRuns();
        // Fresh array — `usePolled` needs a changed reference to re-render.
        return TRAINING_RUNS.map((r) => ({ ...r }));
      case "start_training_run": {
        const body = (a.body ?? {}) as AnyRecord;
        const prompts = (body.sample_prompts as string[]) ?? [];
        const run = mkTrainingRun(`tr-${seq++}`, String(body.name ?? "Training run"), {
          profile_family: "flux2-klein-4b",
          target_model_id: String(body.target_model_id ?? ""),
          dataset_id: String(body.dataset_id ?? ""),
          trigger_word: String(body.trigger_word ?? ""),
          preset: String(body.preset ?? "balanced"),
          hyperparams_json: JSON.stringify(body.hyperparams ?? {}),
          sample_prompts_json: JSON.stringify(prompts),
          state: "running",
          total_steps: 1500,
          pid: 30000 + seq,
          started_at: now(),
        });
        TRAINING_RUNS.unshift(run);
        return { ...run };
      }
      case "get_training_run": {
        progressTrainingRuns();
        const run = TRAINING_RUNS.find((r) => r.id === a.id);
        if (!run) return null;
        // No real bytes exist here, so the preview `<img>` tags 404 in the
        // dev preview -- same caveat as every `output_path` (see module doc).
        const samples = run.last_checkpoint_at ? ["0", "1"] : [];
        return {
          run: { ...run },
          latest_samples: samples,
          log_tail: [
            `${String(run.name)}: dev-mock, no real trainer attached`,
            `${String(run.name)}:  ${String(run.step)}/${String(run.total_steps)} [02:14<11:03, 1.84it/s]`,
          ],
          work_dir: String(run.work_dir),
        };
      }
      case "pause_training_run":
      case "resume_training_run":
      case "cancel_training_run": {
        const run = TRAINING_RUNS.find((r) => r.id === a.id);
        if (!run) throw new Error(`no such training run ${String(a.id)}`);
        run.state =
          cmd === "pause_training_run"
            ? "paused"
            : cmd === "resume_training_run"
              ? "running"
              : "cancelled";
        if (cmd === "cancel_training_run") {
          run.finished_at = now();
          run.pid = null;
        }
        return { ...run };
      }
      case "delete_training_run": {
        const run = TRAINING_RUNS.find((r) => r.id === a.id);
        if (!run) throw new Error(`no such training run ${String(a.id)}`);
        if (!["completed", "failed", "cancelled"].includes(String(run.state))) {
          throw new Error(`"${String(run.name)}" is still ${String(run.state)} — cancel it first`);
        }
        removeWhere(TRAINING_RUNS, (r) => r.id === a.id);
        return null;
      }
      /** Mock-only: flip a running run to `interrupted` so the "Fortsetzen"
       *  branch can be checked in the browser preview. There is no process to
       *  kill here, which is the only reason this command exists. */
      case "__mock_interrupt_training_run": {
        const run = TRAINING_RUNS.find((r) => r.id === a.id);
        if (!run) throw new Error(`no such training run ${String(a.id)}`);
        run.state = "interrupted";
        run.pid = null;
        return { ...run };
      }
      case "storage_report": {
        const kindOf = (m: AnyRecord): string => {
          const roles = (m.roles as string[]) ?? [];
          if (roles.includes("chat") || roles.includes("coding")) return "llm";
          if (roles.includes("base_video")) return "video";
          return "image";
        };
        const byKind: AnyRecord = {};
        let store = 0;
        for (const m of MODELS) {
          const size = Number(m.size_bytes);
          store += size;
          const k = kindOf(m);
          const e = (byKind[k] as AnyRecord) ?? { kind: k, bytes: 0, count: 0 };
          e.bytes = Number(e.bytes) + size;
          e.count = Number(e.count) + 1;
          byKind[k] = e;
        }
        return {
          store_bytes: store,
          volume_free_bytes: 1_496_000_000_000,
          volume_total_bytes: 2_000_000_000_000,
          by_kind: Object.values(byKind),
          models: MODELS.map((m) => ({
            id: m.id, name: m.name, kind: kindOf(m), size_bytes: Number(m.size_bytes),
            last_used_at: m.last_used_at ?? null, use_count: Number(m.use_count ?? 0),
            roles: m.roles ?? [], file_present: true,
          })),
          duplicates:
            MODELS.length >= 2
              ? [
                  {
                    sha256: "dead00beef00cafe00".repeat(2).slice(0, 64),
                    member_ids: [MODELS[0].id, MODELS[1].id],
                    wasted_bytes: Number(MODELS[1].size_bytes),
                  },
                ]
              : [],
          unused: MODELS.filter((m) => !m.last_used_at).map((m) => m.id),
          stale_days: 45,
        };
      }
      case "delete_model": {
        const i = MODELS.findIndex((m) => m.id === a.id);
        if (i < 0) throw new Error(`model ${a.id} is not in the library`);
        const [m] = MODELS.splice(i, 1);
        delete TAGS[String(m.id)];
        return { id: m.id, name: m.name, file_removed: true, freed_bytes: Number(m.size_bytes) };
      }
      case "model_tags":
        return { ...TAGS };
      case "set_model_tags": {
        if (!MODELS.some((m) => m.id === a.id)) throw new Error(`model ${a.id} is not in the library`);
        const clean = [
          ...new Set(
            ((a.tags as string[]) ?? [])
              .map((t) => t.trim().toLowerCase())
              .filter((t) => t && t.length <= 32),
          ),
        ].sort();
        if (clean.length) TAGS[String(a.id)] = clean;
        else delete TAGS[String(a.id)];
        return clean;
      }
      case "set_model_roles": {
        const m = MODELS.find((x) => x.id === a.id);
        if (!m) throw new Error(`model ${a.id} is not in the library`);
        const clean = [
          ...new Set(((a.roles as string[]) ?? []).map((r) => r.trim()).filter(Boolean)),
        ].sort();
        m.roles = clean;
        return clean;
      }
      case "rename_model": {
        const m = MODELS.find((x) => x.id === a.id);
        if (!m) throw new Error(`model ${a.id} is not in the library`);
        const name = String(a.name ?? "").trim();
        if (!name) throw new Error("model name must not be empty");
        m.name = name;
        return { ...m };
      }
      case "registry_status":
        return {
          source_id: "huggingface",
          last_fetch: now(),
          rate_limit_remaining: 471,
          rate_limited_secs: null,
          token_set: HF_TOKEN.length > 0,
          cache_entries: 3,
        };
      case "set_hf_token":
        HF_TOKEN = String(a.token ?? "").trim();
        return null;
      case "civitai_status":
        return {
          source_id: "civitai",
          last_fetch: now(),
          rate_limit_remaining: null,
          rate_limited_secs: null,
          token_set: CIVITAI_TOKEN.length > 0,
          cache_entries: 0,
        };
      case "set_civitai_token":
        CIVITAI_TOKEN = String(a.token ?? "").trim();
        return null;
      case "local_api_status":
        return { endpoint: "http://127.0.0.1:48096/v1", token_set: LOCAL_API_TOKEN.length > 0 };
      case "set_local_api_token":
        LOCAL_API_TOKEN = String(a.token ?? "").trim();
        return null;
      case "external_engines":
        return [{ label: "Ollama", port: 11434, models: ["llama3.1:8b", "qwen2.5:7b"] }];
      case "attach_external_engine": {
        const body = (a.body ?? {}) as AnyRecord;
        const rt = RUNTIMES.find((r) => r.id === "llamacpp");
        if (rt) {
          rt.detail = `attached to ${body.model_id} on :${body.port}`;
          rt.vram_used_mb = body.vram_mb;
          rt.loaded_models = [{ model_id: body.model_id, vram_mb: body.vram_mb }];
        }
        return null;
      }
      case "detach_engine": {
        const rt = RUNTIMES.find((r) => r.id === "llamacpp");
        if (rt) {
          rt.detail = "installed · idle";
          rt.vram_used_mb = 0;
          rt.loaded_models = [];
        }
        return null;
      }
      case "unload_model": {
        const rt = RUNTIMES.find((r) =>
          ((r.loaded_models as AnyRecord[] | undefined) ?? []).some((m) => m.model_id === a.id),
        );
        if (!rt) throw new Error(`model ${a.id} is not currently loaded`);
        rt.detail = "installed · idle";
        rt.vram_used_mb = 0;
        rt.loaded_models = [];
        return null;
      }
      case "list_benchmarks":
        progressBenchJobs();
        return [...new Map(BENCHMARKS.map((b) => [b.model_id, b])).values()];
      case "model_benchmarks":
        progressBenchJobs();
        return BENCHMARKS.filter((b) => b.model_id === a.id);
      case "bench_suites":
        // Fresh copies: the catalogue is static on the core side, and a caller
        // that mutated a suite here would corrupt every later read.
        return BENCH_SUITES.map((s) => ({
          ...s,
          prompts: (s.prompts as AnyRecord[]).map((p) => ({ ...p })),
        }));
      case "benchmark_history": {
        progressBenchJobs();
        const suite = a.suite == null ? null : String(a.suite);
        if (suite && !benchSuite(suite)) throw new Error(`unknown benchmark suite "${suite}"`);
        const asked = Number(a.limit ?? BENCH_HISTORY_LIMIT_DEFAULT);
        const limit = Number.isFinite(asked)
          ? Math.min(BENCH_HISTORY_LIMIT_MAX, Math.max(1, asked))
          : BENCH_HISTORY_LIMIT_DEFAULT;
        return BENCHMARKS.filter((b) => suite == null || b.suite === suite).slice(0, limit);
      }
      case "benchmark_model": {
        const opts = (a.body ?? {}) as AnyRecord;
        const suiteId = opts.suite == null ? null : String(opts.suite);
        if (suiteId && !benchSuite(suiteId)) throw new Error(`unknown benchmark suite "${suiteId}"`);
        const model = MODELS.find((m) => m.id === a.id);
        const params: AnyRecord = { vram_needed_mb: Number(model?.vram_estimate_mb ?? 6000) };
        if (suiteId) params.suite = suiteId;
        if (opts.runs != null) params.runs = Number(opts.runs);
        const job = mkJob(`j-bench-${seq++}`, "bench", "running", {
          model_id: String(a.id),
          runtime_id: "llamacpp",
          params,
          started_at: now(),
        });
        // Fixed per-pass rates, drawn once so the growing event trail stays
        // stable across polls (the real numbers come from llama.cpp).
        const passes = benchPassLabels(job).length;
        job.dev_pass_tps = Array.from(
          { length: passes },
          () => Math.round((38 + Math.random() * 34) * 10) / 10,
        );
        JOBS.unshift(job);
        return job;
      }
      case "upgrade_check": {
        const job = mkJob(`j-up-${seq++}`, "upgrade_check", "running", {
          params: { target_model_id: String(a.id) },
          started_at: now(),
        });
        JOBS.unshift(job);
        return job;
      }
      case "list_agents":
        return AGENTS;
      case "list_agent_runtimes":
        return [
          { id: "opencode", installed: true },
          { id: "hermes", installed: HERMES_INSTALLED, install: { state: "idle" } },
        ];
      case "install_hermes":
        HERMES_INSTALLED = true;
        return "started";
      case "registry_search": {
        const q = String((a.params as AnyRecord)?.q ?? "").toLowerCase();
        const all = DISCOVER_MODELS.filter(
          (m) => !q || String(m.id).toLowerCase().includes(q),
        );
        return { data: all, freshness: { kind: "live" } };
      }
      case "registry_model": {
        const id = String(a.id ?? "");
        const m = DISCOVER_MODELS.find((x) => x.id === id) ?? DISCOVER_MODELS[0];
        const mid = String(m.id);
        const repo = mid.split("/")[1].toLowerCase();
        return {
          ...m,
          revision: "main",
          freshness: { kind: "live" },
          files: [
            {
              path: "README.md", size_bytes: 4200, sha256: null, quant: null,
              shard: null, download_url: `https://huggingface.co/${mid}/resolve/main/README.md`,
              vram_estimate_mb: null, fit: { level: "unknown" },
            },
            {
              path: `${repo}-q4_k_m.gguf`, size_bytes: 4_683_073_536,
              sha256: "509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c",
              quant: "Q4_K_M", shard: null,
              download_url: `https://huggingface.co/${mid}/resolve/main/model-q4_k_m.gguf`,
              vram_estimate_mb: 5_800, fit: { level: "green" },
            },
            {
              path: `${repo}-q5_k_m.gguf`, size_bytes: 5_444_832_000,
              sha256: "bb".repeat(32), quant: "Q5_K_M", shard: null,
              download_url: `https://huggingface.co/${mid}/resolve/main/model-q5_k_m.gguf`,
              vram_estimate_mb: 6_900, fit: { level: "green" },
            },
            {
              path: `${repo}-q8_0.gguf`, size_bytes: 8_100_000_000,
              sha256: "aa".repeat(32), quant: "Q8_0", shard: null,
              download_url: `https://huggingface.co/${mid}/resolve/main/model-q8_0.gguf`,
              vram_estimate_mb: 9_200,
              fit: { level: "yellow", reason: "needs ~9.0 GB of your ~14.8 GB VRAM budget — little head-room for a longer context or a second resident model" },
            },
            // No size estimate from the source -> the core can't judge it:
            // the `unknown` tier, so the preview shows all four.
            {
              path: `${repo}-q6_k.gguf`, size_bytes: 6_300_000_000,
              sha256: "cc".repeat(32), quant: "Q6_K", shard: null,
              download_url: `https://huggingface.co/${mid}/resolve/main/model-q6_k.gguf`,
              vram_estimate_mb: null, fit: { level: "unknown" },
            },
            {
              path: `${repo}-f16.gguf`, size_bytes: 15_240_000_000,
              sha256: "dd".repeat(32), quant: "F16", shard: null,
              download_url: `https://huggingface.co/${mid}/resolve/main/model-f16.gguf`,
              vram_estimate_mb: 17_600,
              fit: { level: "red", reason: "needs ~17.2 GB of your ~14.8 GB VRAM budget — it would have to offload most layers to system RAM" },
            },
          ],
        };
      }
      case "civitai_search": {
        const p = (a.params as AnyRecord) ?? {};
        const q = String(p.q ?? "").toLowerCase();
        const nsfw = Boolean(p.nsfw);
        const types = String(p.types ?? "")
          .split(",")
          .map((t) => t.trim())
          .filter(Boolean);
        const all = CIVITAI_DISCOVER_MODELS.filter((m) => {
          if (q && !String(m.name).toLowerCase().includes(q)) return false;
          // Mirrors the real server's own default-safe NSFW filter.
          if (!nsfw && m.nsfw) return false;
          if (types.length > 0 && !types.includes(String(m.model_kind_hint))) return false;
          return true;
        });
        return { data: all, freshness: { kind: "live" } };
      }
      case "civitai_model": {
        const id = String(a.id ?? "");
        const m = CIVITAI_DISCOVER_MODELS.find((x) => x.id === id) ?? CIVITAI_DISCOVER_MODELS[0];
        return {
          ...m,
          revision: id,
          freshness: { kind: "live" },
          files: CIVITAI_FILES[String(m.id)] ?? [],
        };
      }
      case "list_downloads":
        // Nudge any running download forward so the progress bar moves.
        for (const d of DOWNLOADS) {
          if (d.state === "running") {
            const size = Number(d.size_bytes);
            const next = Math.min(size, Number(d.bytes_done) + 6e8);
            d.bytes_done = next;
            if (next >= size) {
              d.state = "done";
              d.model_id = `m-dl-${seq++}`;
            }
          }
        }
        // Fresh array + fresh rows every poll — the real core serializes a new
        // Vec each call, and `usePolled` needs a changed reference to re-render.
        return DOWNLOADS.map((d) => ({ ...d }));
      case "enqueue_download": {
        const body = (a.body ?? {}) as AnyRecord;
        const d: AnyRecord = {
          id: `dl-${seq++}`, url: String(body.url ?? ""),
          filename: String(body.filename ?? "model.gguf").split("/").pop(),
          model_type: body.model_type ?? null, sha256: body.sha256 ?? null,
          size_bytes: body.size_bytes ?? 4_683_073_536, bytes_done: 0, retries: 0,
          state: "running", error_text: null, model_id: null,
          created_at: now(), updated_at: now(), roles: body.roles ?? [],
        };
        DOWNLOADS.unshift(d);
        return d;
      }
      case "pause_download": {
        const d = DOWNLOADS.find((x) => x.id === a.id);
        if (d && d.state === "running") d.state = "paused";
        return null;
      }
      case "resume_download": {
        const d = DOWNLOADS.find((x) => x.id === a.id);
        if (d && (d.state === "paused" || d.state === "failed")) d.state = "running";
        return null;
      }
      case "cancel_download": {
        const i = DOWNLOADS.findIndex((x) => x.id === a.id);
        if (i >= 0) DOWNLOADS.splice(i, 1);
        return null;
      }
      case "delete_download": {
        const i = DOWNLOADS.findIndex((x) => x.id === a.id);
        if (i < 0) throw new Error("no such download");
        if (!["done", "failed"].includes(String(DOWNLOADS[i].state))) {
          throw new Error("this download is still in progress — cancel it first");
        }
        DOWNLOADS.splice(i, 1);
        return null;
      }
      case "clear_finished_downloads": {
        const before = DOWNLOADS.length;
        for (let i = DOWNLOADS.length - 1; i >= 0; i--) {
          if (["done", "failed"].includes(String(DOWNLOADS[i].state))) DOWNLOADS.splice(i, 1);
        }
        return before - DOWNLOADS.length;
      }
      case "export_backup":
        return "E:\\AI\\data\\exports\\aiwm-export-2026-01-01T00-00-00Z.zip";
      case "import_backup":
        return {
          core_version: "0.0.1",
          created_at: now(),
          model_count: MODELS.length,
          missing_models: ["Qwen2.5 Coder (coder.gguf)"],
          restart_required: true,
        };
      case "create_agent": {
        const body = (a.body ?? {}) as AnyRecord;
        const agent = {
          id: `a-dev-${seq++}`, name: String(body.name ?? "Agent"),
          adapter: String(body.adapter ?? "opencode"), model_id: body.model_id ?? null,
          workspace_path: String(body.workspace_path ?? "E:\\AI"),
          allowed_paths: body.allowed_paths ?? [], toolset: body.toolset ?? null,
          created_at: now(),
        };
        AGENTS.unshift(agent);
        return agent;
      }
      case "delete_agent":
        return null;
      case "open_agent_session": {
        DEV_SESSION = {
          id: `s-dev-${seq++}`,
          agent_id: String((a.body as AnyRecord)?.agent_id ?? "a-coder"),
          state: "awaiting_approval",
          answered: false,
          stopped: false,
        };
        const first = (a.body as AnyRecord)?.first_message;
        DEV_SESSION.firstMessage = typeof first === "string" ? first : null;
        return devSession();
      }
      case "agent_session_detail":
        return DEV_SESSION && DEV_SESSION.id === String(a.id)
          ? { session: devSession(), events: devEvents(), live: !DEV_SESSION.stopped }
          : null;
      case "agent_session_message":
        if (DEV_SESSION) DEV_SESSION.state = "working";
        return null;
      case "agent_session_permission":
        if (DEV_SESSION) {
          DEV_SESSION.answered = true;
          DEV_SESSION.state = "idle";
        }
        return null;
      case "stop_agent_session":
        if (DEV_SESSION) {
          DEV_SESSION.stopped = true;
          DEV_SESSION.state = "stopped";
        }
        return null;
      case "launcher_status":
        return DEV_LAUNCH;
      case "launch_external": {
        const body = (a.body ?? {}) as AnyRecord;
        const modelId = (body.model_id as string) || "m-qwen";
        const model = MODELS.find((m) => m.id === modelId);
        DEV_LAUNCH = {
          tool: String(body.tool ?? "opencode"),
          model_id: modelId,
          model_name: model?.name ?? modelId,
          base_url: "http://127.0.0.1:41234/v1",
          workspace: String(body.workspace ?? "E:\\AI"),
          warning:
            body.tool === "hermes" && Number(model?.ctx_max ?? 0) < 64_000
              ? `${model?.name ?? modelId} declares a ${Number(model?.ctx_max ?? 0)}-token context; Hermes Agent refuses to start below 64000 and will likely error immediately.`
              : null,
        };
        return DEV_LAUNCH;
      }
      case "stop_external_launch":
        DEV_LAUNCH = null;
        return null;

      // --- Story Studio (Phase 1) --------------------------------------
      case "list_stories":
        return STORIES.map((s) => ({ ...s }));
      case "create_story": {
        const body = (a.body ?? {}) as AnyRecord;
        const story = {
          id: `story-dev-${seq++}`,
          name: String(body.name ?? "Untitled Story"),
          setting: String(body.setting ?? ""),
          art_style: String(body.art_style ?? ""),
          premise: String(body.premise ?? ""),
          created_at: now(),
        };
        STORIES.unshift(story);
        return story;
      }
      case "update_story": {
        const s = STORIES.find((x) => x.id === a.id);
        const body = (a.body ?? {}) as AnyRecord;
        if (s) {
          s.name = String(body.name ?? s.name);
          s.setting = String(body.setting ?? s.setting);
          s.art_style = String(body.art_style ?? s.art_style);
          s.premise = String(body.premise ?? s.premise);
        }
        return null;
      }
      case "delete_story": {
        const id = a.id;
        removeWhere(STORIES, (x) => x.id === id);
        removeWhere(CHARACTERS, (x) => x.story_id === id);
        removeWhere(NPCS, (x) => x.story_id === id);
        removeWhere(LOCATIONS, (x) => x.story_id === id);
        removeWhere(SCENES, (x) => x.story_id === id);
        return null;
      }
      case "list_characters":
        return CHARACTERS.filter((c) => c.story_id === a.storyId).map((c) => ({ ...c }));
      case "create_character": {
        const body = (a.body ?? {}) as AnyRecord;
        const character = {
          id: `char-dev-${seq++}`, story_id: String(a.storyId ?? ""),
          name: String(body.name ?? "Unnamed"), traits: String(body.traits ?? ""),
          backstory: String(body.backstory ?? ""), alignment: String(body.alignment ?? ""),
          portrait_job_id: null, inventory: [] as string[], created_at: now(),
        };
        CHARACTERS.push(character);
        return character;
      }
      case "update_character": {
        const c = CHARACTERS.find((x) => x.id === a.id);
        const body = (a.body ?? {}) as AnyRecord;
        if (c) {
          c.name = String(body.name ?? c.name);
          c.traits = String(body.traits ?? c.traits);
          c.backstory = String(body.backstory ?? c.backstory);
          c.alignment = String(body.alignment ?? c.alignment);
        }
        return null;
      }
      case "delete_character": {
        const id = a.id;
        removeWhere(CHARACTERS, (x) => x.id === id);
        removeWhere(CHARACTER_RELATIONSHIPS, (x) => x.character_id === id || x.related_character_id === id);
        return null;
      }
      case "set_character_portrait": {
        const c = CHARACTERS.find((x) => x.id === a.id);
        if (c) c.portrait_job_id = (a.jobId as string | null) ?? null;
        return null;
      }
      case "set_character_inventory": {
        const c = CHARACTERS.find((x) => x.id === a.id);
        if (c) c.inventory = (a.items as string[]) ?? [];
        return null;
      }
      case "list_character_relationships":
        return CHARACTER_RELATIONSHIPS.filter((r) => r.character_id === a.id).map((r) => ({ ...r }));
      case "add_character_relationship": {
        const rel = {
          id: `rel-dev-${seq++}`, character_id: String(a.id ?? ""),
          related_character_id: String(a.relatedCharacterId ?? ""), note: String(a.note ?? ""),
          created_at: now(),
        };
        CHARACTER_RELATIONSHIPS.push(rel);
        return rel;
      }
      case "remove_character_relationship": {
        const id = a.id;
        removeWhere(CHARACTER_RELATIONSHIPS, (x) => x.id === id);
        return null;
      }
      case "character_log":
        return CHARACTER_LOGS.filter((l) => l.character_id === a.id).map((l) => ({ ...l }));
      case "list_npcs":
        return NPCS.filter((n) => n.story_id === a.storyId).map((n) => ({ ...n }));
      case "create_npc": {
        const body = (a.body ?? {}) as AnyRecord;
        const npc = {
          id: `npc-dev-${seq++}`, story_id: String(a.storyId ?? ""),
          name: String(body.name ?? "Unnamed"), role: String(body.role ?? ""),
          location_id: (body.location_id as string | null) ?? null,
          description: String(body.description ?? ""), created_at: now(),
        };
        NPCS.push(npc);
        return npc;
      }
      case "update_npc": {
        const n = NPCS.find((x) => x.id === a.id);
        const body = (a.body ?? {}) as AnyRecord;
        if (n) {
          n.name = String(body.name ?? n.name);
          n.role = String(body.role ?? n.role);
          n.location_id = (body.location_id as string | null) ?? null;
          n.description = String(body.description ?? n.description);
        }
        return null;
      }
      case "delete_npc": {
        const id = a.id;
        removeWhere(NPCS, (x) => x.id === id);
        return null;
      }
      case "list_locations":
        return LOCATIONS.filter((l) => l.story_id === a.storyId).map((l) => ({ ...l }));
      case "create_location": {
        const body = (a.body ?? {}) as AnyRecord;
        const loc = {
          id: `loc-dev-${seq++}`, story_id: String(a.storyId ?? ""),
          name: String(body.name ?? "Unnamed"), description: String(body.description ?? ""),
          reference_job_id: null, created_at: now(),
        };
        LOCATIONS.push(loc);
        return loc;
      }
      case "update_location": {
        const l = LOCATIONS.find((x) => x.id === a.id);
        const body = (a.body ?? {}) as AnyRecord;
        if (l) {
          l.name = String(body.name ?? l.name);
          l.description = String(body.description ?? l.description);
        }
        return null;
      }
      case "delete_location": {
        const id = a.id;
        removeWhere(LOCATIONS, (x) => x.id === id);
        return null;
      }
      case "set_location_reference": {
        const l = LOCATIONS.find((x) => x.id === a.id);
        if (l) l.reference_job_id = (a.jobId as string | null) ?? null;
        return null;
      }
      case "list_scenes":
        return SCENES.filter((s) => s.story_id === a.storyId)
          .sort((x, y) => Number(x.position) - Number(y.position))
          .map((s) => ({ ...s }));
      case "create_scene": {
        const body = (a.body ?? {}) as AnyRecord;
        const storyId = String(a.storyId ?? "");
        const position = SCENES.filter((s) => s.story_id === storyId).length;
        const sceneId = `scene-dev-${seq++}`;
        const dialogue = ((body.dialogue as AnyRecord[]) ?? []).map((d, i) => ({
          id: `dl-dev-${seq++}`, scene_id: sceneId, character_id: String(d.character_id ?? ""),
          position: i, text: String(d.text ?? ""),
        }));
        const participantIds = (body.participant_ids as string[]) ?? [];
        const scene: AnyRecord = {
          id: sceneId, story_id: storyId,
          location_id: (body.location_id as string | null) ?? null,
          narrative: String(body.narrative ?? ""), redline: String(body.redline ?? ""),
          position, created_at: now(),
          participant_ids: participantIds, dialogue, images: [],
        };
        SCENES.push(scene);
        for (const cid of participantIds) {
          CHARACTER_LOGS.push({
            id: `log-dev-${seq++}`, character_id: cid, scene_id: sceneId, created_at: now(),
            text: `Appeared in a scene: ${scene.redline || scene.narrative || "(untitled)"}`,
          });
        }
        return scene;
      }
      case "update_scene": {
        const scene = SCENES.find((x) => x.id === a.id);
        if (!scene) throw new Error(`no such scene ${a.id}`);
        const body = (a.body ?? {}) as AnyRecord;
        const before = new Set((scene.participant_ids as string[]) ?? []);
        const participantIds = (body.participant_ids as string[]) ?? [];
        scene.location_id = (body.location_id as string | null) ?? null;
        scene.narrative = String(body.narrative ?? "");
        scene.redline = String(body.redline ?? "");
        scene.participant_ids = participantIds;
        scene.dialogue = ((body.dialogue as AnyRecord[]) ?? []).map((d, i) => ({
          id: `dl-dev-${seq++}`, scene_id: scene.id, character_id: String(d.character_id ?? ""),
          position: i, text: String(d.text ?? ""),
        }));
        for (const cid of participantIds) {
          if (!before.has(cid)) {
            CHARACTER_LOGS.push({
              id: `log-dev-${seq++}`, character_id: cid, scene_id: scene.id, created_at: now(),
              text: `Appeared in a scene: ${scene.redline || scene.narrative || "(untitled)"}`,
            });
          }
        }
        return { ...scene };
      }
      case "delete_scene": {
        const id = a.id;
        removeWhere(SCENES, (x) => x.id === id);
        return null;
      }
      case "add_scene_image": {
        const scene = SCENES.find((x) => x.id === a.sceneId);
        if (!scene) throw new Error(`no such scene ${a.sceneId}`);
        const images = (scene.images as AnyRecord[]) ?? [];
        const image = {
          id: `si-dev-${seq++}`, scene_id: scene.id, job_id: String(a.jobId ?? ""),
          is_canonical: images.length === 0, created_at: now(),
        };
        images.push(image);
        scene.images = images;
        return image;
      }
      case "set_canonical_scene_image": {
        for (const scene of SCENES) {
          const images = (scene.images as AnyRecord[]) ?? [];
          if (images.some((img) => img.id === a.id)) {
            images.forEach((img) => {
              img.is_canonical = img.id === a.id;
            });
            break;
          }
        }
        return null;
      }
      case "delete_scene_image": {
        for (const scene of SCENES) {
          const images = (scene.images as AnyRecord[]) ?? [];
          const i = images.findIndex((img) => img.id === a.id);
          if (i >= 0) {
            const wasCanonical = images[i].is_canonical;
            images.splice(i, 1);
            if (wasCanonical && images.length > 0) images[images.length - 1].is_canonical = true;
            break;
          }
        }
        return null;
      }

      default:
        if (cmd.startsWith("plugin:")) return null; // opener plugin etc. — no-op
        console.warn("dev-mock: unhandled command", cmd);
        return null;
    }
    // `shouldMockEvents` lets mockIPC handle `plugin:event|listen/unlisten`
    // itself, so `listen()` (useTelemetry) resolves cleanly.
  }, { shouldMockEvents: true });
  mockWindows("main");
}
