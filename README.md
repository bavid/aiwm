# AI Workstation Manager

Eine lokale Control-Plane für die eigene AI-Workstation. Sie orchestriert lokale
AI-Runtimes (LLM, Bild, Video), verwaltet Modelle zentral und stellt eine
einfache, capability-orientierte Oberfläche bereit — **ohne** die Runtimes selbst
neu zu implementieren.

> **Local-first, Cloud-optional.** Nach der Erstinstallation aller Komponenten
> läuft das System vollständig offline. Cloud-Provider sind eine bewusst zu
> aktivierende Erweiterung, niemals eine Voraussetzung.

**Lizenz:** Privates Projekt, keine kommerzielle Nutzung
([DECISIONS.md](docs/DECISIONS.md) ADR-011).

## Status

**Phase 1 (Fundament) abgeschlossen.** Der Kern steht und ist getestet; es gibt
noch keine echte AI-Capability (die kommt ab Phase 2).

| WP | Inhalt |
|---|---|
| WP-0 | Cargo-Workspace (`core` + `src-tauri`), React/Vite-UI, Python-Sidecar, `check.ps1` |
| WP-1 | Core-Bootstrap: Config, Logging, Datenordner, headless `aiwm-cored` mit sauberem Shutdown |
| WP-2 | SQLite (Schema v1, Migrationen, `SettingsRepo` + `JobRepo`) |
| WP-3 | Telemetrie: NVML + sysinfo, 1-Hz-Sampler, graceful degradation |
| WP-4 | `RuntimeAdapter`-Trait, `RuntimeSupervisor` (Windows Job Object, Auto-Restart), `FakeRuntimeAdapter` |
| WP-5 | Job-Zustandsmaschine, `JobEngine`, `HybridScheduler` (VRAM-Budget, Pin/Evict/Block), Crash-Replay |
| WP-6 | Core-API (axum HTTP/WS auf `127.0.0.1` + identische Tauri-Commands), Daemon-Run-Loop |
| WP-7 | UI-Dashboard: Live GPU/RAM/CPU, Jobs, Diagnostics, theme-aware |
| WP-8 | Sidecar-Client (JSON-RPC über stdio, Handshake/Ping, stirbt mit dem Core) |
| WP-9 | CI-Workflow, Git-Hooks (pre-commit fmt, pre-push voller Gate) |
| WP-10 | ADRs finalisiert, Stub-Docs (MODELS/RUNTIMES/SECURITY/BENCHMARKS) |

**272 Rust-Unit + 40 Integrationstests + 5 pytest** grün · `scripts/check.ps1` grün ·
**null `unsafe`** im Produktivcode.

**Phase 2 (MVP) ✅ abgeschlossen** — Scheiben 2.1–2.7 + durchgehender
Modell-Wechsel-Test (`core/tests/model_swap.rs`).
**Phase 3 (Bild / ComfyUI) ✅ abgeschlossen** — Scheiben 3.1–3.7 + LLM↔Diffusion-
Wechsel-Test (`core/tests/llm_diffusion_swap.rs`). Plan + Research:
[docs/PHASE_3_PLAN.md](docs/PHASE_3_PLAN.md). Offen: cu130-Treiber-Check auf der
echten 4080 (alle Smokes fuhren gegen die Fake-ComfyUI).
**Phase 4 (Video / ComfyUI) — Scheiben 4.1–4.5 ✅** (alles gegen `aiwm-fake-comfy`
gebaut). **Offen: 4.0** — die echte ComfyUI auf der 4080 verproben (cu130-Treiber,
GGUF-Ordner-Keys, `SaveVideo`/`av`, Zeit/VRAM kalibrieren) — läuft vermutlich an
der echten Maschine. Plan + Research: [docs/PHASE_4_PLAN.md](docs/PHASE_4_PLAN.md).
**Phase 5 (Agents) — Plan + 5.0 + 5.1a + 5.1b + 5.1c + 5.2 + 5.3 ✅:** [docs/PHASE_5_PLAN.md](docs/PHASE_5_PLAN.md),
**ADR-021**. Nicht selbst bauen — **OpenCode** (`opencode serve`, Adapter 1) +
**Hermes Agent** (Nous Research, Adapter 2) orchestrieren und sandboxen,
Endpoint = lokaler `llama-server --jinja`. Sandkasten = Pfad-Allowlist +
Command-Approval (UI) + erzwungene Offline-Config.

