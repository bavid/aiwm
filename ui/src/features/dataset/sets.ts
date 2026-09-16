import type { DatasetFrame } from "../../lib/ipc";

export type SetGrouping = "clip" | "similarity";

/** How many frames one guided set holds — small enough to judge in one pass,
 *  large enough that a concept comes together in a handful of sets. */
export const SET_SIZE = 30;

/** Groups kept frames into sets of at most SET_SIZE. "clip": consecutive
 *  frames of the same source_path. "similarity": hashes are not sent to the
 *  UI (a stored hash column is deferred to Plan 2), so this uses tag + source
 *  as a coarse proxy on the client; server-side grouping is a follow-up.
 *
 *  Lives in its own module rather than in `LearnSets.tsx` so that file only
 *  exports components (`react-refresh/only-export-components`). */
export function buildSets(frames: DatasetFrame[], grouping: SetGrouping): DatasetFrame[][] {
  const kept = frames.filter((f) => f.rejection_reason === "" && !f.excluded);
  const key = (f: DatasetFrame) =>
    grouping === "clip" ? f.source_path : `${f.tag}::${f.source_path}`;
  const buckets = new Map<string, DatasetFrame[]>();
  for (const f of kept) {
    const k = key(f);
    const bucket = buckets.get(k);
    if (bucket) bucket.push(f);
    else buckets.set(k, [f]);
  }
  const sets: DatasetFrame[][] = [];
  for (const bucket of buckets.values()) {
    for (let i = 0; i < bucket.length; i += SET_SIZE) sets.push(bucket.slice(i, i + SET_SIZE));
  }
  return sets;
}

/** The last path segment of a Windows or POSIX path, for the set header. */
export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}
