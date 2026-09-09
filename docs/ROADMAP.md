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

## Phase 4 — Video *(geplant)*

**Scheibenplan + Research: [PHASE_4_PLAN.md](PHASE_4_PLAN.md).** MVP = Text→Video
und Bild→Video über dieselbe gekapselte ComfyUI-Runtime. Wan 2.2 TI2V-5B zuerst
(`Auto`-Default, ein Modell für T2V + I2V, native ComfyUI-Nodes, mp4 via
`SaveVideo`), LTX-2 als zweites Template. ComfyUI kann Video nativ (`WanImageToVideo`
→ `CreateVideo` → `SaveVideo`, `av` schon im Installer). Frame-Interpolation +
Video-Upscale danach als eigene Slices.

- [ ] 4.0 **die echte ComfyUI verproben** (Phase-3-Rest: cu130, Flux-GGUF-Graph,
  `SaveVideo`) — Voraussetzung
- [x] 4.1 `capability::video` — `job_type=video`, feste `wan_ti2v`-Pipeline,
  `generate_image` → `generate_media`, `ModelKind::VideoModel` (gegen Fake-ComfyUI)
- [x] 4.2 Bild→Video (`init_image` aus einem Bild-Job / Pfad → `input/`-Staging,
  `WanImageToVideo.start_image`; gegen Fake-ComfyUI)
- [ ] 4.3 UI-Tab „Video" + Erwartungssteuerung (Dauer!) + `<video>`-Player + Galerie
- [ ] 4.4 LTX-2-GGUF als zweites Template + `docs/VIDEO_MODELS.md` + Katalog
- [ ] 4.5 Politur: RAM-Warnung, Retention, Settings, Scheduler-Verprobung

## Phase 5 — Agents

- Agent-Profile (Runtime + Modell + Kontext + Tools + Workspace + Pfad-Allowlist)
- **Adapter 1: Hermes Agent** (Nous Research) — Custom endpoint = lokaler
  llama-server; nutzt sein eingebautes Memory/Skills/Sub-Agent-System, unser Tool
  begrenzt Profil/Pfade/Commands. Windows: `bash -l`-Abhängigkeit vorab
  verifizieren (Git-Bash mitliefern oder WSL2 dokumentieren).
- **Adapter 2: OpenCode** (`serve`-Modus), erzwungene lokale Endpoint-Config
- Session-Persistenz + Checkpoints (Hermes: `~/.hermes/` in Backup einbeziehen)
- Context-Kompaktierung + lokaler Repository-Index (Retrieval) — für OpenCode;
  bei Hermes durch dessen Memory teils abgedeckt
- Sicherheitsgrenzen: Command-Approval, kein Netz per Default, Secrets-Isolation
- Backup/Restore (Export/Import) von Config + DB + Agent-Memory
- aider als optionaler dritter Adapter notiert

## Phase 6 — Automatisierung & Model-Manager v2

- **Online-Discovery:** HF-Hub- + Ollama-Library-Quellen-Adapter, Suche mit Filtern
- **Download-Manager:** Queue, Pause/Resume, Verify, Speicherplanung
- **Kompatibilitäts-Engine:** 🟢/🟡/🔴 vor Download
- **Dedup-/Unused-/Alte-Versionen-Reports**
- **Benchmark-System** + gewichteter Quality-Score
- **Auto-Model-Auswahl** nutzt jetzt Benchmark-Daten
- **Auto-Pipeline-Auswahl** verfeinert
- **Collections**, Model-Versionen, Update-Checks
- Cloud-Provider-Adapter (opt-in): Claude/OpenAI als optionale Agent-Backends

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
