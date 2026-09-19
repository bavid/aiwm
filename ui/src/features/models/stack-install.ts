import type { Download, DownloadState, KnownModel, Model, ModelStack } from "../../lib/ipc";

/** Where one file of a catalog stack stands, derived from the model library
 *  (already imported?) and the download queue (in flight? failed?). */
export type MemberPhase =
  | "installed"
  | "queued"
  | "downloading"
  | "paused"
  | "verifying"
  | "failed"
  | "missing";

export interface MemberProgress {
  member: KnownModel;
  phase: MemberPhase;
  bytesDone: number;
  /** The download row behind `phase`, when there is one — a failed one is
   *  resumed rather than queued again. */
  downloadId: string | null;
  error: string | null;
}

/** `installed` only once **every** file is in the library — the same
 *  complete-directory rule the core uses before it calls a captioner
 *  installed. */
export type StackPhase = "installed" | "installing" | "failed" | "missing";

export interface StackProgress {
  phase: StackPhase;
  members: MemberProgress[];
  bytesDone: number;
  bytesTotal: number;
  /** 0–100, by bytes. */
  percent: number;
  installedCount: number;
  /** The file transferring right now, for a one-line summary. */
  current: MemberProgress | null;
}

const ACTIVE_STATES: ReadonlySet<DownloadState> = new Set([
  "queued",
  "running",
  "paused",
  "verifying",
]);

const IN_FLIGHT: ReadonlySet<MemberPhase> = new Set([
  "queued",
  "downloading",
  "paused",
  "verifying",
]);

const sameHash = (a: string | null, b: string) => a !== null && a.toLowerCase() === b.toLowerCase();

/** The download row that says most about `member`: an active one, else the
 *  newest finished one, else the newest failed one. The queue lists newest
 *  first, so `find` picks the latest of each kind. */
function downloadFor(member: KnownModel, downloads: readonly Download[]): Download | null {
  const rows = downloads.filter((d) => sameHash(d.sha256, member.sha256));
  return (
    rows.find((d) => ACTIVE_STATES.has(d.state)) ??
    rows.find((d) => d.state === "done") ??
    rows.find((d) => d.state === "failed") ??
    null
  );
}

function memberProgress(
  member: KnownModel,
  downloads: readonly Download[],
  models: readonly Model[],
): MemberProgress {
  const installed: MemberProgress = {
    member,
    phase: "installed",
    bytesDone: member.size_bytes,
    downloadId: null,
    error: null,
  };
  if (models.some((m) => sameHash(m.sha256, member.sha256))) return installed;

  const d = downloadFor(member, downloads);
  if (!d) return { member, phase: "missing", bytesDone: 0, downloadId: null, error: null };
  // `done` means verified *and* imported; the library poll just has not
  // caught up yet.
  if (d.state === "done") return { ...installed, downloadId: d.id };

  const phase: MemberPhase = d.state === "running" ? "downloading" : d.state;
  const bytesDone = phase === "verifying" ? member.size_bytes : Math.min(d.bytes_done, member.size_bytes);
  return { member, phase, bytesDone, downloadId: d.id, error: d.error_text };
}

function stackPhase(members: readonly MemberProgress[]): StackPhase {
  if (members.every((m) => m.phase === "installed")) return "installed";
  if (members.some((m) => IN_FLIGHT.has(m.phase))) return "installing";
  if (members.some((m) => m.phase === "failed")) return "failed";
  return "missing";
}

export function stackProgress(
  stack: ModelStack,
  downloads: readonly Download[],
  models: readonly Model[],
): StackProgress {
  const members = stack.members.map((m) => memberProgress(m, downloads, models));
  const bytesTotal = members.reduce((sum, m) => sum + m.member.size_bytes, 0);
  const bytesDone = members.reduce((sum, m) => sum + m.bytesDone, 0);
  return {
    phase: stackPhase(members),
    members,
    bytesDone,
    bytesTotal,
    percent: bytesTotal > 0 ? Math.min(100, (bytesDone / bytesTotal) * 100) : 0,
    installedCount: members.filter((m) => m.phase === "installed").length,
    current:
      members.find((m) => m.phase === "downloading" || m.phase === "verifying") ??
      members.find((m) => IN_FLIGHT.has(m.phase)) ??
      null,
  };
}

/** The files an "Install" click still has to start: never downloaded, or
 *  failed (those get resumed). Anything in flight or installed is left alone,
 *  so clicking Install from two tabs never queues a file twice. */
export function pendingMembers(p: StackProgress): MemberProgress[] {
  return p.members.filter((m) => m.phase === "missing" || m.phase === "failed");
}

/** Decimal gigabytes, one place — the unit download sizes are quoted in on
 *  model cards ("1.3 GB"). */
export const decimalGb = (bytes: number) => `${(bytes / 1e9).toFixed(1)} GB`;

/** A stack's whole download size. */
export const stackBytes = (stack: ModelStack) =>
  stack.members.reduce((sum, m) => sum + m.size_bytes, 0);
