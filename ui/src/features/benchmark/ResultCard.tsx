import type { Benchmark, BenchSuite } from "../../lib/ipc";
import {
  formatLoad,
  formatRoundTps,
  formatStability,
  formatTps,
  formatVram,
  isShort,
  promptTitle,
  stoppedEarly,
} from "./benchmark-utils";

type Props = {
  bench: Benchmark;
  /** The suite this run used, when this build still ships that version. */
  suite: BenchSuite | null;
  modelName: string;
};

/** The finished run: one big number, the honest supporting figures around it,
 *  and the per-prompt breakdown underneath. */
export function ResultCard({ bench, suite, modelName }: Props) {
  const early = stoppedEarly(bench);

  return (
    <section className="card bench__result" aria-label="Latest result">
      <div className="bench__hero">
        <p className="bench__hero-num numeric">{formatTps(bench.gen_tps)}</p>
        <p className="bench__hero-unit">
          tok/s generated
          <em>
            {modelName} · {suite?.title ?? bench.suite ?? "quick test"}
          </em>
        </p>
      </div>

      <dl className="bench__stats">
        <div>
          <dt>Prefill</dt>
          <dd className="numeric">{formatRoundTps(bench.prompt_tps)} tok/s</dd>
        </div>
        <div>
          <dt>Load time</dt>
          <dd className="numeric">{formatLoad(bench.load_ms)}</dd>
        </div>
        <div>
          <dt>VRAM peak</dt>
          <dd className="numeric">{formatVram(bench.vram_peak_mb)}</dd>
        </div>
        <div>
          <dt>Stability</dt>
          <dd className="numeric">{formatStability(bench.stability_score)}</dd>
        </div>
        <div>
          <dt>Passes</dt>
          <dd className="numeric">{bench.runs}</dd>
        </div>
      </dl>

      {/* No `role="status"`: this text is part of the card from the moment it
          renders, not something that appears later, and a status region would
          have it announced over whatever the reader was on. */}
      {early && (
        <p className="bench__warn">
          At least one prompt stopped before the token cap, so this run generated fewer tokens
          than a full one — the rate is not comparable with the other runs.
        </p>
      )}

      {bench.detail && bench.detail.length > 0 && (
        <div className="bench__tablewrap">
          <table className="bench__table">
            <caption>Per prompt, averaged over that prompt&rsquo;s passes</caption>
            <thead>
              <tr>
                <th scope="col">Prompt</th>
                <th scope="col">Tokens</th>
                <th scope="col">tok/s</th>
                <th scope="col">Prefill</th>
              </tr>
            </thead>
            <tbody>
              {bench.detail.map((d) => (
                <tr key={d.prompt_id} data-short={isShort(d) || undefined}>
                  <th scope="row">{promptTitle(suite, d.prompt_id)}</th>
                  <td className="numeric">
                    {d.tokens} / {d.max_tokens}
                  </td>
                  <td className="numeric">{formatTps(d.gen_tps)}</td>
                  <td className="numeric">{formatRoundTps(d.prompt_tps)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
