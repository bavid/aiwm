import { useState } from "react";
import { Chat } from "./features/chat/Chat";
import { Dashboard } from "./features/dashboard/Dashboard";
import { Diagnostics } from "./features/diagnostics/Diagnostics";
import { Models } from "./features/models/Models";
import { Settings } from "./features/settings/Settings";
import { useAbout } from "./lib/hooks";

type Tab = "dashboard" | "chat" | "models" | "diagnostics" | "settings";

const TABS: { id: Tab; label: string }[] = [
  { id: "dashboard", label: "Dashboard" },
  { id: "chat", label: "Chat" },
  { id: "models", label: "Models" },
  { id: "diagnostics", label: "Diagnostics" },
  { id: "settings", label: "Settings" },
];

export default function App() {
  const [tab, setTab] = useState<Tab>("dashboard");
  const about = useAbout();

  return (
    <div className="app">
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
        {tab === "dashboard" && <Dashboard onOpenChat={() => setTab("chat")} />}
        {tab === "chat" && <Chat />}
        {tab === "models" && <Models />}
        {tab === "diagnostics" && <Diagnostics />}
        {tab === "settings" && <Settings />}
      </main>
    </div>
  );
}
