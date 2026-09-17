import { useId } from "react";
import type { BenchSuite, Model } from "../../lib/ipc";
import { MAX_RUNS, MIN_RUNS, totalPasses } from "./benchmark-utils";

type Props = {
  /** Benchmarkable *and* marked as chat models — what a run can use. */
  models: Model[];
  /** How many GGUF models the library holds at all, chat-marked or not. It is
   *  what tells "import something" apart from "fix a role". */
  ggufCount: number;
  suites: BenchSuite[];
  modelId: string;
  suiteId: string;
  /** Raw field text, so clearing it leaves an empty box rather than a 1. */
  runsText: string;
  /** What that text means once parsed and clamped — what a run would use. */
  runs: number;
  /** A bench job started (or adopted) by this tab is still in flight, or one
   *  is being queued right now. */
  busy: boolean;
  /** What went wrong queueing, cancelling or watching, if anything. */
  error: string | null;
  onModelChange: (id: string) => void;
  onSuiteChange: (id: string) => void;
  onRunsTextChange: (text: string) => void;
  /** Blur/submit: normalize the field to the value a run would actually use. */
  onRunsCommit: () => void;
  onStart: () => void;
  onStop: () => void;
  /** Opens the Models tab so an empty library can be filled. `null` when the
   *  shell exposes no such hand-over — then the empty state is plain text. */
  onGoToModels: (() => void) | null;
};

/** "Pick a model, pick a suite, say how many passes" — the whole input surface
 *  of the tab. Presentational: every value and handler comes from the
 *  container. */
export function BenchForm({
  models,
  ggufCount,
  suites,
  modelId,
  suiteId,
  runsText,
  runs,
  busy,
  error,
  onModelChange,
  onSuiteChange,
  onRunsTextChange,
  onRunsCommit,
  onStart,
  onStop,
  onGoToModels,
}: Props) {
  const ids = useId();
  const modelField = `${ids}-model`;
  const suiteField = `${ids}-suite`;
  const runsField = `${ids}-runs`;

  const suite = suites.find((s) => s.id === suiteId) ?? null;
  const passes = totalPasses(suite, runs);
  const ready = modelId !== "" && suite !== null;

  if (models.length === 0) return <EmptyLibrary ggufCount={ggufCount} onGoToModels={onGoToModels} />;

  return (
    <form
      className="card bench__form"
      onSubmit={(e) => {
        e.preventDefault();
        onRunsCommit();
        if (ready && !busy) onStart();
      }}
    >
      <div className="bench__fields">
        <label className="bench__field" htmlFor={modelField}>
          <span>Model</span>
          <select id={modelField} value={modelId} onChange={(e) => onModelChange(e.target.value)}>
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
        </label>

        <label className="bench__field" htmlFor={suiteField}>
          <span>Test set</span>
          <select id={suiteField} value={suiteId} onChange={(e) => onSuiteChange(e.target.value)}>
            {suites.length === 0 && <option value="">Loading…</option>}
            {suites.map((s) => (
              <option key={s.id} value={s.id}>
                {s.title}
              </option>
            ))}
          </select>
        </label>

        <label className="bench__field bench__field--runs" htmlFor={runsField}>
          <span>Passes per prompt</span>
          <input
            id={runsField}
            type="number"
            className="numeric"
            min={MIN_RUNS}
            max={MAX_RUNS}
            step={1}
            value={runsText}
            onChange={(e) => onRunsTextChange(e.target.value)}
            onBlur={onRunsCommit}
          />
        </label>
      </div>

      {suite && (
        <div className="bench__suite">
          <p className="bench__suite-desc">{suite.description}</p>
          <p className="bench__suite-meta numeric">
            {suite.prompts.length} prompts × {runs} = <strong>{passes} passes</strong> ·{" "}
            {suite.max_tokens} tokens each
          </p>
          <details className="bench__prompts">
            <summary>Prompts in {suite.title}</summary>
            <ul>
              {suite.prompts.map((p) => (
                <li key={p.id}>
                  <details>
                    <summary>{p.title}</summary>
                    <p>{p.text}</p>
                  </details>
                </li>
              ))}
            </ul>
          </details>
        </div>
      )}

      <div className="bench__actions">
        <button type="submit" className="bench__go" disabled={!ready || busy}>
          {busy ? "Running…" : "Start benchmark"}
        </button>
        {busy && (
          <button type="button" className="chip" onClick={onStop}>
            Stop
          </button>
        )}
        {!ready && !busy && <span className="muted">Waiting for the suite catalogue…</span>}
      </div>

      {error && <p className="bench__err">{error}</p>}
    </form>
  );
}

/** Nothing to benchmark — but for two quite different reasons, and the fix
 *  differs with them: download a model, or fix a role on one already here. */
function EmptyLibrary({
  ggufCount,
  onGoToModels,
}: {
  ggufCount: number;
  onGoToModels: (() => void) | null;
}) {
  const hasGguf = ggufCount > 0;
  return (
    <div className="card bench__empty">
      <h2>
        {hasGguf
          ? "You have GGUF models, but none is marked as a chat model"
          : "No GGUF chat model yet — get one in Discover"}
      </h2>
      <p>
        {hasGguf
          ? "Benchmarking measures chat/coding throughput, so it only offers models carrying the chat role. Set that role in the Model Library and the model appears here."
          : "Benchmarking runs on llama.cpp, so it needs a .gguf chat model in the library. Import one, or pick a recommendation on the Models tab."}
      </p>
      {onGoToModels && (
        <button type="button" className="bench__go" onClick={onGoToModels}>
          {hasGguf ? "Open the Model Library" : "Open Models → Discover"}
        </button>
      )}
    </div>
  );
}
