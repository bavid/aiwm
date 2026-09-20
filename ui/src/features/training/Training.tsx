import { useEffect, useMemo, useState } from "react";
import {
  useAbout,
  useDatasets,
  useLoraLineage,
  useLoras,
  useModels,
  useTrainerStatus,
  useTrainingProfiles,
  useTrainingRuns,
} from "../../lib/hooks";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { humanize } from "../../lib/errors";
import {
  deleteModel,
  type LoraLineage,
  type LoraSummary,
  type TrainingProfile,
  type TrainingRun,
} from "../../lib/ipc";
import { formatBytes } from "../../lib/units";
import { LoraHistory } from "./LoraHistory";
import { LoraList } from "./LoraList";
import { NewRunForm, type RunSeed } from "./NewRunForm";
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

/** What "Continue with another dataset" prefills: the LoRA's last run's
 *  target (when it is still trainable), else the first trainable model of the
 *  LoRA's family; and the last run's trigger word. The dataset stays open —
 *  it is the one thing that is meant to change. */
function continueSeed(
  lora: LoraSummary,
  lineage: LoraLineage | null,
  runs: TrainingRun[],
  profiles: TrainingProfile[],
): RunSeed {
  const last = lineage?.runs.at(-1) ?? null;
  const lastRun = last ? (runs.find((r) => r.id === last.run_id) ?? null) : null;
  const trainable = profiles.flatMap((p) => p.trainable_models);
  const lastTarget = lastRun?.target_model_id ?? null;
  const targetModelId = trainable.some((m) => m.id === lastTarget)
    ? lastTarget
    : (trainable.find((m) => m.family === lora.family)?.id ?? null);
  return {
    initLoraModelId: lora.model_id,
    targetModelId,
    triggerWord: last?.trigger_word ?? "",
  };
}

/** The Training tab: the LoRA overview with its history panel, the run form
 *  as an inline card, and every run the store knows about, newest first. */
