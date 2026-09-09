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

**225 Rust-Unit + 26 Integrationstests + 5 pytest** grün · `scripts/check.ps1` grün ·
**null `unsafe`** im Produktivcode.

**Phase 2 (MVP) ✅ abgeschlossen** — Scheiben 2.1–2.7 + durchgehender
Modell-Wechsel-Test (`core/tests/model_swap.rs`).
**Phase 3 (Bild / ComfyUI) ✅ abgeschlossen** — Scheiben 3.1–3.7 + LLM↔Diffusion-
Wechsel-Test (`core/tests/llm_diffusion_swap.rs`). Plan + Research:
[docs/PHASE_3_PLAN.md](docs/PHASE_3_PLAN.md). Offen: cu130-Treiber-Check auf der
echten 4080 (alle Smokes fuhren gegen die Fake-ComfyUI).

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
| [docs/DEV_SETUP.md](docs/DEV_SETUP.md) | Toolchain-Installation, Build-/Test-Befehle, Git-Hooks, CI |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Architecture Decision Records (ADRs) |
| [docs/RISKS.md](docs/RISKS.md) | Risikoregister mit Gegenmaßnahmen |
| [docs/MODELS.md](docs/MODELS.md) | Modell-Verwaltung: Stand + Plan |
| [docs/IMAGE_MODELS.md](docs/IMAGE_MODELS.md) | Kuratierte Bild-Modelle (SDXL, Flux-Stack): Quelle, SHA-256, Lizenz, Settings |
| [docs/RUNTIMES.md](docs/RUNTIMES.md) | Runtime-Abstraktion + Integrationsstand |
| [docs/SECURITY.md](docs/SECURITY.md) | Sicherheitsmodell (Loopback-only, Prozess-Isolation, Agent-Sandbox) |
| [docs/BENCHMARKS.md](docs/BENCHMARKS.md) | Benchmark-Konzept (Phase 6) |
| [docs/TODO.md](docs/TODO.md) | Parkplatz für zurückgestellte Punkte |
