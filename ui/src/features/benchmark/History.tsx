import type { Benchmark } from "../../lib/ipc";
import { ShortMark } from "./ShortMark";
import {
  formatRoundTps,
  formatStability,
  formatTps,
  formatWhen,
  stoppedEarly,
} from "./benchmark-utils";

type Props = {
  /** The selected model's runs on the selected suite, newest first. */
  rows: Benchmark[];
  modelName: string | null;
  suiteTitle: string;
};

/** "Is this machine still as fast as it was?" — the selected model's own runs
 *  over time, rather than a comparison against other models. */
export function History({ rows, modelName, suiteTitle }: Props) {
  return (
    <section className="card bench__history" aria-label="Run history">
      <h2>History</h2>
      {rows.length === 0 ? (
        <p className="muted">
          {modelName
            ? `No ${suiteTitle} run recorded for ${modelName} yet.`
            : "Pick a model to see its earlier runs."}
        </p>
      ) : (
        <div className="bench__tablewrap">
          <table className="bench__table">
            <caption>
              {modelName} · {suiteTitle}, newest first
            </caption>
            <thead>
              <tr>
                <th scope="col">When</th>
                <th scope="col">tok/s</th>
                <th scope="col">Prefill</th>
                <th scope="col">Stability</th>
                <th scope="col">Passes</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.id}>
                  <th scope="row" className="numeric">
                    {formatWhen(row.created_at)}
                    {stoppedEarly(row) && <ShortMark />}
                  </th>
                  <td className="numeric">{formatTps(row.gen_tps)}</td>
                  <td className="numeric">{formatRoundTps(row.prompt_tps)}</td>
                  <td className="numeric">{formatStability(row.stability_score)}</td>
                  <td className="numeric">{row.runs}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
