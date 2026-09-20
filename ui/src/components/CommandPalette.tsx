import { useEffect, useMemo, useRef, useState } from "react";
import { HELP_AREA_LABEL, HELP_TOPICS } from "../help/index.ts";
import type { OpenHelp } from "../help/HelpContext.ts";
import { useJobs, useModels } from "../lib/hooks";
import "./command-palette.css";

type Entry = {
  id: string;
  group: "Go to" | "Help" | "Model" | "Job";
  label: string;
  hint?: string;
  run: () => void;
};

/** Every tab, in sidebar order (`TABS` in `App.tsx`). */
const TAB_ENTRIES: { tab: string; label: string }[] = [
  { tab: "dashboard", label: "Dashboard" },
  { tab: "chat", label: "Chat" },
  { tab: "image", label: "Image" },
  { tab: "video", label: "Video" },
  { tab: "voice", label: "Voice" },
  { tab: "stories", label: "Stories" },
  { tab: "dataset", label: "Dataset" },
  { tab: "training", label: "Training" },
  { tab: "jobs", label: "Jobs" },
  { tab: "agents", label: "Agents" },
  { tab: "models", label: "Models" },
  { tab: "benchmark", label: "Benchmark" },
  { tab: "diagnostics", label: "Diagnostics" },
  { tab: "settings", label: "Settings" },
  { tab: "help", label: "Help" },
];

/** Global ⌘K / Ctrl+K command palette: jump to a tab, a help topic (→ Help
 *  tab, scrolled to it), a model (→ Models tab), or a recent job (→ Jobs
 *  tab). Pure client-side filtering over data the polling hooks already have
 *  in memory -- no new endpoints. */
export function CommandPalette({
  onNavigate,
  onOpenHelp,
}: {
  onNavigate: (tab: string) => void;
  onOpenHelp: OpenHelp;
}) {
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
    const helpEntries: Entry[] = HELP_TOPICS.map((t) => ({
      id: `help-${t.area}-${t.id}`,
      group: "Help",
      label: t.title,
      hint: HELP_AREA_LABEL[t.area],
      run: () => onOpenHelp(t.area, t.id),
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
    return [...goTo, ...helpEntries, ...modelEntries, ...jobEntries];
  }, [models, jobs, onNavigate, onOpenHelp]);

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
    // Presentational: a click on the backdrop itself closes; Escape is
    // handled by the window listener above.
    <div
      className="cmdk__backdrop"
      role="presentation"
      onClick={(e) => {
        if (e.target === e.currentTarget) setOpen(false);
      }}
    >
      <div className="cmdk" role="dialog" aria-label="Command palette">
        <input
          ref={inputRef}
          className="cmdk__input"
          placeholder="Jump to a tab, help topic, model, or job…"
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
