import type { HelpSetting, HelpTopic } from "./types.ts";

/** Jobs tab: every job of every kind, the filters, the states, Cancel.
 *  Facts from `features/jobs/Jobs.tsx`, `lib/ipc.ts` (`JobState`,
 *  `deleteJob`, `cancelJob`) and the scheduler notes in `docs/TODO.md`. */

const SETTINGS: readonly HelpSetting[] = [
  {
    key: "type-filter",
    label: "Type",
    what: "Show every job, or only one kind: chat, image, video, upscale, bench (benchmark) or upgrade_check.",
    why: "Each tab's own Queue shows only its own kind; this is the one place that shows benchmark and upgrade-check jobs at all.",
    effect: "Filters the table; nothing else changes. A chat job that was really a prompt-assistant draft is labelled \"chat — image prompt\" (or video / edit / narrate) so it is not mistaken for a conversation.",
    benefit: "Find the job you are looking for among the last 300.",
  },
  {
    key: "state-filter",
    label: "State",
    what: "All, Active (queued, scheduled, blocked, preparing, running, post), Completed, Failed or Cancelled.",
    why: "\"What is still waiting?\" and \"what went wrong?\" are the two questions this tab answers.",
    effect: "Filters the table. Active includes blocked jobs — the ones waiting for VRAM.",
    benefit: "One click to the failures, with their error text in the Dashboard's Recent activity.",
  },
  {
    key: "cancel",
    label: "Cancel",
    what: "Stops a job that is queued, scheduled, blocked, preparing or running.",
    why: "A blocked job can wait forever if nothing frees VRAM; a wrong render is not worth the minutes.",
    effect: "The job is signalled to stop and ends as cancelled; a partially generated answer is kept as it was. A job that has already finished cannot be cancelled — delete it from its own tab instead (deleting also removes the output file).",
    benefit: "Free the queue without restarting anything.",
  },
];

export const JOBS_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "jobs",
    title: "What the Jobs tab is for",
    summary: "Every job across every capability — chat turns, renders, upscales, narrations, benchmarks, upgrade checks — in one table, with type and state filters and Cancel.",
    body: [
      {
        kind: "p",
        text: "The table lists the newest 300 jobs with their type, model, session, state and creation time. The Session column resolves chat, image and video sessions to their names. It is read-only apart from Cancel: outputs are viewed and deleted on the tab the job belongs to.",
      },
    ],
    settings: SETTINGS,
  },
  {
    id: "states",
    area: "jobs",
    title: "What the states mean",
    summary: "queued → (scheduled | blocked) → preparing → running → post → completed | failed | cancelled.",
    body: [
      {
        kind: "list",
        items: [
          "queued — accepted, waiting for its turn. Queued jobs always go before blocked ones.",
          "scheduled — picked by the scheduler, about to start.",
          "blocked — the scheduler could not fit the job's VRAM need into what is free right now (other applications count). It is retried whenever a job finishes or a model unloads; a blocked job is retried only when nothing else can run, so one stuck job never starves the rest. The reason is on the job.",
          "preparing — the runtime is starting or the model is loading.",
          "running — generating. Progress streams to the owning tab.",
          "post — finishing: scoring a benchmark, importing a trained LoRA, writing the output.",
          "completed / failed / cancelled — the end states. Failed jobs carry error text; the Image and Video tabs offer a retry that resubmits the same parameters as a new job.",
        ],
      },
    ],
  },
  {
    id: "disk",
    area: "jobs",
    title: "What a job leaves behind",
    summary: "A row in the database, its events, and an output file for media.",
    body: [
      {
        kind: "list",
        items: [
          "Every job is a row (type, model, session, params, state, result, error) plus its event log; the Chat tab's history, the galleries and the Voice history are all read from these rows.",
          "Image, video, upscale and tts jobs write one file to the generated-media folder; deleting the job from its tab deletes the file.",
          "Bench and upgrade_check jobs store their report as the job result and have no file.",
          "Retention (Settings → Storage & data) deletes old media files but keeps the rows; a row whose file is gone shows without a preview.",
        ],
      },
    ],
  },
];
