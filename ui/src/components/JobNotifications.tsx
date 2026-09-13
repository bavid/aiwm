import { useEffect, useRef, useState } from "react";
import { useJobs } from "../lib/hooks";
import type { Job } from "../lib/ipc";
import "./job-notifications.css";

const TERMINAL = new Set(["completed", "failed", "cancelled"]);
const AUTO_DISMISS_MS = 6000;

type Toast = { id: string; job: Job };

/** Watches the job list and pops a toast the moment a job crosses into a
 *  terminal state -- so a long image/video render finishing while you're on
 *  a different tab doesn't go unnoticed until you happen to check Jobs. */
export function JobNotifications({ onNavigate }: { onNavigate: (tab: string) => void }) {
  const { data: jobs } = useJobs();
  const [toasts, setToasts] = useState<Toast[]>([]);
  const prevStates = useRef<Map<string, string>>(new Map());
  const seeded = useRef(false);

  useEffect(() => {
    if (!jobs) return;
    const prev = prevStates.current;

    // The first poll after mount seeds state without toasting -- otherwise
    // every already-finished job in history would fire a toast on load.
    if (!seeded.current) {
      seeded.current = true;
      for (const j of jobs) prev.set(j.id, j.state);
      return;
    }

    const fresh: Toast[] = [];
    for (const j of jobs) {
      const before = prev.get(j.id);
      if (TERMINAL.has(j.state) && before && before !== j.state && !TERMINAL.has(before)) {
        fresh.push({ id: j.id, job: j });
      }
      prev.set(j.id, j.state);
    }
    if (fresh.length > 0) {
      setToasts((ts) => [...ts, ...fresh]);
      fresh.forEach((t) => {
        window.setTimeout(() => {
          setToasts((ts) => ts.filter((x) => x.id !== t.id));
        }, AUTO_DISMISS_MS);
      });
    }
  }, [jobs]);

  const dismiss = (id: string) => setToasts((ts) => ts.filter((t) => t.id !== id));

  if (toasts.length === 0) return null;

  return (
    <div className="toaststack" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          className="toast"
          data-state={t.job.state}
          onClick={() => {
            onNavigate("jobs");
            dismiss(t.id);
          }}
        >
          <div className="toast__body">
            <span className="toast__title">
              {t.job.job_type} {t.job.state === "completed" ? "finished" : t.job.state}
            </span>
            {t.job.model_id && <span className="toast__sub">{t.job.model_id}</span>}
            {t.job.error_text && <span className="toast__sub">{t.job.error_text}</span>}
          </div>
          <button
            type="button"
            className="toast__close"
            onClick={(e) => {
              e.stopPropagation();
              dismiss(t.id);
            }}
            aria-label="Dismiss"
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
