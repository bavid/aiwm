import type { HelpTopic } from "./types.ts";

/** Keyboard shortcuts across the app. Facts from
 *  `components/ShortcutsHelp.tsx`, `CommandPalette.tsx`, `Chat.tsx`,
 *  `dataset/SelectionBar.tsx` / `useFrameSelection.ts`,
 *  `chat/personas/PersonaMenu.tsx`, `models/Catalog.tsx` and
 *  `components/HelpHint.tsx`. */

export const SHORTCUTS_TOPICS: readonly HelpTopic[] = [
  {
    id: "global",
    area: "shortcuts",
    title: "Everywhere",
    summary: "Ctrl+K opens the command palette; ? lists the shortcuts; Escape closes.",
    body: [
      {
        kind: "list",
        items: [
          "Ctrl+K (⌘K on a Mac) — the command palette: jump to a tab, a help topic, a model or a recent job. Type to filter, arrows to move, Enter to go.",
          "? — the shortcut list, with a button to this Help tab. Not while typing in a field.",
          "Escape — closes the palette, the shortcut list, a persona menu, an open ? hint (and returns focus to it), and cancels an in-place rename.",
          "Tab / Shift+Tab — every control, including the ? hints and the fit badges, is in the tab order; Enter or Space opens them.",
        ],
      },
    ],
  },
  {
    id: "chat",
    area: "shortcuts",
    title: "Chat and agents",
    summary: "Enter sends; Shift+Enter is a newline.",
    body: [
      {
        kind: "list",
        items: [
          "Enter — send the message (Chat, the agent composer, the prompt assistant).",
          "Shift+Enter — a newline inside the message.",
          "Persona menu: arrows move between choices with wrap-around, Home / End jump, Enter picks, Escape or Tab closes and returns to the chip.",
        ],
      },
    ],
  },
  {
    id: "dataset",
    area: "shortcuts",
    title: "Dataset curation board",
    summary: "Select with the mouse and the modifiers; K, D, Delete and Escape act on the selection.",
    body: [
      {
        kind: "list",
        items: [
          "Click — select one frame; Ctrl-click adds or removes; Shift-click selects a range; drag on empty space draws a box.",
          "Space on a focused card — toggle its selection. Tab lands on one card per column; arrows move within the column.",
          "K — move the selected frames to Keep; D — to Discard.",
          "Delete — delete the selected frames (asks first).",
          "Escape — clear the selection.",
          "Drag a card (or the selection) to the other column.",
        ],
      },
    ],
  },
  {
    id: "lists",
    area: "shortcuts",
    title: "Tabs, menus, fields",
    summary: "Arrow keys in the catalogue tabs and menus; Enter and Escape in in-place renames.",
    body: [
      {
        kind: "list",
        items: [
          "Models → Recommended models: Left / Right move between category tabs (wrapping), Home / End jump to the first / last.",
          "In-place renames (a model's name, a session, a chat, a tag): Enter or click away saves, Escape reverts.",
          "Number fields accept the arrow keys in their own step (64 px for image size, 4 frames for video length).",
        ],
      },
    ],
  },
];
