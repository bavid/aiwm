import type { AgentEventPayload, ToolStatus } from "../../lib/ipc";

/** One rendered transcript entry — a fold of the raw `AgentEvent` stream.
 *  `ts` is the stamp of the event that opened the block (a text block keeps
 *  the first delta's time, a tool card the time it was announced), so the
 *  transcript can be read as a timeline. */
export type TextBlock = { block: "text"; ts: string; text: string };
export type ToolBlock = {
  block: "tool";
  ts: string;
  id: string;
  name: string;
  status: ToolStatus;
  command: string | null;
  output: string | null;
};
export type PermissionBlock = {
  block: "permission";
  ts: string;
  id: string;
  kind: string;
  summary: string;
  always: string | null;
};
export type IdleBlock = { block: "idle"; ts: string };
export type ErrorBlock = { block: "error"; ts: string; message: string };
export type Block = TextBlock | ToolBlock | PermissionBlock | IdleBlock | ErrorBlock;

/** A short one-line label for a tool call's input — the shell command if there
 *  is one, else compact JSON, else nothing. */
function commandOf(input: unknown): string | null {
  if (input && typeof input === "object" && "command" in input) {
    const c = (input as { command?: unknown }).command;
    if (typeof c === "string") return c;
  }
  if (input == null) return null;
  try {
    const s = JSON.stringify(input);
    return s === "{}" ? null : s;
  } catch {
    return null;
  }
}

/** Fold the raw event stream into a readable transcript: consecutive text
 *  deltas merge, a tool call updates in place by id, a permission ask shows
 *  once. */
export function groupEvents(events: { ts?: string; payload: AgentEventPayload }[]): Block[] {
  const blocks: Block[] = [];
  for (const { ts: rawTs, payload } of events) {
    const ts = rawTs ?? "";
    if (payload.type === "text") {
      const last = blocks[blocks.length - 1];
      if (last && last.block === "text") last.text += payload.text;
      else if (payload.text) blocks.push({ block: "text", ts, text: payload.text });
    } else if (payload.type === "tool") {
      const existing = blocks.find(
        (b): b is ToolBlock => b.block === "tool" && b.id === payload.id,
      );
      const command = commandOf(payload.input);
      if (existing) {
        existing.status = payload.status;
        if (command) existing.command = command;
        if (payload.output != null) existing.output = payload.output;
      } else {
        blocks.push({
          block: "tool",
          ts,
          id: payload.id || `t${blocks.length}`,
          name: payload.name,
          status: payload.status,
          command,
          output: payload.output ?? null,
        });
      }
    } else if (payload.type === "permission") {
      if (!blocks.some((b) => b.block === "permission" && b.id === payload.id)) {
        blocks.push({
          block: "permission",
          ts,
          id: payload.id,
          kind: payload.kind,
          summary: payload.summary,
          always: payload.always_pattern ?? null,
        });
      }
    } else if (payload.type === "idle") {
      if (blocks[blocks.length - 1]?.block !== "idle") blocks.push({ block: "idle", ts });
    } else if (payload.type === "error") {
      blocks.push({ block: "error", ts, message: payload.message });
    }
  }
  return blocks;
}
