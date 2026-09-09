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
  listModels,
  type AboutInfo,
  type Agent,
  type AgentRuntime,
  type Job,
  type KnownModel,
  type Model,
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
