# Roadmap

Prinzip: dünne vertikale Scheiben, jede für sich nutzbar. Nach jedem Meilenstein
ein kurzer DONE-Bericht (Implemented / Not implemented / Known issues / Next
decision).

Gewählter Startpunkt: **Plattform-Kern zuerst** (Discovery-Antwort). Damit ist der
MVP bewusst *ohne* echte AI-Capability — er beweist die Grundmechanik.

---

## Phase 1 — Architektur & Fundament ✅ *(kein Feature-Code)*

**Abgeschlossen.** Detailplan + Ergebnisse: [PHASE_1_PLAN.md](PHASE_1_PLAN.md).

- [x] Offene Entscheidungen A–G geklärt ([DECISIONS.md](DECISIONS.md))
- [x] WP-0…WP-10 — 85 Rust-Unit + 1 Integration + 5 pytest grün, `check.ps1` grün

**Deliverable erreicht:** App startet, legt DB + Datenordner an, zeigt live
GPU/VRAM/RAM/CPU im Dashboard; Runtime-Adapter, Job-Zustandsmaschine und
Hybrid-Scheduler existieren als getestete Schnittstellen mit Fake-Adapter;
Loopback-API + Daemon-Run-Loop laufen. Keine echte Runtime, keine Inferenz.

---

## Phase 2 — MVP: Plattform-Kern ✅

**Abgeschlossen.** Scheibenplan + Ergebnisse: [PHASE_2_PLAN.md](PHASE_2_PLAN.md).

- [x] 2.1 `ModelRepo` + eigener bounded GGUF-Header-Reader + manueller Import
- [x] 2.2a/b `LlamaCppAdapter` (ein `llama-server` pro Modell) + verifizierter Installer (ADR-014)
- [x] 2.3 Link-Manager `core::link` (Passthrough/Junction/Hardlink/Copy, ADR-007)
- [x] 2.4a/b Chat-Job: `Auto`-Modellwahl, `/v1/chat/completions`-Stream, Cancel (ADR-015)
- [x] 2.5 Chat-UI + aktiver Capability-Button
- [x] 2.6 VRAM-Kompat-Check `core::compat` — Gewichte + KV-Cache + Overhead vor dem Load, `blocked` mit Klartext statt OOM (ADR-016)
- [x] 2.7 Settings-UI (Theme/Store/VRAM-Budget/Offline-live/`[llama]`), `GET`/`PUT /config`, Diagnostics erweitert (ADR-017)
- [x] DONE-Kriterium: `core/tests/model_swap.rs` — Chat A → Chat B → Engine tauscht selbst

**Deliverable erreicht:** frisches Windows → App → llama.cpp einrichten → GGUF
importieren → Chat end-to-end (UI bis `llama-server`) → zweites Modell → Wechsel
ohne manuelles VRAM-Management → Offline-Modus. Umfang bewusst eng: **nicht**
enthalten — Online-Discovery, Download-Manager, Benchmarks, Quality-Scores,
Plugin-System, Bild/Video, Agents.

1. **Dashboard**
   - GPU (VRAM, Auslastung, Temp), RAM, CPU — Live (1 Hz, NVML + sysinfo)
   - Aktueller Job + Queue-Länge
   - Vier Capability-Buttons (nur „Chat" aktiv, Rest „bald")
2. **Runtime-Management (Manage-first, llama.cpp)**
   - llama.cpp/llama-server in fest kuratierter Version herunterladen + einrichten
     (CUDA-Build), Health-Check, Start/Stop, Auto-Restart bei Crash
   - Windows Job Object für sauberes Aufräumen
   - *Attach-Fallback:* vorhandenen llama-server auf Port erkennen
3. **Model Library (nur lokal)**
   - Manuelles „Modell hinzufügen": GGUF-Datei/-Ordner wählen → in kanonischen
     Store übernehmen (verschieben/verlinken), SHA256, Metadaten aus GGUF-Header
   - Junction-Verknüpfung Store ↔ Runtime
   - Liste mit: Name, Quant, Größe, geschätztes VRAM, Kontext, Rolle
   - „Modell testen"-Knopf (kurzer Prompt, misst tok/s, Load-Zeit, VRAM-Peak)
