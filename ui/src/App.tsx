import { useEffect, useState } from "react";
import { about, type AboutInfo } from "./lib/ipc";

export default function App() {
  const [info, setInfo] = useState<AboutInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    about()
      .then(setInfo)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const value = (v: string | number | boolean | undefined) =>
    v === undefined ? (error ? "unavailable" : "…") : String(v);

  return (
    <main className="shell">
      <h1>AI Workstation Manager</h1>
      <p className="phase">Phase 1 — foundation scaffold</p>

      <dl className="facts">
        <dt>core</dt>
        <dd>{value(info?.core_version)}</dd>
        <dt>host</dt>
        <dd>{value(info?.tauri_host_version)}</dd>
        <dt>data dir</dt>
        <dd>{value(info?.data_dir)}</dd>
        <dt>store</dt>
        <dd>{value(info?.store_path)}</dd>
        <dt>api port</dt>
        <dd>{value(info?.core_api_port)}</dd>
        <dt>offline</dt>
        <dd>{value(info?.offline_mode)}</dd>
      </dl>

      <p className="hint">
        Live GPU/RAM telemetry, model library and job views arrive in WP-6/WP-7.
      </p>
    </main>
  );
}