export function Training({ pendingDatasetId, onPendingDatasetConsumed, onTestLora }: Props) {
  const about = useAbout();
  const { data: runs, error: runsError, refetch: refetchRuns } = useTrainingRuns();
  const { data: profiles } = useTrainingProfiles();
  const { data: status, refetch: refetchStatus } = useTrainerStatus();
  const { data: datasets, refetch: refetchDatasets } = useDatasets();
  const { data: models } = useModels();
  const { data: loras, error: lorasError, refetch: refetchLoras } = useLoras();

  const [showForm, setShowForm] = useState(false);
  /** Which dataset the (re-keyed) form should start on. */
  const [seedDatasetId, setSeedDatasetId] = useState<string | null>(null);
  /** Which LoRA the (re-keyed) form should continue from. */
  const [seed, setSeed] = useState<RunSeed | null>(null);
  const [selectedLoraId, setSelectedLoraId] = useState<string | null>(null);
  const { data: lineage, error: lineageError } = useLoraLineage(selectedLoraId);
  /** The LoRA whose delete confirmation is open, plus that dialog's state. */
  const [deleteLora, setDeleteLora] = useState<LoraSummary | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  /** Remove a LoRA from the library. The core refuses while a run still
   *  continues from it (migration 0020) — that refusal is shown verbatim in the
   *  dialog and the LoRA stays. */
  async function confirmDeleteLora() {
    if (!deleteLora) return;
    setDeleteBusy(true);
    setDeleteError(null);
    try {
      await deleteModel(deleteLora.model_id);
      if (selectedLoraId === deleteLora.model_id) setSelectedLoraId(null);
      setDeleteLora(null);
      refetchLoras();
      refetchRuns();
    } catch (e) {
      setDeleteError(humanize(e));
    } finally {
      setDeleteBusy(false);
    }
  }

  useEffect(() => {
    if (!pendingDatasetId) return;
    setSeedDatasetId(pendingDatasetId);
    setSeed(null);
    setShowForm(true);
    // The export that just happened is what makes this dataset selectable at
    // all; without a fresh read the option is missing for up to one poll and
    // the preselection looks like it did not take.
    refetchDatasets();
    onPendingDatasetConsumed();
  }, [pendingDatasetId, onPendingDatasetConsumed, refetchDatasets]);

  const profileList = useMemo(() => profiles ?? [], [profiles]);
  const loraList = useMemo(() => loras ?? [], [loras]);
  const runList = useMemo(() => runs ?? [], [runs]);

  const labelByFamily = useMemo(() => {
    const map: Record<string, string> = {};
    for (const p of profileList) map[p.family] = p.label;
    return map;
  }, [profileList]);

  const profileByFamily = useMemo(() => {
    const map: Record<string, TrainingProfile> = {};
    for (const p of profileList) map[p.family] = p;
    return map;
  }, [profileList]);

  const nameByModelId = useMemo(() => {
    const map: Record<string, string> = {};
    for (const m of models ?? []) map[m.id] = m.name;
    for (const l of loraList) map[l.model_id] = l.name;
    return map;
  }, [models, loraList]);

  const nameByDatasetId = useMemo(() => {
    const map: Record<string, string> = {};
    for (const d of datasets ?? []) map[d.id] = d.name;
    return map;
  }, [datasets]);

  const ordered = useMemo(
    () => [...runList].sort((a, b) => b.created_at.localeCompare(a.created_at)),
    [runList],
  );

  const selectedLora = loraList.find((l) => l.model_id === selectedLoraId) ?? null;
  // The poll keeps the previous LoRA's answer until the next tick; a history
  // under the wrong heading, even for a second, reads as a lie.
  const selectedLineage =
    lineage !== null && lineage.lora.model_id === selectedLoraId ? lineage : null;

  const openFreshForm = () => {
    setSeedDatasetId(null);
    setSeed(null);
    setShowForm(true);
  };

  const openContinueForm = (lora: LoraSummary) => {
    setSeedDatasetId(null);
    setSeed(continueSeed(lora, selectedLineage, runList, profileList));
    setShowForm(true);
  };

  const formKey = seed ? `continue:${seed.initLoraModelId}` : (seedDatasetId ?? "new");

  return (
    <section className="training" aria-label="Training">
      <header className="training__head">
        <h2>Training</h2>
        <span className="training__status">{status ? status.detail : "…"}</span>
        <button type="button" className="training__new" onClick={openFreshForm} disabled={showForm}>
          New run
        </button>
      </header>

      {showForm && (
        <NewRunForm
          // A fresh hand-over (a dataset from the Dataset tab, a LoRA from
          // the history) has to reach a form that is already open -- remount
          // it rather than reach into its state.
          key={formKey}
          profiles={profileList}
          status={status}
          datasets={datasets ?? []}
          loras={loraList}
          initialDatasetId={seedDatasetId}
          seed={seed}
          onStatusChanged={refetchStatus}
          onStarted={() => {
            setShowForm(false);
            setSeedDatasetId(null);
            setSeed(null);
            refetchRuns();
            refetchLoras();
          }}
          onClose={() => setShowForm(false)}
        />
      )}

      <LoraList
        loras={loraList}
        isLoading={loras === null}
        error={lorasError}
        selectedId={selectedLoraId}
        onSelect={(id) => setSelectedLoraId((cur) => (cur === id ? null : id))}
        onDelete={(lora) => {
          setDeleteError(null);
          setDeleteLora(lora);
        }}
      />

      <ConfirmDialog
        isOpen={deleteLora !== null}
        title={`Delete “${deleteLora?.name ?? ""}”?`}
        confirmLabel="Delete"
        tone="danger"
        isBusy={deleteBusy}
        error={deleteError}
        onConfirm={confirmDeleteLora}
        onCancel={() => setDeleteLora(null)}
      >
        <p>
          This removes the LoRA from the library and deletes its file
          {deleteLora ? ` (${formatBytes(deleteLora.size_bytes)})` : ""} from disk. Runs that used
          it keep their history, but they can no longer be continued from it.
        </p>
        <p className="muted">Training runs, datasets and your source media are not touched.</p>
      </ConfirmDialog>

      {selectedLora && (
        <LoraHistory
          lora={selectedLora}
          lineage={selectedLineage}
          error={lineageError}
          profiles={profileList}
          corePort={about?.core_api_port ?? null}
          onTestLora={onTestLora}
          onContinue={() => openContinueForm(selectedLora)}
          onClose={() => setSelectedLoraId(null)}
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
              presetValues={profileByFamily[run.profile_family]?.presets[run.preset] ?? null}
              datasetName={run.dataset_id ? (nameByDatasetId[run.dataset_id] ?? null) : null}
              loraName={run.result_model_id ? (nameByModelId[run.result_model_id] ?? null) : null}
              initLoraName={
                run.init_lora_model_id ? (nameByModelId[run.init_lora_model_id] ?? null) : null
              }
              corePort={about?.core_api_port ?? null}
              onChanged={() => {
                refetchRuns();
                refetchLoras();
              }}
              onTestLora={onTestLora}
            />
          ))}
        </div>
      )}
    </section>
  );
}
