# Architektur

Ursprünglich als Vorschlag entworfen; der Kern (Tauri-Host, Rust-Core,
Runtime-Adapter, Job-Engine, Hybrid-Scheduler, SQLite, Loopback-API, Sidecar)
ist in **Phase 1** umgesetzt — siehe [PHASE_1_PLAN.md](PHASE_1_PLAN.md) für den
Ist-Stand pro Baustein. Dieses Dokument beschreibt das Gesamtbild inkl. der noch
nicht gebauten Teile (Discovery, Pipelines, Agents).

---

## 1. Überblick

```
┌──────────────────────────────────────────────────────────────┐
│  Tauri-App (Desktop, Windows)                                 │
│                                                              │
│  ┌────────────────────────┐   WebView (React + TS)           │
│  │  UI                    │   - Dashboard / Capabilities      │
│  │                        │   - Model Library / Jobs / Diag   │
│  └───────────┬────────────┘                                   │
│              │ Tauri IPC (typed commands + events)            │
│  ┌───────────▼────────────────────────────────────────────┐   │
│  │  Core (Rust)                                            │   │
│  │  - Orchestrator / Job Engine                            │   │
│  │  - Resource Manager (Hybrid-Scheduler, session-aware)   │   │
│  │  - Runtime Supervisor (Child-Prozess-Lifecycle)         │   │
│  │  - Model Registry + Storage/Link-Manager                │   │
│  │  - Config + SQLite (rusqlite/sqlx)                      │   │
│  │  - GPU/System Telemetry (NVML)                          │   │
│  │  - HTTP/WS-API auf 127.0.0.1 (nur lokal)                │   │
│  └───────┬───────────────────┬───────────────┬────────────┘   │
└──────────┼───────────────────┼───────────────┼───────────────┘
           │ spawn + HTTP      │ spawn + HTTP  │ spawn (stdio/HTTP)
   ┌───────▼──────┐   ┌────────▼───────┐  ┌────▼─────────────┐
   │ LLM Runtime  │   │ ComfyUI        │  │ Python Sidecar   │
   │ llama-server │   │ (Bild/Video)   │  │ (venv, uv)       │
   │ (+ Ollama    │   │  :8188         │  │ - HF Hub API     │
   │   optional)  │   │                │  │ - VRAM-Estimator │
   └──────┬───────┘   └────────┬───────┘  │ - GGUF/safetensor│
          │                    │          │   Metadaten      │
          │                    │          │ - Agent-Runner   │
   ┌──────▼────────────────────▼──────────▼──────────────────┐
   │  Kanonischer Model Store  +  Runtime-spezifische Links   │
   │  E:\AI\models\...                                        │
   └─────────────────────────────────────────────────────────┘
```

### Warum diese Aufteilung

- **Rust-Core** besitzt alles, was Zuverlässigkeit braucht: Prozess-Lifecycle,
  Scheduling, DB, Telemetrie, API. Rust ist hier stark (Fehlerbehandlung,
  kein GC-Ruckeln bei Monitoring, sauberes Killen von Kindprozessen unter Windows
  via Job Objects).
- **Python-Sidecar** nur dort, wo das AI-Ökosystem in Python lebt: `huggingface_hub`,
  Modell-Datei-Inspektion, spätere Agent-Integrationen, ggf. kleine Torch-Utilities.
  Ein einziger langlebiger Sidecar-Prozess mit lokalem JSON-RPC/HTTP, kein
  Prozess-pro-Aufgabe.
- **Runtimes** (llama-server, ComfyUI) laufen als eigene, vom Core überwachte
  Kindprozesse und werden ausschließlich über ihre HTTP-APIs angesprochen.

---

## 2. Kern-Abstraktionen

Die zentrale Kette (aus Brief 10.22):

```
Model  →  Capability  →  Runtime  →  Pipeline  →  Job  →  Hardware
```

### 2.1 Capability

Was der Nutzer tun kann. Feste Enum-artige Menge, erweiterbar:

