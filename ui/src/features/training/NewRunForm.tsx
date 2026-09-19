import { useEffect, useId, useMemo, useState } from "react";
import {
  startTrainingRun,
  type Dataset,
  type TrainerStatus,
  type TrainingPreset,
  type TrainingPresetValues,
  type TrainingProfile,
} from "../../lib/ipc";
import { StorageDirField } from "../../components/StorageDirField";
import { humanize } from "../../lib/errors";
import { useAbout } from "../../lib/hooks";
import { withDataDir } from "../../lib/storage-locations";
import { tokenWarning } from "../dataset/tokens";
import { FineTune } from "./FineTune";
import { Preflight } from "./Preflight";
import { parseTune, type TuneDraft } from "./tune";

const MAX_TRIGGER = 30;
/** Every fine-tuning field blank — the run uses the preset's own values. */
const NO_OVERRIDES: TuneDraft = { rank: "", lr: "", resolution: "", steps: "" };
const MAX_PROMPTS = 3;
/** The core refuses to start a run below this much free space on its drive. */
const RUN_MIN_FREE_GB = 20;

const PRESETS: { id: TrainingPreset; label: string }[] = [
  { id: "fast", label: "Fast" },
  { id: "balanced", label: "Balanced" },
  { id: "thorough", label: "Thorough" },
];

/** A trigger word is a made-up token, so whitespace is never meaningful in it —
 *  strip it as it is typed rather than rejecting the input afterwards. */
const sanitizeTrigger = (raw: string) => raw.replace(/\s+/g, "").slice(0, MAX_TRIGGER);

/** A sample-prompt slot. The text alone would not do as a React key: two empty
 *  slots collide, and removing one would re-key the survivors onto the wrong
 *  `<input>`. */
type SamplePrompt = { id: string; text: string };

let promptSeq = 0;
const newPrompt = (text: string): SamplePrompt => ({ id: `p${++promptSeq}`, text });

/** Whether every *blocking* preflight item is green — the Start button's gate.
 *  Kept in step with what `Preflight` renders in red. */
function preflightPasses(
  status: TrainerStatus | null,
  profile: TrainingProfile | null,
): boolean {
  return !!status && status.installed && !status.env_broken && !!profile && profile.base_installed;
}

const summarize = (p: TrainingPresetValues) =>
  `${p.steps} steps · rank ${p.rank} · ${p.resolution}px · lr ${p.lr}`;

type Props = {
  profiles: TrainingProfile[];
  status: TrainerStatus | null;
  datasets: Dataset[];
  /** Handed over by the Dataset tab's "Train LoRA" button; `null` otherwise. */
  initialDatasetId: string | null;
  onStatusChanged: () => void;
  onStarted: () => void;
  onClose: () => void;
};

/** The whole "start a LoRA run" form: target model, name/trigger, preset with
 *  optional fine-tuning, sample prompts, dataset, and the blocking preflight. */
