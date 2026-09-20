#!/usr/bin/env node
// Consistency check for the in-app help (Plan 12), run by `pnpm lint:help`
// (and so by `pnpm lint`). Fails when:
//   - a `<HelpHint area="…" setting="…">` anywhere under src/ names an area
//     or setting key that is not in the registry (or writes them as anything
//     but string literals — the scan is textual, so a computed value cannot
//     be checked);
//   - an area has no topic;
//   - a setting has an empty `what`, `why`, `effect` or `benefit`;
//   - a topic id or setting key is not unique within its area, or is not
//     `[a-z0-9-]` (they become DOM ids).
//
// How the content is loaded: the modules under src/help/ are plain TypeScript
// with `.ts` import specifiers and no React, and Node >= 22.6 strips types on
// import (on by default since 23.6; `tsconfig` has `allowImportingTsExtensions`
// so the same specifiers satisfy `tsc` and Vite). No build step, no extra
// dependency. Node 24 is what the repo uses.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const srcDir = join(here, "..", "src");
const problems = [];
const ID_RE = /^[a-z0-9-]+$/;

const { HELP_AREAS, findSetting } = await import(
  pathToFileURL(join(srcDir, "help", "index.ts")).href
);

// --- 1. content ---------------------------------------------------------
let settingsTotal = 0;
for (const area of HELP_AREAS) {
  if (area.topics.length === 0) problems.push(`area "${area.id}" has no topic`);
  const topicIds = new Set();
  const keys = new Set();
  for (const topic of area.topics) {
    if (topic.area !== area.id) {
      problems.push(`topic "${topic.id}" says area "${topic.area}" but is listed under "${area.id}"`);
    }
    if (!ID_RE.test(topic.id)) problems.push(`topic id "${topic.id}" (${area.id}) is not [a-z0-9-]`);
    if (topicIds.has(topic.id)) problems.push(`duplicate topic id "${topic.id}" in ${area.id}`);
    topicIds.add(topic.id);
    if (!topic.title.trim() || !topic.summary.trim()) {
      problems.push(`topic "${topic.id}" (${area.id}) has an empty title or summary`);
    }
    for (const s of topic.settings ?? []) {
      settingsTotal += 1;
      if (!ID_RE.test(s.key)) problems.push(`setting key "${s.key}" (${area.id}) is not [a-z0-9-]`);
      if (keys.has(s.key)) problems.push(`duplicate setting key "${s.key}" in ${area.id}`);
      keys.add(s.key);
      for (const field of ["label", "what", "why", "effect", "benefit"]) {
        if (!String(s[field] ?? "").trim()) {
          problems.push(`setting "${s.key}" (${area.id}) has an empty ${field}`);
        }
      }
    }
  }
}

// --- 2. hints in the source ----------------------------------------------
function* walk(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) yield* walk(p);
    else if (/\.tsx$/.test(name)) yield p;
  }
}

const HINT_RE = /<HelpHint\b([\s\S]*?)\/?>/g;
let hints = 0;
for (const file of walk(srcDir)) {
  const text = readFileSync(file, "utf8");
  const where = relative(join(here, ".."), file);
  for (const m of text.matchAll(HINT_RE)) {
    hints += 1;
    const props = m[1];
    const area = /\barea="([^"]*)"/.exec(props)?.[1];
    const key = /\bsetting="([^"]*)"/.exec(props)?.[1];
    if (area === undefined || key === undefined) {
      problems.push(`${where}: a <HelpHint> without literal area="…" and setting="…" props`);
      continue;
    }
    if (!HELP_AREAS.some((a) => a.id === area)) {
      problems.push(`${where}: <HelpHint area="${area}"> — unknown area`);
    } else if (!findSetting(area, key)) {
      problems.push(`${where}: <HelpHint area="${area}" setting="${key}"> — no such setting`);
    }
  }
}

// --- report ---------------------------------------------------------------
if (problems.length > 0) {
  console.error(`check-help: ${problems.length} problem(s)`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log(
  `check-help: ok — ${HELP_AREAS.length} areas, ${HELP_AREAS.reduce((n, a) => n + a.topics.length, 0)} topics, ${settingsTotal} settings, ${hints} hints`,
);