- **5.0** ✅ Voraussetzungen verprobt (ADR-021): `opencode serve` + erzwungene
  Config + der Approval-Zyklus laufen; `--jinja` emittiert OpenAI-`tool_calls`;
  Hermes 0.19 läuft nativ auf Windows (Git-Bash, kein WSL).
- **5.1a** ✅ Fundament: `core::agent` (`AgentAdapter`-Trait, `AgentEvent`,
  `SessionSpec`, `PermissionDecision`, `FakeAgentAdapter`), Migration `0005`
  (`agents` / `agent_sessions` / `agent_session_events`) + `db::AgentRepo`.
  `LlamaServerOptions.jinja` (default an) + `chat_template` — `--jinja` /
  `--chat-template` im Spawn, `[llama]`-Config + Settings-Toggle.
- **5.1b** ✅ **OpenCode-Adapter**: `opencode serve` pro Session unter
  `RuntimeSupervisor`, `cwd` = Workspace, Config erzwungen über
  `OPENCODE_CONFIG_CONTENT` (nur lokaler Endpoint, `webfetch: deny`). `GET
  /event` SSE → `AgentEvent` (Text / Tool / Permission / Idle), `roles`-Cache
  gegen Prompt-Echo. Gegen `aiwm-fake-opencode` getestet: open → send →
  approve → idle, Deny, `interrupt`.
- **5.1ca** ✅ **Agent-Subsystem** (Core): `capability::agent::AgentSessions` —
  eigenes Subsystem, kein Job. `open` löst das Coding-Modell auf (`coding`-Rolle
  \| explizit), ein `CodingRuntime` platziert+pinnt es (`HybridScheduler` +
  `LlamaCppAdapter::base_url()`), dann `adapter.open_session` gegen `<llama>/v1`;
  ein Drain-Task schreibt `AgentEvent`s in `agent_session_events` und führt
  `agent_sessions.state`. `stop` = entpinnen + entladen. MVP: eine Session
  gleichzeitig. Integrationstest fake-llama + fake-opencode.
- **5.1cb** ✅ **Agent-API**: `App.agents` + API `GET/POST /agents`,
  `POST /agent-sessions`, `GET /agent-sessions/:id`,
  `POST /agent-sessions/:id/{message,permission,stop}` + 8 Tauri-Commands +
  `ipc.ts`-Bindings. [`docs/AGENT_MODELS.md`](docs/AGENT_MODELS.md) — kuratierte
  Coding-GGUFs (Qwen2.5-Coder-7B als `Auto`) + manuelle Smoke-Prozedur. Der echte
  GGUF-Lauf ist manuell (wie 4.0). UI-Tab = 5.3.
- **5.2** ✅ **Sandkasten** (config-level, ADR-010): erzwungene OpenCode-`permission`
  — `edit`/`write` = `{"*":"deny","<ws>/**":"ask"}` (Edits nur im Workspace),
  `external_directory` read-only für Profil-Extras, `bash` ask, `webfetch`/
  `websearch` deny + `tools` beide aus (Netz immer aus). `SpawnSpec.env_remove` +
  `SCRUBBED_ENV` strippt Cloud-Credentials (`ANTHROPIC_API_KEY`, `AWS_*`,
  `GITHUB_TOKEN`, …) aus dem `opencode`-Kind. Echte Prozess-Isolation vertagt.
  [`docs/SECURITY.md`](docs/SECURITY.md).
