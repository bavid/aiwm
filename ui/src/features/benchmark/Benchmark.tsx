import { useEffect, useMemo, useState } from "react";
import { useBenchmarkHistory, useBenchSuites, useJobs, useModels } from "../../lib/hooks";
import {
  benchmarkModel,
  cancelJob,
  isBenchmarkable,
  isJobActive,
  jobDetail,
  type Job,
  type JobDetail,
  type JobState,
} from "../../lib/ipc";
import { BenchForm } from "./BenchForm";
import { Comparison } from "./Comparison";
import { History } from "./History";
import { LiveRun } from "./LiveRun";
import { ResultCard } from "./ResultCard";
import {
  DEFAULT_RUNS,
  HISTORY_LIMIT,
  chatModelsToBenchmark,
  jobPasses,
  jobSuite,
  jobSuiteId,
  parseRuns,
} from "./benchmark-utils";
import "./benchmark.css";

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

type Props = {
  /** The shell's tab switch, for the empty states' hand-over to Models. */
  onNavigate?: (tab: string) => void;
};

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The newest still-unfinished `bench` job, whoever started it. */
function liveBenchJob(jobs: Job[] | null): Job | null {
  return (jobs ?? [])
    .filter((j) => j.job_type === "bench" && isJobActive(j.state))
    .reduce<Job | null>((newest, j) => (!newest || j.created_at > newest.created_at ? j : newest), null);
}

/** The Benchmark tab: pick a model and a fixed test set, run it, and compare
 *  the tokens per second against earlier runs and other models. Speed only —
 *  nothing here judges what the model actually said. */
