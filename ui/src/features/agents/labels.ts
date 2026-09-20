import type { AgentSessionState, ToolStatus } from "../../lib/ipc";

/** Display names, plain-word glosses and the one hard number this tab has to
 *  reason about. Everything here is shared by the panels, so a runtime,
 *  a state or a permission kind reads the same wherever it appears. */

export const RUNTIME_LABEL: Record<string, string> = {
  opencode: "OpenCode",
  hermes: "Hermes",
};

export function runtimeName(id: string): string {
  return RUNTIME_LABEL[id] ?? id;
}

/** One sentence on what each runtime is and how it gets installed. */
export const RUNTIME_BLURB: Record<string, string> = {
  opencode:
    "A Node program you install yourself: opencode-ai on npm, with opencode on your PATH.",
  hermes: "A local Python service. AIWM can install it for you into its own folder.",
};

/** Hermes refuses to start below this context window — for its main model and
 *  for its auxiliary compression model alike. From the real runs logged in
 *  `docs/TODO.md` (2026-09-12); OpenCode has no such floor. */
export const HERMES_CTX_FLOOR = 64_000;

/** A library model carrying the `coding` role, reduced to what this tab shows. */
export type CodingModel = {
  id: string;
  name: string;
  ctx_max: number | null;
  last_used_at: string | null;
};

/** Exact token counts, never a rounded "64K": the Hermes floor is a precise
 *  number, and a 63,000-token model must not read as if it cleared it. */
export function ctxLabel(ctx: number | null): string {
  return ctx == null ? "context unknown" : `${ctx.toLocaleString("en-US")}-token context`;
}

export function meetsHermesFloor(model: CodingModel): boolean {
  return (model.ctx_max ?? 0) >= HERMES_CTX_FLOOR;
}

/** Coding models, most-recently-used first — the order "Auto" draws from. */
export function byRecency(models: readonly CodingModel[]): CodingModel[] {
  return [...models].sort((a, b) =>
    (b.last_used_at ?? "").localeCompare(a.last_used_at ?? ""),
  );
}

/** The model "Auto" would pick right now, or null while no coding model has
 *  ever been used (nothing is most-recently-used yet). */
export function autoPick(models: readonly CodingModel[]): CodingModel | null {
  return byRecency(models).find((m) => m.last_used_at != null) ?? null;
}

/** A session state as a word plus a sentence saying whose turn it is. The
 *  sentence is what makes the status scannable — "working" alone does not say
 *  whether the person is meant to do anything. */
export const SESSION_STATE: Record<AgentSessionState, { label: string; hint: string }> = {
  starting: {
    label: "Starting",
    hint: "Loading the runtime and its model. Nothing to do yet.",
  },
  idle: {
    label: "Your turn",
    hint: "The agent is waiting for a message.",
  },
  working: {
    label: "Working",
    hint: "The agent is thinking or running a tool.",
  },
  awaiting_approval: {
    label: "Needs your approval",
    hint: "Answer the prompt in the transcript to let it continue.",
  },
  stopped: {
    label: "Stopped",
    hint: "The session ended and the model was released.",
  },
  failed: {
    label: "Failed",
    hint: "The runtime stopped with an error.",
  },
};

export const TOOL_STATUS_LABEL: Record<ToolStatus, string> = {
  pending: "queued",
  running: "running",
  done: "finished",
  error: "failed",
};

/** Plain-word glosses for the tool names the adapters emit. */
const TOOL_BLURB: Record<string, string> = {
  bash: "shell command",
  terminal: "shell command",
  edit: "file edit",
  write: "file write",
  read: "file read",
  glob: "file search",
  grep: "text search",
};

export function toolBlurb(name: string): string | null {
  return TOOL_BLURB[name.toLowerCase()] ?? null;
}

/** What answering "Allow" would let the agent do. The adapters pass the tool
 *  name (Hermes) or OpenCode's permission name through unchanged, so unknown
 *  values fall back to naming the tool rather than guessing. */
const PERMISSION_KIND: Record<string, string> = {
  bash: "run a shell command",
  terminal: "run a shell command",
  edit: "edit a file in the workspace",
  write: "write a file in the workspace",
  external_directory: "reach a folder outside the workspace",
  tool: "use a tool",
};

export function permissionAction(kind: string): string {
  return PERMISSION_KIND[kind.toLowerCase()] ?? `use the ${kind} tool`;
}

export const DECISION_LABEL = {
  allow_once: "Allowed once",
  allow_always: "Allowed for this session",
  deny: "Denied",
} as const;

/** The DOM id of an approval block, so the session header can send focus to
 *  the prompt that is holding the turn up. */
export function approvalDomId(requestId: string): string {
  return `agent-approval-${requestId}`;
}

/** `2026-09-20T18:04:11Z` → `18:04:11`, or "" when the stamp is unusable. */
export function clockOf(ts: string): string {
  const d = new Date(ts);
  return Number.isNaN(d.getTime())
    ? ""
    : d.toLocaleTimeString("en-GB", { hour12: false });
}

/** The last segment of a Windows or POSIX path — the folder name people
 *  actually recognise. */
export function folderName(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** A path split at its last separator, so the folder name can be emphasised
 *  without printing the whole path twice: `E:\projects\repo` → `E:\projects\`
 *  plus `repo`. */
export function splitPath(path: string): { parent: string; name: string } {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut < 0
    ? { parent: "", name: path }
    : { parent: path.slice(0, cut + 1), name: path.slice(cut + 1) || path };
}
