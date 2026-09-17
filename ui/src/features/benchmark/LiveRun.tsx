import type { JobDetail, JobState } from "../../lib/ipc";
import { formatTps, parsePassEvent } from "./benchmark-utils";

type Props = {
  detail: JobDetail;
  /** Prompts × passes for the suite this job is running. */
  total: number;
};

/** What each job state means for a benchmark, in the tab's own words — the
 *  raw state name alone ("post", "blocked") explains nothing. */
const STATE_TEXT: Record<JobState, string> = {
  queued: "queued",
  scheduled: "waiting for a slot",
  blocked: "waiting for VRAM to free up",
  preparing: "loading the model",
  running: "generating",
  post: "scoring",
  completed: "done",
  failed: "failed",
  cancelled: "stopped",
};

/** The panel that replaces "nothing is happening" while a run is in flight:
 *  where the job is, how far through the passes it is, what the last pass
 *  measured, and the engine's own event trail underneath. */
export function LiveRun({ detail, total }: Props) {
  const { job, events } = detail;
  const passes = events.map((e) => parsePassEvent(e.message)).filter((p) => p !== null);
  const done = Math.min(passes.length, total || passes.length);
  const last = passes.at(-1) ?? null;
  const max = total || Math.max(1, passes.length);

  return (
    <section className="card bench__live" aria-label="Current benchmark run">
      <header className="bench__live-head">
        <h2>
          <span className="bench__live-dot" data-state={job.state} />
          {STATE_TEXT[job.state]}
        </h2>
        <span className="bench__live-count numeric">
          pass {done} / {total || "?"}
        </span>
        {last && (
          <span className="bench__live-last numeric">{formatTps(last.tps)} tok/s last pass</span>
        )}
      </header>

      <progress
        className="bench__progress"
        value={done}
        max={max}
        aria-label={`Benchmark progress: ${done} of ${total || max} passes`}
      />

      {job.state === "blocked" && (
        <p className="bench__warn">
          Not enough free VRAM right now. The job stays queued until another model unloads — or
          stop it and free something yourself.
        </p>
      )}
      {job.error_text && <p className="bench__err">{job.error_text}</p>}

      <ol className="bench__trail">
        {/* Keyed by position: the trail is append-only, and event timestamps
            are not unique (several passes can land inside one second). */}
        {events.map((e, i) => (
          <li key={i} data-level={e.level}>
            {e.message}
          </li>
        ))}
      </ol>
    </section>
  );
}
