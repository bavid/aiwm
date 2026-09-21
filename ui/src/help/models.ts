import type { HelpSetting, HelpTopic } from "./types.ts";

/** Models tab: the library, importing, the curated catalogue and stacks,
 *  the captioner stacks, Discover (Hugging Face / Civitai / local-model
 *  ranking), the fit badge, downloads and the offline gate, the storage
 *  report, Colibri. Facts from `features/models/*` and the measurements in
 *  `docs/TODO.md` (2026-09-12, 2026-09-16, 2026-09-17, 2026-09-19). */

const LIBRARY_SETTINGS: readonly HelpSetting[] = [
  {
    key: "vram-estimate",
    label: "VRAM est.",
    what: "The VRAM a model is expected to take when loaded: weights + KV cache (for a chat model, at the configured context) or sampler activations (for a diffusion model) + a runtime overhead.",
    why: "It is the number the scheduler plans with and the fit badge judges by.",
    effect: "Informational here. For a GGUF with full architecture metadata the KV part is computed exactly; otherwise a rough per-1K-context reserve is used.",
    benefit: "See at a glance which models fit a 16 GB card.",
    measured: "2026-09-16: the estimate's fixed overhead was recalibrated from a real load of Mistral-Small-3.2-24B IQ3_M (measured 11,690 MB; the old estimate was 717 MB too high) to 350 MB. For image and video models the sampler headroom is applied only to checkpoint / diffusion / video files, not to companions like a 235 MB CLIP (fixed 2026-09-12).",
  },
  {
    key: "score",
    label: "Score / Test",
    what: "A heuristic score from a quick benchmark (speed, fit and stability — not answer quality), with the generation tok/s beside it; Test runs the quick benchmark, re-test repeats it.",
    why: "Auto model selection can rank by it (Settings → Performance), and it is the fastest way to tell two quants apart.",
    effect: "Test queues a bench job without a test set (3 passes, 128 tokens); the score lands in this column when it finishes. It leaves prompt caching on, so its prefill figure is low and not comparable with a suite run.",
    benefit: "A number per model without leaving the tab.",
    pitfalls: "Only GGUF models can be tested; the full Benchmark tab is where comparable numbers live.",
  },
  {
    key: "better",
    label: "Better?",
    what: "Asks Hugging Face for newer or bigger models like this one, then has your local model rank the results.",
    why: "\"Is there a better version of what I have?\" without browsing.",
    effect: "One online request (it asks first), then an upgrade_check job; the report appears under Maintenance → Upgrade check with a fit badge and a one-click download per candidate. The automatic pick downloads the smallest quant that fits, never an unrated one over a too-big one.",
    benefit: "Upgrades found for you; quality still yours to judge.",
    pitfalls: "Needs network; blocked by Offline mode. Gated repos are shown but not downloaded.",
  },
  {
    key: "roles",
    label: "Roles",
    what: "chat, coding, reasoning, embedding — toggled per GGUF model; other kinds get their roles at import.",
    why: "Tabs pick models by role: Chat lists chat models, Agents coding models, Benchmark chat models.",
    effect: "Saved at once. Image/video roles (base_diffusion, base_video, vae, text_encoder) are assigned from the file kind and are not editable.",
    benefit: "Add \"coding\" to a model downloaded before that role existed, without re-importing.",
  },
];

const IMPORT_SETTINGS: readonly HelpSetting[] = [
  {
    key: "import-type",
    label: "Type",
    what: "What kind of file is being imported: chat GGUF, image checkpoint, diffusion model / UNet, VAE, LoRA, text encoder, CLIP vision, IP-Adapter, video model, voice model, voice data, WD tagger, a Florence-2 or Qwen2.5-VL file.",
    why: "The kind decides where the file is linked for ComfyUI or the sidecar and which roles it gets. Only files from elsewhere need it — Discover's Get and the catalogue download straight into the library with the right kind.",
    effect: "Changes the accepted extensions, the placeholder and the roles offered. A .gguf can never be a checkpoint, VAE or LoRA — the type is corrected from the file if they disagree.",
    benefit: "One form for every kind of model.",
  },
  {
    key: "import-path",
    label: "Path or link",
    what: "A file on this machine, or an https link to a weight file.",
    why: "A path imports right away; a link goes through the download manager, which verifies and imports it when it lands.",
    effect: "Path: the file is moved into the model store (or copied — see below), its metadata read, its roles set, its runtime links created; \"already imported\" if the same file is there. Link: a download is queued and imports itself when done.",
    benefit: "Drag from Downloads, or paste a Hugging Face link.",
    pitfalls: "Importing a pickled (.ckpt/.pt) file is refused by the import guard regardless of what any site's scan says.",
  },
  {
    key: "keep-original",
    label: "Keep the original file",
    what: "Copy the file into the store instead of moving it.",
    why: "Moving is fast and saves disk; copying leaves your download folder as it was.",
    effect: "Copy: twice the disk space until you delete one. Move: the source path is gone. Not offered for links.",
    benefit: "Your choice between disk and tidiness.",
  },
];

