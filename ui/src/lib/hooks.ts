import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  about,
  benchmarkHistory,
  characterLog,
  civitaiSearch,
  civitaiStatus,
  getRecentLogs,
  getRuntimes,
  getTelemetry,
  launcherStatus,
  listAgentRuntimes,
  listAgents,
  listBenchSuites,
  listBenchmarks,
  listCharacterRelationships,
  listCharacters,
  listCaptioners,
  listConcepts,
  listDatasetFrames,
  listDatasetFramesForDataset,
  listDatasets,
  frameConceptMap,
  listDocuments,
  listFeaturedModels,
  listJobs,
  listKnownModels,
  listLocations,
  listModelStacks,
  listNpcs,
  listScenes,
  listStories,
  externalEngines,
  jobProgressWsUrl,
  listDownloads,
  listModels,
  listSessions,
  listTrainingProfiles,
  listTrainingRuns,
  listVoiceIdentities,
  localApiStatus,
  modelTags,
  getTrainingRun,
  registrySearch,
  registryStatus,
  storageReport,
  trainerStatus,
  type AboutInfo,
  type Agent,
  type AgentRuntime,
  type Benchmark,
  type BenchSuite,
  type Character,
  type CharacterLogEntry,
  type CharacterRelationship,
  type Captioner,
  type CivitaiSearchParams,
  type Dataset,
  type DatasetConcept,
  type DatasetFrame,
  type Document,
  type Download,
  type ExternalEngine,
  type Job,
  type JobProgress,
  type JobState,
  type LaunchInfo,
  type LocalApiStatus,
  type Npc,
  type RegistryStatus,
  type SceneDetail,
  type StorageReport,
  type Story,
  type StoryLocation,
  type FeaturedModel,
  type KnownModel,
  type Model,
  type ModelStack,
  type RegistrySearchParams,
  type RegistrySearchResult,
  type RunDetail,
  type RuntimeStatus,
  type Session,
  type SessionCapability,
  type SystemTelemetry,
  type TrainerStatus,
  type TrainingProfile,
  type TrainingRun,
  type VoiceIdentity,
} from "./ipc";

/** Live telemetry: seeded by one `get_telemetry` call, then updated by the
 *  `telemetry` event the host emits every second. */
export function useTelemetry() {
  const [telemetry, setTelemetry] = useState<SystemTelemetry | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getTelemetry().then(setTelemetry).catch((e) => setError(String(e)));
    const unlisten = listen<SystemTelemetry>("telemetry", (e) => setTelemetry(e.payload));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  return { telemetry, error };
}

/** Real per-step render progress for one in-flight job, straight from
 *  ComfyUI's own `/ws` (`core::progress` -> `GET /ws/jobs/{id}`) — connects a
 *  plain `WebSocket` to the loopback server, the same way the `<img>`/
 *  `<video>` output URLs hit it directly, no Tauri IPC involved. `null` while
 *  nothing has streamed yet: an earlier render phase ComfyUI doesn't report
 *  progress for (checkpoint load, VAE decode), or the connection failed —
 *  the dev-mock browser preview has no real core to connect to, so it always
 *  stays `null` there; the caller's own last-log-line fallback still shows. */
export function useJobProgress(port: number | null, jobId: string | null) {
  const [progress, setProgress] = useState<JobProgress | null>(null);

  useEffect(() => {
    setProgress(null);
    if (port == null || jobId == null) return;
    let live = true;
    let ws: WebSocket;
    try {
      ws = new WebSocket(jobProgressWsUrl(port, jobId));
    } catch {
      return; // no WebSocket in this environment
    }
    ws.onmessage = (e) => {
      if (!live) return;
      try {
        setProgress(JSON.parse(String(e.data)) as JobProgress);
      } catch {
        /* a malformed frame -- ignore it, keep the last good reading */
      }
    };
    ws.onerror = () => {
      /* connection refused (dev-mock, or the core isn't up yet) -- the
       * caller's own fallback display covers this, nothing to surface here */
    };
    return () => {
      live = false;
      ws.close();
    };
  }, [port, jobId]);

  return progress;
}

export function useAbout() {
  const [info, setInfo] = useState<AboutInfo | null>(null);
  useEffect(() => {
    about().then(setInfo).catch(() => setInfo(null));
  }, []);
  return info;
}

/** The curated image/video catalogue, fit-checked against the VRAM budget.
 *  Fetched once. */
