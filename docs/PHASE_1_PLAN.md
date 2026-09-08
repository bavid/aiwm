# Phase 1 — Detailplan: Architektur & Fundament

**Ziel:** Ein lauffähiges, getestetes Skelett. Kein Feature, keine echte Runtime,
keine Inferenz. Am Ende startet die App, legt DB + Datenordner an, zeigt live
GPU/VRAM/RAM/CPU, und alle Kern-Abstraktionen (Runtime-Adapter, Job-Zustands­
maschine, Scheduler) existieren als getestete Schnittstellen mit einem *Fake*-
Adapter.

**Nicht in Phase 1:** llama.cpp-Download/-Integration, Modell-Import, echte
Inferenz, ComfyUI, Agents, Online-Discovery, Downloads, Benchmarks. Das ist
Phase 2+.

**Dauer-Schätzung:** ~10 Arbeitspakete, je 0,5–2 Tage. Reihenfolge und
Abhängigkeiten unten.

---

## 1. Toolchain & Zielumgebung

| Werkzeug | Version / Wahl | Begründung |
|---|---|---|
| Rust | stable, gepinnt via `rust-toolchain.toml` | Reproduzierbar |
| Tauri | 2.x | ADR-001 |
| Node | LTS (22.x), Paketmanager **pnpm** | schnell, disk-effizient |
| Vite + React + TypeScript | aktuell | ADR (Frontend-Vorschlag) |
| Python | 3.11, verwaltet mit **uv** | Hermes/HF-Ökosystem-Anforderung, reproduzierbare venv |
| SQLite | gebündelt (via `sqlx`/`libsqlite3-sys` bundled) | kein System-SQLite auf Windows nötig |

Alles offline-installierbar nach einmaligem Bootstrap. Keine Cloud-Dienste im
Build.

---

## 2. Repository-Struktur (Cargo-Workspace + UI + Sidecar)

```
E:\AI\
  Cargo.toml                 # Workspace
  rust-toolchain.toml
  rustfmt.toml  clippy.toml  .editorconfig  .gitignore
  /core                      # Rust lib crate — die gesamte Logik
    /src
      lib.rs
      config.rs              # config.toml laden/validieren
      telemetry/             # NVML + sysinfo
      db/                    # sqlx pool, migrations runner, repositories
      runtime/               # RuntimeAdapter trait, Supervisor, FakeAdapter
      orchestrator/          # Job state machine, JobEngine
      scheduler/             # Scheduler trait, HybridScheduler (Skelett)
      sidecar/               # SidecarClient (spawn + JSON-RPC)
      api/                   # gemeinsame Handler (von Tauri + axum genutzt)
      error.rs               # thiserror-Typen
    /migrations              # 0001_init.sql, ...
    /tests                   # Integrationstests (Fake-Adapter, in-memory DB)
  /src-tauri                 # Tauri-Host: dünn, ruft nur core::api
    tauri.conf.json
    /src/main.rs
  /ui                        # Vite + React + TS
    /src
      /features/dashboard
      /features/diagnostics
      /lib/ipc.ts            # typisierte Bridge zu Tauri-Commands + Events
    package.json
  /sidecar                   # uv-Projekt
    pyproject.toml
    /src/aiwm_sidecar/main.py
  /docs
  /.github/workflows         # CI (falls Repo je gespiegelt wird; sonst lokale Skripte)
  /scripts                   # check.ps1 / check.sh — dieselben Gates wie CI
```

Zielgrößen: Rust-Dateien 150–350 Zeilen, klare Modulgrenzen. Der Tauri-Host
enthält **keine** Logik — nur Command-Registrierung und Fenster-Setup.

---

## 3. Rust-Abhängigkeiten (Kern)

| Zweck | Crate | Anmerkung |
|---|---|---|
| Async-Runtime | `tokio` | überall |
| DB | `sqlx` (sqlite, runtime-tokio, migrate, bundled) | compile-time-geprüfte Queries, eingebaute Migrations |
| GPU-Telemetrie | `nvml-wrapper` | VRAM, Util, Temp, Per-Prozess-VRAM |
| System-Telemetrie | `sysinfo` | RAM, CPU |
| Prozess-Aufsicht | `tokio::process` + `win32job` (oder `windows` crate direkt) | Windows Job Object, Kill-on-Close |
| HTTP-Server (lokale API) | `axum` | teilt Handler mit Tauri |
| HTTP-Client | `reqwest` (rustls, **kein** OpenSSL) | erst ab Phase 2/6 aktiv gebraucht |
| Serialisierung | `serde`, `serde_json` | |
| Logging | `tracing`, `tracing-subscriber`, `tracing-appender` | rotierende Datei + stderr |
| Fehler | `thiserror` (Bibliothek), `anyhow` (Ränder) | |
| Config | `serde` + `toml` | eine `config.toml` |
| Zeit / IDs | `time`, `uuid` (v7, zeitsortierbar) | |

