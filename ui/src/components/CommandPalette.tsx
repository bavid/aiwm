import { useEffect, useMemo, useRef, useState } from "react";
import { useJobs, useModels } from "../lib/hooks";
import "./command-palette.css";

type Entry = {
  id: string;
  group: "Go to" | "Model" | "Job";
  label: string;
  hint?: string;
  run: () => void;
};

const TAB_ENTRIES: { tab: string; label: string }[] = [
  { tab: "dashboard", label: "Dashboard" },
  { tab: "chat", label: "Chat" },
  { tab: "image", label: "Image" },
  { tab: "video", label: "Video" },
  { tab: "jobs", label: "Jobs" },
  { tab: "agents", label: "Agents" },
  { tab: "models", label: "Models" },
  { tab: "benchmark", label: "Benchmark" },
  { tab: "diagnostics", label: "Diagnostics" },
  { tab: "settings", label: "Settings" },
];

/** Global ⌘K / Ctrl+K command palette: jump to a tab, a model (→ Models tab),
 *  or a recent job (→ Jobs tab). Pure client-side filtering over data the
 *  polling hooks already have in memory -- no new endpoints. */
export function CommandPalette({ onNavigate }: { onNavigate: (tab: string) => void }) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const { data: models } = useModels();
  const { data: jobs } = useJobs();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const isCombo = (e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k";
      if (isCombo) {
        e.preventDefault();
        setOpen((o) => !o);
      } else if (e.key === "Escape" && open) {
        setOpen(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      // Focus after the dialog paints.
      const id = window.setTimeout(() => inputRef.current?.focus(), 0);
      return () => window.clearTimeout(id);
    }
  }, [open]);

  const entries = useMemo<Entry[]>(() => {
    const goTo: Entry[] = TAB_ENTRIES.map((t) => ({
      id: `tab-${t.tab}`,
      group: "Go to",
      label: t.label,
      run: () => onNavigate(t.tab),
    }));
    const modelEntries: Entry[] = (models ?? []).map((m) => ({
      id: `model-${m.id}`,
      group: "Model",
      label: m.name,
      hint: m.roles.join(", "),
      run: () => onNavigate("models"),
    }));
    const jobEntries: Entry[] = (jobs ?? []).slice(0, 30).map((j) => ({
      id: `job-${j.id}`,
      group: "Job",
      label: `${j.job_type} · ${j.model_id ?? "auto"}`,
      hint: j.state,
      run: () => onNavigate("jobs"),
    }));
    return [...goTo, ...modelEntries, ...jobEntries];
  }, [models, jobs, onNavigate]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return entries.slice(0, 20);
    return entries.filter((e) => e.label.toLowerCase().includes(q)).slice(0, 20);
  }, [entries, query]);

  const choose = (e: Entry) => {
    e.run();
    setOpen(false);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter" && filtered[active]) {
      e.preventDefault();
      choose(filtered[active]);
    }
  };

  if (!open) return null;

  let lastGroup: string | null = null;

  return (
    <div className="cmdk__backdrop" onClick={() => setOpen(false)}>
      <div
        className="cmdk"
        role="dialog"
        aria-label="Command palette"
        onClick={(e) => e.stopPropagation()}
      >
        <input
          ref={inputRef}
          className="cmdk__input"
          placeholder="Jump to a tab, model, or job…"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
          }}
          onKeyDown={onKeyDown}
        />
        <ul className="cmdk__list">
          {filtered.length === 0 && <li className="cmdk__empty">No matches</li>}
          {filtered.map((e, i) => {
            const showGroup = e.group !== lastGroup;
            lastGroup = e.group;
            return (
              <li key={e.id}>
                {showGroup && <div className="cmdk__group">{e.group}</div>}
                <button
                  type="button"
                  className="cmdk__item"
                  data-active={i === active}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => choose(e)}
                >
                  <span>{e.label}</span>
                  {e.hint && <span className="cmdk__hint">{e.hint}</span>}
                </button>
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}
