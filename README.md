# AI Workstation Manager

Eine lokale Control-Plane für die eigene AI-Workstation. Sie orchestriert lokale
AI-Runtimes (LLM, Bild, Video), verwaltet Modelle zentral und stellt eine
einfache, capability-orientierte Oberfläche bereit — **ohne** die Runtimes selbst
neu zu implementieren.

> **Local-first, Cloud-optional.** Nach der Erstinstallation aller Komponenten
> läuft das System vollständig offline. Cloud-Provider sind eine bewusst zu
> aktivierende Erweiterung, niemals eine Voraussetzung.

## Status

**Phase 1 – Fundament (in Arbeit).** Architektur abgestimmt ([docs/](docs/)).
**WP-0…WP-5 abgeschlossen**: Cargo-Workspace (`core` + `src-tauri`),
React/Vite-UI, Python-Sidecar; Core-Bootstrap, headless `aiwm-cored`, SQLite
(Schema v1 + `SettingsRepo` + `JobRepo`), Telemetrie (NVML + sysinfo),
Runtime-Adapter + `RuntimeSupervisor` (Windows Job Object, Auto-Restart) +
`FakeRuntimeAdapter`, Job-Zustandsmaschine + `JobEngine` + `HybridScheduler`
(VRAM-Budget, Pin/Evict/Block, Szenariomatrix-Tests) + Crash-Replay.
76 Tests grün, `scripts/check.ps1` grün, null `unsafe` im Produktivcode.
Als Nächstes WP-6 (Core-API + Daemon-Run-Loop). Setup:
[docs/DEV_SETUP.md](docs/DEV_SETUP.md).

**Lizenz:** Privates Projekt, keine kommerzielle Nutzung (siehe [DECISIONS.md](docs/DECISIONS.md) ADR-011).

## Zielhardware

| Komponente | Wert |
|---|---|
| GPU | NVIDIA RTX 4080 Super, 16 GB GDDR6X |
| RAM | 32 GB DDR5 |
| CPU | AMD Ryzen 7 7800X3D (8C/16T) |
| Storage | `E:` mit ~1,5 TB frei · Modell-Store: `E:\AI\models` |
| OS | Windows 11 Pro (nativ, kein Docker/WSL-Zwang) |

## Dokumente

| Datei | Inhalt |
|---|---|
| [docs/ANALYSIS.md](docs/ANALYSIS.md) | Kritische Analyse des Briefs: Lücken, Widersprüche, unrealistische Erwartungen, offene Entscheidungen |
| [docs/PRODUCT_VISION.md](docs/PRODUCT_VISION.md) | Produktvision, Leitprinzipien, Nicht-Ziele |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Systemarchitektur, Komponenten, Datenmodell, Abstraktionen |
| [docs/TECHNOLOGY.md](docs/TECHNOLOGY.md) | Technologievergleich (Runtimes, Stack, Frontend) mit Offline/Cloud/Account-Matrix |
| [docs/HARDWARE.md](docs/HARDWARE.md) | Realistische Einschätzung: welche Modelle/Auflösungen/Geschwindigkeiten auf dieser Hardware |
| [docs/ROADMAP.md](docs/ROADMAP.md) | MVP-Definition und Phasenplan |
| [docs/PHASE_1_PLAN.md](docs/PHASE_1_PLAN.md) | Detailplan Phase 1: Toolchain, Repo-Struktur, Schema v1, Kern-Traits, Arbeitspakete |
| [docs/DEV_SETUP.md](docs/DEV_SETUP.md) | Toolchain-Installation (Windows) und Build-/Test-Befehle |
| [docs/TODO.md](docs/TODO.md) | Parkplatz für zurückgestellte Punkte |
| [docs/RISKS.md](docs/RISKS.md) | Risikoregister mit Gegenmaßnahmen |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Architecture Decision Records (ADRs) |

## Nächster Schritt

Review dieser Dokumente durch den Projektinhaber. Erst nach Freigabe der
Architektur beginnt die Implementierung (Phase 2: Plattform-Kern).
