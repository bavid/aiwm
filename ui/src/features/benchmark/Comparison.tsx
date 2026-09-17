import type { Benchmark } from "../../lib/ipc";
import { barScale, fastestTps, formatTps, latestPerModel, stoppedEarly } from "./benchmark-utils";

type Props = {
  /** History already filtered to one suite — rows from different suites run at
   *  different token caps and must never share a bar chart. */
  rows: Benchmark[];
  /** model id -> display name; a missing id means the model was deleted. */
  nameByModel: Record<string, string>;
  suiteTitle: string;
  /** The run that just finished here, so it stands out among the others. */
  highlightBenchId: string | null;
};

/** One bar per model: its newest run on this suite, fastest on top. The bars
 *  are the comparison — the numbers beside them are the receipt. */
export function Comparison({ rows, nameByModel, suiteTitle, highlightBenchId }: Props) {
  const latest = latestPerModel(rows);
  const fastest = fastestTps(latest);

  if (latest.length === 0) {
    return (
      <section className="card bench__compare" aria-label="Model comparison">
        <h2>Comparison</h2>
        <p className="muted">
          Nothing measured with {suiteTitle} yet. Run it once and this fills in.
        </p>
      </section>
    );
  }

  return (
    <section className="card bench__compare" aria-label="Model comparison">
      <h2>Comparison</h2>
      <div className="bench__tablewrap">
        <table className="bench__table bench__table--bars">
          <caption>Newest run per model with {suiteTitle}, fastest first</caption>
          <thead>
            <tr>
              <th scope="col">Model</th>
              <th scope="col">Generation</th>
              <th scope="col">tok/s</th>
            </tr>
          </thead>
          <tbody>
            {latest.map((row) => (
              <tr key={row.id} data-current={row.id === highlightBenchId || undefined}>
                <th scope="row">
                  {nameByModel[row.model_id] ?? <em className="muted">removed model</em>}
                  {stoppedEarly(row) && (
                    <span className="bench__short" title="Stopped early — not comparable">
                      short
                    </span>
                  )}
                </th>
                <td>
                  <span className="bench__bar">
                    <span
                      className="bench__bar-fill"
                      style={{ transform: `scaleX(${barScale(row.gen_tps, fastest)})` }}
                    />
                  </span>
                </td>
                <td className="numeric bench__bar-num">{formatTps(row.gen_tps)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
