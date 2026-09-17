import { useEffect, useMemo, useState } from "react";
import { useBenchmarkHistory, useBenchSuites, useJobs, useModels } from "../../lib/hooks";
import { ACTIVE_JOB_STATES, isBenchmarkable } from "../../lib/ipc";
import { BenchForm } from "./BenchForm";
import { Comparison } from "./Comparison";
import { History } from "./History";
import { LiveRun } from "./LiveRun";
import { ResultSlot } from "./ResultSlot";
import { useBenchJob } from "./use-bench-job";
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

/** How long a finished run's stored row may take to show up before the tab
 *  says so rather than waiting on it silently. */
const RESULT_WAIT_MS = 15_000;

type Props = {
  /** The shell's tab switch, for the empty states' hand-over to Models. */
  onNavigate?: (tab: string) => void;
};

/** The Benchmark tab: pick a model and a fixed test set, run it, and compare
 *  the tokens per second against earlier runs and other models. Speed only —
 *  nothing here judges what the model actually said. */
export function Benchmark({ onNavigate }: Props) {
  const { data: models } = useModels();
  const { data: suites } = useBenchSuites();
  const { data: jobs } = useJobs({ states: ACTIVE_JOB_STATES });

  const [modelChoice, setModelChoice] = useState("");
  const [suiteChoice, setSuiteChoice] = useState("");
  /** Raw text, not a number: clearing the field must not snap back to 1. */
  const [runsText, setRunsText] = useState(String(DEFAULT_RUNS));
  const [resultTimedOut, setResultTimedOut] = useState(false);

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

  // Queueing, polling, adopting and giving up on the one job this tab watches.
  const job = useBenchJob(jobs, refetchHistory);
  const { detail } = job;

  const nameByModel = useMemo(() => {
    const map: Record<string, string> = {};
    for (const m of models ?? []) map[m.id] = m.name;
    return map;
  }, [models]);

  const nameOf = (id: string | null) =>
    id ? (nameByModel[id] ?? "removed model") : "removed model";

  /** A finished run on screen belongs to the selection that produced it; once
   *  the user picks something else it is answering a different question, so it
   *  goes. A run still in flight always stays. */
  const dropStaleRun = (nextModelId: string, nextSuiteId: string) => {
    if (!detail) return;
    const sameModel = detail.job.model_id === nextModelId;
    const sameSuite = (jobSuiteId(detail.job.params) ?? "") === nextSuiteId;
    if (!sameModel || !sameSuite) job.clearDetail();
  };

  // The run being shown, and whether its stored row can turn up here at all.
  // `rows` is the history for the suite the *form* shows: a run on another
  // suite (adopted after a reload) or a suite-less quick test is simply not in
  // that query, and waiting for it would never end.
  const finishedJob = detail && detail.job.state === "completed" ? detail.job : null;
  const runSuiteId = finishedJob ? jobSuiteId(finishedJob.params) : null;
  const resolvable = finishedJob !== null && runSuiteId === (suiteId || null);
  const resultRow =
    finishedJob && resolvable ? (rows.find((r) => r.job_id === finishedJob.id) ?? null) : null;
  const resultSuite = resultRow
    ? (suiteList.find((s) => s.id === resultRow.suite) ?? null)
    : null;

  const finishedJobId = finishedJob?.id ?? null;
  const hasResult = resultRow !== null;
  useEffect(() => {
    setResultTimedOut(false);
    if (!finishedJobId || !resolvable || hasResult) return;
    const id = setTimeout(() => setResultTimedOut(true), RESULT_WAIT_MS);
    return () => clearTimeout(id);
  }, [finishedJobId, resolvable, hasResult]);

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
        busy={job.busy}
        error={job.error}
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
        onStart={() => void job.start(modelId, suiteId, runs)}
        onStop={() => void job.stop()}
        onGoToModels={onNavigate ? () => onNavigate("models") : null}
      />

      {detail && (
        <LiveRun
          detail={detail}
          total={jobPasses(detail.job.params, suiteList)}
          suite={jobSuite(detail.job.params, suiteList)}
          modelName={nameOf(detail.job.model_id)}
        />
      )}

      {finishedJob && (
        <ResultSlot
          key={finishedJob.id}
          bench={resultRow}
          benchSuite={resultSuite}
          modelName={nameOf(resultRow?.model_id ?? finishedJob.model_id)}
          runSuiteId={runSuiteId}
          runSuiteTitle={suiteList.find((s) => s.id === runSuiteId)?.title ?? null}
          resolvable={resolvable}
          timedOut={resultTimedOut}
          // Straight to the run's own suite: going through `onSuiteChange`
          // would run `dropStaleRun`, which is for navigating *away* from a
          // finished run -- here the whole point is to navigate towards it.
          onShowRunSuite={() => runSuiteId && setSuiteChoice(runSuiteId)}
        />
      )}

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