export function NewRunForm({
  profiles,
  status,
  datasets,
  initialDatasetId,
  onStatusChanged,
  onStarted,
  onClose,
}: Props) {
  const ids = {
    target: useId(),
    name: useId(),
    trigger: useId(),
    dataset: useId(),
  };

  /** Only exported datasets can be trained from — the trainer reads the
   *  `NNNN.png` + `NNNN.txt` pairs the export wrote, not the curation set. */
  const exported = useMemo(() => datasets.filter((d) => d.export_dir !== null), [datasets]);

  const trainable = useMemo(
    () => profiles.filter((p) => p.trainable_models.length > 0),
    [profiles],
  );

  const [targetModelId, setTargetModelId] = useState("");
  const [name, setName] = useState("");
  const [trigger, setTrigger] = useState("");
  const [preset, setPreset] = useState<TrainingPreset>("balanced");
  const [tune, setTune] = useState<TuneDraft>(NO_OVERRIDES);
  const [prompts, setPrompts] = useState<SamplePrompt[]>(() => [newPrompt("")]);
  const [promptsDirty, setPromptsDirty] = useState(false);
  const [datasetId, setDatasetId] = useState(initialDatasetId ?? "");
  /** "Store run in" — blank keeps the default training folder. */
  const [dataDir, setDataDir] = useState("");
  const about = useAbout();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // First trainable model wins until the curator picks one; re-runs only when
  // the profile list itself arrives or changes.
  useEffect(() => {
    setTargetModelId((cur) => cur || (trainable[0]?.trainable_models[0]?.id ?? ""));
  }, [trainable]);

  // The dataset handed over by the Dataset tab may only show up in the poll a
  // moment after the form mounted.
  useEffect(() => {
    if (initialDatasetId) setDatasetId(initialDatasetId);
  }, [initialDatasetId]);

  // Sample prompts start from the trigger word and keep tracking it until the
  // curator edits one -- then they are theirs.
  useEffect(() => {
    if (promptsDirty) return;
    setPrompts([newPrompt(trigger ? `${trigger}, portrait, soft light` : "")]);
  }, [trigger, promptsDirty]);

  const profile = useMemo(
    () => trainable.find((p) => p.trainable_models.some((m) => m.id === targetModelId)) ?? null,
    [trainable, targetModelId],
  );
  const presetValues = profile?.presets[preset] ?? null;
  const triggerWarn = trigger === "" ? null : tokenWarning(trigger);
  const filledPrompts = prompts.map((p) => p.text.trim()).filter((p) => p !== "");
  const parsedTune = useMemo(() => parseTune(tune), [tune]);
  const tuneOk = Object.keys(parsedTune.errors).length === 0;

  const ready =
    preflightPasses(status, profile) &&
    targetModelId !== "" &&
    datasetId !== "" &&
    name.trim() !== "" &&
    trigger !== "" &&
    filledPrompts.length > 0 &&
    tuneOk;

  const editPrompt = (id: string, text: string) => {
    setPromptsDirty(true);
    setPrompts((cur) => cur.map((p) => (p.id === id ? { ...p, text } : p)));
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await startTrainingRun(
        withDataDir(
          {
            name: name.trim(),
            target_model_id: targetModelId,
            dataset_id: datasetId,
            trigger_word: trigger,
            preset,
            hyperparams: parsedTune.values,
            sample_prompts: filledPrompts,
          },
          dataDir,
        ),
      );
      onStarted();
    } catch (e) {
      setError(humanize(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      className="card runform"
      onSubmit={(e) => {
        e.preventDefault();
        if (ready && !busy) void submit();
      }}
    >
      <header className="runform__head">
        <h3>New training run</h3>
        <button type="button" className="chip runform__close" onClick={onClose}>
          Close
        </button>
      </header>

      <div className="runform__grid">
        <label className="datasetform__field" htmlFor={ids.target}>
          <span>Target model</span>
          <select
            id={ids.target}
            value={targetModelId}
            onChange={(e) => setTargetModelId(e.target.value)}
          >
            <option value="">Pick a model…</option>
            {trainable.map((p) => (
              <optgroup key={p.family} label={p.label}>
                {p.trainable_models.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name} — {p.fit_label}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        </label>

        <label className="datasetform__field" htmlFor={ids.dataset}>
          <span>Dataset</span>
          <select id={ids.dataset} value={datasetId} onChange={(e) => setDatasetId(e.target.value)}>
            <option value="">Pick an exported dataset…</option>
            {exported.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
          </select>
        </label>

        <label className="datasetform__field" htmlFor={ids.name}>
          <span>Run name</span>
          <input
            id={ids.name}
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Kenji Character v1"
          />
        </label>

        <label className="datasetform__field" htmlFor={ids.trigger}>
          <span>Trigger word</span>
          <input
            id={ids.trigger}
            type="text"
            value={trigger}
            maxLength={MAX_TRIGGER}
            onChange={(e) => setTrigger(sanitizeTrigger(e.target.value))}
            placeholder="kenji_xy"
          />
        </label>
      </div>

      <p className="datasetform__hint">
        Models without a training profile are not shown.
        {exported.length === 0 && " No dataset has been exported yet — export one on the Dataset tab first."}
      </p>
      {triggerWarn && <p className="dataset__warn">{triggerWarn}</p>}

      <fieldset className="runform__presets">
        <legend>Preset</legend>
        {PRESETS.map((p) => (
          <label key={p.id} className="runform__preset">
            <input
              type="radio"
              name="training-preset"
              value={p.id}
              checked={preset === p.id}
              onChange={() => setPreset(p.id)}
            />
            <span>
              <strong>{p.label}</strong>
              <em>{profile ? summarize(profile.presets[p.id]) : "pick a target model"}</em>
            </span>
          </label>
        ))}
      </fieldset>

      <FineTune
        value={tune}
        onChange={setTune}
        errors={parsedTune.errors}
        preset={presetValues}
      />

      <fieldset className="runform__presets">
        <legend>Sample prompts (1–{MAX_PROMPTS})</legend>
        <div className="runform__prompts">
          {prompts.map((p, i) => (
            <div className="runform__prompt" key={p.id}>
              <input
                type="text"
                value={p.text}
                aria-label={`Sample prompt ${i + 1}`}
                onChange={(e) => editPrompt(p.id, e.target.value)}
                placeholder={`${trigger || "trigger"}, a wide shot`}
              />
              {prompts.length > 1 && (
                <button
                  type="button"
                  className="chip"
                  onClick={() => {
                    setPromptsDirty(true);
                    setPrompts((cur) => cur.filter((q) => q.id !== p.id));
                  }}
                >
                  Remove
                </button>
              )}
            </div>
          ))}
          {prompts.length < MAX_PROMPTS && (
            <button
              type="button"
              className="chip"
              onClick={() => {
                setPromptsDirty(true);
                setPrompts((cur) => [...cur, newPrompt("")]);
              }}
            >
              Add a prompt
            </button>
          )}
        </div>
      </fieldset>

      <StorageDirField
        label="Store run in"
        value={dataDir}
        onChange={setDataDir}
        defaultDir={about?.training_dir ?? null}
        locationKey="training"
        minFreeGB={RUN_MIN_FREE_GB}
        help={
          <>
            Optional. Checkpoints, samples and logs go to <code>&lt;folder&gt;\&lt;run id&gt;</code> —
            pick a roomy drive for long runs. Resume and cleanup follow the run there. At least{" "}
            {RUN_MIN_FREE_GB} GB free is needed to start.
          </>
        }
      />

      <Preflight status={status} profile={profile} onStatusChanged={onStatusChanged} />

      <div className="runform__actions">
        <button type="submit" className="datasetform__go" disabled={!ready || busy}>
          {busy ? "Starting…" : "Start training"}
        </button>
        {!ready && !busy && (
          <span className="muted">Every blocking check above has to be green first.</span>
        )}
      </div>
      {error && (
        <p className="training__err" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}
