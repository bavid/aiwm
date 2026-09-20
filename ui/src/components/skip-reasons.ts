import type { SkippedFile } from "../lib/ipc";

/** Human text for the core's skip reasons — the dataset guard's, the
 *  cleanup page's — keyed by the wire string. */
const SKIP_REASON_LABEL: Record<string, string> = {
  outside_app_folders: "outside the app's folders — not the app's to delete",
  source_file: "a source file — never deleted",
  in_use: "still used by another frame",
  used_by_other_dataset: "used by another dataset",
  not_a_file: "not a file",
  not_offered: "no longer offered by the last scan — scan again",
  missing: "already gone",
  link_not_followed: "a link — never followed",
};

/** Human text for one `SkippedFile.reason`; `"error: …"` and the guard's
 *  sentences are shown verbatim. */
export function skipReasonLabel(file: SkippedFile): string {
  return SKIP_REASON_LABEL[file.reason] ?? file.reason;
}
