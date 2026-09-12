import { useState } from "react";
import { useAbout, useJobs, useTelemetry } from "../../lib/hooks";
import { Meter } from "../../components/Meter";
import { cancelJob, type Job, type JobState } from "../../lib/ipc";
import "./dashboard.css";

const GB = 1024;
const CANCELLABLE: JobState[] = ["queued", "scheduled", "blocked", "preparing", "running"];
const CAPABILITIES: { key: string; label: string; hint: string; tab?: string }[] = [
  { key: "chat", label: "Chat", hint: "ready", tab: "chat" },
  { key: "image", label: "Generate Image", hint: "ready", tab: "image" },
  { key: "video", label: "Generate Video", hint: "ready", tab: "video" },
  { key: "code", label: "Coding", hint: "agents", tab: "agents" },
];

const gb = (mb: number) => (mb / GB).toFixed(1);

export function Dashboard({ onNavigate }: { onNavigate: (tab: string) => void }) {
  const { telemetry, error } = useTelemetry();
  const { data: jobs, refetch: refetchJobs } = useJobs();
  const about = useAbout();

  const gpu = telemetry?.gpu;
  const host = telemetry?.host;
  const activeJob = jobs?.find((j) =>
    ["preparing", "running", "post", "scheduled"].includes(j.state),
  );
  const queued = jobs?.filter((j) => j.state === "queued").length ?? 0;

  return (
    <div className="dash">
      <section className="card card--gpu" aria-labelledby="gpu-h">
        <header className="card__head">
          <h2 id="gpu-h">GPU</h2>
          {gpu?.state === "available" && <span className="card__sub">{gpu.name}</span>}
        </header>

        {gpu?.state === "available" ? (
          <>
            <Meter
              label="VRAM"
              value={gpu.vram_used_mb}
              max={gpu.vram_total_mb}
              unit="GB"
              format={gb}
            />
            <div className="stat-row">
              <Stat label="Utilization" value={`${gpu.utilization_pct}%`} />
              <Stat label="Temperature" value={`${gpu.temperature_c}°C`} />
              <Stat
                label="Budget"
                value={about ? `${gb(about.vram_budget_mb)} GB` : "…"}
              />
            </div>
          </>
        ) : (
          <p className="muted">
            {gpu?.state === "unavailable"
              ? `GPU telemetry unavailable — ${gpu.reason}`
              : error
                ? "GPU telemetry unavailable"
                : "Reading GPU…"}
          </p>
        )}
      </section>

      <section className="card" aria-labelledby="host-h">
        <header className="card__head">
          <h2 id="host-h">Host</h2>
        </header>
        {host ? (
          <>
            <Meter
              label="RAM"
              value={host.ram_used_mb}
              max={host.ram_total_mb}
              unit="GB"
              format={gb}
            />
            <div style={{ height: "var(--space-4)" }} />
            <Meter label="CPU" value={host.cpu_total_pct} max={100} format={(v) => `${v}%`} />
          </>
        ) : (
          <p className="muted">Reading host…</p>
        )}
      </section>

      <section className="card card--wide" aria-labelledby="jobs-h">
        <header className="card__head">
          <h2 id="jobs-h">Jobs</h2>
          <span className="card__sub numeric">
            {activeJob ? "1 running" : "idle"} · {queued} queued
          </span>
          <button type="button" className="dash__jobs-link" onClick={() => onNavigate("jobs")}>
            View all →
          </button>
        </header>
        <JobTable jobs={jobs} onCancelled={refetchJobs} />
      </section>

      <section className="card card--wide" aria-labelledby="do-h">
        <header className="card__head">
          <h2 id="do-h">What do you want to do?</h2>
        </header>
        <div className="capabilities">
          {CAPABILITIES.map((c) => (
            <button
              key={c.key}
              className="capability"
              disabled={!c.tab}
              onClick={c.tab ? () => onNavigate(c.tab as string) : undefined}
              title={c.tab ? `Open ${c.label}` : `Available in ${c.hint}`}
            >
              <span className="capability__label">{c.label}</span>
              <span className="capability__hint">{c.hint}</span>
            </button>
          ))}
        </div>
      </section>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="stat">
      <span className="stat__label">{label}</span>
      <span className="stat__value numeric">{value}</span>
    </div>
  );
}

function JobTable({ jobs, onCancelled }: { jobs: Job[] | null; onCancelled: () => void }) {
  if (!jobs) return <p className="muted">Loading…</p>;
  if (jobs.length === 0) return <p className="muted">No jobs yet.</p>;

  return (
    <table className="jobs">
      <thead>
        <tr>
          <th>Type</th>
          <th>Model</th>
          <th>State</th>
          <th>Created</th>
          <th />
        </tr>
      </thead>
      <tbody>
        {jobs.slice(0, 12).map((j) => (
          <tr key={j.id}>
            <td>{j.job_type}</td>
            <td className="muted">{j.model_id ?? "—"}</td>
            <td>
              <span className="job-state" data-state={j.state}>
                {j.state}
              </span>
            </td>
            <td className="muted numeric">{new Date(j.created_at).toLocaleTimeString()}</td>
            <td>
              {CANCELLABLE.includes(j.state) && (
                <CancelButton id={j.id} onDone={onCancelled} />
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function CancelButton({ id, onDone }: { id: string; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const click = async () => {
    setBusy(true);
    try {
      await cancelJob(id);
    } finally {
      setBusy(false);
      onDone();
    }
  };
  return (
    <button type="button" className="job-cancel" onClick={click} disabled={busy}>
      {busy ? "…" : "Cancel"}
    </button>
  );
}