4. **Resource Manager (Hybrid, session-aware) — Grundausbau**
   - VRAM-Budget-Rechnung (konservativ), ein resident Slot
   - Job-Queue mit Zuständen; `blocked`-Zustand mit Klartext-Dialog
   - Auto-load/unload für Nicht-gepinnte Modelle
5. **Job-System**
   - Job „Chat/Completion": Modell wählen (oder Auto) → laden → Prompt → Antwort
   - Persistenz + Job-Events + Cancel
6. **Fehler & Diagnostics**
   - Klartext-Fehler mit Handlungsoptionen (VRAM, Runtime tot, Datei fehlt)
   - Advanced „Diagnostics"-View: Logs, Runtime-Status, letzte Job-Events
7. **Settings**
   - Store-Pfad, Offline-Modus-Schalter, Runtime-Version, Theme

**DONE-Kriterium:** Frisches Windows → App installieren → llama.cpp wird
eingerichtet → GGUF-Modell hinzufügen → Chat-Job läuft → zweites Modell
hinzufügen → Wechsel ohne manuelles VRAM-Management → Netz trennen → alles
funktioniert weiter.

---

## Phase 3 — Bild ✅

**Abgeschlossen.** Scheibenplan + Ergebnisse: [PHASE_3_PLAN.md](PHASE_3_PLAN.md).
Text→Bild über eine gekapselte ComfyUI-Runtime (gepinnter Tag, `uv`-venv, genau
ein Custom Node). SDXL (`Auto`-Default) + Flux.1-dev-GGUF als zweites Template.
Feste Workflow-JSON-Pipelines, Companion-Auflösung, getypter Bild-Store den
ComfyUI über `extra_model_paths.yaml` liest (ADR-019), Galerie mit Prompt/Seed/
Modell, Scheduler koordiniert LLM ↔ Diffusion ums VRAM-Budget
(`core/tests/llm_diffusion_swap.rs`).

**Offen:** cu130-torch + der Flux-GGUF-Graph gegen die **echte** ComfyUI
verproben — alle Smokes fuhren gegen `aiwm-fake-comfy` (siehe [TODO.md](TODO.md)).

- [x] 3.1 `ComfyUiAdapter` — ein langlebiger, lazy gestarteter Server; Spawn/
  Health/Attach/`/free`, `aiwm-fake-comfy`-Fixture (ADR-018)
- [x] 3.2 Installer — `uv` + Quelle + venv + torch(cu130) + deps + der eine
  Custom Node `city96/ComfyUI-GGUF` (`CmdRunner`-getestet, echter Smoke 70 s,
  `POST /runtimes/comfyui/install`, geteiltes `runtime::download`)
- [x] 3.3 Getypter Bild-Store `<store>/image/{checkpoints,diffusion_models,vae,
  loras,text_encoders}/`; `ModelKind` + `import_model` nimmt `.safetensors`
  (+ Pickle-Ablehnung), routet per Typ-Hint/Endung; ComfyUI-Zugriff über
  `extra_model_paths.yaml` statt Junction (ADR-019); UI-Typ-Dropdown
- [x] 3.4 `capability::image` — `job_type=image`, feste SDXL-txt2img-Pipeline
  (`core::pipeline`), `/prompt` → `/history` → `/view` → `<outputs>/<job_id>.png`,
  Cancel via `/interrupt`, `Auto` über Rolle `base_diffusion`
- [x] 3.5 UI-Tab „Image" — Prompt/Negativ, Größe/Steps/CFG/Seed, Model [Auto],
  „Generate", Ergebnisbild, Galerie; Bild via `GET /jobs/{id}/output`
- [x] 3.6 Flux.1-dev-GGUF als zweites Template, Companion-Auflösung,
  `docs/IMAGE_MODELS.md`, `core::model::catalog` + „Known models"-Panel
- [x] 3.7 Politur — `[comfyui]`-Settings (VRAM-Modus), Diagnostics-Statuszeile,
  Output-Retention-Hinweis; `core/tests/llm_diffusion_swap.rs`

