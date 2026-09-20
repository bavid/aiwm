import { HelpHint } from "../../components/HelpHint";
import type { AgentRuntime } from "../../lib/ipc";
import { runtimeName, type CodingModel } from "./labels";

/** The first thing on the tab: the three things that must exist before an
 *  agent can run, each either ticked off or spelling out what to do. Once all
 *  three hold it shrinks to a single line naming the next action, so it stops
 *  competing with the work. */
export function StartHere({
  codingModels,
  runtimes,
  profileCount,
  sessionOpen,
  formOpen,
  onNewProfile,
}: {
  codingModels: readonly CodingModel[];
  runtimes: AgentRuntime[] | null;
  profileCount: number;
  sessionOpen: boolean;
  formOpen: boolean;
  onNewProfile: () => void;
}) {
  const installed = (runtimes ?? []).filter((r) => r.installed).map((r) => runtimeName(r.id));
  const hasModel = codingModels.length > 0;
  const hasRuntime = installed.length > 0;
  const hasProfile = profileCount > 0;
  const ready = hasModel && hasRuntime && hasProfile;

  if (ready) {
    return (
      <p className="agents__ready" data-session={sessionOpen}>
        <span className="agents__ready-dot" data-state={sessionOpen ? "live" : "ok"} aria-hidden="true" />
        {sessionOpen ? (
          <>
            <strong>A session is running.</strong> Read it on the right; the profile list is
            on hold until you stop it.
          </>
        ) : (
          <>
            <strong>Ready.</strong> Pick a profile below and press <em>Start session</em>; the
            transcript opens on the right.
          </>
        )}
      </p>
    );
  }

  const steps: {
    done: boolean;
    label: string;
    hint: string;
    action?: (() => void) | null;
  }[] = [
    {
      done: hasModel,
      label: "A coding model",
      hint: hasModel
        ? `${codingModels.length} in the library`
        : "Models tab: import a coding GGUF and tick the “coding” role.",
    },
    {
      done: hasRuntime,
      label: "An installed runtime",
      hint: hasRuntime
        ? `${installed.join(" and ")} ready`
        : "Install one below — Hermes from its card, OpenCode from npm.",
    },
    {
      done: hasProfile,
      label: "A profile",
      hint: hasProfile
        ? `${profileCount} saved`
        : "A runtime plus a workspace folder plus a model, saved under a name.",
      action: hasProfile || formOpen ? null : onNewProfile,
    },
  ];

  return (
    <section className="card agents__start" aria-labelledby="agents-start-h">
      <header className="card__head">
        <h2 id="agents-start-h">Start here</h2>
        <span className="card__sub">
          <HelpHint area="agents" setting="start-here" />
        </span>
      </header>
      <p className="muted agents__start-lede">
        Three things have to exist before an agent can run. The transcript panel stays empty
        until they do.
      </p>
      <ol className="startlist">
        {steps.map((s) => (
          <li key={s.label} className="startstep" data-done={s.done}>
            <span className="startstep__mark" aria-hidden="true">
              {s.done ? "✓" : ""}
            </span>
            <span className="startstep__body">
              <span className="startstep__label">
                {s.label}
                <span className="visually-hidden">{s.done ? " — done" : " — still missing"}</span>
              </span>
              <span className="startstep__hint">{s.hint}</span>
            </span>
            {s.action && (
              <button type="button" className="agents__step-btn" onClick={s.action}>
                New profile
              </button>
            )}
          </li>
        ))}
      </ol>
    </section>
  );
}
