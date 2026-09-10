import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  about,
  getRecentLogs,
  getRuntimes,
  getTelemetry,
  listAgentRuntimes,
  listAgents,
  listJobs,
  listKnownModels,
  listDownloads,
  listModels,
  registrySearch,
  type AboutInfo,
  type Agent,
  type AgentRuntime,
  type Download,
  type Job,
  type KnownModel,
  type Model,
  type RegistrySearchParams,
  type RegistrySearchResult,
  type RuntimeStatus,
  type SystemTelemetry,
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

export function useAbout() {
  const [info, setInfo] = useState<AboutInfo | null>(null);
  useEffect(() => {
    about().then(setInfo).catch(() => setInfo(null));
  }, []);
  return info;
}

/** The curated image-model catalogue. Static — fetched once. */
export function useKnownModels() {
  const [data, setData] = useState<KnownModel[] | null>(null);
  useEffect(() => {
    listKnownModels().then(setData).catch(() => setData([]));
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
    // `key` identifies the endpoint; `fetcher` is a stable closure per hook.
  }, [key, intervalMs, nonce]);

  const refetch = () => setNonce((n) => n + 1);
  return { data, error, refetch };
}

export const useJobs = () => usePolled<Job[]>("jobs", () => listJobs({ limit: 50 }), 2000);
export const useRuntimes = () => usePolled<RuntimeStatus[]>("runtimes", getRuntimes, 3000);
export const useLogs = () => usePolled<string[]>("logs", () => getRecentLogs(300), 3000);
export const useModels = () => usePolled<Model[]>("models", listModels, 3000);
export const useAgents = () => usePolled<Agent[]>("agents", listAgents, 4000);
export const useAgentRuntimes = () =>
  usePolled<AgentRuntime[]>("agent-runtimes", listAgentRuntimes, 3000);
export const useDownloads = () =>
  usePolled<Download[]>("downloads", listDownloads, 1500);

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
    // `key` is the serialized params — the real dependency; `params` itself is
    // a fresh object each render.
  }, [key, enabled]);

  return { result, error, loading };
}
