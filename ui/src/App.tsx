import { useState } from "react";
import { Dashboard } from "./features/dashboard/Dashboard";
import { Diagnostics } from "./features/diagnostics/Diagnostics";
import { useAbout } from "./lib/hooks";

type Tab = "dashboard" | "diagnostics";

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
          <button
            className="tab"
            role="tab"
            aria-selected={tab === "dashboard"}
            onClick={() => setTab("dashboard")}
          >
            Dashboard
          </button>
          <button
            className="tab"
            role="tab"
            aria-selected={tab === "diagnostics"}
            onClick={() => setTab("diagnostics")}
          >
            Diagnostics
          </button>
        </nav>
      </header>

      <main className="main">{tab === "dashboard" ? <Dashboard /> : <Diagnostics />}</main>
    </div>
  );
}