export function useKnownModels() {
  const [data, setData] = useState<KnownModel[] | null>(null);
  useEffect(() => {
    listKnownModels().then(setData).catch(() => setData([]));
  }, []);
  return data;
}

/** The curated image/video "stacks" — a base model plus every companion
 *  file it needs — fit-checked against the VRAM budget. Fetched once. */
export function useModelStacks() {
  const [data, setData] = useState<ModelStack[] | null>(null);
  useEffect(() => {
    listModelStacks().then(setData).catch(() => setData([]));
  }, []);
  return data;
}

/** The curated chat/coding recommendations, fit-checked against the VRAM
 *  budget. Fetched once. */
export function useFeaturedModels() {
  const [data, setData] = useState<FeaturedModel[] | null>(null);
  useEffect(() => {
    listFeaturedModels().then(setData).catch(() => setData([]));
  }, []);
  return data;
}

function usePolled<T>(key: string, fetcher: () => Promise<T>, intervalMs: number) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    let alive = true;
    const tick = () =>
      fetcher()
        .then((d) => {
          if (alive) {
            setData(d);
            setError(null);
          }
        })
        .catch((e) => alive && setError(String(e)));
    tick();
    const id = setInterval(tick, intervalMs);
    return () => {
      alive = false;
      clearInterval(id);
    };
    // `key` identifies the endpoint and encodes everything `fetcher` closes
    // over (see call sites below); `fetcher` itself is a fresh closure on
    // every render for several callers (e.g. `useJobs`), so depending on it
    // directly would restart the poll interval every render instead of only
    // when the endpoint/params actually change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, intervalMs, nonce]);

  // Stable across renders: consumers put `refetch` in their own `useCallback`
  // dep arrays, and a fresh closure here would invalidate those handlers on
  // every poll tick -- which is what defeats `React.memo` further down.
  const refetch = useCallback(() => setNonce((n) => n + 1), []);
  return { data, error, refetch };
}

/** A `dataset_prep` job's curation set, polled while the job is running so
 *  the grid fills in as the pipeline works through the tree; `null` disables
 *  polling (no job selected yet). */
export const useDatasetFrames = (jobId: string | null) =>
  usePolled<DatasetFrame[]>(
    `dataset-frames:${jobId ?? ""}`,
    () => (jobId ? listDatasetFrames(jobId) : Promise.resolve([])),
    1500,
  );

/** The captioner registry with its install state — the "Beschreiben mit"
 *  dropdown. Polled slowly: it only changes when a model is imported. */
export const useCaptioners = () => usePolled<Captioner[]>("captioners", listCaptioners, 5000);

/** Every curation set, newest prep run included. */
export const useDatasets = () => usePolled<Dataset[]>("datasets", listDatasets, 3000);

/** A dataset's curation set. Unlike {@link useDatasetFrames} this keeps
 *  working once the prep job is gone; `null` disables polling. */
export const useDatasetFramesForDataset = (datasetId: string | null) =>
  usePolled<DatasetFrame[]>(
    `dataset-frames-by-dataset:${datasetId ?? ""}`,
    () => (datasetId ? listDatasetFramesForDataset(datasetId) : Promise.resolve([])),
    3000,
  );

/** A dataset's concepts with their frame counts; `null` disables polling. */
export const useConcepts = (datasetId: string | null) =>
  usePolled<DatasetConcept[]>(
    `concepts:${datasetId ?? ""}`,
    () => (datasetId ? listConcepts(datasetId) : Promise.resolve([])),
    3000,
  );

/** frame id -> concept ids, for the grid's per-item concept chips. */
export const useFrameConceptMap = (datasetId: string | null) =>
  usePolled<Record<string, string[]>>(
    `frame-concepts:${datasetId ?? ""}`,
    () => (datasetId ? frameConceptMap(datasetId) : Promise.resolve({})),
    3000,
  );

export const useJobs = (opts?: { states?: JobState[]; limit?: number }) =>
  usePolled<Job[]>(
    `jobs:${JSON.stringify(opts ?? null)}`,
    () => listJobs(opts ?? { limit: 50 }),
    2000,
  );
export const useRuntimes = () => usePolled<RuntimeStatus[]>("runtimes", getRuntimes, 3000);
export const useLogs = () => usePolled<string[]>("logs", () => getRecentLogs(300), 3000);
export const useModels = () => usePolled<Model[]>("models", listModels, 3000);
export const useAgents = () => usePolled<Agent[]>("agents", listAgents, 4000);
export const useAgentRuntimes = () =>
  usePolled<AgentRuntime[]>("agent-runtimes", listAgentRuntimes, 3000);