`chat`, `code-agent`, `text-to-image`, `image-to-image`, `inpaint`, `upscale`,
`face-restore`, `background-remove`, `object-remove`, `text-to-video`,
`image-to-video`, `frame-interpolate`, `video-upscale`.

Eine Capability kennt: erforderliche Modell-Rollen, kompatible Pipelines,
Default-Qualitätsstufen.

### 2.2 Runtime-Adapter (Trait)

```rust
trait RuntimeAdapter {
    fn id(&self) -> RuntimeId;
    async fn install(&self, version: &Version) -> Result<()>;      // Manage-first
    async fn start(&self) -> Result<RuntimeHandle>;
    async fn stop(&self, h: RuntimeHandle) -> Result<()>;
    async fn health(&self) -> HealthStatus;
    async fn load_model(&self, m: &ModelRef, opts: LoadOpts) -> Result<LoadedModel>;
    async fn unload_model(&self, l: &LoadedModel) -> Result<()>;
    fn vram_report(&self) -> VramReport;                            // wo verfügbar
    fn supported_formats(&self) -> &[ModelFormat];
    fn model_link_strategy(&self) -> LinkStrategy;                   // Junction | Copy | Import
}
```

Adapter im MVP: `LlamaCppAdapter`. Später: `ComfyUiAdapter`, `OllamaAdapter`,
`LmStudioAdapter`.

### 2.3 Pipeline

Eine geordnete Folge von Schritten für eine Capability. Nicht fest an ein Modell
gebunden — bindet **Modell-Rollen** (`upscaler`, `face_model`, `base_diffusion`),
die der Resource Manager zur Laufzeit auf konkrete installierte Modelle auflöst.

MVP: Pipelines sind **statisch in Code/TOML definiert**, kein Editor. Beispiel
`enhance-portrait`: detect → face-restore → upscale → sharpen.

### 2.4 Job

Jede AI-Aufgabe ist ein Job mit Zustandsmaschine:

```
queued → scheduled → preparing (model load) → running → post → completed
                                   │                        └→ failed
                                   └→ blocked (waiting for VRAM / user)
```

Persistiert in SQLite. Cancelbar. Retry/Pause später.

### 2.5 Resource Manager (Hybrid-Scheduler)

Kernlogik, siehe [DECISIONS.md](DECISIONS.md) ADR-003.

- **Ein „resident" Slot** pro Modalität-Klasse für das zuletzt/häufig genutzte
  Modell (typisch: das aktive Coding-Modell bleibt geladen).
- **Job-Queue** für alles Weitere. Jobs deklarieren ihr VRAM-Budget (aus
  Estimator + Messung).
- **Session-Awareness:** Ein Modell mit aktiver Agent-Session ist *pinned* und
  wird nie automatisch evakuiert. Konkurriert ein Job um VRAM, gibt es drei
  Auswege in dieser Reihenfolge:
  1. Passt es zusätzlich ins Budget? → parallel ausführen.
  2. Ist der resident Slot *nicht* gepinnt? → unload → load → run → (optional) restore.
  3. Sonst → Job `blocked`, Nutzer entscheidet: „Coding-Agent pausieren?" /
     „In Queue lassen" / „kleineres Modell".
- **VRAM-Budget** wird konservativ gerechnet (Gewichte + KV-Cache + Aktivierungs-
  Headroom + Treiber-Overhead ~1 GB).
- Entscheidungen werden geloggt und der UI als Klartext-Begründung gereicht.

---

## 3. Model Management

### 3.1 Kanonischer Store

```
E:\AI\models\
  llm\<publisher>\<model>\<quant>\model.gguf
  diffusion\<family>\<model>\...
  upscale\...
  aux\ (VAE, text-encoder, controlnet, ...)
  .catalog\           # lokaler Metadaten-Cache (JSON), offline nutzbar
```

Jede Datei: SHA256, Größe, Quelle, Quell-Revision, Format, erkannte Quant,
Architektur (aus GGUF-Header / safetensors-Index), Import-Zeitpunkt.

