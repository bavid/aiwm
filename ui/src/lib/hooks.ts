import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  about,
  getRecentLogs,
  getRuntimes,
  getTelemetry,
  listJobs,
  type AboutInfo,
  type Job,
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

function usePolled<T>(key: string, fetcher: () => Promise<T>, intervalMs: number) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);

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
  }, [key, intervalMs]);

  return { data, error };
}

export const useJobs = () => usePolled<Job[]>("jobs", () => listJobs({ limit: 50 }), 2000);
export const useRuntimes = () => usePolled<RuntimeStatus[]>("runtimes", getRuntimes, 3000);
export const useLogs = () => usePolled<string[]>("logs", () => getRecentLogs(300), 3000);
