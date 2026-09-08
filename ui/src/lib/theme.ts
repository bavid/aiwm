/** App theme override. "system" follows the OS (prefers-color-scheme); "light"
 *  and "dark" force a palette via a `data-theme` attribute on <html> that
 *  `styles/tokens.css` keys off. Persisted per-machine in localStorage — it is a
 *  pure display preference, never sent to the core. */

export type Theme = "system" | "light" | "dark";

const KEY = "aiwm.theme";
const THEMES: readonly Theme[] = ["system", "light", "dark"];

export function getTheme(): Theme {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw && (THEMES as readonly string[]).includes(raw)) return raw as Theme;
  } catch {
    /* private mode / disabled storage — fall back to system */
  }
  return "system";
}

export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === "system") {
    delete root.dataset.theme;
  } else {
    root.dataset.theme = theme;
  }
}

export function setTheme(theme: Theme): void {
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    /* not persisted, but still applied for this session */
  }
  applyTheme(theme);
}