Auswahlprinzip: rein lokal, keine Telemetrie, wartungsfreundlich, Windows-tauglich.

---

## 4. SQLite-Schema v1 (`migrations/0001_init.sql`)

Nur was Phase 1 + früher Phase 2 braucht. Erweiterungen später als neue Migration.

```sql
CREATE TABLE settings (
  key         TEXT PRIMARY KEY,
  value       TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE runtimes (
  id            TEXT PRIMARY KEY,        -- 'llamacpp', 'comfyui', ...
  kind          TEXT NOT NULL,
  version       TEXT,
  install_path  TEXT,
  state         TEXT NOT NULL,           -- not_installed | stopped | starting | running | error
  last_health   TEXT,                    -- ISO ts
  last_error    TEXT
);

CREATE TABLE models (
  id             TEXT PRIMARY KEY,       -- uuid v7
  publisher      TEXT,
  name           TEXT NOT NULL,
  family         TEXT,
  format         TEXT NOT NULL,          -- gguf | safetensors | ...
  quant          TEXT,
  arch           TEXT,
  param_count    INTEGER,
  file_path      TEXT NOT NULL,
  sha256         TEXT,
  size_bytes     INTEGER NOT NULL,
  ctx_max        INTEGER,
  vram_estimate_mb INTEGER,
  ram_estimate_mb  INTEGER,
  source         TEXT,                   -- 'manual' in Phase 1/2
  source_revision TEXT,
  imported_at    TEXT NOT NULL,
  last_used_at   TEXT,
  use_count      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE model_roles (
  model_id  TEXT NOT NULL REFERENCES models(id) ON DELETE CASCADE,
  role      TEXT NOT NULL,               -- coding | chat | upscaler | base_diffusion | ...
  PRIMARY KEY (model_id, role)
);

CREATE TABLE model_links (
  model_id   TEXT NOT NULL REFERENCES models(id) ON DELETE CASCADE,
  runtime_id TEXT NOT NULL REFERENCES runtimes(id),
  strategy   TEXT NOT NULL,              -- junction | copy | import
  link_path  TEXT NOT NULL,
  PRIMARY KEY (model_id, runtime_id)
);

CREATE TABLE jobs (
  id           TEXT PRIMARY KEY,         -- uuid v7
  type         TEXT NOT NULL,            -- 'chat' | 'noop' (Phase 1)
  capability   TEXT,
  state        TEXT NOT NULL,            -- queued | scheduled | preparing | running | post | completed | failed | blocked | cancelled
  params_json  TEXT NOT NULL,
  runtime_id   TEXT REFERENCES runtimes(id),
  model_id     TEXT REFERENCES models(id),
  created_at   TEXT NOT NULL,
  started_at   TEXT,
  finished_at  TEXT,
  error_text   TEXT,
  output_path  TEXT
);

CREATE TABLE job_events (
  job_id  TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  ts      TEXT NOT NULL,
  level   TEXT NOT NULL,                 -- info | warn | error
  message TEXT NOT NULL
);
CREATE INDEX idx_job_events_job ON job_events(job_id, ts);
CREATE INDEX idx_jobs_state ON jobs(state);
```

Historie ist append-only (`job_events`), Statusübergänge überschreiben nur
`jobs.state`. `agents`, `benchmarks`, `downloads`, `collections` kommen als
spätere Migrationen.

**Default-Settings, die Phase 1 schreibt:** `schema_version`, `offline_mode=false`,
`store_path=E:\AI\models`, `core_api_port`, `llamacpp_pinned_version`.

---

## 5. Kern-Schnittstellen (als Code-Gerüst mit Tests, ohne echte Runtime)

### 5.1 `RuntimeAdapter` (trait) + `RuntimeSupervisor`

