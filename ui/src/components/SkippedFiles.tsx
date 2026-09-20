import type { SkippedFile } from "../lib/ipc";
import { skipReasonLabel } from "./skip-reasons";
import "./skipped-files.css";

/** The files a deletion left alone, and why — collapsed behind a count, since
 *  a large cleanup can skip many (a shared still, a failing drive). */
export function SkippedFiles({ files }: { files: readonly SkippedFile[] }) {
  if (files.length === 0) return null;
  return (
    <details className="skipped">
      <summary>
        {files.length.toLocaleString()} {files.length === 1 ? "file was" : "files were"} left
        on disk
      </summary>
      <ul>
        {files.map((f) => (
          <li key={`${f.path}|${f.reason}`}>
            <span className="skipped__path">{f.path}</span>
            <span className="skipped__reason">{skipReasonLabel(f)}</span>
          </li>
        ))}
      </ul>
    </details>
  );
}
