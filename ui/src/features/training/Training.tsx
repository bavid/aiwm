import { useEffect, useMemo, useState } from "react";
import {
  useAbout,
  useDatasets,
  useModels,
  useTrainerStatus,
  useTrainingProfiles,
  useTrainingRuns,
} from "../../lib/hooks";
import { NewRunForm } from "./NewRunForm";
import { RunCard } from "./RunCard";
import "./training.css";

type Props = {
  /** A dataset handed over by the Dataset tab's "Train LoRA" button. Opens the
   *  form with it preselected; `null` the rest of the time. */
  pendingDatasetId: string | null;
  /** Clears that hand-over so re-visiting the tab does not re-open the form. */
  onPendingDatasetConsumed: () => void;
  /** Opens the Image tab with this LoRA preselected and `prompt` prefilled. */
  onTestLora: (loraModelId: string, prompt: string) => void;
};

/** The Training tab: every run the store knows about, newest first, plus the
 *  run form as an inline card on top. */
export function Training({ pendingDatasetId, onPendingDatasetConsumed, onTestLora }: Props) {
  const about = useAbout();
  const { data: runs, error: runsError, refetch: refetchRuns } = useTrainingRuns();
  const { data: profiles } = useTrainingProfiles();
  const { data: status, refetch: refetchStatus } = useTrainerStatus();
  const { data: datasets, refetch: refetchDatasets } = useDatasets();
  const { data: models } = useModels();

  const [showForm, setShowForm] = useState(false);
  /** Which dataset the (re-keyed) form should start on. */
  const [seedDatasetId, setSeedDatasetId] = useState<string | null>(null);

  useEffect(() => {
    if (!pendingDatasetId) return;
    setSeedDatasetId(pendingDatasetId);
    setShowForm(true);
    // The export that just happened is what makes this dataset selectable at
    // all; without a fresh read the option is missing for up to one poll and
    // the preselection looks like it did not take.
    refetchDatasets();
    onPendingDatasetConsumed();
  }, [pendingDatasetId, onPendingDatasetConsumed, refetchDatasets]);

  const profileList = useMemo(() => profiles ?? [], [profiles]);
  const labelByFamily = useMemo(() => {
    const map: Record<string, string> = {};
    for (const p of profileList) map[p.family] = p.label;
    return map;
  }, [profileList]);

  const nameByModelId = useMemo(() => {
    const map: Record<string, string> = {};
    for (const m of models ?? []) map[m.id] = m.name;
    return map;
  }, [models]);

  const ordered = useMemo(
    () => [...(runs ?? [])].sort((a, b) => b.created_at.localeCompare(a.created_at)),
    [runs],
  );

  return (
    <section className="training" aria-label="Training">
      <header className="training__head">
        <h2>Training</h2>
        <span className="training__status">{status ? status.detail : "…"}</span>
        <button
          type="button"
          className="training__new"
          onClick={() => {
            setSeedDatasetId(null);
            setShowForm(true);
          }}
          disabled={showForm}
        >
          New run
        </button>
      </header>

      {showForm && (
        <NewRunForm
          // A fresh hand-over from the Dataset tab has to reach a form that is
          // already open -- remount it rather than reach into its state.
          key={seedDatasetId ?? "new"}
          profiles={profileList}
          status={status}
          datasets={datasets ?? []}
          initialDatasetId={seedDatasetId}
          onStatusChanged={refetchStatus}
          onStarted={() => {
            setShowForm(false);
            setSeedDatasetId(null);
            refetchRuns();
          }}
          onClose={() => setShowForm(false)}
        />
      )}

      {runsError && <p className="training__err">{runsError}</p>}

      {ordered.length === 0 ? (
        <div className="card training__empty">
          <h3>No training runs yet</h3>
          <p>A LoRA teaches an existing model one new thing — a character, a look, a style.</p>
          <ol>
            <li>Curate a dataset on the Dataset tab (extract frames, caption, assign concepts).</li>
            <li>Export it — that writes the image/caption pairs the trainer reads.</li>
            <li>
              Hit <strong>Train LoRA</strong> there, or <strong>New run</strong> here, and pick a
              target model, a trigger word and a preset.
            </li>
          </ol>
        </div>
      ) : (
        <div className="runlist">
          {ordered.map((run) => (
            <RunCard
              key={run.id}
              run={run}
              profileLabel={labelByFamily[run.profile_family] ?? run.profile_family}
              loraName={run.result_model_id ? (nameByModelId[run.result_model_id] ?? null) : null}
              corePort={about?.core_api_port ?? null}
              onChanged={refetchRuns}
              onTestLora={onTestLora}
            />
          ))}
        </div>
      )}
    </section>
  );
}