- **5.3** ✅ **Agents-UI-Tab** (`ui/src/features/agents/`): Profil-Liste + „New
  profile" (Runtime, Coding-Modell [Auto/Pick], Workspace-Pfad, Extra-Reads);
  Session-View — Transkript (Text gefaltet, `tool`-Karten mit Command/Output/Status,
  `permission` inline mit Allow once / Always / Deny), Zustands-Badge, Composer
  (nur bei `idle`), „Stop". „Coding"-Dashboard-Button → Tab. UI-only, gegen die
  5.1cb-Bindings + dev-mock.

- **4.1** ✅ **`capability::video`**: `job_type=video` → feste `wan_ti2v`-Pipeline
  (`WanImageToVideo` → `KSampler` → `VAEDecode` → `CreateVideo` → `SaveVideo`,
  mp4) → `<outputs>/<job_id>.mp4`. `generate_image` → **`generate_media`**
  (Bild + Video, Timeout-Param, `.mp4`-Output-Key). `ModelKind::VideoModel`
  (Rolle `base_video`, `<store>/video/diffusion_models/`, zweiter
  `aiwm_video:`-Block in `extra_model_paths.yaml`), Familie `wan`. `capability::
  media` bündelt Seed-/Datei-Helfer für Bild + Video. Auto wählt das
  `base_video`-Modell, umt5-Encoder + Wan-VAE per Rolle/Namen aufgelöst.
  `GET /jobs/{id}/output` liefert `video/mp4`. (Gegen `aiwm-fake-comfy`; echte
  ComfyUI = 4.0.)
- **4.2** ✅ **Bild→Video**: `init_image`-Param — eine Job-ID (Output eines
  fertigen Bild-Jobs) oder ein Bildpfad. `capability::video` löst ihn auf,
  kopiert den Frame nach `<comfyui-data>/input/<job_id>.<ext>` und verdrahtet ihn
  als `LoadImage` → `WanImageToVideo.start_image`; ein `Drop`-Guard entfernt die
  Kopie nach dem Render (auch bei Fehler/Cancel). Fehlender/ungültiger Frame →
  Klartext-Fehler vor dem langen Render. `aiwm-fake-comfy` prüft, dass der Frame
  wirklich in `input/` liegt.
- **4.3** ✅ **Video-UI**: Tab „Video" (spiegelt „Image") — Prompt/Negativ,
  Größe-Presets, Frames/FPS/Steps/CFG/Seed, Model [Auto], „Start from an image"
  (Galerie-Pick oder Pfad + Thumbnail). **Erwartungssteuerung**: Cliplänge +
  grobe Minuten-Schätzung, Warnung > 480p / 81 Frames, „Video ist langsam".
  Fortschritt = die letzte Render-Event-Zeile (kein Prozentbalken). `<video>`-
  Player auf `GET /jobs/{id}/output` (CSP `media-src`), Galerie-Kacheln als
  stummes `<video>` mit „▶". „Video model" als Import-Typ auf dem Models-Tab,
  „Generate Video" auf dem Dashboard aktiv. Neu: `components/NumField`,
  `lib/dev-mock` (dev-only IPC-Bridge für den Browser).
- **4.4** ✅ **Zweites Video-Template**: `pipeline::ltx_video` — **LTX-Video 0.9.5
  (2B)**, nur Core-ComfyUI-Nodes (`CheckpointLoaderSimple` + `CLIPLoader ltxv` +
  `LTXVConditioning` + `LTXVScheduler` → `SamplerCustom` + `CreateVideo` +
  `SaveVideo`; `LTXVImgToVideo` mit Startframe). `VideoRecipe::for_family`
  (`ltx` → LTX, sonst Wan). `capability::video` löst die Begleiter je Rezept auf
  (Wan: umt5 + Wan-VAE · LTX: nur ein t5). **Katalog** bekommt die ersten
  Video-Einträge (Wan-Stack + LTX, echte HF-SHA-256) → `GET /models/known` ·
  [`docs/VIDEO_MODELS.md`](docs/VIDEO_MODELS.md) · **ADR-020** (Abweichung vom
  Plan: nicht LTX-2 GGUF). Gegen `aiwm-fake-comfy`.
