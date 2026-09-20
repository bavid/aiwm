import { useMemo, useRef, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { useAgentRuntimes, useAgents, useModels } from "../../lib/hooks";
import { deleteAgent, openAgentSession, type Agent } from "../../lib/ipc";
import type { CodingModel } from "./labels";
import { LauncherPanel } from "./LauncherPanel";
import { NewProfileForm } from "./NewProfileForm";
import { ProfileList } from "./ProfileList";
import { RuntimeCards } from "./RuntimeCards";
import { SessionPanel } from "./SessionPanel";
import { StartHere } from "./StartHere";
import "./agents.css";

/** The Agents tab, top to bottom: what the tab is, what still has to be set
 *  up, the state of the two runtimes, then the actual work — profiles on the
 *  left, the live session on the right — and the external terminal last. */
export function AgentsWorkbench() {
  const { data: profiles, refetch } = useAgents();
  const { data: models } = useModels();
  const { data: runtimes } = useAgentRuntimes();

  const codingModels = useMemo<CodingModel[]>(
    () =>
      (models ?? [])
        .filter((m) => m.roles.includes("coding"))
        .map((m) => ({
          id: m.id,
          name: m.name,
          ctx_max: m.ctx_max,
          last_used_at: m.last_used_at,
        })),
    [models],
  );

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [activeProfile, setActiveProfile] = useState<Agent | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const profilesRef = useRef<HTMLElement>(null);

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

  const openForm = () => {
    setFormOpen(true);
    profilesRef.current?.scrollIntoView({ block: "nearest" });
  };

  return (
    <section className="agents" aria-label="Agents">
      <header className="agents__head">
        <h1>Agents</h1>
        <p className="agents__lede">
          A coding agent on your own machine and your own model, locked to one folder. Every
          command it wants to run and every file it wants to change waits for your approval,
          and it has no network access. <HelpHint area="agents" setting="start-here" />
        </p>
      </header>

      <StartHere
        codingModels={codingModels}
        runtimes={runtimes}
        profileCount={profiles?.length ?? 0}
        sessionOpen={sessionId != null}
        formOpen={formOpen}
        onNewProfile={openForm}
      />

      <RuntimeCards runtimes={runtimes} codingModels={codingModels} />

      <div className="agents__work">
        <section className="card agents__profiles" ref={profilesRef} tabIndex={-1}
          aria-labelledby="agents-prof-h">
          <header className="card__head">
            <h2 id="agents-prof-h">Profiles</h2>
            <span className="card__sub numeric">{profiles?.length ?? 0}</span>
          </header>

          <ProfileList
            profiles={profiles}
            codingModels={codingModels}
            runtimes={runtimes}
            sessionOpen={sessionId != null}
            onStart={start}
            onDelete={async (p) => {
              await deleteAgent(p.id);
              if (activeProfile?.id === p.id) setSessionId(null);
              refetch();
              profilesRef.current?.focus();
            }}
          />

          <NewProfileForm
            codingModels={codingModels}
            runtimes={runtimes}
            open={formOpen}
            onOpenChange={setFormOpen}
            onCreated={refetch}
          />
          {startError && <p className="agents__err">{startError}</p>}
        </section>

        <SessionPanel
          sessionId={sessionId}
          profileName={activeProfile?.name ?? null}
          onClosed={() => {
            setSessionId(null);
            profilesRef.current?.focus();
          }}
        />
      </div>

      <LauncherPanel codingModels={codingModels} runtimes={runtimes} />
    </section>
  );
}
