import { useCallback, useState, type ReactNode } from "react";
import { AgentsWorkbench } from "./features/agents/Agents";
import { Chat } from "./features/chat/Chat";
import { Dashboard } from "./features/dashboard/Dashboard";
import { DatasetStudio } from "./features/dataset/Dataset";
import { Diagnostics } from "./features/diagnostics/Diagnostics";
import { ImageStudio, type ImagePrefill } from "./features/image/Image";
import { Jobs } from "./features/jobs/Jobs";
import { Models } from "./features/models/Models";
import { Settings } from "./features/settings/Settings";
import { Stories } from "./features/stories/Stories";
import { Training } from "./features/training/Training";
import { VideoStudio } from "./features/video/Video";
import { Voice } from "./features/voice/Voice";
import { CommandPalette } from "./components/CommandPalette";
import { JobNotifications } from "./components/JobNotifications";
import { ShortcutsHelp } from "./components/ShortcutsHelp";
import { useAbout, useRuntimes } from "./lib/hooks";

type Tab =
  | "dashboard"
  | "chat"
  | "image"
  | "video"
  | "voice"
  | "stories"
  | "dataset"
  | "training"
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
    id: "voice",
    label: "Voice",
    icon: (
      <svg viewBox="0 0 20 20">
        <path d="M10 3.2a2.6 2.6 0 0 1 2.6 2.6v4.4a2.6 2.6 0 1 1-5.2 0V5.8A2.6 2.6 0 0 1 10 3.2Z" />
        <path d="M5.2 9.4v0.8a4.8 4.8 0 0 0 9.6 0v-0.8M10 15v2" />
      </svg>
    ),
  },
  {
    id: "stories",
    label: "Stories",
    icon: (
      <svg viewBox="0 0 20 20">
        <path d="M3 4.8c2-1 4.4-1 6.5.3v9.9c-2.1-1.3-4.5-1.3-6.5-.3Z" />
        <path d="M17 4.8c-2-1-4.4-1-6.5.3v9.9c2.1-1.3 4.5-1.3 6.5-.3Z" />
      </svg>
    ),
  },
  {
    id: "dataset",
    label: "Dataset",
    icon: (
      <svg viewBox="0 0 20 20">
        <rect x="2.5" y="2.5" width="6" height="6" rx="1.2" />
        <rect x="11.5" y="2.5" width="6" height="6" rx="1.2" />
        <rect x="2.5" y="11.5" width="6" height="6" rx="1.2" />
        <rect x="11.5" y="11.5" width="6" height="6" rx="1.2" />
        <path d="M8.5 5.5h3M5.5 8.5v3M14.5 8.5v3M8.5 14.5h3" />
      </svg>
    ),
  },
  {
    id: "training",
    label: "Training",
    icon: (
      <svg viewBox="0 0 20 20">
        <path d="M2.5 15.5V9M7.2 15.5V5.5M11.8 15.5v-7M16.5 15.5v-11" />
        <path d="M2 17.5h16" />
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

  // Two cross-tab hand-overs. Every tab stays mounted (see the comment on
  // <main> below), so these are plain lifted state rather than router state:
  // the source tab sets one, switches tabs, and the target tab clears it once
  // it has taken the value.
  /** Dataset tab -> Training tab: "train a LoRA from this dataset". */
  const [pendingTrainingDataset, setPendingTrainingDataset] = useState<string | null>(null);
  /** Training tab -> Image tab: "test this LoRA with its trigger word". */
  const [imagePrefill, setImagePrefill] = useState<ImagePrefill | null>(null);

  const clearPendingTrainingDataset = useCallback(() => setPendingTrainingDataset(null), []);
  const clearImagePrefill = useCallback(() => setImagePrefill(null), []);

  const trainFromDataset = useCallback((datasetId: string) => {
    setPendingTrainingDataset(datasetId);
    setTab("training");
  }, []);

  const testLora = useCallback((loraModelId: string, prompt: string) => {
    setImagePrefill({ loraModelId, prompt });
    setTab("image");
  }, []);

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

      {/* Every tab stays mounted -- only visibility toggles via `hidden`. A
          tab used to fully unmount on navigation, which wiped any in-progress
          draft (a typed prompt, unsent form state) the moment you looked at
          another tab and came back -- a real, reported bug for Image/Video,
          but the same conditional-mount pattern affected every tab equally. */}
      <main className="main">
        <div hidden={tab !== "dashboard"}>
          <Dashboard onNavigate={navigate} />
        </div>
        <div hidden={tab !== "chat"}>
          <Chat />
        </div>
        <div hidden={tab !== "image"}>
          <ImageStudio prefill={imagePrefill} onPrefillConsumed={clearImagePrefill} />
        </div>
        <div hidden={tab !== "video"}>
          <VideoStudio />
        </div>
        <div hidden={tab !== "voice"}>
          <Voice />
        </div>
        <div hidden={tab !== "stories"}>
          <Stories />
        </div>
        <div hidden={tab !== "dataset"}>
          <DatasetStudio onTrainLora={trainFromDataset} />
        </div>
        <div hidden={tab !== "training"}>
          <Training
            pendingDatasetId={pendingTrainingDataset}
            onPendingDatasetConsumed={clearPendingTrainingDataset}
            onTestLora={testLora}
          />
        </div>
        <div hidden={tab !== "jobs"}>
          <Jobs />
        </div>
        <div hidden={tab !== "agents"}>
          <AgentsWorkbench />
        </div>
        <div hidden={tab !== "models"}>
          <Models />
        </div>
        <div hidden={tab !== "diagnostics"}>
          <Diagnostics />
        </div>
        <div hidden={tab !== "settings"}>
          <Settings />
        </div>
      </main>
    </div>
  );
}