Nach dem txt2img-MVP: Image→Image, Inpaint, Upscale, Enhancement (eigene Slices).

## Phase 4 — Video *(4.1–4.5 ✅, 4.0 offen)*

**Scheibenplan + Research: [PHASE_4_PLAN.md](PHASE_4_PLAN.md).** MVP = Text→Video
und Bild→Video über dieselbe gekapselte ComfyUI-Runtime. Wan 2.2 TI2V-5B zuerst
(`Auto`-Default, ein Modell für T2V + I2V, native ComfyUI-Nodes, mp4 via
`SaveVideo`), **LTX-Video 0.9.5 2B** als zweites Template (ADR-020, nicht LTX-2).
Frame-Interpolation + Video-Upscale danach als eigene Slices. **4.1–4.5 stehen —
alles gegen `aiwm-fake-comfy` gebaut; 4.0 (echte ComfyUI verproben) ist offen und
läuft vermutlich an der echten Maschine.**

- [ ] 4.0 **die echte ComfyUI verproben** (Phase-3-Rest: cu130, Flux-GGUF-Graph,
  `SaveVideo`) — Voraussetzung
- [x] 4.1 `capability::video` — `job_type=video`, feste `wan_ti2v`-Pipeline,
  `generate_image` → `generate_media`, `ModelKind::VideoModel` (gegen Fake-ComfyUI)
- [x] 4.2 Bild→Video (`init_image` aus einem Bild-Job / Pfad → `input/`-Staging,
  `WanImageToVideo.start_image`; gegen Fake-ComfyUI)
- [x] 4.3 UI-Tab „Video" + Erwartungssteuerung (Dauer!) + `<video>`-Player + Galerie
  + „Start from image" (Galerie-Pick / Pfad) + `media-src` CSP
- [x] 4.4 zweites Template **LTX-Video 0.9.5 2B** (Core-Nodes, ADR-020) +
  `docs/VIDEO_MODELS.md` + Wan/LTX-Katalog-Einträge (gegen Fake-ComfyUI)
- [x] 4.5 Politur: RAM-Warnung (`capability::video`), `[comfyui]` `--reserve-vram`
  + `extra_args`, `about.outputs_bytes` + „reveal", `core/tests/video_swap.rs`

## Phase 5 — Agents

**Scheibenplan + Research: [PHASE_5_PLAN.md](PHASE_5_PLAN.md).** Nicht selbst
bauen — zwei fertige Open-Source-Agents orchestrieren + begrenzen, Endpoint =
lokaler `llama-server`. **OpenCode zuerst** (`opencode serve`, Config per Env
erzwingbar, kein `bash -l` — Empfehlung, dreht die Reihenfolge unten um),
**Hermes Agent** (Nous Research, Memory/Skills/Sub-Agents) zweiter.

- [x] 5.0 Windows-Voraussetzungen verprobt (ADR-021) — `opencode serve` + forced
  config + Approval-API OK; Hermes 0.19 nativ Windows (Git-Bash, kein WSL-Zwang,
  aber ~120 Deps + node/Browser/ripgrep/ffmpeg); `llama-server --jinja` emittiert
  OpenAI-`tool_calls` (Qwen2.5-Coder/Hermes/Llama3.x) — echter Modell-Lauf = 5.1
- [x] 5.1a Agent-Fundament — `core::agent` (`AgentAdapter`-Trait, `AgentEvent`,
  `FakeAgentAdapter`), Migration `0005` + `db::AgentRepo`, `LlamaServerOptions.jinja`
- [x] 5.1b **OpenCode-Adapter** — supervised `opencode serve` pro Session,
  erzwungene Config (`OPENCODE_CONFIG_CONTENT`), `GET /event` SSE → `AgentEvent`;
  gegen `aiwm-fake-opencode` getestet (open→send→approve→idle)
- [x] 5.1ca **Agent-Subsystem (Core)** — `capability::agent::AgentSessions` +
  `CodingRuntime` (Scheduler-Plan + Pin, `LlamaCppAdapter::base_url()`), Event-Drain
  → `agent_session_events`/`state`; Integrationstest fake-llama + fake-opencode
