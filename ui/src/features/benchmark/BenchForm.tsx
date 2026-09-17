import { useId } from "react";
import type { BenchSuite, Model } from "../../lib/ipc";
import { MAX_RUNS, MIN_RUNS, totalPasses } from "./benchmark-utils";

type Props = {
  /** Already filtered to what the core will accept (GGUF / llama.cpp). */
  models: Model[];
  suites: BenchSuite[];
  modelId: string;
  suiteId: string;
  runs: number;
  /** A bench job started here is still queued or running. */
  busy: boolean;
  /** What went wrong queueing or cancelling, if anything. */
  error: string | null;
  onModelChange: (id: string) => void;
  onSuiteChange: (id: string) => void;
  onRunsChange: (runs: number) => void;
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
  suites,
  modelId,
  suiteId,
  runs,
  busy,
  error,
  onModelChange,
  onSuiteChange,
  onRunsChange,
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

  if (models.length === 0) {
    return (
      <div className="card bench__empty">
        <h2>No GGUF chat model yet — get one in Discover</h2>
        <p>
          Benchmarking runs on llama.cpp, so it needs a <code>.gguf</code> chat model in the
          library. Import one, or pick a recommendation on the Models tab.
        </p>
        {onGoToModels && (
          <button type="button" className="bench__go" onClick={onGoToModels}>
            Open Models → Discover
          </button>
        )}
      </div>
    );
  }

  return (
    <form
      className="card bench__form"
      onSubmit={(e) => {
        e.preventDefault();
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
            value={runs}
            onChange={(e) => onRunsChange(Number(e.target.value))}
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