- **4.5** ✅ **Politur** (schließt Phase 4): RAM-Vorabwarnung (`Warn`-Event, wenn
  freies RAM unter Modellgröße + 6 GB Offload-Slack liegt — nicht blockierend);
  `[comfyui]` bekommt `reserve_vram_mb` (→ `--reserve-vram <GB>`) + `extra_args`
  (Settings-UI); `about.outputs_bytes` + „Generated media"-Karte mit Größe +
  „reveal"-Knopf; `core/tests/video_swap.rs` (Video nimmt den VRAM-Slot wie Bild
  — Chat ↔ Video-Swap, kein Mensch nötig).

- **3.1** ✅ `ComfyUiAdapter` — ComfyUI als **ein** langlebiger, lazy gestarteter
  Server (`RuntimeAdapter`): Spawn/Health (`/system_stats`) über
  `RuntimeSupervisor`, Attach-Fallback, `unload` = `POST /free` (Server bleibt
  oben), `aiwm-fake-comfy`-Fixture (ADR-018).
- **3.2** ✅ ComfyUI-**Installer**: `uv` bootstrappen (verifiziert) → Quelle am
  gepinnten Tag → `uv venv` (Python 3.13) → torch (cu130) + `requirements.txt` +
  der eine Custom Node `city96/ComfyUI-GGUF` (gepinnter Commit); `uv`-Schritte
  hinter `CmdRunner` (unit-getestet), echter End-to-End-Smoke in 70 s.
  `POST /runtimes/comfyui/install` + „Set up"-Knopf. Geteiltes
  `runtime::download`-Modul.
- **3.3** ✅ **Getypter Bild-Store**: `core::model::ModelKind`
  (Checkpoint/Diffusion/VAE/LoRA/Text-Encoder), `import_model` nimmt
  `.safetensors` und routet per Typ-Hint/Endung nach
  `<store>/image/<typ>/`; `.ckpt`/`.bin`/`.pt` → Klartext-Ablehnung (Pickle).
  ComfyUI liest den Store über sein natives `extra_model_paths.yaml`
  (`LinkStrategy::ExtraPath` statt Junction — die überspannt keine Volumes,
  ADR-019), bei jedem Server-Start neu geschrieben. UI-Import-Formular mit
  Typ-Dropdown.
- **3.4** ✅ **`capability::image`**: `job_type=image` → feste SDXL-txt2img-
  Pipeline (`core::pipeline`, Workflow-JSON + Param-Substitution) → `POST /prompt`
  → `/history` pollen → Bild über `/view` nach `<outputs>/<job_id>.png`,
  `jobs.output_path`; Cancel = `POST /interrupt`. `Auto` wählt das
  `base_diffusion`-Modell, der zufällige Seed wird in die Job-Params
  zurückgeschrieben. Der Scheduler tauscht LLM ↔ Diffusion ums VRAM-Budget (wie
  beim Modell-Wechsel).
- **3.5** ✅ **Image-UI**: Tab „Image" — Prompt/Negativ, Größe-Presets +
  W/H/Steps/CFG/Seed, Model [Auto], „Generate"; Ergebnisbild live gepollt (wie
  Chat); einfache **Galerie** (fertige Bild-Jobs, Klick → Prompt/Seed/Modell/
  Größe). Das Bild kommt über eine neue Loopback-Route `GET /jobs/{id}/output`
  (CSP `img-src` erweitert). Dashboard-Button „Generate Image" aktiv.
- **3.6** ✅ **Flux als zweites Template**: `core::pipeline` trägt jetzt
  `checkpoint_txt2img` **und** `flux_txt2img` (`UnetLoaderGGUF` +
  `DualCLIPLoaderGGUF` + `VAELoader` + `FluxGuidance`, via `ComfyUI-GGUF`);
  `Recipe::for_family` wählt. Companion-Modelle (T5 / CLIP-L / VAE) werden per
  Rolle + Namen aufgelöst, fehlende mit Klartext-Fehler.
  `core::model::catalog` (SDXL + Flux-Stack, echte SHA-256) · `GET /models/known`
  · „Known models"-Panel im Models-Tab · [`docs/IMAGE_MODELS.md`](docs/IMAGE_MODELS.md).