export const useLauncherStatus = () =>
  usePolled<LaunchInfo | null>("launcher-status", launcherStatus, 3000);
export const useDownloads = () =>
  usePolled<Download[]>("downloads", listDownloads, 1500);
export const useBenchmarks = () =>
  usePolled<Benchmark[]>("benchmarks", listBenchmarks, 3000);
/** The built-in suite catalogue. It is a compile-time constant in the core, so
 *  this only re-reads slowly -- enough to fill the picker in once the core is
 *  up, without a 3 s poll for something that never changes. */
export const useBenchSuites = () =>
  usePolled<BenchSuite[]>("bench-suites", listBenchSuites, 60_000);
/** Cross-model benchmark history; `null` suite = every row, quick tests too.
 *  `limit` is the API's own page size (default 50, max 200): the comparison
 *  needs a window wide enough that no model silently drops out of it. */
export const useBenchmarkHistory = (suite: string | null, limit?: number) =>
  usePolled<Benchmark[]>(
    `benchmark-history:${suite ?? ""}:${limit ?? ""}`,
    () => benchmarkHistory({ suite: suite ?? undefined, limit }),
    3000,
  );
export const useStorage = () =>
  usePolled<StorageReport>("storage", storageReport, 5000);
export const useDocuments = (sessionId: string | null) =>
  usePolled<Document[]>(
    `documents:${sessionId ?? ""}`,
    () => (sessionId ? listDocuments(sessionId) : Promise.resolve([])),
    4000,
  );
export const useModelTags = () =>
  usePolled<Record<string, string[]>>("model-tags", modelTags, 4000);
export const useRegistryStatus = () =>
  usePolled<RegistryStatus>("registry-status", registryStatus, 5000);
export const useCivitaiStatus = () =>
  usePolled<RegistryStatus>("civitai-status", civitaiStatus, 5000);
export const useLocalApiStatus = () =>
  usePolled<LocalApiStatus>("local-api-status", localApiStatus, 5000);
export const useExternalEngines = () =>
  usePolled<ExternalEngine[]>("external-engines", externalEngines, 5000);

const PINNED_MODELS_KEY = "aiwm:pinned-models";

function readPinned(): Set<string> {
  try {
    const raw = window.localStorage.getItem(PINNED_MODELS_KEY);
    return new Set(raw ? (JSON.parse(raw) as string[]) : []);
  } catch {
    return new Set();
  }
}

/** Favorite/pin a model for quick access -- per-browser only (`localStorage`),
 *  no backend round-trip. `toggle` is optimistic and never throws: a blocked
 *  or cleared store just means pins don't persist, not a broken UI. */
export function usePinnedModels() {
  const [pinned, setPinned] = useState<Set<string>>(() => readPinned());

  const toggle = (modelId: string) => {
    setPinned((prev) => {
      const next = new Set(prev);
      if (next.has(modelId)) next.delete(modelId);
      else next.add(modelId);
      try {
        window.localStorage.setItem(PINNED_MODELS_KEY, JSON.stringify([...next]));
      } catch {
        /* localStorage unavailable -- pin still applies for this session */
      }
      return next;
    });
  };

  return { pinned, isPinned: (id: string) => pinned.has(id), toggle };
}

/** Sessions for one capability (Chat/Image/Video "projects"), active first. */
export const useSessions = (capability: SessionCapability) =>
  usePolled<Session[]>(`sessions:${capability}`, () => listSessions(capability), 3000);

/** Saved Dia voice-cloning identities (Voice tab) — set up once, reused by
 *  name across many narration calls. */
export const useVoiceIdentities = () =>
  usePolled<VoiceIdentity[]>("voice-identities", listVoiceIdentities, 4000);

// --- training orchestrator -----------------------------------------------

/** The trainer's install/probe state. Polled slowly — it only moves when the
 *  user sets the trainer up or repairs it, and an install reports its own
 *  progress through `install_state`. */
export const useTrainerStatus = () =>
  usePolled<TrainerStatus>("trainer-status", trainerStatus, 5000);

/** The trainable model families with their base-weight and library joins.
 *  Slow: it only changes when a model is imported or downloaded. */
