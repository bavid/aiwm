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

**140 Rust-Unit + 9 Integrationstests + 5 pytest** grün · `scripts/check.ps1` grün ·
**null `unsafe`** im Produktivcode.

**Phase 2 (MVP) läuft.** Plan + Fortschritt: [docs/PHASE_2_PLAN.md](docs/PHASE_2_PLAN.md).

- **2.1** ✅ `ModelRepo`, eigener bounded GGUF-Header-Reader, manueller Import in
  den kanonischen Store, UI-Tab „Models".
- **2.2a** ✅ `LlamaCppAdapter`: `llama-server` als Kindprozess pro residentem
  Modell (Start/Stop/Health über `RuntimeSupervisor`), Binär-Auflösung
  (Env/Managed/PATH), Attach-Fallback, nicht-streamendes `complete()`.
- **2.2b** ✅ llama.cpp-Installer: SHA-256-verifizierter Download des gepinnten
  CUDA-Builds + Entpacken, `POST /runtimes/llamacpp/install`, „Set up"-Knopf mit
  Fortschritt, `RuntimeRepo`.
- **2.4a** ✅ Chat-Job: `job_type=chat` (`Auto`-Modellwahl über Rolle) → Scheduler
  → `llama-server` → `/v1/chat/completions` gestreamt → Antwort progressiv in
  `jobs.result`. Erste echte AI-Capability.
- **2.4b** (nächste) Chat-Cancel; dann **2.5** Chat-UI.

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
| [docs/DEV_SETUP.md](docs/DEV_SETUP.md) | Toolchain-Installation, Build-/Test-Befehle, Git-Hooks, CI |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Architecture Decision Records (ADRs) |
| [docs/RISKS.md](docs/RISKS.md) | Risikoregister mit Gegenmaßnahmen |
| [docs/MODELS.md](docs/MODELS.md) | Modell-Verwaltung: Stand + Plan |
| [docs/RUNTIMES.md](docs/RUNTIMES.md) | Runtime-Abstraktion + Integrationsstand |
| [docs/SECURITY.md](docs/SECURITY.md) | Sicherheitsmodell (Loopback-only, Prozess-Isolation, Agent-Sandbox) |
| [docs/BENCHMARKS.md](docs/BENCHMARKS.md) | Benchmark-Konzept (Phase 6) |
| [docs/TODO.md](docs/TODO.md) | Parkplatz für zurückgestellte Punkte |