### 3.2 Runtime-Verknüpfung

Der **Link-Manager** entscheidet pro (Modell, Runtime):

| Runtime | Strategie |
|---|---|
| llama.cpp / LM Studio | NTFS-Junction vom Runtime-Ordner auf kanonische Datei — **kein Kopieren** |
| ComfyUI | Junction in `models/checkpoints` etc. |
| Ollama | `ollama create` aus GGUF → **Kopie** in Ollama-Blob-Store (unvermeidbar); im Dedup-Report sichtbar |

### 3.3 Discovery / Download (NICHT im MVP)

Ab Phase „Model Manager v2":

- **Quellen-Adapter:** `HuggingFaceSource` (via `huggingface_hub` im Sidecar),
  `OllamaLibrarySource`. GitHub/andere später.
- **Download-Manager** im Rust-Core: Queue, Pause/Resume (HTTP Range), Retry,
  SHA256-Verify, Speicherplatz-Check *vor* Start.
- **Kompatibilitäts-Engine:** VRAM-Schätzung aus Parametern + Quant + Kontext +
  KV-Cache-Modell; Ergebnis 🟢/🟡/🔴 mit Begründung. Rein heuristisch, klar so
  benannt.
- **Trust-Anzeige:** Quelle, Autor, Lizenz, Downloads, letzte Aktualisierung.
  Unbekannte Quelle → „⚠ Unverified". Nie automatische Ausführung von
  Nicht-Daten-Artefakten (Custom Nodes, Pickle).
