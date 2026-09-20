import { createContext, useContext } from "react";
import type { HelpArea } from "./types.ts";

/** A request to open the Help tab at one area, optionally scrolled to a
 *  setting (its key) or a topic (its id). Lifted state in `App.tsx`, like
 *  `ModelsFocus`: the Help tab clears it once it has jumped. */
export type HelpFocus = { area: HelpArea; key?: string };

export type OpenHelp = (area: HelpArea, key?: string) => void;

/** How a `HelpHint` anywhere in the tree reaches the Help tab. A context
 *  rather than a prop: the hints sit five or six components below `App`, in
 *  forms that would otherwise all have to thread one callback through. */
export const HelpNavigationContext = createContext<OpenHelp | null>(null);

/** `null` outside a provider — the hint then simply has no "More in Help". */
export const useOpenHelp = (): OpenHelp | null => useContext(HelpNavigationContext);
