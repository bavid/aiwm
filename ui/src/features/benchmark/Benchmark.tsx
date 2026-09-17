import { useEffect, useMemo, useState } from "react";
import { useBenchmarkHistory, useBenchSuites, useModels } from "../../lib/hooks";
import { benchmarkModel, cancelJob, jobDetail, type JobDetail, type JobState } from "../../lib/ipc";
import { BenchForm } from "./BenchForm";
import { Comparison } from "./Comparison";
import { History } from "./History";
import { LiveRun } from "./LiveRun";
import { ResultCard } from "./ResultCard";
import {
  DEFAULT_RUNS,
  MAX_RUNS,
  MIN_RUNS,
  chatModelsToBenchmark,
  jobPasses,
} from "./benchmark-utils";
import "./benchmark.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
/** Fast enough that the pass lines appear as they are measured, slow enough
 *  that a long suite run does not hammer the core. */
const POLL_MS = 700;

type Props = {
  /** The shell's tab switch, for the "no GGUF model" hand-over to Models. */
  onNavigate?: (tab: string) => void;
};

const clampRuns = (n: number) =>
  Number.isFinite(n) ? Math.min(MAX_RUNS, Math.max(MIN_RUNS, Math.round(n))) : DEFAULT_RUNS;

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The Benchmark tab: pick a model and a fixed test set, run it, and compare
 *  the tokens per second against earlier runs and other models. Speed only —
 *  nothing here judges what the model actually said. */
export function Benchmark({ onNavigate }: Props) {
  const { data: models } = useModels();
  const { data: suites } = useBenchSuites();

  const [modelChoice, setModelChoice] = useState("");
  const [suiteChoice, setSuiteChoice] = useState("");
  const [runs, setRuns] = useState(DEFAULT_RUNS);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  const benchmarkable = useMemo(() => chatModelsToBenchmark(models ?? []), [models]);
  const suiteList = useMemo(() => suites ?? [], [suites]);

  // Both selects fall back to the first option instead of an effect that
  // writes state once the poll arrives -- an explicit pick simply wins.
  const modelId = modelChoice || (benchmarkable[0]?.id ?? "");
  const suiteId = suiteChoice || (suiteList[0]?.id ?? "");
  const suite = suiteList.find((s) => s.id === suiteId) ?? null;

  const { data: history } = useBenchmarkHistory(suiteId || null);
  const rows = useMemo(() => history ?? [], [history]);

  const nameByModel = useMemo(() => {
    const map: Record<string, string> = {};
    for (const m of models ?? []) map[m.id] = m.name;
    return map;
  }, [models]);

  useEffect(() => {
    if (!pendingId) return;
    let alive = true;
    const tick = async () => {
      let d: JobDetail | null;
      try {
        d = await jobDetail(pendingId);
      } catch {
        return;
      }
      if (!alive || !d) return;
      setDetail(d);
      if (DONE.includes(d.job.state)) setPendingId(null);
    };
    void tick();
    const id = setInterval(() => void tick(), POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [pendingId]);

  const start = async () => {
    setError(null);
    try {
      const job = await benchmarkModel(modelId, { suite: suiteId, runs });
      setDetail({ job, events: [] });
      setPendingId(job.id);
    } catch (e) {
      setError(message(e));
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

  // The row `core::bench` wrote for the run that just finished here. It shows
  // up one history poll after the job flips to `completed`.
  const finishedJobId = detail?.job.state === "completed" ? detail.job.id : null;
  const resultRow = finishedJobId
    ? (rows.find((r) => r.job_id === finishedJobId) ?? null)
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
        suites={suiteList}
        modelId={modelId}
        suiteId={suiteId}
        runs={runs}
        busy={pendingId !== null}
        error={error}
        onModelChange={setModelChoice}
        onSuiteChange={setSuiteChoice}
        onRunsChange={(n) => setRuns(clampRuns(n))}
        onStart={() => void start()}
        onStop={() => void stop()}
        onGoToModels={onNavigate ? () => onNavigate("models") : null}
      />

      {detail && <LiveRun detail={detail} total={jobPasses(detail.job.params, suiteList)} />}

      {resultRow && (
        <ResultCard
          bench={resultRow}
          suite={resultSuite}
          modelName={nameByModel[resultRow.model_id] ?? "removed model"}
        />
      )}

      <Comparison
        rows={rows}
        nameByModel={nameByModel}
        suiteTitle={suite?.title ?? suiteId}
        highlightBenchId={resultRow?.id ?? null}
      />

      <History
        rows={historyRows}
        modelName={nameByModel[modelId] ?? null}
        suiteTitle={suite?.title ?? suiteId}
      />
    </section>
  );
}