- **3.7** ✅ **Politur**: `[comfyui]`-Config-Tabelle (VRAM-Modus `--*vram`) in der
  Settings-UI · Diagnostics-Statuszeile nennt ComfyUI-Version + Modus
  (`GET /system_stats` bei jedem `health()` gecacht) · „Generated images"-Pfad +
  Retention-Hinweis · `outputs_dir` in `GET /about` ·
  **`core/tests/llm_diffusion_swap.rs`**: Chat → Bild-Job evictet das LLM selbst
  → Chat evictet ComfyUI, alle drei `Completed`.

- **2.1** ✅ `ModelRepo`, eigener bounded GGUF-Header-Reader, manueller Import in
  den kanonischen Store, UI-Tab „Models".
- **2.2a** ✅ `LlamaCppAdapter`: `llama-server` als Kindprozess pro residentem
  Modell (Start/Stop/Health über `RuntimeSupervisor`), Binär-Auflösung
  (Env/Managed/PATH), Attach-Fallback, nicht-streamendes `complete()`.
- **2.2b** ✅ llama.cpp-Installer: SHA-256-verifizierter Download des gepinnten
  CUDA-Builds + Entpacken, `POST /runtimes/llamacpp/install`, „Set up"-Knopf mit
  Fortschritt, `RuntimeRepo`.
- **2.4a/b** ✅ Chat-Job: `job_type=chat` (`Auto`-Modellwahl über Rolle) →
  Scheduler → `llama-server` → `/v1/chat/completions` gestreamt → Antwort
  progressiv in `jobs.result`; **Cancel** (`POST /jobs/{id}/cancel` + Knopf).
- **2.5** ✅ **Chat-UI**: eigener Tab, Prompt/Antwort, Antwort erscheint live aus
  `jobs.result`-Polling, „Stop"-Knopf, Auto-Modell + tok/s aus dem Event-Stream;
  „Chat"-Capability-Button auf dem Dashboard aktiv.
- **2.3** ✅ **Link-Manager** (`core::link`): `LinkStrategy`
  (Passthrough/Junction/Hardlink/Copy), NTFS-Junction via `junction`-Crate,
  `model_links` live, Import verlinkt GGUF → llama.cpp (`passthrough`),
  Models-UI-Spalte „Runtimes".
- **2.6** ✅ **VRAM-Kompatibilitäts-Check** (`core::compat`): Gewichte + KV-Cache
  (aus GGUF-Arch-Dims) + Overhead, geschätzt für den Chat-Kontext; der Scheduler
  plant gegen diese Zahl statt der Dateigröße, `llama-server` bekommt ein
  passendes `-c`. Passt es nicht → `blocked` mit Klartext-Aufschlüsselung
  (kein OOM-Modell-Load), und der Job ruht statt die Schleife heiß zu drehen.
- **2.7** ✅ **Settings-UI**: Theme (System/Hell/Dunkel, sofort), Store-Pfad,
  VRAM-Budget, **Offline-Schalter live** (`Arc<AtomicBool>`, kein Neustart),
  `[llama]`-Optionen; `GET`/`PUT /config`. Diagnostics erweitert: VRAM-Budget,
  GPU-Prozesse, „Copy" für Bug-Reports.
