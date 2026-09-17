import { useEffect, useRef, useState, type ReactNode } from "react";
import { humanize } from "../../lib/errors";
import { installTrainer, type TrainerStatus, type TrainingProfile } from "../../lib/ipc";

/** How a preflight row reads. `"note"` also covers "not answered yet": a check
 *  still waiting on a poll is neutral, never red — nothing has failed, the
 *  answer simply has not arrived. */
type RowState = "pass" | "fail" | "note";

type ItemProps = {
  state: RowState;
  label: string;
  children?: ReactNode;
};

const MARK: Record<RowState, string> = { pass: "✓", fail: "✗", note: "·" };

function Item({ state, label, children }: ItemProps) {
  return (
    <div className="preflight__item" data-ok={state}>
      <p className="preflight__line">
        <span className="preflight__mark" aria-hidden="true">
          {MARK[state]}
        </span>
        <span>{label}</span>
      </p>
      {children}
    </div>
  );
}

/** How long the "Copied" confirmation stays up. */
const COPIED_MS = 1600;

type Props = {
  status: TrainerStatus | null;
  profile: TrainingProfile | null;
  /** Re-poll the trainer status right after an install was kicked off. */
  onStatusChanged: () => void;
};

/** Everything that has to be true before a run can start, in plain text: the
 *  trainer venv, the profile's base weights, and the two things that are not
 *  checks but still have to be said out loud (the GPU gets taken, and the base
 *  weights come with a licence). */
export function Preflight({ status, profile, onStatusChanged }: Props) {
  const [busy, setBusy] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const commandRef = useRef<HTMLElement>(null);

  // The confirmation is a timer, so it has to be cancelled if the form closes
  // (or the command changes) before it lapses.
  useEffect(() => {
    if (!copied) return;
    const id = setTimeout(() => setCopied(false), COPIED_MS);
    return () => clearTimeout(id);
  }, [copied]);

  const startInstall = async () => {
    setBusy(true);
    setInstallError(null);
    try {
      await installTrainer();
      onStatusChanged();
    } catch (e) {
      setInstallError(humanize(e));
    } finally {
      setBusy(false);
    }
  };

  // Built server-side from the same manifest the runner verifies the download
  // against (`core::training::bases`), so the exclusions and the target
  // directory shown here cannot drift from what Start then expects to find.
  // The store path still arrives separately because the rest of this panel
  // talks about it.
  const command = profile?.base_download_command ?? "";

  /** Selects the command text in place. The fallback for every case where the
   *  clipboard is not ours to write: an insecure origin, a denied permission,
   *  an embedded webview without the API. Leaving it selected means Ctrl+C
   *  still works, which silently doing nothing does not. */
  const selectCommand = () => {
    const node = commandRef.current;
    const selection = window.getSelection();
    if (!node || !selection) return;
    const range = document.createRange();
    range.selectNodeContents(node);
    selection.removeAllRanges();
    selection.addRange(range);
  };

  const copyCommand = async () => {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
    } catch {
      selectCommand();
    }
  };

  const trainerState: RowState = !status
    ? "note"
    : status.installed && !status.env_broken
      ? "pass"
      : "fail";
  const trainerLabel = !status
    ? "Checking the trainer…"
    : status.installing
      ? "Trainer is being set up"
      : status.env_broken
        ? "Trainer environment is broken"
        : status.installed
          ? "Trainer installed"
          : "Trainer not installed";

  const baseState: RowState = !profile ? "note" : profile.base_installed ? "pass" : "fail";
  const baseLabel = !profile
    ? "Pick a target model to check its base weights"
    : profile.base_installed
      ? `Base weights for ${profile.label} are in the library`
      : `Base weights for ${profile.label} are missing (about ${profile.base_approx_gb} GB)`;

  return (
    <section className="preflight" aria-label="Preflight">
      <h4>Before the run starts</h4>

      <Item state={trainerState} label={trainerLabel}>
        {status && (status.installing || !status.installed || status.env_broken) && (
          <>
            <p className="preflight__fix">{status.detail}</p>
            {!status.installing && (
              <p className="preflight__fix">
                <button type="button" className="chip" onClick={startInstall} disabled={busy}>
                  {busy ? "Starting…" : status.env_broken ? "Set up again" : "Install trainer"}
                </button>
              </p>
            )}
            {installError && <p className="training__err">{installError}</p>}
          </>
        )}
      </Item>

      <Item state={baseState} label={baseLabel}>
        {profile && !profile.base_installed && (
          <>
            <div className="preflight__cmd">
              <code ref={commandRef}>{command}</code>
              <button type="button" className="chip" onClick={() => void copyCommand()}>
                {copied ? "Copied" : "Copy"}
              </button>
            </div>
            <p className="preflight__fix">
              …then register the folder on the Models tab with role {profile.base_role}.
            </p>
          </>
        )}
      </Item>

      <Item
        state="note"
        label="The chat model and ComfyUI will be unloaded; image/video jobs wait while training runs."
      />

      {profile?.license_note && <Item state="note" label={profile.license_note} />}
    </section>
  );
}
