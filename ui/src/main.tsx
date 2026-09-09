import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, getTheme } from "./lib/theme";
import "./index.css";

// Apply the saved theme before first paint so there is no flash.
applyTheme(getTheme());

function mount() {
  const root = document.getElementById("root");
  if (!root) throw new Error("root element missing");
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

// Running `pnpm dev` in a plain browser (no Tauri shell): install a canned IPC
// bridge first so the studios can be eyeballed. Dead code in a production build.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  import("./lib/dev-mock")
    .then((m) => m.installDevMock())
    .finally(mount);
} else {
  mount();
}
