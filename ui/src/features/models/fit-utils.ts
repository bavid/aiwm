import type { FitVerdict } from "../../lib/ipc";

/** The four hardware-fit tiers, exactly as the core reports them
 *  (`core::compat`). The UI never re-judges fit — it only labels, sorts and
 *  counts the verdict the server already produced. */
export type FitTier = FitVerdict["level"];

/** Short tier names for the badge. Deliberately terse so a row stays one
 *  line; the core's own `reason` carries the detail. */
export const FIT_TIER_LABEL: Record<FitTier, string> = {
  green: "Fits",
  yellow: "Tight",
  red: "Too big",
  unknown: "Unknown",
};

/** The one tier order in the UI, best first — both the sort rank and the
 *  summary line read it. `unknown` sorts *before* `red`: an unjudged file is
 *  still worth a look (the source gave no size estimate), one that is known
 *  not to fit is the last resort.
 *
 *  This is a *display* order. An unattended pick (e.g. the upgrade check's
 *  automatic download) must not read it as "unknown beats too big" — see
 *  `UpgradeChecks.tsx`. */
const FIT_TIER_ORDER: FitTier[] = ["green", "yellow", "unknown", "red"];

export const fitTierRank = (tier: FitTier): number => FIT_TIER_ORDER.indexOf(tier);

/** The core's plain-language explanation, when there is one — `green` and
 *  `unknown` carry no reason, so this returns `null` for them. */
export const fitReason = (fit: FitVerdict): string | null =>
  "reason" in fit ? fit.reason : null;

/** A VRAM estimate as GB (`~6.1 GB`), or `null` when the source did not
 *  estimate one. Never guesses a number. */
export const fitVramGb = (mb: number | null | undefined): string | null =>
  mb == null ? null : `~${(mb / 1024).toFixed(1)} GB`;

/** Tier-first ordering. `Array#sort` is stable (ES2019), so files inside the
 *  same tier keep the order the source listed them in. Returns a new array —
 *  the input is never reordered in place. */
export function sortByFitTier<T extends { fit: FitVerdict }>(items: readonly T[]): T[] {
  return [...items].sort((a, b) => fitTierRank(a.fit.level) - fitTierRank(b.fit.level));
}

export type FitTierCounts = Record<FitTier, number>;

export function countFitTiers(items: readonly { fit: FitVerdict }[]): FitTierCounts {
  const of = (tier: FitTier) => items.filter((i) => i.fit.level === tier).length;
  return { green: of("green"), yellow: of("yellow"), red: of("red"), unknown: of("unknown") };
}

const FIT_SUMMARY_WORD: Record<FitTier, string> = {
  green: "fit",
  yellow: "tight",
  red: "too big",
  unknown: "unrated",
};

/** One line like `3 fit · 2 tight · 4 too big for this GPU`. Empty tiers are
 *  left out; `null` when there is nothing to summarise. */
export function fitTierSummary(counts: FitTierCounts): string | null {
  const parts = FIT_TIER_ORDER.filter((t) => counts[t] > 0).map(
    (t) => `${counts[t]} ${FIT_SUMMARY_WORD[t]}`,
  );
  return parts.length === 0 ? null : `${parts.join(" · ")} for this GPU`;
}
