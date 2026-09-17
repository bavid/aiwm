import type { Benchmark, BenchSuite } from "../../lib/ipc";
import { ResultCard } from "./ResultCard";

type Props = {
  /** The stored row for the finished run, once the history query has it. */
  bench: Benchmark | null;
  /** The suite that row was measured with, as far as this build knows it. */
  benchSuite: BenchSuite | null;
  modelName: string;
  /** The suite the finished run itself used; `null` = a suite-less quick test
   *  (the Model Library's own button). */
  runSuiteId: string | null;
  /** Its title, when this build still ships that version. */
  runSuiteTitle: string | null;
  /** The run's suite matches the one the form is showing, so the row can turn
   *  up in the history query the tab is running. When it does not, waiting is
   *  pointless — the query never returns that row. */
  resolvable: boolean;
  /** The row still had not arrived well after the job finished. */
  timedOut: boolean;
  /** Switch the form to the run's own suite, which makes it resolvable. */
  onShowRunSuite: () => void;
};

/** What sits where the result card goes, for a run that has finished but whose
 *  stored row may or may not be reachable from the current view. Every branch
 *  ends somewhere: none of them can leave a spinner running forever. */
export function ResultSlot({
  bench,
  benchSuite,
  modelName,
  runSuiteId,
  runSuiteTitle,
  resolvable,
  timedOut,
  onShowRunSuite,
}: Props) {
  // A quick test carries no suite at all, so no suite view can ever show it.
  // Its score lives in the Model Library, and saying so beats waiting.
  if (runSuiteId === null) {
    return (
      <p className="card bench__reading">
        That was a quick test without a test set — see its score in the Model Library.
      </p>
    );
  }

  // The run used a different suite than the one on screen (an adopted run, or
  // the form moved on). One click brings the view to it -- but only when that
  // suite still exists in this build's catalogue; offering to "show" a suite
  // that no longer ships would just land on an empty picker.
  if (!resolvable) {
    if (runSuiteTitle === null) {
      return (
        <p className="card bench__reading">
          That run used <code>{runSuiteId}</code>, which this version no longer ships.
        </p>
      );
    }
    return (
      <div className="card bench__reading bench__reading--action">
        <span>That run used {runSuiteTitle}.</span>
        <button type="button" className="chip" onClick={onShowRunSuite}>
          Show {runSuiteTitle}
        </button>
      </div>
    );
  }

  if (bench) {
    return <ResultCard bench={bench} suite={benchSuite} modelName={modelName} />;
  }

  if (timedOut) {
    return (
      <p className="card bench__reading">
        The result row has not appeared — check the Jobs tab.
      </p>
    );
  }

  return <p className="card bench__reading">Reading the results…</p>;
}
