import { useState } from "react";
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
import { useAbout } from "./lib/hooks";

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

const TABS: { id: Tab; label: string }[] = [
  { id: "dashboard", label: "Dashboard" },
  { id: "chat", label: "Chat" },
  { id: "image", label: "Image" },
  { id: "video", label: "Video" },
  { id: "jobs", label: "Jobs" },
  { id: "agents", label: "Agents" },
  { id: "models", label: "Models" },
  { id: "diagnostics", label: "Diagnostics" },
  { id: "settings", label: "Settings" },
];

export default function App() {
  const [tab, setTab] = useState<Tab>("dashboard");
  const about = useAbout();
  const navigate = (t: string) => setTab(t as Tab);

  return (
    <div className="app">
      <CommandPalette onNavigate={navigate} />
      <JobNotifications onNavigate={navigate} />
      <ShortcutsHelp />
      <header className="topbar">
        <div className="topbar__brand">
          AI Workstation <span>Manager</span>
        </div>
        {about?.offline_mode && <span className="topbar__brand">· offline</span>}
        <nav className="tabs" role="tablist" aria-label="Views">
          {TABS.map((t) => (
            <button
              key={t.id}
              className="tab"
              role="tab"
              aria-selected={tab === t.id}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>
      </header>

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
