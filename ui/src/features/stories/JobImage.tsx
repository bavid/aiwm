import { useEffect, useState } from "react";
import { useAbout } from "../../lib/hooks";
import { jobDetail, jobOutputUrl, type Job } from "../../lib/ipc";

const DONE_STATES = new Set(["completed", "failed", "cancelled"]);
const POLL_MS = 1500;

/** Renders a `job_type=image` job's output once it finishes, polling while
 *  it's still in flight. Used everywhere Story Studio shows a portrait,
 *  location reference, or scene image -- all of them are just a job id. */
export function JobImage({
  jobId,
  alt,
  className,
}: {
  jobId: string | null;
  alt: string;
  className?: string;
}) {
  const about = useAbout();
  const [job, setJob] = useState<Job | null>(null);

  useEffect(() => {
    if (!jobId) {
      setJob(null);
      return;
    }
    let alive = true;
    const tick = () => {
      jobDetail(jobId)
        .then((detail) => {
          if (alive) setJob(detail?.job ?? null);
        })
        .catch(() => {
          if (alive) setJob(null);
        });
    };
    tick();
    const id = setInterval(() => {
      if (job && DONE_STATES.has(job.state)) return;
      tick();
    }, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
    // `job` is read only to decide whether to keep polling, not to change
    // what is polled -- re-subscribing on every state change would just
    // restart the same interval pointlessly.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId]);

  const wrapperClass = ["job-image", className].filter(Boolean).join(" ");

  if (!jobId) {
    return (
      <div className={`${wrapperClass} job-image--empty`}>
        <span>{alt}</span>
      </div>
    );
  }
  if (!about || !job || job.state !== "completed") {
    const label = job?.state === "failed" ? "generation failed" : "generating…";
    return (
      <div className={`${wrapperClass} job-image--pending`}>
        <span>{label}</span>
      </div>
    );
  }
  return (
    <img
      className={wrapperClass}
      src={jobOutputUrl(about.core_api_port, jobId)}
      alt={alt}
      loading="lazy"
    />
  );
}
