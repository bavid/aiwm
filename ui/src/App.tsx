import { useState, type ReactNode } from "react";
import { AgentsWorkbench } from "./features/agents/Agents";
import { Chat } from "./features/chat/Chat";
import { Dashboard } from "./features/dashboard/Dashboard";
import { Diagnostics } from "./features/diagnostics/Diagnostics";
import { ImageStudio } from "./features/image/Image";
import { Jobs } from "./features/jobs/Jobs";
import { Models } from "./features/models/Models";
import { Settings } from "./features/settings/Settings";
import { VideoStudio } from "./features/video/Video";
import { CommandPalette } from "./components/CommandPalette";
import { JobNotifications } from "./components/JobNotifications";
import { ShortcutsHelp } from "./components/ShortcutsHelp";
import { useAbout, useRuntimes } from "./lib/hooks";

type Tab =
  | "dashboard"
  | "chat"
  | "image"
  | "video"
  | "jobs"
  | "agents"
  | "models"
  | "diagnostics"
  | "settings";

const TABS: { id: Tab; label: string; icon: ReactNode }[] = [
  {
    id: "dashboard",
    label: "Dashboard",
    icon: (
      <svg viewBox="0 0 20 20">
        <rect x="2.5" y="2.5" width="6.2" height="6.2" rx="1.4" />
        <rect x="11.3" y="2.5" width="6.2" height="10.3" rx="1.4" />
        <rect x="2.5" y="11.3" width="6.2" height="6.2" rx="1.4" />
        <rect x="11.3" y="15.4" width="6.2" height="2.1" rx="1" />
      </svg>
    ),
  },
  {
    id: "chat",
    label: "Chat",
    icon: (
      <svg viewBox="0 0 20 20">
        <path d="M3 4.5h14a1 1 0 0 1 1 1V13a1 1 0 0 1-1 1H8l-3.6 3v-3H3a1 1 0 0 1-1-1V5.5a1 1 0 0 1 1-1Z" />
      </svg>
    ),
  },
  {
    id: "image",
    label: "Image",
    icon: (
      <svg viewBox="0 0 20 20">
        <rect x="2.5" y="3.5" width="15" height="13" rx="1.6" />
        <circle cx="7.3" cy="8" r="1.5" />
        <path d="m4 14 3.6-4 3 2.7 2.4-3 3 4.3" />
      </svg>
    ),
  },
  {
    id: "video",
    label: "Video",
    icon: (
      <svg viewBox="0 0 20 20">
        <rect x="2.5" y="4.5" width="12" height="11" rx="1.6" />
        <path d="M14.5 8.6 18 6.4v7.2l-3.5-2.2Z" />
      </svg>
    ),
  },
  {
    id: "jobs",
    label: "Jobs",
    icon: (
      <svg viewBox="0 0 20 20">
        <path d="M3 5.5h14M3 10h14M3 14.5h9" />
        <circle cx="16.3" cy="14.5" r="1.1" fill="currentColor" />
      </svg>
    ),
  },
  {
    id: "agents",
    label: "Agents",
    icon: (
      <svg viewBox="0 0 20 20">
        <rect x="2.8" y="3.5" width="14.4" height="10.3" rx="1.6" />
        <path d="M6.5 9.2l2 2-2 2M10.5 13.2h3M2.8 16.5h14.4" />
      </svg>
    ),
  },
  {
    id: "models",
    label: "Models",
    icon: (
      <svg viewBox="0 0 20 20">
        <ellipse cx="10" cy="4.8" rx="6.8" ry="2.3" />
        <path d="M3.2 4.8V15c0 1.27 3.04 2.3 6.8 2.3s6.8-1.03 6.8-2.3V4.8" />
        <path d="M3.2 10c0 1.27 3.04 2.3 6.8 2.3s6.8-1.03 6.8-2.3" />
      </svg>
    ),
  },
  {
    id: "diagnostics",
    label: "Diagnostics",
    icon: (
      <svg viewBox="0 0 20 20">
        <path d="M2.5 11.5 6 8l3 3.2L15 5l2.5 2.6" />
        <circle cx="10" cy="15.6" r="1" fill="currentColor" />
      </svg>
    ),
  },
  {
    id: "settings",
    label: "Settings",
    icon: (
      <svg viewBox="0 0 20 20">
        <circle cx="10" cy="10" r="2.6" />
        <path d="M10 2.8v2M10 15.2v2M17.2 10h-2M4.8 10h-2M15.1 4.9l-1.4 1.4M6.3 13.7l-1.4 1.4M15.1 15.1l-1.4-1.4M6.3 6.3 4.9 4.9" />
      </svg>
    ),
  },
];

const SIDEBAR_COLLAPSED_KEY = "aiwm:sidebar-collapsed";

export default function App() {
  const [tab, setTab] = useState<Tab>("dashboard");
  const [collapsed, setCollapsed] = useState(
    () => window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "true",
  );
  const about = useAbout();
  const { data: runtimes } = useRuntimes();
  const navigate = (t: string) => setTab(t as Tab);

  const toggleCollapsed = () => {
    setCollapsed((c) => {
      const next = !c;
      try {
        window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(next));
      } catch {
        /* localStorage unavailable -- collapse state just won't stick */
      }
      return next;
    });
  };

  const runtimesOnline = (runtimes ?? []).filter((r) => r.health === "healthy").length;

  return (
    <div className="app" data-sidebar-collapsed={collapsed}>
      <CommandPalette onNavigate={navigate} />
      <JobNotifications onNavigate={navigate} />
      <ShortcutsHelp />

      <aside className="sidebar" aria-label="Primary">
        <div className="brand">
          <div className="brand__mark">Ai</div>
          <div className="brand__meta">
            <div className="brand__name">AIWM</div>
            <div className="brand__rig">{about?.offline_mode ? "offline" : "AI Workstation Manager"}</div>
          </div>
        </div>

        <nav className="navgroup" role="tablist" aria-label="Views">
          {TABS.map((t) => (
            <button
              key={t.id}
              className="navitem"
              role="tab"
              aria-selected={tab === t.id}
              onClick={() => setTab(t.id)}
            >
              {t.icon}
              <span className="label">{t.label}</span>
            </button>
          ))}
        </nav>

        <div className="sidebar__spacer" />

        <div className="sidebar__status">
          <span className="status-dot" data-state={runtimesOnline > 0 ? "healthy" : undefined} />
          <span>{runtimes ? `${runtimesOnline} of ${runtimes.length} runtimes online` : "…"}</span>
        </div>
        <button
          type="button"
          className="collapse-btn"
          onClick={toggleCollapsed}
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          <svg viewBox="0 0 20 20">
            <path d="M12.5 4 7 10l5.5 6" />
          </svg>
          <span className="label">Collapse</span>
        </button>
      </aside>

      <main className="main">
        {tab === "dashboard" && <Dashboard onNavigate={navigate} />}
        {tab === "chat" && <Chat />}
        {tab === "image" && <ImageStudio />}
        {tab === "video" && <VideoStudio />}
        {tab === "jobs" && <Jobs />}
        {tab === "agents" && <AgentsWorkbench />}
        {tab === "models" && <Models />}
        {tab === "diagnostics" && <Diagnostics />}
        {tab === "settings" && <Settings />}
      </main>
    </div>
  );
}
