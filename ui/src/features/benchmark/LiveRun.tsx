import { useEffect, useRef } from "react";
import type { BenchSuite, JobDetail, JobState } from "../../lib/ipc";
import { formatTps, parsePassEvent, promptPosition } from "./benchmark-utils";

type Props = {
  detail: JobDetail;
  /** Prompts × passes for the suite this job is running. */
  total: number;
  /** The suite the *job* named, not the one the form shows — they differ for
   *  an adopted run (a reload, or a test started from the Model Library). */
  suite: BenchSuite | null;
  modelName: string;
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

/** Within this many pixels of the bottom still counts as "following along", so
 *  new lines keep scrolling into view; further up means the user is reading
 *  and must not be yanked back down. */
const STICK_PX = 24;

/** The panel that replaces "nothing is happening" while a run is in flight:
 *  where the job is, how far through the passes it is, what the last pass
 *  measured, and the engine's own event trail underneath. */
export function LiveRun({ detail, total, suite, modelName }: Props) {
  const { job, events } = detail;
  const passes = events.map((e) => parsePassEvent(e.message)).filter((p) => p !== null);
  const done = Math.min(passes.length, total || passes.length);
  const last = passes.at(-1) ?? null;
  const max = total || Math.max(1, passes.length);
  const at = promptPosition(suite, last?.promptTitle ?? null);

  const trailRef = useRef<HTMLOListElement>(null);
  const stick = useRef(true);

  useEffect(() => {
    const el = trailRef.current;
    if (!el || !stick.current) return;
    el.scrollTop = el.scrollHeight;
  }, [events.length]);

  const onTrailScroll = () => {
    const el = trailRef.current;
    if (el) stick.current = el.scrollHeight - el.scrollTop - el.clientHeight <= STICK_PX;
  };

  return (
    <section className="card bench__live" aria-label="Current benchmark run">
      <header className="bench__live-head">
        <h2 className="bench__live-title">
          <span className="bench__live-dot" data-state={job.state} />
          {/* One line, announced as a whole: a screen reader hears "generating
              — pass 4 of 6", not a stream of half-sentences. The event trail
              below stays out of it -- it would read every engine line aloud. */}
          <span className="bench__live-state numeric" aria-live="polite" aria-atomic="true">
            {STATE_TEXT[job.state]} — pass {done} of {total || "?"}
          </span>
        </h2>
        <span className="bench__live-target">
          {modelName}
          {suite && ` · ${suite.title}`}
        </span>
        {last && (
          <span className="bench__live-last numeric">
            {at && `prompt ${at.index}/${at.of}, `}pass {last.pass}/{last.runs} —{" "}
            {formatTps(last.tps)} tok/s
          </span>
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

      {/* Focusable so the trail can be scrolled with the keyboard alone -- it
          is a scroll container with no other focusable content in it. */}
      <ol
        className="bench__trail"
        ref={trailRef}
        onScroll={onTrailScroll}
        tabIndex={0}
        aria-label="Engine events"
      >
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
