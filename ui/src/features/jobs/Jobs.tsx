import { useMemo, useState } from "react";
import { useJobs, useSessions } from "../../lib/hooks";
import { assistantKindOf, cancelJob, type Job, type JobState } from "../../lib/ipc";
import "./jobs.css";

const CANCELLABLE: JobState[] = ["queued", "scheduled", "blocked", "preparing", "running"];
const ACTIVE: JobState[] = ["queued", "scheduled", "blocked", "preparing", "running", "post"];

const JOB_TYPES = ["chat", "image", "video", "upscale", "bench", "upgrade_check"] as const;
type JobTypeFilter = "all" | (typeof JOB_TYPES)[number];

type StateFilter = "all" | "active" | "completed" | "failed" | "cancelled";

const STATE_FILTERS: { key: StateFilter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "active", label: "Active" },
  { key: "completed", label: "Completed" },
  { key: "failed", label: "Failed" },
  { key: "cancelled", label: "Cancelled" },
];

/** "chat" alone for a real conversation turn; a Prompt Assistant completion
 *  (drafting an image/video prompt) gets its own label so it reads as what
 *  it is, distinct from Chat's own history. */
function typeLabel(job: Job): string {
  const kind = assistantKindOf(job);
  return kind ? `${job.job_type} — ${kind} prompt` : job.job_type;
}

function matchesState(job: Job, filter: StateFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "active":
      return ACTIVE.includes(job.state);
    case "completed":
      return job.state === "completed";
    case "failed":
      return job.state === "failed";
    case "cancelled":
      return job.state === "cancelled";
  }
}

/** Every job across every capability -- Chat/Image/Video's own Queue lists
 *  only show their own job_type; this is the one place to see (and cancel)
 *  everything at once, including bench/upgrade-check jobs that have no tab of
 *  their own. */
export function Jobs() {
  const { data: jobs, refetch } = useJobs({ limit: 300 });
  const { data: chatSessions } = useSessions("chat");
  const { data: imageSessions } = useSessions("image");
  const { data: videoSessions } = useSessions("video");
  const [typeFilter, setTypeFilter] = useState<JobTypeFilter>("all");
  const [stateFilter, setStateFilter] = useState<StateFilter>("all");

  const sessionNames = useMemo(() => {
    const m = new Map<string, string>();
    for (const s of [...(chatSessions ?? []), ...(imageSessions ?? []), ...(videoSessions ?? [])]) {
      m.set(s.id, s.name);
    }
    return m;
  }, [chatSessions, imageSessions, videoSessions]);

  const filtered = (jobs ?? []).filter(
    (j) => (typeFilter === "all" || j.job_type === typeFilter) && matchesState(j, stateFilter),
  );

  return (
    <section className="card card--wide jobspage">
      <header className="card__head">
        <h2>Jobs</h2>
        <span className="card__sub numeric">{filtered.length}</span>
      </header>

      <div className="jobspage__filters">
        <select value={typeFilter} onChange={(e) => setTypeFilter(e.target.value as JobTypeFilter)}>
          <option value="all">All types</option>
          {JOB_TYPES.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
        <div className="jobspage__chips">
          {STATE_FILTERS.map((f) => (
            <button
              key={f.key}
              type="button"
              className="chip"
              aria-pressed={stateFilter === f.key}
              onClick={() => setStateFilter(f.key)}
            >
              {f.label}
            </button>
          ))}
        </div>
      </div>

      {!jobs ? (
        <p className="muted">Loading…</p>
      ) : filtered.length === 0 ? (
        <p className="muted">No jobs match this filter.</p>
      ) : (
        <table className="jobs">
          <thead>
            <tr>
              <th>Type</th>
              <th>Model</th>
              <th>Session</th>
              <th>State</th>
              <th>Created</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {filtered.map((j) => (
              <tr key={j.id}>
                <td>{typeLabel(j)}</td>
                <td className="muted">{j.model_id ?? "—"}</td>
                <td className="muted">
                  {j.session_id ? (sessionNames.get(j.session_id) ?? j.session_id) : "—"}
                </td>
                <td>
                  <span className="job-state" data-state={j.state}>
                    {j.state}
                  </span>
                </td>
                <td className="muted numeric">{new Date(j.created_at).toLocaleString()}</td>
                <td>
                  {CANCELLABLE.includes(j.state) && (
                    <CancelButton id={j.id} onDone={refetch} />
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
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