const CATALOG_SETTINGS: readonly HelpSetting[] = [
  {
    key: "stack",
    label: "Download entire stack",
    what: "Queues every file a curated model needs — base model, text encoder(s), VAE — in one click; each file is pinned by URL, SHA-256 and size.",
    why: "A FLUX stack is four separate files; a Wan stack needs its umt5 encoder and VAE. Hunting them one at a time is how imports go wrong.",
    effect: "One download per member, one at a time, verified against the pinned hash, imported with the right kind and roles. Every member can also be fetched alone to top up a missing piece. The fit badge on the stack judges the sum of all members, not each alone — a FLUX stack whose members look fine individually needs about 17 GB together (2026-09-12).",
    benefit: "A working image, video or voice setup without knowing what a VAE is.",
    pitfalls: "Refused while Offline mode is on. Gated files need your Hugging Face token and the licence accepted on the site.",
  },
  {
    key: "captioner-stacks",
    label: "Training & captioning",
    what: "The dataset captioners as stacks: WD tagger (tags, CPU), Florence-2 (prose, GPU ~2.5 GiB), Qwen2.5-VL (second opinion, GPU ~9 GiB at 4-bit) — each with purpose, size, licence, fit and an Install button that turns into a progress bar and then \"Installed\".",
    why: "Captions decide what a LoRA learns from your frames; these are the tools that write them.",
    effect: "Install queues the pinned files; the card shows per-file progress; when the core confirms the files it says Installed. If the files are present but the core refuses them (a bad file), the card says so and offers Re-download. A captioner with a known issue is marked and not offered.",
    benefit: "Recommended: WD tagger for anime and illustration, Florence-2 for photos; add Qwen2.5-VL for a second opinion on unsure captions.",
    measured: "2026-09-19: WD tagger installed in 30.1 s (2 files, 1,260,744,467 B); Florence-2 in 36.3–40.2 s (11 files, ~1.56 GB). Resident VRAM: Florence-2 ~2,187 MiB; Florence-2 + Qwen2.5-VL ~10,035 MiB over the desktop baseline; Qwen alone ~7,864 MiB. The core reserves measurement + 15 % rounded up to 512 MiB.",
  },
  {
    key: "fit-badge",
    label: "Fit badge",
    what: "Fits / Tight / Too big / Unknown for a model, file or stack, with its VRAM estimate and — for Tight and Too big — the core's reason behind a ? disclosure.",
    why: "It answers \"will this run on my card?\" before the download, from the same estimate the scheduler uses.",
    effect: "Informational only: a Too big verdict never disables a download (a bigger GPU next month, CPU offloading, curiosity are all legitimate). File lists sort by tier and can hide the too-big ones with a checkbox that always says how many it hid.",
    benefit: "No surprise \"does not fit this card\" after a 10 GB download.",
    pitfalls: "Unknown means the source gave no size — it sorts before Too big for a human, but an automatic pick never prefers it.",
  },
];