- **Abschluss** ✅ `core/tests/model_swap.rs`: Chat auf Modell A → Chat auf
  Modell B → die Engine evictet A und lädt B selbst („made room on the GPU"),
  beide Jobs `Completed`. Phase-2-DONE-Kriterium durchgehend getestet.

## Zielhardware

| Komponente | Wert |
|---|---|
| GPU | NVIDIA RTX 4080 Super, 16 GB GDDR6X |
| RAM | 32 GB DDR5 |
| CPU | AMD Ryzen 7 7800X3D (8C/16T) |
| Storage | `E:` mit ~1,5 TB frei · Modell-Store: `E:\AI\models` |
| OS | Windows 11 Pro (nativ, kein Docker/WSL-Zwang) |

## Bauen & Ausführen

Setup (einmalig): [docs/DEV_SETUP.md](docs/DEV_SETUP.md).

```powershell
powershell -File scripts/check.ps1        # voller Quality Gate
cargo run -p aiwm-core --bin aiwm-cored   # headless Core + Loopback-API (127.0.0.1:48160)
pnpm -C ui exec tauri dev                 # Desktop-App (aus E:\AI ausführen)
```

## Dokumente

| Datei | Inhalt |
|---|---|
| [docs/ANALYSIS.md](docs/ANALYSIS.md) | Kritische Analyse des Briefs: Lücken, Widersprüche, unrealistische Erwartungen |
| [docs/PRODUCT_VISION.md](docs/PRODUCT_VISION.md) | Produktvision, Leitprinzipien, Nicht-Ziele |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Systemarchitektur, Komponenten, Datenmodell, Abstraktionen |
| [docs/TECHNOLOGY.md](docs/TECHNOLOGY.md) | Technologievergleich mit Offline/Cloud/Account/OSS-Matrix |
| [docs/HARDWARE.md](docs/HARDWARE.md) | Realistische Modell-/Auflösungs-/Geschwindigkeitseinschätzung für die Hardware |
| [docs/ROADMAP.md](docs/ROADMAP.md) | MVP-Definition und Phasenplan |
| [docs/PHASE_1_PLAN.md](docs/PHASE_1_PLAN.md) | Detailplan + Ergebnisse aller Phase-1-Arbeitspakete |
| [docs/PHASE_2_PLAN.md](docs/PHASE_2_PLAN.md) | Phase-2-Scheibenplan + Fortschritt |
| [docs/PHASE_3_PLAN.md](docs/PHASE_3_PLAN.md) | Phase-3-Scheibenplan (Bild / ComfyUI) + Research + Ergebnisse |
| [docs/PHASE_4_PLAN.md](docs/PHASE_4_PLAN.md) | Phase-4-Scheibenplan (Video) + Research |
| [docs/PHASE_5_PLAN.md](docs/PHASE_5_PLAN.md) | Phase-5-Scheibenplan (Agents: OpenCode + Hermes) + Research |
| [docs/VIDEO_MODELS.md](docs/VIDEO_MODELS.md) | Video-Modelle — kuratierte Liste (Wan 2.2, LTX-Video) |
| [docs/AGENT_MODELS.md](docs/AGENT_MODELS.md) | Agent-Coding-GGUFs (`coding`-Rolle, Tool-Calling) + manuelle Smoke-Prozedur |
| [docs/DEV_SETUP.md](docs/DEV_SETUP.md) | Toolchain-Installation, Build-/Test-Befehle, Git-Hooks, CI |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Architecture Decision Records (ADRs) |
| [docs/RISKS.md](docs/RISKS.md) | Risikoregister mit Gegenmaßnahmen |
| [docs/MODELS.md](docs/MODELS.md) | Modell-Verwaltung: Stand + Plan |
| [docs/IMAGE_MODELS.md](docs/IMAGE_MODELS.md) | Kuratierte Bild-Modelle (SDXL, Flux-Stack): Quelle, SHA-256, Lizenz, Settings |
| [docs/RUNTIMES.md](docs/RUNTIMES.md) | Runtime-Abstraktion + Integrationsstand |
| [docs/SECURITY.md](docs/SECURITY.md) | Sicherheitsmodell (Loopback-only, Prozess-Isolation, Agent-Sandbox) |
| [docs/BENCHMARKS.md](docs/BENCHMARKS.md) | Benchmark-Konzept (Phase 6) |
| [docs/TODO.md](docs/TODO.md) | Parkplatz für zurückgestellte Punkte |
