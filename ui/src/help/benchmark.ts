import type { HelpSetting, HelpTopic } from "./types.ts";

/** Benchmark tab: suites, passes, the live run, the result card, comparison
 *  and history. Facts from `features/benchmark/*` (`BenchForm.tsx`,
 *  `Benchmark.tsx`, `ResultCard.tsx`, `Comparison.tsx`, `History.tsx`,
 *  `benchmark-utils.ts`) and the measurements in `docs/TODO.md`
 *  (2026-09-17). */

const SETTINGS: readonly HelpSetting[] = [
  {
    key: "model",
    label: "Model",
    what: "The GGUF model to measure — only models with the \"chat\" role are offered.",
    why: "Benchmarks run on llama.cpp and measure chat/coding throughput; a GGUF diffusion model passes the format check but has nothing to say about tokens per second.",
    effect: "Changing it drops a finished result that belonged to another selection (a run still in flight stays) and switches History to this model's runs.",
    benefit: "Measure exactly the model you are deciding about.",
    pitfalls: "A GGUF without the chat role does not appear — set the role in the Model Library.",
  },
  {
    key: "suite",
    label: "Test set",
    what: "A fixed list of prompts with a fixed token cap per answer (chat-v1, coding-v1, …); the prompts are listed under the picker.",
    why: "Runs are only comparable when they generate the same number of tokens from the same prompts.",
    effect: "Sets the prompts and the cap. The Comparison and History below show only runs on this suite — different suites never share a chart.",
    benefit: "Apples to apples across models and across time.",
  },
  {
    key: "passes",
    label: "Passes per prompt",
    what: "How many times each prompt is generated: 1 to 5 here (the core accepts up to 10); 2 by default.",
    why: "One pass is noisy; the average over passes is the number.",
    effect: "Total passes = prompts × passes; the line under the picker shows it. Time grows with it — the measured chat-v1 run (3 prompts × 2) took 37 s including a cold load.",
    benefit: "Quick with 1, trustworthy with 3.",
  },
  {
    key: "start",
    label: "Start benchmark",
    what: "Queues a bench job for the model and suite; Stop cancels it.",
    why: "The measurement needs the GPU to itself, so it is a job like any other.",
    effect: "The model is loaded if it is not resident (that load time is part of the result; \"already loaded\" when it was). The Current run panel shows the state, pass k of n, the last pass's tok/s and the engine's event trail. When done, the result card appears and the row joins Comparison and History. A run started elsewhere (the Model Library's \"Test\") is adopted here too.",
    benefit: "A number, not a feeling, in under a minute.",
    pitfalls: "Blocked means not enough free VRAM: unload something or stop the run. A model that cannot fit the card is refused with the reason.",
    measured: "2026-09-17, RTX 4080 SUPER, real llama-server: Mistral-Small-3.2-24B IQ3_M (10.7 GB) on chat-v1 — 56.35 tok/s, prefill 2,464 tok/s, cold load 6.09 s, VRAM peak 12,406 MB, stability 0.9995, 6 passes, every pass exactly 256 tokens; coding-v1 (resident) 56.32 tok/s, 384 tokens per pass. Qwen2.5-7B-Instruct F16 (15.2 GB) was refused: ~15.0 GB needed against 14,840 MB usable.",
  },
  {
    key: "result",
    label: "Result card",
    what: "The big number is tokens per second generated; below it prefill (prompt processing) tok/s, load time, VRAM peak, stability and passes, then a per-prompt table.",
    why: "tok/s is what you feel while chatting; prefill is what you feel on a long prompt; stability says whether the passes agreed.",
    effect: "Stability is 0–1 (shown as a percentage): how consistent the per-pass rates were. \"already loaded\" for load time means the run paid no load cost. A \"short\" mark means a prompt stopped before the cap, so that rate is not comparable.",
    benefit: "Everything you need to compare, nothing that pretends to judge quality.",
  },
  {
    key: "comparison",
    label: "Comparison",
    what: "One bar per model: its newest run on the selected suite, fastest first, with the run that just finished highlighted.",
    why: "The question is usually \"which of my models is fastest here?\"",
    effect: "Reads the last 200 runs of the suite; if there are more, the caption says so. Rows without a rate sort last; a removed model is named as such.",
    benefit: "The answer in one glance.",
  },
  {
    key: "history",
    label: "History",
    what: "The selected model's own runs on the selected suite, newest first: when, tok/s, prefill, stability, passes.",
    why: "\"Is this machine still as fast as it was?\" — a driver update or a background process shows up here.",
    effect: "Read-only.",
    benefit: "Catch a slowdown before you blame the model.",
  },
];

export const BENCHMARK_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "benchmark",
    title: "What the Benchmark tab is for",
    summary: "Measure how fast a chat model generates on this machine — tokens per second on fixed test sets — and compare models and runs. Speed only; nothing here judges what the model said.",
    body: [
      {
        kind: "p",
        text: "Every pass generates exactly the suite's token cap, so two runs are comparable. The score column on the Models tab (a heuristic of speed, fit and stability from a suite-less quick test) comes from the same machinery; a quick test has no suite and therefore never appears here — its number lives in the Model Library.",
      },
    ],
  },
  {
    id: "flow",
    area: "benchmark",
    title: "Step by step",
    summary: "Pick a model and a test set, choose passes, start, read the card, compare.",
    body: [
      {
        kind: "steps",
        items: [
          "Pick a chat GGUF model (import one on the Models tab if the list is empty).",
          "Pick a test set; open \"Prompts in …\" to see what it asks.",
          "Set passes per prompt (2 is the default; 3 for a number you will quote).",
          "Start benchmark. Watch pass k of n; Stop if you change your mind.",
          "Read the result card; then Comparison for other models and History for earlier runs of this one.",
        ],
      },
    ],
    settings: SETTINGS,
  },
  {
    id: "disk-gpu",
    area: "benchmark",
    title: "What happens on disk and on the GPU",
    summary: "The model on llama.cpp for the whole run; one row per run in the database; no files.",
    body: [
      {
        kind: "list",
        items: [
          "GPU: the model is loaded (cold) or reused (resident); the card's peak VRAM during the run is recorded. Other jobs wait.",
          "Disk: the result is a row (per-prompt detail included) and the bench job's event trail; no output file.",
          "The measured 24B IQ3_M model peaked at 12,406 MB on a 16 GB card with 705 MB idle (2026-09-17).",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "benchmark",
    title: "Limits and pitfalls",
    summary: "GGUF chat models only, one at a time, speed is not quality.",
    body: [
      {
        kind: "list",
        items: [
          "Only GGUF models with the chat role; Colibri, Hermes and ComfyUI models have no comparable number.",
          "A quick test from the Model Library uses no suite; its rate is not comparable with suite runs (it also leaves prompt caching on, so its prefill figure is low — 79 tok/s versus 2,464 in the measured run).",
          "A prompt that stops early (EOS before the cap) marks the run \"short\"; compare with care.",
          "The comparison reads the newest 200 runs of a suite; older runs are not in the chart.",
          "A model that does not fit the card is refused, not measured.",
        ],
      },
    ],
  },
];