const PACKAGES_SETTINGS: readonly HelpSetting[] = [
  {
    key: "packages",
    label: "Packages",
    what: "Your image and video models grouped by the base they belong to: per base family the checkpoint (✓ installed or ✗ missing), its text encoders and VAE, and the LoRAs made for it. Each group reads Ready, Works with your … (made for another base of the same architecture), Missing: … — N GB, or Not runnable here.",
    why: "A LoRA is worth nothing without the base it was trained for, and a base without its encoder or VAE does not render. This view shows at a glance which of your files can actually be used.",
    effect: "Reading only — the families are worked out on read (recorded family, catalogue, file header, file name) and nothing is written until you press a button. It re-reads when the library changes.",
    benefit: "\"4 LoRAs for Pony, no Pony checkpoint\" or \"FLUX.2 [klein] without its text encoder\" is visible before a render fails.",
    pitfalls: "A Pony, Illustrious or NoobAI group without its own checkpoint runs on your SDXL checkpoint — it reads \"Works with your SDXL\", not missing. Wan 14B and LTX-2 groups are listed but cannot run on 16 GB; they offer nothing to download.",
  },
  {
    key: "download-missing",
    label: "Download missing",
    what: "Opens the package for the group — the same dialog as Discover's Get — with each missing part, its size and where it comes from; \"Get the Pony checkpoint\" opens it with Civitai's top checkpoints for that base to choose from.",
    why: "Completing a base is several files from different places; the dialog lists them with pinned hashes (catalogue) or Civitai's (a community checkpoint).",
    effect: "\"Download missing (N GB)\" queues every ticked part through the download queue, grouped under the family in Downloads; each file is verified and imported with its base family recorded. Optional parts (the FLUX.2 edit VAE, a made-for checkpoint) start unticked.",
    benefit: "One click from \"missing\" to a group that renders.",
    pitfalls: "The Civitai checkpoint list needs the network and is empty in Offline mode. The size shown on the card counts catalogue files only — a checkpoint you still have to choose adds its own size in the dialog.",
  },
  {
    key: "what-base",
    label: "What base is this for?",
    what: "For a LoRA whose base could not be worked out (no recorded family, nothing in its header or name): a list of the base families this app knows, and Save.",
    why: "Without a base the LoRA pickers can only guess — an unknown LoRA is offered on every checkpoint, where most will not load.",
    effect: "Saves your choice as the LoRA's base family (source \"set by you\"); the LoRA moves into that group and the pickers offer it only on matching checkpoints. A later detection never overwrites your choice.",
    benefit: "Every LoRA in its place, once.",
    pitfalls: "The LoRA's page on Civitai or Hugging Face says which base it was trained for — a wrong choice hides it from the checkpoints it would work on. Choosing a base marked \"not runnable here\" is allowed; the LoRA then shows as unusable.",
  },
  {
    key: "save-detected",
    label: "Save detected families",
    what: "A preview of every family the grouping worked out but the library has not stored yet — model → base family, and where it came from (catalogue, file header, file name) — and a button that stores the ticked ones.",
    why: "Detection runs on every read and a file-name guess can be wrong; storing the confirmed ones makes the pickers and this view stable, and a weak guess stays visible as such.",
    effect: "Writes each ticked row with its detection source; then reports how many were written, kept (a base you chose yourself is never replaced) and skipped (the detection changed since the preview). Guesses from the file name are flagged and start unticked.",
    benefit: "One press instead of setting families model by model.",
    pitfalls: "Only tick a weak guess after checking it. Nothing is written by just opening the view.",
  },
];