- Trait wie in [ARCHITECTURE.md](ARCHITECTURE.md) §2.2.
- `RuntimeSupervisor` besitzt Kindprozesse: Start, Stop, Health-Loop, Auto-Restart
  mit Backoff, alles in einem **Windows Job Object** (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`)
  → beim App-Exit/Crash sterben alle Kinder zuverlässig mit.
- `FakeRuntimeAdapter`: kein echter Prozess. Konfigurierbar: Start-Latenz,
  Health-Verhalten, Modell-Load-Zeit, simuliertes VRAM. Basis für alle Tests.

**Tests:** Contract-Tests gegen `FakeRuntimeAdapter`; ein echter Job-Object-Test,
der einen Dummy-Prozess (`ping -t` / `timeout`) startet und prüft, dass er beim
Droppen des Supervisors stirbt.

### 5.2 Job-Zustandsmaschine + `JobEngine`

- Erlaubte Übergänge explizit als Tabelle; ungültige Übergänge sind Fehler.
- `JobEngine`: nimmt Jobs an, persistiert, führt sie über den Scheduler aus,
  schreibt `job_events`, unterstützt Cancel. In Phase 1 ist ein „Job" ein
  No-Op oder ein Fake-Adapter-Load.
- Replay: Beim Start werden `running`/`preparing`-Jobs aus einem Crash als
  `failed` markiert (mit Event), `queued` bleiben in der Queue.

**Tests:** Übergangsmatrix, Persistenz + Replay, Cancel in jedem Zustand.

### 5.3 `Scheduler` (trait) + `HybridScheduler` (Skelett)

- VRAM-Budget-Rechnung: `frei = total − treiber_overhead(~1 GB) − Σ(geladene Modelle) − headroom`.
- Ein „resident" Slot; `pin(model)` / `unpin(model)` für aktive Sessions.
- Entscheidungsfunktion `plan(job) -> Decision`:
  `RunNow | LoadThenRun | EvictThenLoad(victim) | Blocked(reason)`.
- In Phase 1 ohne echte Modelle: gespeist mit synthetischen VRAM-Zahlen +
  Fake-Adapter.

**Tests (Tabellen-getrieben, die Szenariomatrix aus ADR-003):**

| resident | gepinnt? | Job-Bedarf | frei | erwartete Decision |
|---|---|---|---|---|
| — | — | 8 GB | 15 GB | RunNow |
| 14B (10 GB) | nein | 6 GB | 4 GB | EvictThenLoad(14B) |
| 14B (10 GB) | **ja** | 6 GB | 4 GB | Blocked("Agent-Session aktiv …") |
| 7B (6 GB) | nein | 6 GB | 8 GB | LoadThenRun (parallel) |
| … | | | | (vollständige Matrix im Test) |

### 5.4 `SidecarClient` + Sidecar-Contract

- Transport: JSON-RPC 2.0 über stdio (langlebiger Prozess, im Job Object).
- Methoden Phase 1: `handshake{protocol_version}` → `{sidecar_version, capabilities[]}`;
  `ping` → `pong`. `inspect_model_file{path}` ist **definiert, aber Stub**
  (`error: not_implemented`) — echte Implementierung in Phase 2.
- Rust spawnt Sidecar, macht Handshake, bricht bei Versions-Mismatch sauber ab.

**Tests:** Spawn + Handshake + `ping`-Roundtrip; Sidecar stirbt mit dem Core.

---

## 6. Telemetrie-Modul

- `telemetry::Sampler`: pollt NVML + `sysinfo` bei 1 Hz, publiziert via
  `tokio::sync::watch<SystemTelemetry>`.
- `SystemTelemetry`: GPU-Name, VRAM total/used/free, GPU-Util %, Temp, Top-VRAM-
  Prozesse; RAM total/used; CPU-Util gesamt + pro Kern; Zeitstempel.
- **Graceful degradation:** kein NVIDIA-Treiber / CI → `gpu: Unavailable{reason}`,
  App läuft weiter.

**Tests:** NVML hinter einem kleinen Trait → Mock im Test; Smoke-Test auf der
echten Maschine (zeigt reale 4080S-Werte).

---

## 7. Core-API-Fläche (Phase 1)

Dieselben Handler, zwei Transporte: **Tauri IPC** (für die UI) und **axum HTTP/WS
auf `127.0.0.1:<port>`** (für spätere Automatisierung; im MVP kein LAN).

| Command | Rückgabe |
|---|---|
| `get_system_telemetry` | aktueller `SystemTelemetry` |
| `get_settings` / `set_setting{key,value}` | Settings |
| `list_models` | `[]` in Phase 1 |
| `list_jobs{filter}` | Jobs + letzte Events |
| `get_runtime_status` | Runtimes-Tabelle (alle `not_installed` in Phase 1) |
| `get_recent_logs{lines}` | Tail des App-Logs (für Diagnostics) |

**Events (WS / Tauri emit):** `telemetry` (1 Hz), `job_updated`, `runtime_health`.

**Test:** HTTP-Server bindet nachweislich nur Loopback; identische JSON-Antwort
über beide Transporte.

---

## 8. UI-Shell (Phase 1)

- Vite + React + TS, `@tauri-apps/api` für IPC, typisierte `ipc.ts`.
- **Ein Screen — Dashboard:** Live GPU (VRAM-Balken, Util, Temp), RAM, CPU;
  „Keine Jobs"-Zustand; vier deaktivierte Capability-Buttons; Panel
  „Diagnostics" (Log-Tail, Runtime-Status).
- Theme-aware (hell/dunkel folgt dem OS). **Visuelle Designrichtung** (Typo,
  Palette, Layout-Charakter) wird zu Beginn von Phase 2 festgelegt — Phase 1 ist
  bewusst nur Gerüst, kein Design-Investment.

**Test:** `pnpm tauri dev` zeigt live aktualisierte, echte Telemetrie.

---

## 9. CI & Quality Gates

`scripts/check.ps1` (und identisch als GitHub-Action-Workflow, falls das Repo je
gespiegelt wird):

1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `pnpm -C ui lint` + `pnpm -C ui typecheck` + `pnpm -C ui build`
5. `uv run --project sidecar ruff check` + `uv run --project sidecar pytest`

Pre-commit-Hook ruft dasselbe Skript. Coverage-Ziel 80 % gilt ab dem ersten
Logik-Code (Scheduler, Job-Engine, Repos) — Gerüst/Tauri-Host ausgenommen.

---

## 10. Arbeitspakete, Reihenfolge, Definition of Done

| WP | Inhalt | Hängt ab von | DoD |
|---|---|---|---|
| **WP-0 ✅** | Toolchain, Workspace-Skelett, `.gitignore`/fmt/clippy, `git init` + erster Commit | — | `cargo build`, `pnpm install`, `uv sync` laufen auf der Zielmaschine |
| **WP-1** | Core-Bootstrap: tokio-main, `config.toml` laden/validieren, `tracing` → rotierende Datei, Datenordner (`%APPDATA%\AIWorkstationManager\`) anlegen, sauberer Ctrl-C-Shutdown | WP-0 | Core startet, schreibt Log, legt Ordner an, beendet sauber |
| **WP-2** | `sqlx`-Pool, Migration-Runner, Schema v1, Repository-Traits + SQLite-Impls, Default-Settings | WP-1 | Frische DB aus Migration; Repo-Unit-Tests (in-memory) grün |
| **WP-3** | Telemetrie-Modul (NVML + sysinfo), 1-Hz-Sampler, `watch`-Channel, Graceful degradation | WP-1 | Mock-Test grün; reale 4080S-Werte auf der Maschine |
| **WP-8** | Sidecar-Contract (JSON-RPC), `uv`-Projekt, `main.py` (handshake/ping), `SidecarClient` | WP-1 | Spawn + Handshake + `ping`-Roundtrip; Sidecar stirbt mit Core |
| **WP-4** | `RuntimeAdapter`-Trait, `RuntimeSupervisor`, Windows Job Object, `FakeRuntimeAdapter` | WP-1 | Job-Object-Kill-Test grün; Contract-Tests gegen Fake |
| **WP-5** | Job-Zustandsmaschine, `JobEngine`, `Scheduler`-Trait, `HybridScheduler`-Skelett, Szenariomatrix-Tests | WP-2, WP-4 | Übergangs- + Szenariomatrix-Tests grün; Persistenz + Crash-Replay |
| **WP-6** | Core-API-Handler, Tauri-Commands, axum HTTP/WS (Loopback), Event-Streams | WP-2, WP-3, WP-5 | Beide Transporte liefern identisches JSON; Loopback-only nachgewiesen |
| **WP-7** | UI-Shell: Dashboard + Diagnostics, `ipc.ts`, Live-Telemetrie | WP-6 | `pnpm tauri dev` zeigt live echte Telemetrie |
| **WP-9** | `check.ps1` + CI-Workflow + Pre-commit-Hook | WP-0 (dann laufend) | Pipeline auf sauberem Checkout grün |
| **WP-10** | ADR-005/007/009 finalisieren; Stub-Docs `MODELS.md`, `RUNTIMES.md`, `SECURITY.md`, `BENCHMARKS.md`, `TODO.md` anlegen; Docs-Konsistenzcheck | alle | Docs konsistent; `TODO.md` mit zurückgestellten Punkten befüllt |

**Kritischer Pfad:** WP-0 → WP-1 → WP-2 → WP-4 → WP-5 → WP-6 → WP-7.
**Parallelisierbar:** WP-3, WP-8 (nach WP-1); WP-9 durchgehend; WP-10 zum Schluss.

### WP-0 — Ergebnis (abgeschlossen)

Alles grün via `scripts/check.ps1` (läuft unter Windows PowerShell 5.1):
`cargo fmt` · `cargo clippy -D warnings` · `cargo test --workspace` ·
`ui typecheck` · `ui lint` · `ui build` · `sidecar ruff` · `sidecar pytest` (4).
`tauri dev` startet Vite + Fenster (`aiwm-tauri.exe`).

Abweichungen vom Plan (bewusst, dokumentiert):

- **Core-Module flach** (`core/src/telemetry.rs` statt `telemetry/mod.rs`). Wird
  zum Ordner promoted, sobald ein Modul eine zweite Datei bekommt.
- **Node 24 LTS** (nicht 22) — winget liefert aktuell 24.19.0.
- **pnpm via `npm i -g pnpm`** — corepack scheiterte an Schreibrechten in
  `C:\Program Files\nodejs`.
- **`tauri-plugin-opener`** ist als einziges Tauri-Plugin drin (Standard-Konvention);
  Capabilities: `core:default`, `opener:default`.
- **Kein VS-BuildTools-Install** nötig — MSVC-Linker war über die vorhandene
  VS-2022-Installation bereits da.
- **`ui/pnpm-workspace.yaml`** erlaubt gezielt den `esbuild`-Build-Skript;
  `typescript-eslint` auf `8.69.0` gepinnt wegen pnpm-Supply-Chain-Policy.
- **`icon.png`** ist ein generierter Platzhalter (`scripts/gen_icon.py`), echtes
  Branding später ([TODO.md](TODO.md)).

Toolchain-Details: [DEV_SETUP.md](DEV_SETUP.md).

---

## 11. Phase-1-Abschluss (DONE-Bericht-Vorlage)

```
DONE — Phase 1

Implemented:
- Repo-Skelett (Cargo-Workspace + UI + Sidecar), CI grün
- Core: Config, Logging, Datenordner, SQLite-Schema v1 + Repos
- Telemetrie: live GPU/VRAM/RAM/CPU via NVML/sysinfo
- Runtime-Adapter-Trait + Supervisor + Job Object + FakeAdapter
- Job-Zustandsmaschine + JobEngine + HybridScheduler-Skelett (Szenariomatrix getestet)
- Sidecar-Handshake
- Core-API (Tauri + Loopback-HTTP/WS), UI-Dashboard mit Live-Telemetrie

Not implemented (kommt in Phase 2):
- echte llama.cpp-Runtime, Modell-Import, echte Inferenz, „Chat"-Capability

Known issues:
- <hier eintragen>

Next decision:
- Visuelle Designrichtung für die UI (Start Phase 2)
- llama.cpp: gepinnte Version + Windows-CUDA-Build-Bezugsquelle
```

---

## 12. Zur Umsetzung (wenn Phase 1 freigegeben ist)

Deine Vorgaben: **Auto-Accept-Modus**, **Subagent-driven Development**.

- Jedes WP oben ist bewusst als *eigenständige, testbare Aufgabe mit klarem DoD*
  geschnitten → direkt an einen Subagent delegierbar.
- Pro WP: **TDD** (Tests zuerst — Zustandsmaschine, Scheduler, Repos), dann
  Implementierung, dann `code-reviewer` + bei sicherheitsnahen Teilen
  (Prozess-Spawning, Pfade) `security-reviewer`.
- Reihenfolge entlang des kritischen Pfads; WP-3/WP-8/WP-9 können parallel laufen.
- Nach jedem WP: `check.ps1` grün + kurzer Fortschrittsvermerk. Nach Phase 1: der
  DONE-Bericht oben.

### Automatischer Modellwechsel „für das beste Ergebnis"

Das ist eine **Produktfunktion**, keine Phase-1-Arbeit. Einordnung:

- **Phase 2:** regelbasiertes `Model: AUTO` (Aufgabe + VRAM-Budget + installierte
  Modelle → eine Wahl, mit sichtbarer Begründung, überschreibbar). Brief §15.
- **Phase 6:** benchmark-/qualitätsgetriebene Auswahl; optional „Best-of-N"
  (dieselbe Aufgabe mit mehreren Modellen/Settings, dann bestes Ergebnis
  auswählen). Braucht die Benchmark-Datenbasis + ein Bewertungskriterium.
- **Phase-1-Konsequenz:** `Scheduler`- und `JobEngine`-Schnittstellen so bauen,
  dass (a) ein Job das Modell als `Explicit(id)` **oder** `Auto{capability,
  constraints}` angeben kann und (b) ein Job mehrere Kind-Jobs erzeugen kann
  (für späteres Best-of-N). Nicht mehr — nur die Tür offen halten.
