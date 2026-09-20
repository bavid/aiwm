import { useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { installHermes, type AgentInstallStatus, type AgentRuntime } from "../../lib/ipc";
import {
  ctxLabel,
  HERMES_CTX_FLOOR,
  meetsHermesFloor,
  RUNTIME_BLURB,
  runtimeName,
  type CodingModel,
} from "./labels";

/** The two agent runtimes, side by side, as the first thing on the tab: is it
 *  installed, does the library hold a model it can actually use, and what to
 *  press if not. Both facts used to be buried inside the collapsed "New
 *  profile" form, where a first-time visitor could not see them at all. */
export function RuntimeCards({
  runtimes,
  codingModels,
}: {
  runtimes: AgentRuntime[] | null;
  codingModels: readonly CodingModel[];
}) {
  /** Until the poll answers, assume nothing: an unknown runtime reads as
   *  "checking…" rather than claiming either state. */
  const rows: { id: string; installed: boolean | null; install?: AgentInstallStatus }[] =
    runtimes
      ? runtimes.map((r) => ({ id: r.id, installed: r.installed, install: r.install }))
      : [
          { id: "opencode", installed: null },
          { id: "hermes", installed: null },
        ];

  return (
    <section className="rtrow" aria-labelledby="agents-rt-h">
      <header className="rtrow__head">
        <h2 id="agents-rt-h">Runtimes</h2>
        <HelpHint area="agents" setting="runtime-status" />
      </header>
      <div className="rtrow__cards">
        {rows.map((r) => (
          <RuntimeCard
            key={r.id}
            id={r.id}
            installed={r.installed}
            install={r.install}
            codingModels={codingModels}
          />
        ))}
      </div>
    </section>
  );
}

function statusWord(installed: boolean | null): { label: string; state: string } {
  if (installed == null) return { label: "Checking…", state: "unknown" };
  return installed
    ? { label: "Installed", state: "ok" }
    : { label: "Not installed", state: "crit" };
}

function RuntimeCard({
  id,
  installed,
  install,
  codingModels,
}: {
  id: string;
  installed: boolean | null;
  install?: AgentInstallStatus;
  codingModels: readonly CodingModel[];
}) {
  const status = statusWord(installed);
  return (
    <article className="card rtcard" data-installed={installed ?? "unknown"}>
      <header className="rtcard__head">
        <h3>{runtimeName(id)}</h3>
        <span className="rtcard__status" data-state={status.state}>
          <span className="rtcard__dot" data-state={status.state} aria-hidden="true" />
          {status.label}
        </span>
      </header>
      <p className="muted rtcard__blurb">{RUNTIME_BLURB[id] ?? "An agent runtime."}</p>
      <ModelReadiness runtimeId={id} codingModels={codingModels} />
      {installed === false &&
        (id === "hermes" ? (
          <HermesInstall install={install} />
        ) : (
          <p className="rtcard__todo">
            Install <code>opencode-ai</code> with npm and make sure <code>opencode</code> is
            on your PATH; this card turns to “Installed” on the next poll.
          </p>
        ))}
    </article>
  );
}

/** Which of the library's coding models this runtime can actually drive —
 *  the Hermes context floor is the whole reason this line exists. */
function ModelReadiness({
  runtimeId,
  codingModels,
}: {
  runtimeId: string;
  codingModels: readonly CodingModel[];
}) {
  if (codingModels.length === 0) {
    return (
      <p className="rtcard__todo">
        No model carries the “coding” role yet. Import a coding GGUF on the Models tab and
        tick “coding” — Qwen2.5-Coder for OpenCode, Hermes-3 for Hermes.
      </p>
    );
  }

  if (runtimeId !== "hermes") {
    return (
      <p className="rtcard__models">
        <strong className="numeric">{codingModels.length}</strong> coding{" "}
        {codingModels.length === 1 ? "model" : "models"} in the library — no context
        requirement, any of them can drive it.
      </p>
    );
  }

  const fits = codingModels.filter(meetsHermesFloor);
  const floor = HERMES_CTX_FLOOR.toLocaleString("en-US");
  return (
    <>
      <p className="rtcard__models">
        <strong className="numeric">{fits.length}</strong> of{" "}
        <span className="numeric">{codingModels.length}</span> coding models reach the{" "}
        {floor}-token context Hermes needs.
      </p>
      {fits.length === 0 && (
        <p className="rtcard__todo">
          Hermes refuses to start below {floor} tokens, for its main model and its
          auxiliary compression model alike.{" "}
          {codingModels.length === 1
            ? `The one coding model here has a ${ctxLabel(codingModels[0].ctx_max)}.`
            : "None of the coding models here reaches it."}{" "}
          A session started anyway will error immediately.
        </p>
      )}
    </>
  );
}

/** `installing hermes 42%…` — the running phase in words plus its share. */
function phaseLabel(s: Extract<AgentInstallStatus, { state: "running" }>): string {
  const pct =
    s.total_bytes > 0 ? ` ${Math.round((s.done_bytes / s.total_bytes) * 100)}%` : "";
  return `${s.phase.replace(/_/g, " ")}${pct}…`;
}

/** Hermes is the one runtime AIWM can install itself. */
function HermesInstall({ install }: { install?: AgentInstallStatus }) {
  const [starting, setStarting] = useState(false);
  const progress = install?.state === "running" ? phaseLabel(install) : null;
  const running = progress != null;

  const start = async () => {
    setStarting(true);
    try {
      await installHermes();
    } catch {
      /* the polled status carries the failure — see the error line below */
    } finally {
      setStarting(false);
    }
  };

  return (
    <div className="rtsetup">
      <p className="muted">
        Setup pulls about 120 packages plus its own toolchain — a few minutes, with a
        progress line.
      </p>
      <button
        type="button"
        onClick={start}
        disabled={starting || running}
        className="rtsetup__btn"
      >
        {progress ?? (starting ? "Starting…" : "Install Hermes")}
      </button>
      <p className="visually-hidden" aria-live="polite">
        {progress ? `Installing Hermes: ${progress}` : ""}
      </p>
      {install?.state === "failed" && <p className="agents__err">{install.error}</p>}
    </div>
  );
}