const DISCOVER_SETTINGS: readonly HelpSetting[] = [
  {
    key: "get",
    label: "Get (Civitai)",
    what: "Opens the package for a Civitai pick: the model plus everything it needs to run here — the base checkpoint, its text encoders and VAE — each marked Installed ✓, Download ↓ (with its size), Choose a checkpoint ? or Not runnable ✗.",
    why: "A LoRA is useless without the base it was trained for, and Civitai hosts LoRAs for bases this app cannot run at all. Get checks your library and the catalogue before anything is downloaded.",
    effect: "\"Download LoRA only\" queues just the file. \"Download LoRA + missing (N GB)\" also queues every ticked need: catalogue files (pinned hashes) and, for a base the catalogue lacks, the Civitai checkpoint you picked from the top three by downloads. Everything goes through the download queue and is recorded with its source and base family; \"View downloads\" shows them grouped under the package.",
    benefit: "One click from a search result to something that actually renders — no hunting for the right base, encoder or VAE.",
    pitfalls: "\"Works with your SDXL — made for Pony\" means it loads on what you have but was tuned on Pony; that suggestion starts unticked. A base Civitai names that this app does not know is said so plainly (\"may not load\"); only \"Download … only\" is offered then. Suggested checkpoints are community uploads: their hash is Civitai's claim, re-checked by the download manager.",
  },
  {
    key: "not-runnable",
    label: "\"Will not load\" line (Civitai)",
    what: "A warning on a result card when the model is made for a base this app cannot run — Wan 14B (does not fit 16 GB; the app runs the 5B) or LTX-2.",
    why: "You should know before a multi-gigabyte download, not after.",
    effect: "Shown on the card itself, from the base labels Civitai lists — no extra request per card. When only some versions target such a base it says so; Get then checks the version you would actually download.",
    benefit: "No more downloading LoRAs that can never load here.",
  },
  {
    key: "source",
    label: "Hugging Face / Civitai",
    what: "Which site the search box searches.",
    why: "Hugging Face has the GGUFs and official checkpoints; Civitai has community SDXL/FLUX checkpoints and LoRAs, explicitly typed.",
    effect: "Each source has its own filters: GGUF only and sort for Hugging Face; Checkpoints / LoRAs, sort and an NSFW opt-in for Civitai. Results show downloads, likes, licence, tags and (Civitai) a preview image and commercial-use terms.",
    benefit: "One search box for both.",
  },
  {
    key: "gguf-only",
    label: "GGUF only",
    what: "Restricts Hugging Face results to repos tagged gguf.",
    why: "Chat models for llama.cpp are GGUF; everything else is noise when that is what you want.",
    effect: "Off shows checkpoints and other formats too; the file list of a repo still lists every weight file it offers.",
    benefit: "Fewer irrelevant repos.",
    pitfalls: "A community repo that was never tagged gguf is hidden by this filter although its files are .gguf.",
  },
  {
    key: "sort",
    label: "Sort",
    what: "Most downloaded, most liked, trending, recently updated or newest.",
    why: "Downloads find the standard choice; newest finds this week's release.",
    effect: "Re-runs the search with that ordering (live as you type, from two characters).",
    benefit: "Two orderings answer two different questions.",
  },
  {
    key: "ai-rank",
    label: "Ask my local model to rank & explain",
    what: "Instead of a live list, an explicit search that filters spam and hardware fit, then has one of your chat/coding models pick a shortlist and explain each pick.",
    why: "\"best local coding model\" is a question, not a keyword; a model can read the candidates' cards and say why.",
    effect: "A recommend job runs (queued, blocked, running like any job); the result lists candidates with a fit badge, a \"picked by your local model\" mark and a why. Costs a model load and a completion, so it runs on Submit, not as you type. Hugging Face only.",
    benefit: "Answers instead of a list.",
    pitfalls: "Blocked when VRAM is short; refused by Offline mode.",
  },
  {
    key: "reasoner",
    label: "Which local model reasons",
    what: "Auto, or the chat/coding model that ranks the results.",
    why: "A bigger model reads model cards better; a smaller one answers sooner.",
    effect: "That model is loaded for the ranking job.",
    benefit: "Your call between speed and judgement.",
  },
  {
    key: "kind",
    label: "Kind (ranked search)",
    what: "Chat model, coding model, image model, video model or LoRA — what you are looking for.",
    why: "It steers the ranking and sets the import type and roles the download will get.",
    effect: "chat → chat role; coding → chat + coding roles; image → checkpoint or diffusion model by format; video and LoRA accordingly.",
    benefit: "The download lands with the right roles.",
  },
  {
    key: "nsfw",
    label: "Show NSFW (Civitai)",
    what: "Include models Civitai marks as NSFW in the results.",
    why: "Civitai is an open upload platform where such content is common and explicitly tagged, unlike Hugging Face — so it is off by default and you must tick it yourself.",
    effect: "Re-runs the search; NSFW results carry a badge.",
    benefit: "Your choice, made once per search.",
  },
  {
    key: "hide-too-big",
    label: "Hide files that won't fit",
    what: "Hides the files in a repo's list whose fit is Too big.",
    why: "A big repo lists a dozen quants; the ones that cannot run here only distract.",
    effect: "Off by default. Offered only when there is something to hide, and the list always says how many files it is holding back.",
    benefit: "Only the candidates that matter.",
  },
  {
    key: "scan",
    label: "Scan badge (Civitai)",
    what: "Civitai's own malware scan for a file: a warning naming the verdict (pickle / virus) when it is not \"Success\", a quiet \"scan ok\" when both passed. Hugging Face runs no such scan, so its files show nothing.",
    why: "It is information you should see before downloading — but it is Civitai's self-report.",
    effect: "It never gates the button. AIWM's own import guard refuses pickle-format files unconditionally on the downloaded bytes, whatever the badge said.",
    benefit: "Informed choice, with the real protection done by the app.",
  },
];

