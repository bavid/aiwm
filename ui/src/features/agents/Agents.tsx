import { useState } from "react";
import { useAgents, useModels } from "../../lib/hooks";
import { deleteAgent, openAgentSession, type Agent } from "../../lib/ipc";
import { LauncherPanel } from "./LauncherPanel";
import { NewProfileForm } from "./NewProfileForm";
import { SessionPanel } from "./SessionPanel";
import "./agents.css";

export function AgentsWorkbench() {
  const { data: profiles, refetch } = useAgents();
  const { data: models } = useModels();
  const codingModels = (models ?? []).filter((m) => m.roles.includes("coding"));

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [activeProfile, setActiveProfile] = useState<Agent | null>(null);
  const [startError, setStartError] = useState<string | null>(null);

  const start = async (profile: Agent) => {
    setStartError(null);
    try {
      const session = await openAgentSession(profile.id);
      setActiveProfile(profile);
      setSessionId(session.id);
    } catch (err) {
      setStartError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="agents">
      <section className="card agents__profiles">
        <header className="card__head">
          <h2>Agent profiles</h2>
          <span className="card__sub numeric">{profiles?.length ?? 0}</span>
        </header>

        {profiles && profiles.length === 0 && (
          <p className="muted">
            No profiles yet. A profile binds an agent runtime to a workspace folder and a
            coding model.
          </p>
        )}
        <ul className="prof-list">
          {(profiles ?? []).map((p) => (
            <ProfileRow
              key={p.id}
              profile={p}
              modelName={p.model_id ? nameFor(codingModels, p.model_id) : null}
              disabled={sessionId != null}
              onStart={() => start(p)}
              onDelete={async () => {
                await deleteAgent(p.id);
                if (activeProfile?.id === p.id) setSessionId(null);
                refetch();
              }}
            />
          ))}
        </ul>

        <NewProfileForm codingModels={codingModels} onCreated={refetch} />
        {startError && <p className="agents__err">{startError}</p>}
      </section>

      <SessionPanel
        sessionId={sessionId}
        profileName={activeProfile?.name ?? null}
        onClosed={() => setSessionId(null)}
      />

      <LauncherPanel codingModels={codingModels} />
    </div>
  );
}

function nameFor(models: { id: string; name: string }[], id: string): string {
  return models.find((m) => m.id === id)?.name ?? id;
}

function ProfileRow({
  profile,
  modelName,
  disabled,
  onStart,
  onDelete,
}: {
  profile: Agent;
  modelName: string | null;
  disabled: boolean;
  onStart: () => void;
  onDelete: () => void;
}) {
  return (
    <li className="prof">
      <div className="prof__main">
        <span className="prof__name">{profile.name}</span>
        <span className="prof__badges">
          <span className="badge">{profile.adapter}</span>
          <span className="badge badge--soft">{modelName ?? "Auto · coding"}</span>
        </span>
        <span className="prof__path numeric">{profile.workspace_path}</span>
        {profile.allowed_paths.length > 0 && (
          <span className="prof__extra numeric">
            + reads {profile.allowed_paths.join(", ")}
          </span>
        )}
      </div>
      <div className="prof__actions">
        <button type="button" className="prof__go" onClick={onStart} disabled={disabled}>
          New session
        </button>
        <button
          type="button"
          className="prof__del"
          onClick={onDelete}
          disabled={disabled}
          aria-label={`Delete ${profile.name}`}
        >
          Delete
        </button>
      </div>
    </li>
  );
}
