import { useMemo, useState } from "react";
import { useAbout, useJobs, useModels, useRuntimes, useStorage, useTelemetry } from "../../lib/hooks";
import { Meter } from "../../components/Meter";
import { cancelJob, unloadModel, type Job, type JobState, type RuntimeStatus } from "../../lib/ipc";
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
const gbFromBytes = (bytes: number) => (bytes / (GB * GB * 1024)).toFixed(1);

const DAY_MS = 24 * 60 * 60 * 1000;
const DAY_LABEL = new Intl.DateTimeFormat(undefined, { weekday: "short" });

/** How many of `jobs` were created on each of the last `n` calendar days
 *  (local time), oldest first -- a client-side approximation since the
 *  backend keeps no time-series data, only the latest telemetry snapshot. */
function daysBack(n: number, jobs: Job[]): { label: string; date: string; count: number }[] {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const buckets = Array.from({ length: n }, (_, i) => {
    const d = new Date(today.getTime() - (n - 1 - i) * DAY_MS);
    return { label: DAY_LABEL.format(d), date: d.toDateString(), count: 0 };
  });
  const byDate = new Map(buckets.map((b) => [b.date, b]));
  for (const j of jobs) {
    const bucket = byDate.get(new Date(j.created_at).toDateString());
    if (bucket) bucket.count += 1;
  }
  return buckets;
}

export function Dashboard({ onNavigate }: { onNavigate: (tab: string) => void }) {
  const { telemetry, error } = useTelemetry();
  const { data: jobs, refetch: refetchJobs } = useJobs();
  const { data: history } = useJobs({ limit: 300 });
  const { data: runtimes, refetch: refetchRuntimes } = useRuntimes();
  const { data: models } = useModels();
  const { data: storage } = useStorage();
  const about = useAbout();

  const gpu = telemetry?.gpu;
  const host = telemetry?.host;
  const activeJob = jobs?.find((j) =>
    ["preparing", "running", "post", "scheduled"].includes(j.state),
  );
  const queued = jobs?.filter((j) => j.state === "queued").length ?? 0;

  const modelName = useMemo(() => {
    const byId = new Map((models ?? []).map((m) => [m.id, m.name]));
    return (id: string) => byId.get(id) ?? id;
  }, [models]);

  const dailyCounts = useMemo(() => daysBack(7, history ?? []), [history]);

  const recentActivity = useMemo(() => {
    return (jobs ?? [])
      .filter((j) => j.finished_at)
      .slice()
      .sort((a, b) => (b.finished_at ?? "").localeCompare(a.finished_at ?? ""))
      .slice(0, 6);
  }, [jobs]);

  return (
    <div className="dash">
      <SetupChecklist runtimes={runtimes} models={models} jobs={jobs} onNavigate={onNavigate} />

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
            {storage && storage.volume_total_bytes && (
              <>
                <div style={{ height: "var(--space-4)" }} />
                <Meter
                  label="Model store"
                  value={storage.volume_total_bytes - (storage.volume_free_bytes ?? 0)}
                  max={storage.volume_total_bytes}
                  unit="GB"
                  format={gbFromBytes}
                />
              </>
            )}
          </>
        ) : (
          <p className="muted">Reading host…</p>
        )}
      </section>

      <section className="card card--wide" aria-labelledby="resident-h">
        <header className="card__head">
          <h2 id="resident-h">Resident right now</h2>
          <span className="card__sub numeric">{runtimes?.length ?? 0} runtimes</span>
        </header>
        <ResidentList runtimes={runtimes} modelName={modelName} onUnloaded={refetchRuntimes} />
      </section>

      <section className="card card--wide" aria-labelledby="usage-h">
        <header className="card__head">
          <h2 id="usage-h">Usage, last 7 days</h2>
          <span className="card__sub numeric">
            {dailyCounts.reduce((sum, d) => sum + d.count, 0)} jobs
          </span>
        </header>
        <UsageSparkline days={dailyCounts} />
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

      <section className="card card--wide" aria-labelledby="activity-h">
        <header className="card__head">
          <h2 id="activity-h">Recent activity</h2>
        </header>
        <ActivityFeed jobs={recentActivity} modelName={modelName} />
      </section>
    </div>
  );
}

const SETUP_DISMISSED_KEY = "aiwm:setup-dismissed";

/** A dismissible first-run checklist: install a runtime, import a model,
 *  generate something. Hides itself once every step is done, or once the
 *  user dismisses it by hand -- never nags twice. */
