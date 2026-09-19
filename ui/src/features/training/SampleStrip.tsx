import { useState } from "react";
import { trainingSampleUrl } from "../../lib/ipc";

type Props = {
  runId: string;
  /** For the images' alt text. */
  runName: string;
  /** Opaque tokens (`RunDetail.latest_samples` / `LineageRun.samples`),
   *  newest checkpoint first — never paths. */
  tokens: string[];
  /** `AboutInfo.core_api_port` — where the sample images are served from. */
  corePort: number | null;
};

/** A run's preview images in a row. A token whose image cannot be fetched
 *  (checkpoint still writing, dev mock) turns into a labelled placeholder
 *  rather than a broken-image icon. Shared by the run card and the LoRA
 *  history so the two never drift apart. */
export function SampleStrip({ runId, runName, tokens, corePort }: Props) {
  // Keyed by token, not position: a live run prepends newer samples, and a
  // positional flag would then land on the wrong image.
  const [broken, setBroken] = useState<ReadonlySet<string>>(() => new Set());

  if (tokens.length === 0) return null;

  return (
    <div className="runcard__samples">
      {tokens.map((token, i) =>
        broken.has(token) || corePort === null ? (
          <div key={token} className="runcard__sample runcard__sample--missing">
            no preview yet
          </div>
        ) : (
          <img
            key={token}
            className="runcard__sample"
            src={trainingSampleUrl(corePort, runId, i)}
            alt={`Preview ${i + 1} of ${runName}`}
            width={120}
            height={120}
            onError={() => setBroken((cur) => new Set(cur).add(token))}
          />
        ),
      )}
    </div>
  );
}
