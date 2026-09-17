import { useState, type ReactNode } from "react";
import { installTrainer, type TrainerStatus, type TrainingProfile } from "../../lib/ipc";

/** One blocking (or purely informational) preflight row. */
type ItemProps = {
  /** `true` green, `false` red, `"note"` a neutral remark that never blocks. */
  ok: boolean | "note";
  label: string;
  children?: ReactNode;
};

function Item({ ok, label, children }: ItemProps) {
  const mark = ok === "note" ? "·" : ok ? "✓" : "✗";
  return (
    <div className="preflight__item" data-ok={String(ok)}>
      <p className="preflight__line">
        <span className="preflight__mark" aria-hidden="true">
          {mark}
        </span>
        <span>{label}</span>
      </p>
      {children}
    </div>
  );
}

type Props = {
  status: TrainerStatus | null;
  profile: TrainingProfile | null;
  /** `AboutInfo.store_path` — where the `hf download` hint points. */
  storePath: string;
  /** Re-poll the trainer status right after an install was kicked off. */
  onStatusChanged: () => void;
};

/** Everything that has to be true before a run can start, in plain text: the
 *  trainer venv, the profile's base weights, and the two things that are not
 *  checks but still have to be said out loud (the GPU gets taken, and the base
 *  weights come with a licence). */
export function Preflight({ status, profile, storePath, onStatusChanged }: Props) {
  const [busy, setBusy] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);

  const startInstall = async () => {
    setBusy(true);
    setInstallError(null);
    try {
      await installTrainer();
      onStatusChanged();
    } catch (e) {
      setInstallError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const trainerOk = !!status && status.installed && !status.env_broken;
  const trainerLabel = !status
    ? "Checking the trainer…"
    : status.installing
      ? "Trainer is being set up"
      : status.env_broken
        ? "Trainer environment is broken"
        : status.installed
          ? "Trainer installed"
          : "Trainer not installed";

  // The store path is whatever the platform writes (`E:\AI\models` on Windows,
  // `/srv/models` elsewhere) -- a hard-coded `/` would produce a mixed-
  // separator path nobody can paste into a shell unchanged.
  const sep = storePath.includes("\\") ? "\\" : "/";
  const command = profile
    ? `hf download ${profile.base_repo} --local-dir ${storePath}${sep}training${sep}${profile.family} --exclude "*.jpg"`
    : "";

  return (
    <section className="preflight" aria-label="Preflight">
      <h4>Before the run starts</h4>

      <Item ok={trainerOk} label={trainerLabel}>
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

      <Item
        ok={!!profile && profile.base_installed}
        label={
          !profile
            ? "Pick a target model to check its base weights"
            : profile.base_installed
              ? `Base weights for ${profile.label} are in the library`
              : `Base weights for ${profile.label} are missing (about ${profile.base_approx_gb} GB)`
        }
      >
        {profile && !profile.base_installed && (
          <>
            <div className="preflight__cmd">
              <code>{command}</code>
              <button
                type="button"
                className="chip"
                onClick={() => void navigator.clipboard?.writeText(command)}
              >
                Copy
              </button>
            </div>
            <p className="preflight__fix">
              …then register the folder on the Models tab with role {profile.base_role}.
            </p>
          </>
        )}
      </Item>

      <Item
        ok="note"
        label="The chat model and ComfyUI will be unloaded; image/video jobs wait while training runs."
      />

      {profile?.license_note && <Item ok="note" label={profile.license_note} />}
    </section>
  );
}