function SetupChecklist({
  runtimes,
  models,
  jobs,
  onNavigate,
}: {
  runtimes: RuntimeStatus[] | null;
  models: { id: string }[] | null;
  jobs: Job[] | null;
  onNavigate: (tab: string) => void;
}) {
  const [dismissed, setDismissed] = useState(
    () => window.localStorage.getItem(SETUP_DISMISSED_KEY) === "true",
  );

  if (dismissed || !runtimes || !models || !jobs) return null;

  const hasRuntime = runtimes.some((r) => r.detail !== "not installed");
  const hasModel = models.length > 0;
  const hasRun = jobs.length > 0;
  if (hasRuntime && hasModel && hasRun) return null;

  const dismiss = () => {
    setDismissed(true);
    try {
      window.localStorage.setItem(SETUP_DISMISSED_KEY, "true");
    } catch {
      /* localStorage unavailable -- dismissal just won't stick across reloads */
    }
  };

  const steps: { done: boolean; label: string; hint: string; tab: string }[] = [
    { done: hasRuntime, label: "Install a runtime", hint: "llama.cpp, ComfyUI, or Colibri", tab: "diagnostics" },
    { done: hasModel, label: "Import a model", hint: "a .gguf or .safetensors file", tab: "models" },
    { done: hasRun, label: "Generate something", hint: "Chat, Image, Video, or an agent", tab: "chat" },
  ];

  return (
    <section className="card card--wide card--setup" aria-labelledby="setup-h">
      <header className="card__head">
        <h2 id="setup-h">Get set up</h2>
        <button type="button" className="dash__jobs-link" onClick={dismiss}>
          Dismiss
        </button>
      </header>
      <ol className="setup-steps">
        {steps.map((s) => (
          <li key={s.label} className="setup-step" data-done={s.done}>
            <span className="setup-step__mark">{s.done ? "✓" : ""}</span>
            <span className="setup-step__body">
              <span className="setup-step__label">{s.label}</span>
              <span className="setup-step__hint">{s.hint}</span>
            </span>
            {!s.done && (
              <button type="button" className="job-cancel" onClick={() => onNavigate(s.tab)}>
                Open
              </button>
            )}
          </li>
        ))}
      </ol>
    </section>
  );
}

function UsageSparkline({ days }: { days: { label: string; date: string; count: number }[] }) {
  const max = Math.max(1, ...days.map((d) => d.count));
  return (
    <div className="usage-bars">
      {days.map((d) => (
        <div key={d.date} className="usage-bar" title={`${d.label}: ${d.count} job${d.count === 1 ? "" : "s"}`}>
          <div className="usage-bar__track">
            <div className="usage-bar__fill" style={{ height: `${(d.count / max) * 100}%` }} />
          </div>
          <span className="usage-bar__count numeric">{d.count}</span>
          <span className="usage-bar__label">{d.label}</span>
        </div>
      ))}
    </div>
  );
}

function ResidentList({
  runtimes,
  modelName,
  onUnloaded,
}: {
  runtimes: RuntimeStatus[] | null;
  modelName: (id: string) => string;
  onUnloaded: () => void;
}) {
  if (!runtimes) return <p className="muted">Loading…</p>;

  return (
    <ul className="resident-list">
      {runtimes.map((rt) => (
        <li key={rt.id} className="resident-row" data-idle={rt.loaded_models.length === 0}>
          <span className="resident-row__dot" data-health={rt.health} />
          <span className="resident-row__rt">{rt.id}</span>
          {rt.loaded_models.length > 0 ? (
            <span className="resident-row__main">
              {rt.loaded_models.map((m) => modelName(m.model_id)).join(", ")}
            </span>
          ) : (
            <span className="resident-row__main muted">{rt.detail ?? "idle"}</span>
          )}
          <span className="resident-row__size numeric">
            {rt.vram_used_mb > 0 ? `${gb(rt.vram_used_mb)} GB` : "—"}
          </span>
          {rt.loaded_models.map((m) => (
            <UnloadButton key={m.model_id} modelId={m.model_id} onUnloaded={onUnloaded} />
          ))}
        </li>
      ))}
    </ul>
  );
}

function UnloadButton({ modelId, onUnloaded }: { modelId: string; onUnloaded: () => void }) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const click = async () => {
    setBusy(true);
    setErr(null);
    try {
      await unloadModel(modelId);
      onUnloaded();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <span className="resident-row__unload">
      <button
        type="button"
        className="resident-row__unload-btn"
        onClick={click}
        disabled={busy}
        title="Free this model's VRAM/RAM right now"
      >
        {busy ? "…" : "Unload"}
      </button>
      {err && <span className="resident-row__unload-err">{err}</span>}
    </span>
  );
}

function ActivityFeed({
  jobs,
  modelName,
}: {
  jobs: Job[];
  modelName: (id: string) => string;
}) {
  if (jobs.length === 0) return <p className="muted">Nothing finished yet this session.</p>;

  return (
    <ul className="activity-list">
      {jobs.map((j) => (
        <li key={j.id} className="activity-row">
          <span className="activity-row__time numeric">
            {new Date(j.finished_at ?? j.created_at).toLocaleTimeString()}
          </span>
          <span className="activity-row__text">
            <b>{j.job_type}</b>
            {j.model_id ? ` · ${modelName(j.model_id)}` : ""}
            {" · "}
            <span className="job-state" data-state={j.state}>
              {j.state}
            </span>
            {j.error_text ? ` — ${j.error_text}` : ""}
          </span>
        </li>
      ))}
    </ul>
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