export const useTrainingProfiles = () =>
  usePolled<TrainingProfile[]>("training-profiles", listTrainingProfiles, 10000);

/** Every run, newest first. Polled at the runner's own cadence so the history
 *  list tracks a live run without hammering the store. */
export const useTrainingRuns = () =>
  usePolled<TrainingRun[]>("training-runs", listTrainingRuns, 3000);

/** One run with its log tail and preview images; `null` disables polling (no
 *  run selected). Faster than the list: this is the view someone actually
 *  sits and watches. */
export const useTrainingRun = (runId: string | null) =>
  usePolled<RunDetail | null>(
    `training-run:${runId ?? ""}`,
    () => (runId ? getTrainingRun(runId) : Promise.resolve(null)),
    2000,
  );

// --- Story Studio (Phase 1) ---------------------------------------------

export const useStories = () => usePolled<Story[]>("stories", listStories, 4000);

export const useCharacters = (storyId: string | null) =>
  usePolled<Character[]>(
    `characters:${storyId ?? ""}`,
    () => (storyId ? listCharacters(storyId) : Promise.resolve([])),
    4000,
  );

export const useNpcs = (storyId: string | null) =>
  usePolled<Npc[]>(
    `npcs:${storyId ?? ""}`,
    () => (storyId ? listNpcs(storyId) : Promise.resolve([])),
    4000,
  );

export const useLocations = (storyId: string | null) =>
  usePolled<StoryLocation[]>(
    `locations:${storyId ?? ""}`,
    () => (storyId ? listLocations(storyId) : Promise.resolve([])),
    4000,
  );

/** Every scene in a story's timeline, each with participants/dialogue/images. */
export const useScenes = (storyId: string | null) =>
  usePolled<SceneDetail[]>(
    `scenes:${storyId ?? ""}`,
    () => (storyId ? listScenes(storyId) : Promise.resolve([])),
    3000,
  );

export const useCharacterRelationships = (characterId: string | null) =>
  usePolled<CharacterRelationship[]>(
    `char-relationships:${characterId ?? ""}`,
    () => (characterId ? listCharacterRelationships(characterId) : Promise.resolve([])),
    5000,
  );

/** A character's append-only Character Log, oldest first. */
export const useCharacterLog = (characterId: string | null) =>
  usePolled<CharacterLogEntry[]>(
    `char-log:${characterId ?? ""}`,
    () => (characterId ? characterLog(characterId) : Promise.resolve([])),
    5000,
  );

/** Debounced Hugging Face search for the Discover panel. Runs when `params`
 *  change (400 ms after the last one) and `enabled`; not polled. */
export function useRegistrySearch(params: RegistrySearchParams, enabled: boolean) {
  const [result, setResult] = useState<RegistrySearchResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const key = JSON.stringify(params);

  useEffect(() => {
    if (!enabled) {
      setResult(null);
      setError(null);
      return;
    }
    let alive = true;
    setLoading(true);
    const id = setTimeout(() => {
      registrySearch(params)
        .then((r) => {
          if (alive) {
            setResult(r);
            setError(null);
          }
        })
        .catch((e) => alive && setError(e instanceof Error ? e.message : String(e)))
        .finally(() => alive && setLoading(false));
    }, 400);
    return () => {
      alive = false;
      clearTimeout(id);
    };
    // `key` is the serialized params and the real dependency; `params` may be
    // a fresh object identity on every render for callers that don't memoize
    // it, but its JSON content — the only thing that reaches `registrySearch`
    // — is unchanged whenever `key` is unchanged.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, enabled]);

  return { result, error, loading };
}

/** Debounced Civitai search for the Discover panel — same shape as
 *  `useRegistrySearch`, against the Civitai source instead. */
export function useCivitaiSearch(params: CivitaiSearchParams, enabled: boolean) {
  const [result, setResult] = useState<RegistrySearchResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const key = JSON.stringify(params);

  useEffect(() => {
    if (!enabled) {
      setResult(null);
      setError(null);
      return;
    }
    let alive = true;
    setLoading(true);
    const id = setTimeout(() => {
      civitaiSearch(params)
        .then((r) => {
          if (alive) {
            setResult(r);
            setError(null);
          }
        })
        .catch((e) => alive && setError(e instanceof Error ? e.message : String(e)))
        .finally(() => alive && setLoading(false));
    }, 400);
    return () => {
      alive = false;
      clearTimeout(id);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, enabled]);

  return { result, error, loading };
}
