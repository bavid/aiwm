import { useId, useState } from "react";
import type { Agent, AgentRuntime } from "../../lib/ipc";
import {
  ctxLabel,
  HERMES_CTX_FLOOR,
  meetsHermesFloor,
  runtimeName,
  splitPath,
  type CodingModel,
} from "./labels";

/** Everything a profile row needs to say whether it can run right now. */
type RowInfo = {
  profile: Agent;
  /** The pinned model, or null when the profile is on Auto. */
  model: CodingModel | null;
  /** null while the runtime poll has not answered yet. */
  runtimeInstalled: boolean | null;
};

function infoFor(
  profile: Agent,
  codingModels: readonly CodingModel[],
  runtimes: AgentRuntime[] | null,
): RowInfo {
  const rt = runtimes?.find((r) => r.id === profile.adapter);
  return {
    profile,
    model: profile.model_id
      ? (codingModels.find((m) => m.id === profile.model_id) ?? null)
      : null,
    runtimeInstalled: runtimes ? (rt?.installed ?? false) : null,
  };
}

/** The saved profiles, each row readable on its own: what drives it, which
 *  model, which folder it may edit, and — new — whether it can start at all. */
export function ProfileList({
  profiles,
  codingModels,
  runtimes,
  sessionOpen,
  onStart,
  onDelete,
}: {
  profiles: Agent[] | null;
  codingModels: readonly CodingModel[];
  runtimes: AgentRuntime[] | null;
  sessionOpen: boolean;
  onStart: (profile: Agent) => void;
  onDelete: (profile: Agent) => Promise<void>;
}) {
  if (!profiles) return <p className="muted">Loading profiles…</p>;

  if (profiles.length === 0) {
    return (
      <p className="agents__empty">
        <strong>No profiles yet.</strong> A profile is a saved combination: one runtime, one
        workspace folder the agent may edit, and one coding model. Create one below and it
        stays until you delete it.
      </p>
    );
  }

  return (
    <ul className="prof-list">
      {profiles.map((p) => (
        <ProfileRow
          key={p.id}
          info={infoFor(p, codingModels, runtimes)}
          sessionOpen={sessionOpen}
          onStart={() => onStart(p)}
          onDelete={() => onDelete(p)}
        />
      ))}
    </ul>
  );
}

/** Why "Start session" is off, in words — a disabled button with no reason is
 *  the thing this page got wrong most often. */
function blockedReason(info: RowInfo, sessionOpen: boolean): string | null {
  if (sessionOpen) return "One session at a time — stop the running one first.";
  if (info.runtimeInstalled === false) {
    return `${runtimeName(info.profile.adapter)} is not installed. Install it above first.`;
  }
  return null;
}

/** A warning that does not block the start but explains the error that would
 *  follow: Hermes against a model under its context floor. */
function ctxWarning(info: RowInfo): string | null {
  if (info.profile.adapter !== "hermes" || !info.model) return null;
  if (meetsHermesFloor(info.model)) return null;
  return `${info.model.name} has a ${ctxLabel(info.model.ctx_max)}; Hermes needs ${HERMES_CTX_FLOOR.toLocaleString("en-US")} tokens and will error immediately.`;
}

function ProfileRow({
  info,
  sessionOpen,
  onStart,
  onDelete,
}: {
  info: RowInfo;
  sessionOpen: boolean;
  onStart: () => void;
  onDelete: () => Promise<void>;
}) {
  const { profile } = info;
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const ids = useId();
  const noteId = `${ids}-note`;
  const blocked = blockedReason(info, sessionOpen);
  const warning = ctxWarning(info);
  const workspace = splitPath(profile.workspace_path);

  const remove = async () => {
    setBusy(true);
    try {
      await onDelete();
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  };

  return (
    <li className="prof" data-blocked={blocked != null}>
      <div className="prof__main">
        <span className="prof__name">{profile.name}</span>
        <span className="prof__badges">
          <span className="badge">{runtimeName(profile.adapter)}</span>
          <span className="badge badge--soft">
            {info.model ? info.model.name : "Auto · coding role"}
          </span>
          {info.runtimeInstalled === false && (
            <span className="badge badge--warn">runtime missing</span>
          )}
        </span>
        <span className="prof__path">
          <span className="prof__caption">Edits</span>{" "}
          <span className="numeric">
            <span className="prof__parent">{workspace.parent}</span>
            <span className="prof__folder">{workspace.name}</span>
          </span>
        </span>
        {profile.allowed_paths.length > 0 && (
          <span className="prof__path">
            <span className="prof__caption">Also reads</span>{" "}
            <span className="numeric prof__parent">
              {profile.allowed_paths.join(" · ")}
            </span>
          </span>
        )}
        {(blocked || warning) && (
          <span className="prof__note" id={noteId}>
            {blocked ?? warning}
          </span>
        )}
      </div>
      <div className="prof__actions">
        {confirming ? (
          <>
            <button
              type="button"
              className="prof__del prof__del--go"
              onClick={remove}
              disabled={busy}
            >
              {busy ? "Deleting…" : "Confirm delete"}
            </button>
            <button type="button" className="prof__del" onClick={() => setConfirming(false)}>
              Keep
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              className="prof__go"
              onClick={onStart}
              disabled={blocked != null}
              aria-describedby={blocked ? noteId : undefined}
            >
              Start session
            </button>
            <button
              type="button"
              className="prof__del"
              onClick={() => setConfirming(true)}
              disabled={sessionOpen}
              aria-label={`Delete profile ${profile.name}`}
            >
              Delete
            </button>
          </>
        )}
      </div>
      {confirming && (
        <p className="visually-hidden" aria-live="polite">
          Delete {profile.name}? Confirm or keep.
        </p>
      )}
    </li>
  );
}