- [x] 5.1cb **Agent-Vertikale (API-Anbindung)** — `App.agents` + `OpenCodeAdapter::discover`,
  API (`/agents`, `/agent-sessions/*`) + 8 Tauri-Commands + `ipc.ts`; `docs/AGENT_MODELS.md`
  (kuratierte GGUFs + manuelle Smoke). UI-Tab = 5.3
- [x] 5.2 **Sandkasten** (config-level, ADR-010) — erzwungene OpenCode-`permission`
  (`edit`/`write` auf den Workspace, `external_directory` read-only, `bash` ask,
  Netz-Tools aus) + `SpawnSpec.env_remove`/`SCRUBBED_ENV` gegen Cloud-Credentials.
  Echte Prozess-Isolation vertagt. `docs/SECURITY.md` aktualisiert
- [x] 5.3 **Agents-UI-Tab** (`ui/src/features/agents/`) — Profil-Liste + „New
  profile", Session-View: Transkript mit Tool-Call-Karten + inline Approval-Prompts
  (Allow once / Always / Deny) + „Stop"; „Coding"-Dashboard-Button → Tab.
  Workspace-Registry im Core vertagt (das Profil ist die Bindung)
- [x] 5.4a **Hermes-Adapter** (`agent::hermes`) — `hermes gateway` pro Session,
  managed `HERMES_HOME` + erzwungene `config.yaml`, Bearer-Auth, `chat/stream`-SSE
  → `AgentEvent` (lenient), Approval via `/v1/runs/:id/approval`; gegen
  `aiwm-fake-hermes` getestet. `CLOUD_CREDENTIAL_ENV`/`scrubbed_env` nach `agent/mod.rs` gehoben
- [x] 5.4b **Hermes-Installer + Anbindung** — `uv`-Installer (`agent::hermes::install`
  + geteiltes `runtime::download::ensure_uv`), `HermesAgentAdapter::install`,
  `POST /runtimes/hermes/install` + `GET /agent-runtimes`, `App`-Registrierung,
  „Install Hermes"-Fluss im Profil-Formular. Realer `hermes`-Lauf (manuell)
  kalibriert SSE-Namen + Sandbox-Keys
- [x] 5.5a **Session-Recovery + Pin/Unpin-Lebenszyklus** — `AgentRepo::recover_orphaned()`
  failt beim Start jede nicht-terminale Session (Runtime-Prozess tot); der Drain-Task
  hält ein `Weak` auf die Live-Map und entpinnt/`Failed`-t selbst, wenn ein Runtime
  ohne `stop()` stirbt
- [x] 5.5b **Export/Import-Backup** (`core::backup`) — `.zip` aus `aiwm.db`-Snapshot
  (`VACUUM INTO`) + `config.toml` + Modell-Manifest (SHA-256, **nicht** die Dateien);
  Import staged nach `<data>/.pending-import/`, `App::load` tauscht beim Neustart
  (alt → `*.pre-import`). `GET /export` / `POST /import` + Tauri + Settings-Karte
- 5.5c (vertagt) Kompaktierungs-Anzeige, „Agent pausieren?"-Fluss, Diagnostics-Zeile pro Agent
- Post-MVP: `aider`, lokaler Repository-Index, MCP-Verwaltung, parallele Sessions

## Phase 6 — Automatisierung & Model-Manager v2

Scheibenplan + Research: [PHASE_6_PLAN.md](PHASE_6_PLAN.md). Aus dem manuellen
Modell-Umgang (Datei selbst laden, Pfad eingeben) wird ein Model-Manager: online
suchen, verifiziert laden, lokal messen, Upgrade-Check. Alles offline-first
(ADR-009), kein Anspruch auf einen objektiven Qualitäts-Score (R12).

- [x] 6.0 **Registry-/Benchmark-Spike** (`ADR-022` + `ADR-024`) — HF-Hub-API
  live geprüft: `expand[]` geht auch auf `/api/models`, `lfs.oid` = die
  Verify-SHA-256, `filter=base_model:<id>` findet Abkömmlinge, anon 500 Calls/
  5 min reichen, Gated durchsuchbar. Ollama = kein Such-API. **HF Open LLM
  Leaderboard abgeschaltet** → kein gebündeltes externes Leaderboard, nur lokale
  Mikro-Benchmarks. Schließt R9 / R10