const DOWNLOADS_SETTINGS: readonly HelpSetting[] = [
  {
    key: "downloads",
    label: "Downloads",
    what: "The download queue: one transfer at a time, resumable, verified against the pinned SHA-256 where one is known, then imported with the requested kind and roles.",
    why: "Model files are gigabytes; a queue with pause, resume and verification beats a browser download you then have to import by hand.",
    effect: "Pause / Resume / Cancel per row; failed rows can be resumed (retries are counted). A finished row reads Imported. \"Clear finished\" removes done and failed rows from the list — not the models. Files that one Get queued together are listed under the package's name with a tally (until the app restarts; each file keeps its own source and base family either way).",
    benefit: "Queue a whole stack and walk away.",
    pitfalls: "Every download is refused while Offline mode is on (Settings → Network); the message says so. The same file requested twice joins the transfer already running instead of starting a second one.",
    measured: "2026-09-19: the WD tagger's two files (1,260,744,467 B) arrived in 30.1 s, about 40 MB/s; Florence-2's 11 files in 36.3 s, about 43 MB/s, every hash checked.",
  },
];

const MAINTENANCE_SETTINGS: readonly HelpSetting[] = [
  {
    key: "storage",
    label: "Storage",
    what: "How much the model store holds, per kind, and two \"safe to delete\" lists: duplicates (same SHA-256 imported twice, with the wasted bytes) and unused (never used, or not used for 45 days).",
    why: "Models are the biggest files on the machine and the easiest to forget.",
    effect: "Delete removes the file, its runtime links and its benchmark history after a confirmation — not undoable. In a duplicate group the first copy is marked keep.",
    benefit: "Reclaim tens of GB with the facts in front of you.",
  },
  {
    key: "colibri",
    label: "Colibri (large local models)",
    what: "A CPU engine for mixture-of-experts models too large for a single consumer GPU. The app installs the engine; the model itself you fetch with Hugging Face's hf CLI (the exact command is shown) and then register by folder.",
    why: "The single-stream downloader would be far slower than hf for a directory of dozens of shards.",
    effect: "Install Colibri sets up the engine (refused in Offline mode). Register points the library at the folder; the model then appears in Chat's picker and runs on CPU and RAM — a RAM warning shows when free RAM is below the model's estimate (it still runs, streaming more from disk).",
    benefit: "Run a model that would never fit the card.",
    pitfalls: "Auto never picks a Colibri model; select it explicitly. Slower than the GPU.",
  },
];

