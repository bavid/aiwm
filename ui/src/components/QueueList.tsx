import type { Job } from "../lib/ipc";
import "./queue-list.css";

interface QueueListProps {
  jobType: string;
  jobs: Job[];
  modelNames: Map<string, string>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onCancel: (id: string) => void;
  promptOf: (job: Job) => string;
}

/** Every non-terminal job of this tab's type (queued/running/blocked), across
 *  every session -- so a job started from a different session, or left
 *  waiting behind another tab's work, doesn't just disappear from view. Click
 *  a row to load it into the Result panel below. */
export function QueueList({
  jobType,
  jobs,
  modelNames,
  selectedId,
  onSelect,
  onCancel,
  promptOf,
}: QueueListProps) {
  const queued = jobs
    .filter((j) => j.job_type === jobType && ["queued", "running", "blocked"].includes(j.state))
    .sort((a, b) => a.created_at.localeCompare(b.created_at));

  return (
    <section className="card queue-list">
      <header className="card__head">
        <h2>Queue</h2>
        <span className="card__sub numeric">{queued.length}</span>
      </header>
      {queued.length === 0 ? (
        <p className="muted">Nothing queued right now.</p>
      ) : (
        <ul className="queue-list__rows">
          {queued.map((j) => (
            <li key={j.id}>
              <button
                type="button"
                className="queue-list__row"
                aria-pressed={j.id === selectedId}
                onClick={() => onSelect(j.id)}
              >
                <span className="status-dot" data-state={j.state === "running" ? "ok" : j.state === "blocked" ? "warn" : undefined} />
                <span className="queue-list__prompt">{promptOf(j) || "untitled"}</span>
                <span className="queue-list__meta">
                  {j.model_id ? (modelNames.get(j.model_id) ?? j.model_id) : "auto"} · {j.state}
                </span>
              </button>
              <button
                type="button"
                className="queue-list__cancel"
                onClick={() => onCancel(j.id)}
                aria-label={`Cancel ${promptOf(j) || "this job"}`}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
