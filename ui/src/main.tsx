import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, getTheme } from "./lib/theme";
import "./index.css";

// Apply the saved theme before first paint so there is no flash.
applyTheme(getTheme());

const root = document.getElementById("root");
if (!root) throw new Error("root element missing");

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