export const MODELS_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "models",
    title: "What the Models tab is for",
    summary: "The model library — everything imported, with roles, tags, fit, score and disk use — plus the ways to fill it: the curated catalogue, Discover, links and files.",
    body: [
      {
        kind: "p",
        text: "A model is a file in the model store plus a row in the library: its family, quant, parameter count, size, context, VRAM estimate, roles and the runtimes that can serve it. Everything else in the app picks models from this library by role. The tab has six sections: Library, Packages (the library by base family), Add models (catalogue + import form), Discover, Downloads, Maintenance (upgrade checks, storage, Colibri).",
      },
    ],
  },
  {
    id: "flow",
    area: "models",
    title: "Step by step",
    summary: "Catalogue first; Discover for anything else; import what you already have.",
    body: [
      {
        kind: "steps",
        items: [
          "Add models → pick a category tab (Image, Video, Voice, Chat, Code, Training & captioning). Each pick is fit-checked against your VRAM budget and one per group is marked ★ recommended.",
          "Download entire stack (or one file); watch Downloads; the model appears in the Library when verified and imported.",
          "Discover → search Hugging Face or Civitai, open a repo's files, read the fit badges, Download & import the quant you want — or tick \"Ask my local model\" and let it shortlist. On Civitai, Get downloads a LoRA or checkpoint together with the base, encoders and VAE it still lacks.",
          "Have a file already? Add models → Import: type, path (or link), roles, Import.",
          "Library: rename by clicking the name, add tags, toggle roles, Test for a score, Better? for an upgrade check, pin favourites, Delete.",
        ],
      },
    ],
  },
  {
    id: "library",
    area: "models",
    title: "Library",
    summary: "The table: name, family, quant, params, size (decimal GB), context, VRAM estimate (GiB), score, tags, roles, runtimes, actions.",
    body: [
      {
        kind: "p",
        text: "Sizes are decimal gigabytes (1 GB = 10⁹ bytes, as model cards quote them); memory figures are binary GiB, labelled as such, so the two are never confused. Pinned models sort first; the tag filter narrows the table.",
      },
    ],
    settings: LIBRARY_SETTINGS,
  },
  {
    id: "packages",
    area: "models",
    title: "Packages",
    summary: "The library grouped by base family: base, text encoders, VAE and the LoRAs for it — what is ready, what is missing, what cannot run here.",
    body: [
      {
        kind: "p",
        text: "Each base family you have a checkpoint or a LoRA for gets a card. \"Download missing\" completes a group through the download queue; a Pony, Illustrious or NoobAI group without its own checkpoint works with your SDXL and offers its own checkpoint as an option. LoRAs of unknown base ask which base they are for; families detected on read can be stored with \"Save detected families\".",
      },
    ],
    settings: PACKAGES_SETTINGS,
  },
  {
    id: "import",
    area: "models",
    title: "Import a model",
    summary: "A file or a link, a type, roles, move or copy.",
    body: [
      {
        kind: "p",
        text: "Importing links the file into the runtime that serves it (ComfyUI's model folders for image/video kinds, the sidecar's store for captioners and voices) and reads its metadata. A file already in the store is recognised by hash.",
      },
    ],
    settings: IMPORT_SETTINGS,
  },
  {
    id: "catalog",
    area: "models",
    title: "Recommended models and stacks",
    summary: "Curated picks with pinned files, the captioner stacks, and the fit badge.",
    body: [
      {
        kind: "p",
        text: "Image, Video, Voice and Training picks are stacks with pinned URLs, hashes and sizes (a download needs no lookup). Chat and Code picks pin a repo and a preferred quant and resolve the real file list live, so you can choose a smaller or bigger quant — each download carries the roles the pick needs (a Code pick gets \"coding\").",
      },
    ],
    settings: CATALOG_SETTINGS,
  },
  {
    id: "discover",
    area: "models",
    title: "Discover",
    summary: "Search Hugging Face or Civitai; browse live, or let your local model rank and explain.",
    body: [
      {
        kind: "p",
        text: "Plain browsing is live as you type (from two characters) and remembers your last six searches. When the site is unreachable, or Offline mode is on, the last cached result is shown with its age. \"Files\" resolves a repo's weight files with a fit badge each, each one downloadable straight into the library. On Civitai, \"Get\" shows what a model needs to run here and downloads it together with whatever is missing.",
      },
    ],
    settings: DISCOVER_SETTINGS,
  },
  {
    id: "downloads",
    area: "models",
    title: "Downloads and the offline gate",
    summary: "One verified transfer at a time; nothing leaves the machine while Offline mode is on.",
    body: [
      {
        kind: "p",
        text: "Offline mode (Settings → Network & API) blocks every outbound call: downloads, stack and captioner installs, runtime installs, Hugging Face and Civitai searches, the ranked search, upgrade and tool-update checks. Each refusal says \"Offline mode is on\" at the point you pressed. Turn it off there to install; searches then show live results again.",
      },
    ],
    settings: DOWNLOADS_SETTINGS,
  },
  {
    id: "maintenance",
    area: "models",
    title: "Maintenance",
    summary: "Upgrade-check reports, the storage report, Colibri.",
    body: [
      {
        kind: "p",
        text: "Upgrade check lists the results of \"Better?\" runs: candidates with fit, downloads, date and the model's own reasoning, and a one-click download where the file is not gated. Quality is not locally verifiable — the note says so on every report.",
      },
    ],
    settings: MAINTENANCE_SETTINGS,
  },
  {
    id: "disk-gpu",
    area: "models",
    title: "What happens on disk and on the GPU",
    summary: "Files in the store, links for the runtimes, nothing loaded until a job asks.",
    body: [
      {
        kind: "list",
        items: [
          "Import moves (or copies) the file into the model store and links it where its runtime looks for it; delete removes the file and the links.",
          "Downloads stage in a fixed folder next to the data folder until verified, then import.",
          "Nothing here loads a model on the GPU except Test (a quick benchmark) and the ranked search (a chat completion).",
          "The Storage card's bar shows the store's share of its drive; Settings → Storage & data shows every folder.",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "models",
    title: "Limits and pitfalls",
    summary: "Fit is an estimate, scans are self-reports, split files are manual.",
    body: [
      {
        kind: "list",
        items: [
          "The fit badge judges against the current VRAM budget; it never blocks a download.",
          "Split (multi-part) files have no one-click download yet; gated files need a token and an accepted licence on the site.",
          "Roles can be edited only on GGUF models.",
          "A Civitai scan verdict is informational; the import guard is what protects you.",
          "Offline mode refuses every download and install; the cached search results may be minutes or days old.",
        ],
      },
    ],
  },
];