export function Benchmark({ onNavigate }: Props) {
  const { data: models } = useModels();
  const { data: suites } = useBenchSuites();
  const { data: jobs } = useJobs();

  const [modelChoice, setModelChoice] = useState("");
  const [suiteChoice, setSuiteChoice] = useState("");
  /** Raw text, not a number: clearing the field must not snap back to 1. */
  const [runsText, setRunsText] = useState(String(DEFAULT_RUNS));
  /** Set synchronously by `start()`, before the `await` — without it two fast
   *  clicks both see `pendingId === null` and queue two jobs. */
  const [starting, setStarting] = useState(false);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  const benchmarkable = useMemo(() => chatModelsToBenchmark(models ?? []), [models]);
  const ggufCount = useMemo(() => (models ?? []).filter(isBenchmarkable).length, [models]);
  const suiteList = useMemo(() => suites ?? [], [suites]);

  // Both selects fall back to the first option instead of an effect that
  // writes state once the poll arrives -- an explicit pick simply wins.
  const modelId = modelChoice || (benchmarkable[0]?.id ?? "");
  const suiteId = suiteChoice || (suiteList[0]?.id ?? "");
  const suite = suiteList.find((s) => s.id === suiteId) ?? null;
  const runs = parseRuns(runsText);

  const { data: history, refetch: refetchHistory } = useBenchmarkHistory(
    suiteId || null,
    HISTORY_LIMIT,
  );
  const rows = useMemo(() => history ?? [], [history]);

  const nameByModel = useMemo(() => {
    const map: Record<string, string> = {};
    for (const m of models ?? []) map[m.id] = m.name;
    return map;
  }, [models]);

  // Adopt a run this tab did not start, or lost: a reload mid-run, or the
  // Model Library's own "Test model" button. Showing it is all that happens --
  // the form keeps the user's own selection, and the live panel names the
  // model and suite the adopted job actually uses.
  const adoptable = liveBenchJob(jobs);
  useEffect(() => {
    if (pendingId || !adoptable) return;
    // Already on screen (typically the run that just finished): the jobs poll
    // can lag a state change by a tick, and re-adopting would loop.
    if (detail?.job.id === adoptable.id) return;
    setDetail({ job: adoptable, events: [] });
    setPendingId(adoptable.id);
  }, [adoptable, pendingId, detail]);

  useEffect(() => {
    if (!pendingId) return;
    let alive = true;
    let failures = 0;
    const release = (text: string) => {
      setPendingId(null);
      setError(text);
    };
    const tick = async () => {
      let d: JobDetail | null;
      try {
        d = await jobDetail(pendingId);
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
        // The row lands a moment after the job flips; ask for it now rather
        // than sitting on the "reading the results…" placeholder for a poll.
        refetchHistory();
      }
    };
    void tick();
    const id = setInterval(() => void tick(), POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [pendingId, refetchHistory]);

  const busy = starting || pendingId !== null;

  const start = async () => {
    if (busy || modelId === "" || suite === null) return;
    setStarting(true);
    setError(null);
    try {
      const job = await benchmarkModel(modelId, { suite: suiteId, runs });
      setDetail({ job, events: [] });
      setPendingId(job.id);
    } catch (e) {
      setError(message(e));
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
      setError(message(e));
    }
  };

  /** A finished run on screen belongs to the selection that produced it; once
   *  the user picks something else it is answering a different question, so it
   *  goes. A run still in flight always stays. */
  const dropStaleRun = (nextModelId: string, nextSuiteId: string) => {
    if (pendingId || !detail) return;
    const sameModel = detail.job.model_id === nextModelId;
    const sameSuite = (jobSuiteId(detail.job.params) ?? "") === nextSuiteId;
    if (!sameModel || !sameSuite) setDetail(null);
  };

  // The row `core::bench` wrote for the run being shown. Between the job
  // flipping to `completed` and that row arriving there is a real gap, and the
  // card says so instead of silently not being there.
  const finishedJob = detail && detail.job.state === "completed" ? detail.job : null;
  const resultRow = finishedJob
    ? (rows.find((r) => r.job_id === finishedJob.id) ?? null)
    : null;
  const resultSuite = resultRow
    ? (suiteList.find((s) => s.id === resultRow.suite) ?? null)
    : null;

  const historyRows = rows.filter((r) => r.model_id === modelId);

  return (
    <section className="bench" aria-label="Benchmark">
      <header className="bench__head">
        <h1>Benchmark</h1>
        <p className="bench__note">
          Measures speed on this machine, not answer quality. Every pass generates exactly the
          same number of tokens, so runs are comparable.
        </p>
      </header>

      <BenchForm
        models={benchmarkable}
        ggufCount={ggufCount}
        suites={suiteList}
        modelId={modelId}
        suiteId={suiteId}
        runsText={runsText}
        runs={runs}
        busy={busy}
        error={error}
        onModelChange={(id) => {
          setModelChoice(id);
          dropStaleRun(id, suiteId);
        }}
        onSuiteChange={(id) => {
          setSuiteChoice(id);
          dropStaleRun(modelId, id);
        }}
        onRunsTextChange={setRunsText}
        onRunsCommit={() => setRunsText(String(runs))}
        onStart={() => void start()}
        onStop={() => void stop()}
        onGoToModels={onNavigate ? () => onNavigate("models") : null}
      />

      {detail && (
        <LiveRun
          detail={detail}
          total={jobPasses(detail.job.params, suiteList)}
          suite={jobSuite(detail.job.params, suiteList)}
          modelName={detail.job.model_id ? (nameByModel[detail.job.model_id] ?? "removed model") : "—"}
        />
      )}

      {finishedJob &&
        (resultRow ? (
          <ResultCard
            key={finishedJob.id}
            bench={resultRow}
            suite={resultSuite}
            modelName={nameByModel[resultRow.model_id] ?? "removed model"}
          />
        ) : (
          <p className="card bench__reading">Reading the results…</p>
        ))}

      <Comparison
        rows={rows}
        nameByModel={nameByModel}
        suiteTitle={suite?.title ?? suiteId}
        highlightBenchId={resultRow?.id ?? null}
        windowed={rows.length >= HISTORY_LIMIT}
      />

      <History
        rows={historyRows}
        modelName={nameByModel[modelId] ?? null}
        suiteTitle={suite?.title ?? suiteId}
      />
    </section>
  );
}
