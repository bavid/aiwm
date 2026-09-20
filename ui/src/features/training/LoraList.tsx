import { useMemo } from "react";
import { HelpHint } from "../../components/HelpHint";
import type { LoraSummary } from "../../lib/ipc";
import { formatBytes } from "../../lib/units";
import { formatCount, formatWhen } from "./format";

/** Spec `2026-09-19-lora-lineage-design`, "Data-amount guidance": verbatim. */
const DATA_GUIDANCE =
  "A LoRA typically needs a few hundred to a few thousand well-chosen images; the pipeline " +
  "samples frames (default 1.5 fps) and filters blur and duplicates.";

type Props = {
  loras: LoraSummary[];
  /** `null` while the list has not answered yet. */
  isLoading: boolean;
  error: string | null;
  selectedId: string | null;
  onSelect: (modelId: string) => void;
  /** Opens the confirmation for removing this LoRA from the library. */
  onDelete: (lora: LoraSummary) => void;
};

/** "Your LoRAs": every library LoRA with what its lineage adds up to, newest
 *  first. Picking one opens its history below; the row's name button carries
 *  the pressed state so a keyboard user hears which one is open. */
export function LoraList({ loras, isLoading, error, selectedId, onSelect, onDelete }: Props) {
  const ordered = useMemo(
    () => [...loras].sort((a, b) => b.created_at.localeCompare(a.created_at)),
    [loras],
  );

  return (
    <section className="card loralist" aria-labelledby="loralist-heading">
      <header className="loralist__head">
        <div className="training__headrow">
          <h3 id="loralist-heading">Your LoRAs</h3>
          <HelpHint area="training" setting="lora-overview" />
        </div>
        <p className="loralist__guidance">{DATA_GUIDANCE}</p>
      </header>

      {error && <p className="training__err">{error}</p>}

      {ordered.length === 0 ? (
        <p className="muted">
          {isLoading
            ? "Looking for LoRAs in the library…"
            : "No LoRA in the library yet — a completed run puts one here, and so does importing a file on the Models tab."}
        </p>
      ) : (
        <table className="loralist__table">
          <thead>
            <tr>
              <th scope="col">Name</th>
              <th scope="col">Family</th>
              <th scope="col" className="loralist__num">
                Rank
              </th>
              <th scope="col" className="loralist__num">
                Size
              </th>
              <th scope="col">Added</th>
              <th scope="col" className="loralist__num">
                Runs
              </th>
              <th scope="col" className="loralist__num">
                Steps
              </th>
              <th scope="col" className="loralist__num">
                Images
              </th>
              <th scope="col">
                <span className="visually-hidden">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {ordered.map((l) => {
              const isSelected = l.model_id === selectedId;
              return (
                <tr key={l.model_id} aria-current={isSelected ? "true" : undefined}>
                  <th scope="row">
                    <button
                      type="button"
                      className="loralist__name"
                      aria-pressed={isSelected}
                      onClick={() => onSelect(l.model_id)}
                    >
                      {l.name}
                    </button>
                    {!l.trained && <span className="loralist__tag">Imported</span>}
                  </th>
                  <td>{l.family ?? "—"}</td>
                  <td className="loralist__num numeric">{l.rank ?? "—"}</td>
                  <td className="loralist__num numeric">{formatBytes(l.size_bytes)}</td>
                  <td className="numeric">{formatWhen(l.created_at)}</td>
                  <td className="loralist__num numeric">{l.runs}</td>
                  <td className="loralist__num numeric">{l.total_steps.toLocaleString()}</td>
                  <td className="loralist__num numeric">{formatCount(l.total_images)}</td>
                  <td>
                    <button
                      type="button"
                      className="chip loralist__del"
                      aria-label={`Delete ${l.name} from the library`}
                      onClick={() => onDelete(l)}
                    >
                      Delete…
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </section>
  );
}
