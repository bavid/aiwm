import { useEffect, useRef, useState } from "react";
import {
  benchmarkModel,
  cancelJob,
  jobDetail,
  type Job,
  type JobDetail,
  type JobState,
} from "../../lib/ipc";
import { liveBenchJob } from "./benchmark-utils";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
/** Fast enough that the pass lines appear as they are measured, slow enough
 *  that a long suite run does not hammer the core. */
const POLL_MS = 700;
/** How many polls in a row may fail before the tab stops waiting on a job it
 *  evidently cannot reach, and hands the user their form back. */
const MAX_POLL_FAILURES = 5;

const GONE_MESSAGE = "The benchmark job is no longer available.";
const UNREACHABLE_MESSAGE =
  "Lost contact with the core while watching this benchmark — it may still be running. Check the Jobs tab.";

export type BenchJobWatch = {
  /** The run on screen: in flight, or the last one that finished. */
  detail: JobDetail | null;
  /** A run is being queued or is still going — Start must stay disabled. */
  busy: boolean;
  error: string | null;
  setError: (text: string | null) => void;
  /** Queue a suite run and start watching it. Safe to call twice. */
  start: (modelId: string, suite: string, runs: number) => Promise<void>;
  /** Ask the watched job to stop. */
  stop: () => Promise<void>;
  /** Take a finished run off the screen. A live one is never dropped. */
  clearDetail: () => void;
};

/** Owns the whole life of the one bench job this tab is looking at: queueing
 *  it, polling it, adopting one it did not start, and — the part that is easy
 *  to forget — letting go of one it can no longer reach.
 *
 *  `jobs` is the active-job poll the caller already runs; `onFinished` is
 *  called once when a run completes, so the caller can refresh what the run
 *  wrote. It must be stable across renders. */
export function useBenchJob(jobs: Job[] | null, onFinished: () => void): BenchJobWatch {
  /** Set synchronously by `start()`, before the `await` — without it two fast
   *  clicks both see `pendingId === null` and queue two jobs. */
  const [starting, setStarting] = useState(false);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** A job the tab gave up on. `detail` normally keeps adoption from picking
   *  the same job straight back up, but `detail` can be cleared — this cannot. */
  const releasedId = useRef<string | null>(null);

  // Adopt a run this tab did not start, or lost: a reload mid-run, or the
  // Model Library's own "Test model" button.
  const adoptable = liveBenchJob(jobs);
  useEffect(() => {
    if (pendingId || starting || !adoptable) return;
    // Already on screen (typically the run that just finished): the jobs poll
    // can lag a state change by a tick, and re-adopting would loop.
    if (detail?.job.id === adoptable.id) return;
    // Explicitly given up on; picking it back up would undo the release.
    if (releasedId.current === adoptable.id) return;
    setDetail({ job: adoptable, events: [] });
    setPendingId(adoptable.id);
  }, [adoptable, pendingId, starting, detail]);

  useEffect(() => {
    if (!pendingId) return;
    let alive = true;
    let failures = 0;
    const watched = pendingId;
    const release = (text: string) => {
      releasedId.current = watched;
      setPendingId(null);
      setError(text);
    };
    const tick = async () => {
      let d: JobDetail | null;
      try {
        d = await jobDetail(watched);
      } catch {
        failures += 1;
        // Transient IPC hiccups are normal; a run of them is not, and waiting
        // forever would leave Start disabled with no way back.
        if (alive && failures >= MAX_POLL_FAILURES) release(UNREACHABLE_MESSAGE);
        return;
      }
      if (!alive) return;
      failures = 0;
      // The job is gone (deleted, or a core that lost its history) -- without
      // this the tab waits on it for the rest of the session.
      if (!d) {
        release(GONE_MESSAGE);
        return;
      }
      setDetail(d);
      if (DONE.includes(d.job.state)) {
        setPendingId(null);
        onFinished();
      }
    };
    void tick();
    const id = setInterval(() => void tick(), POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [pendingId, onFinished]);

  const busy = starting || pendingId !== null;

  const start = async (modelId: string, suite: string, runs: number) => {
    if (busy || modelId === "" || suite === "") return;
    setStarting(true);
    setError(null);
    try {
      const job = await benchmarkModel(modelId, { suite, runs });
      setDetail({ job, events: [] });
      setPendingId(job.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setStarting(false);
    }
  };

  const stop = async () => {
    if (!pendingId) return;
    setError(null);
    try {
      await cancelJob(pendingId);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const clearDetail = () => {
    if (!busy) setDetail(null);
  };

  return { detail, busy, error, setError, start, stop, clearDetail };
}