- **Safety/Policy:** Discovery zeigt vom Repository bereitgestellte Infos
  (auch „uncensored"-Tags) neutral an; keine Bewertung, keine Bevorzugung, keine
  erfundenen Capabilities. Keine gezielte Optimierung für Missbrauch.

### 3.4 Offline-Verhalten

- Lokale Library: voll funktionsfähig ohne Netz, unabhängig von Online-Metadaten.
- Online-Suche: sauberer „nicht verfügbar"-Zustand, kein Fehler.
- Bereits gecachte Metadaten bleiben nutzbar.

---

## 4. Datenmodell (SQLite, erste Skizze)

```
runtimes(id, kind, version, install_path, state, last_health)
models(id, publisher, name, family, format, quant, arch, param_count,
       file_path, sha256, size_bytes, source, source_revision,
       ctx_max, vram_estimate_mb, ram_estimate_mb, imported_at, last_used_at, use_count)
model_links(model_id, runtime_id, strategy, link_path)
model_roles(model_id, role)                       -- coding, upscaler, base_diffusion, ...
collections(id, name) / collection_items(collection_id, model_id, role_label)
capabilities(id, ...)                             -- meist statisch, hier nur Overrides
pipelines(id, capability, name, definition_toml, is_default)
jobs(id, type, capability, state, params_json, runtime_id, model_id,
     created_at, started_at, finished_at, error_text, output_path)
job_events(job_id, ts, level, message)
agents(id, name, agent_runtime, model_id, ctx_size, tools_json,
       workspace_path, allowed_paths_json, memory_enabled, autostart)
agent_sessions(id, agent_id, started_at, ended_at, status, checkpoint_path)
benchmarks(model_id, ts, tokens_per_sec, load_ms, vram_peak_mb, ...)  -- Phase 6
settings(key, value)                              -- App-State (schema_version, first_run_at, cloud_optin);
                                                  -- Startkonfig (store_path, offline_mode, vram_budget_mb,
                                                  -- [llama]) lebt in config.toml (ADR-017)
downloads(id, model_ref, url, state, bytes_done, bytes_total, sha256_expected)  -- v2
```

Immutable-Stil: Statusübergänge erzeugen neue `job_events`, kein Überschreiben von
Historie.

---

## 5. Agent-Subsystem (Phase 5, hier nur Rahmen)

- **Agent-Profil** = Runtime + Modell + Kontextgröße + Toolset + Workspace +
  erlaubte Pfade + Memory an/aus.
- **Agent-Runner** im Python-Sidecar startet die gewählte Agent-CLI (z. B.
  OpenCode) mit erzwungener Konfiguration: Endpoint = lokaler llama-server,
  Arbeitsverzeichnis + Pfad-Allowlist gesetzt, Shell-Kommandos brauchen
  Bestätigung.
- **Langlauf-Unterstützung** (Antwort auf Brief Abschnitt 4):
  - Context-Kompaktierung: ältere Turns zusammenfassen, Tool-Outputs kürzen.
  - Repository-Index (lokal, z. B. embeddings via kleines lokales Modell) für
    Retrieval statt Voll-Dump.
  - Sub-Agent-Muster: Teilaufgabe bekommt frischen Kontext, liefert Ergebnis zurück.
  - Checkpoints: Session-State + Zusammenfassung periodisch persistiert →
    Wiederaufsetzen nach Absturz/Neustart.
  - „Pinned model" während aktiver Session (Scheduler).
- **Sicherheitsgrenzen (MVP-Niveau):** Pfad-Allowlist, Command-Approval,
  kein Netzzugriff per Default, Secrets nicht in Env des Agents.
  Echte Prozess-/FS-Isolation (WSL2/Container) ist optional und später.

---

## 6. Prozess- & Fehler-Management (Windows-spezifisch)

- Kindprozesse in einem **Windows Job Object** → beim App-Exit/Crash sterben
  alle Runtimes zuverlässig mit, keine verwaisten `python.exe`.
- Health-Checks: HTTP-Ping + Port-Reachability + optional NVML-VRAM-Plausibilität.
- Alle Runtimes binden **`127.0.0.1`**. Firewall-Dialog wird dokumentiert.
- Crash-Recovery: Runtime neu starten, Job auf `failed` mit Klartext, nicht die App.
- Strukturierte Logs pro Runtime + zentrales App-Log; „Diagnostics"-View im
  Advanced-Modus.

---

## 7. API zwischen UI und Core

- Primär **Tauri IPC** (typisierte Commands + Event-Stream) für die Desktop-UI.
- Zusätzlich **HTTP/WS auf `127.0.0.1:<port>`** im Core — nützlich für spätere
  Automatisierung/Skripting und um dieselbe API-Fläche einheitlich zu halten.
  Kein LAN-Listener im MVP.
- WS-Events: Job-Fortschritt, Runtime-Health, GPU/RAM-Telemetrie (1 Hz).

---

## 8. Modul-/Ordnerstruktur (Vorschlag)

```
/core            (Rust crate)
  /orchestrator  (job engine, scheduler)
  /runtime       (adapter trait + impls)
  /model         (registry, store, link-manager)
  /telemetry     (nvml, sysinfo)
  /api           (tauri commands + http/ws)
  /db            (migrations, queries)
/sidecar         (Python, uv-managed venv)
  /hub           (huggingface_hub wrapper)
  /inspect       (gguf/safetensors metadata)
  /agents        (agent runners)
/ui              (React + TS + Vite)
  /features/dashboard  /features/models  /features/jobs  /features/diagnostics
  /components/ui
/docs
```

Zielgrößen: Dateien 200–400 Zeilen, klare Trait-/Modulgrenzen, jede Einheit
isoliert testbar.

---

## 9. Teststrategie (Kurz)

- **Rust-Unit:** Scheduler-Entscheidungen (VRAM-Szenarien als Tabellentests),
  Link-Strategie-Auswahl, Job-Zustandsmaschine, VRAM-Estimator.
- **Adapter-Contract-Tests:** gegen echte, lokal laufende Runtime (Integration),
  hinter Feature-Flag.
- **Sidecar:** pytest für Hub-Wrapper (gemockt) und Metadaten-Parser (echte
  kleine GGUF-Fixtures).
- **UI:** Komponenten-Tests für Capability-Flows; visuelle Regression für
  Dashboard.
- **E2E (ab MVP-Ende):** „Runtime installieren → Modell verknüpfen → Chat-Job
  → Ergebnis" als ein durchgehender Test.
