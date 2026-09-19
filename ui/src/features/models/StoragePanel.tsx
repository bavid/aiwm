import { useMemo, useState } from "react";
import { useStorage } from "../../lib/hooks";
import { deleteModel, type ModelDisk } from "../../lib/ipc";
import { formatGB } from "../../lib/units";


/** Disk usage + the "safe to delete" reports. */
export function StoragePanel() {
  const { data: report } = useStorage();
  const byId = useMemo(
    () => new Map((report?.models ?? []).map((m) => [m.id, m])),
    [report],
  );
  if (!report) return null;

  const { store_bytes, volume_free_bytes, volume_total_bytes, by_kind, duplicates, unused } =
    report;
  const pct =
    volume_total_bytes && volume_total_bytes > 0
      ? (store_bytes / volume_total_bytes) * 100
      : null;
  const wasted = duplicates.reduce((s, d) => s + d.wasted_bytes, 0);

  return (
    <section className="card card--wide">
      <header className="card__head">
        <h2>Storage</h2>
        <span className="card__sub numeric">
          {formatGB(store_bytes)} in models
          {volume_free_bytes != null && ` · ${formatGB(volume_free_bytes)} free on the store volume`}
        </span>
      </header>

      {pct != null && (
        <div className="st__bar" title={`store models take ~${pct.toFixed(1)}% of the volume`}>
          <div className="st__bar-fill" style={{ width: `${Math.min(100, pct)}%` }} />
        </div>
      )}

      <div className="st__kinds">
        {by_kind.map((k) => (
          <span key={k.kind} className="chip">
            {k.kind} <span className="muted numeric">{formatGB(k.bytes)} · {k.count}</span>
          </span>
        ))}
      </div>

      {duplicates.length > 0 && (
        <div className="st__group">
          <h3>
            Duplicates <span className="muted">— ~{formatGB(wasted)} wasted across {duplicates.length} group{duplicates.length === 1 ? "" : "s"}</span>
          </h3>
          <ul className="st__list">
            {duplicates.map((d) => (
              <li key={d.sha256} className="st__dup">
                <code className="muted">{d.sha256.slice(0, 10)}…</code>
                {d.member_ids.map((id, i) => {
                  const m = byId.get(id);
                  if (!m) return null;
                  return (
                    <span key={id} className="st__member">
                      {m.name}
                      {i === 0 ? (
                        <span className="badge">keep</span>
                      ) : (
                        <DeleteButton model={m} />
                      )}
                    </span>
                  );
                })}
              </li>
            ))}
          </ul>
        </div>
      )}

      {unused.length > 0 && (
        <div className="st__group">
          <h3>
            Unused <span className="muted">— never used, or idle for {report.stale_days}+ days</span>
          </h3>
          <ul className="st__list">
            {unused
              .map((id) => byId.get(id))
              .filter((m): m is ModelDisk => !!m)
              .map((m) => (
                <li key={m.id} className="st__unused">
                  <span className="st__name">{m.name}</span>
                  <span className="muted numeric">
                    {formatGB(m.size_bytes)} · {m.last_used_at ? `last used ${m.last_used_at.slice(0, 10)}` : "never used"}
                  </span>
                  <DeleteButton model={m} />
                </li>
              ))}
          </ul>
        </div>
      )}

      {duplicates.length === 0 && unused.length === 0 && (
        <p className="muted">Nothing to clean up — no duplicates, everything used recently.</p>
      )}
    </section>
  );
}

export function DeleteButton({
  model,
}: {
  model: { id: string; name: string; size_bytes: number };
}) {
  const [state, setState] = useState<"idle" | "working" | "error">("idle");

  const run = async () => {
    if (
      !window.confirm(
        `Permanently delete “${model.name}” (${formatGB(model.size_bytes)})?\n\nThe file, its runtime links and its benchmark history are removed. This cannot be undone.`,
      )
    )
      return;
    setState("working");
    try {
      await deleteModel(model.id);
      // the polled lists drop it within a few seconds
    } catch (e) {
      setState("error");
      window.alert(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <button
      type="button"
      className="st__del"
      disabled={state === "working"}
      onClick={run}
    >
      {state === "working" ? "…" : state === "error" ? "retry" : "Delete"}
    </button>
  );
}