- [x] 6.1 **`core::registry`** — `ModelSource`-Trait + `HuggingFaceSource`
  (nativer `reqwest`-Client, kein Sidecar — ADR-022), `Registry`-Wrapper mit
  wegwerfbarem TTL-JSON-Cache + `Freshness` (Live/Stale/Offline), SHA-256 aus
  `lfs.oid`. `aiwm-fake-hfhub` + `#[ignore]`-Live-Test
- [x] 6.2 **Discovery-UI** — `App.registry` + `GET /registry/{search,models/{id}}`
  + Tauri + ein „Discover"-Panel im Models-Tab (Suche, „GGUF only", Sort,
  Ergebnis-Karten, „Files" mit 🟢/🟡/🔴-Fit-Punkt via `core::compat` + „Copy
  link"). `Freshness`-Banner bei Stale/Offline. Kein Download-Manager (6.4)
- [x] 6.3 **Kompatibilitäts-Engine v2** (verlängert ADR-016) — 6.3a
  `compat::verdict → FitVerdict {Green|Yellow{reason}|Red{reason}|Unknown}`
  (VRAM-Budget + freier RAM für Offload) in der „Discover"-Dateiliste;
  6.3b bounded `.safetensors`-Header-Reader (Param-Count/Precision/`__metadata__`)
  im Import, familien-bewusste `media_headroom_mb`. Konstanten in `HARDWARE.md`
  dokumentiert; echte Messkalibrierung wartet auf 4.0
- 6.4 **Download-Manager** — `downloads`-Tabelle, Queue, Range/Resume, Verify →
  `import_model`, Speicherplanung (ADR-023)
- 6.5 **`core::bench`** — lokale Mikro-Benchmarks (tok/s, Ladezeit, VRAM/RAM-
  Peak), optionaler externer Score, gewichtete Heuristik
- 6.6 **Benchmark-gestützte `Auto`-Auswahl** — `pick_for_role` nutzt Fit +
  Score + Nutzung
- 6.7 **Upgrade-Check** — „Gibt es was Besseres?" pro Modell / Rolle: HF-Hub
  fragen → Fit-Filter → lokales LLM rankt die echten Treffer (erfindet nichts)
  → Liste + „Download & import". Per-Aktion-Consent, im `offline_mode` gesperrt
  (ADR-025)
- 6.8 **Aufräum-Reports** — Dedup / Unused / Old versions, Storage-Ansicht
- 6.9 **Collections + Politur** — Sammlungen, Rate-Limit-Backoff, optionales
  `HF_TOKEN`, Registry-Diagnostics
- Cloud-Provider-Adapter (Claude/OpenAI als opt-in **Agent**-Backends) gehört
  zu Phase 5, eigener ADR, nach Phase 6

## Später / bewusst offen

- Plugin-/Adapter-Plattform für Dritt-Runtimes (erst wenn interne Adapter stabil)
- LAN-/Remote-Zugriff (opt-in)
- Multi-GPU-Scheduling
- Relighting, Generative Fill, Video-Restoration
- Weitere Agent-Runtimes (aider, Continue, …)

---

## Was der MVP bewusst NICHT tut

| Nicht im MVP | Grund | Kommt in |
|---|---|---|
| Online-Modell-Suche/-Download | eigene große Baustelle, Offline-Fallbacks überall nötig | Phase 6 |
| Bild-/Video-Generierung | erst Kern stabil | Phase 3/4 |
| Coding-Agents | braucht Kern + Memory + Sandbox | Phase 5 |
| Benchmarks / Quality-Score | braucht erst Datenbasis | Phase 6 |
| Ollama-/LM-Studio-Adapter | llama.cpp reicht für Kern | Phase 3+ (Ollama optional) |
| Plugin-System | interne Adapter zuerst reifen lassen | nach Phase 6 |
| Multi-User / Remote | Single-User-Produkt | evtl. nie |
