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

  return (
    <main className="shell">
      <h1>AI Workstation Manager</h1>
      <p className="phase">Phase 1 — foundation scaffold</p>

      <dl className="facts">
        <dt>core</dt>
        <dd>{info?.core_version ?? (error ? "unavailable" : "…")}</dd>
        <dt>host</dt>
        <dd>{info?.tauri_host_version ?? (error ? "unavailable" : "…")}</dd>
      </dl>

      <p className="hint">
        Live GPU/RAM telemetry, model library and job views arrive in WP-6/WP-7.
      </p>
    </main>
  );
}
