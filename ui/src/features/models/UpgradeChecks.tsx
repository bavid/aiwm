import { useMemo, useState } from "react";
import { useJobs } from "../../lib/hooks";
import {
  enqueueDownload,
  registryModel,
  type FitVerdict,
  type Job,
  type UpgradeCandidate,
  type UpgradeReport,
} from "../../lib/ipc";

const ACTIVE = new Set(["queued", "scheduled", "blocked", "preparing", "running", "post"]);

const FIT_COLOR: Record<FitVerdict["level"], string> = {
  green: "var(--load-ok)",
  yellow: "var(--load-warn)",
  red: "var(--load-crit)",
  unknown: "var(--border)",
};

const count = (n: number) =>
  n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : `${n}`;
const params = (n: number | null) =>
  n == null ? "" : n >= 1e9 ? `${(n / 1e9).toFixed(1)}B` : `${(n / 1e6).toFixed(0)}M`;

function parseReport(raw: string | null): UpgradeReport | null {
  if (!raw) return null;
  try {
    return JSON.parse(raw) as UpgradeReport;
  } catch {
    return null;
  }
}

/** Running + finished "is there something better?" checks. Hidden when none. */
export function UpgradeChecks() {
  const { data: jobs } = useJobs();
  const checks = useMemo(
    () => (jobs ?? []).filter((j: Job) => j.job_type === "upgrade_check"),
    [jobs],
  );
  if (checks.length === 0) return null;

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Upgrade check</h2>
        <span className="card__sub">newer / bigger models that fit — quality is not locally verifiable</span>
      </header>
      <ul className="up__list">
        {checks.map((j) => (
          <CheckRow key={j.id} job={j} />
        ))}
      </ul>
    </section>
  );
}

function CheckRow({ job }: { job: Job }) {
  if (ACTIVE.has(job.state)) {
    return <li className="up__row muted">Checking Hugging Face…</li>;
  }
  if (job.state === "failed") {
    return (
      <li className="up__row up__row--failed">
        Check failed{job.error_text ? ` — ${job.error_text}` : ""}.
      </li>
    );
  }
  const report = parseReport(job.result);
  if (!report) return <li className="up__row muted">No result.</li>;

  return (
    <li className="up__row">
      <div className="up__head">
        <strong>{report.target}</strong>
        <span className="muted">
          {" · "}
          {report.candidates.length} suggestion{report.candidates.length === 1 ? "" : "s"}
        </span>
      </div>
      <p className="up__note">{report.note}</p>
      {report.candidates.length > 0 && (
        <ul className="up__cands">
          {report.candidates.map((c) => (
            <Candidate key={c.id} c={c} />
          ))}
        </ul>
      )}
    </li>
  );
}

function Candidate({ c }: { c: UpgradeCandidate }) {
  const [dl, setDl] = useState<"idle" | "working" | "queued" | "error">("idle");

  const download = async () => {
    setDl("working");
    try {
      const details = await registryModel(c.id);
      const files = details.files
        .filter((f) => f.quant && !f.shard)
        .sort((a, b) => {
          const rank = (lvl: string) => (lvl === "green" ? 0 : lvl === "yellow" ? 1 : 2);
          return rank(a.fit.level) - rank(b.fit.level) || a.size_bytes - b.size_bytes;
        });
      const pick = files[0];
      if (!pick || c.gated) {
        setDl("error");
        return;
      }
      await enqueueDownload({
        url: pick.download_url,
        filename: pick.path,
        model_type: c.format === "gguf" ? "chat" : "checkpoint",
        sha256: pick.sha256 ?? undefined,
        size_bytes: pick.size_bytes,
      });
      setDl("queued");
    } catch {
      setDl("error");
    }
  };

  return (
    <li className="up__cand">
      <span className="up__dot" style={{ background: FIT_COLOR[c.fit.level] }} />
      <a
        className="up__id"
        href={`https://huggingface.co/${c.id}`}
        target="_blank"
        rel="noopener noreferrer"
      >
        {c.id}
      </a>
      <span className="up__meta numeric">
        {params(c.param_count) && `${params(c.param_count)} · `}↓ {count(c.downloads)}
        {c.last_modified && ` · ${c.last_modified.slice(0, 7)}`}
      </span>
      <span className="up__why">{c.why}</span>
      {c.installed && <span className="badge">installed</span>}
      {c.gated && <span className="badge badge--warn">gated</span>}
      {!c.installed && !c.gated && (
        <button
          type="button"
          className="discover__dlbtn"
          disabled={dl === "queued" || dl === "working"}
          onClick={download}
        >
          {dl === "queued"
            ? "Queued ✓"
            : dl === "working"
              ? "…"
              : dl === "error"
                ? "retry"
                : "Download & import"}
        </button>
      )}
    </li>
  );
}
