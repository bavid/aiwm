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

**`settings`-Tabelle vs. `config.toml`:** `config.toml` ist die einzige Quelle
für Startkonfiguration (`store_path`, `core_api_port`, `offline_mode`,
`log_filter`). Die `settings`-Tabelle hält nur **app-verwaltete** Marker, die das
Tool selbst schreibt. Phase 1 seedet: `schema_version=1`, `first_run_at`.
`installed_llamacpp_version` u. Ä. kommen mit ihren Consumern.

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
| **WP-1 ✅** | Core-Bootstrap: tokio-main, `config.toml` laden/validieren, `tracing` → rotierende Datei, Datenordner (`%APPDATA%\AIWorkstationManager\`) anlegen, sauberer Ctrl-C-Shutdown | WP-0 | Core startet, schreibt Log, legt Ordner an, beendet sauber |
| **WP-2 ✅** | `sqlx`-Pool, Migration-Runner, Schema v1, Repository-Traits + SQLite-Impls, Default-Settings | WP-1 | Frische DB aus Migration; Repo-Unit-Tests (in-memory) grün |
| **WP-3 ✅** | Telemetrie-Modul (NVML + sysinfo), 1-Hz-Sampler, `watch`-Channel, Graceful degradation | WP-1 | Mock-Test grün; reale 4080S-Werte auf der Maschine |
| **WP-8** | Sidecar-Contract (JSON-RPC), `uv`-Projekt, `main.py` (handshake/ping), `SidecarClient` | WP-1 | Spawn + Handshake + `ping`-Roundtrip; Sidecar stirbt mit Core |
| **WP-4 ✅** | `RuntimeAdapter`-Trait, `RuntimeSupervisor`, Windows Job Object, `FakeRuntimeAdapter` | WP-1 | Job-Object-Kill-Test grün; Contract-Tests gegen Fake |
| **WP-5 ✅** | Job-Zustandsmaschine, `JobEngine`, `Scheduler`-Trait, `HybridScheduler`-Skelett, Szenariomatrix-Tests | WP-2, WP-4 | Übergangs- + Szenariomatrix-Tests grün; Persistenz + Crash-Replay |
| **WP-6 ✅** | Core-API-Handler, Tauri-Commands, axum HTTP/WS (Loopback), Event-Streams, `JobEngine`-Run-Loop im Daemon | WP-2, WP-3, WP-5 | Beide Transporte liefern identisches JSON; Loopback-only nachgewiesen |
| **WP-7 ✅** | UI-Shell: Dashboard + Diagnostics, `ipc.ts`, Live-Telemetrie | WP-6 | `pnpm tauri dev` zeigt live echte Telemetrie |
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

### WP-1 — Ergebnis (abgeschlossen)

`core`-Module: `paths` (Layout, `AIWM_DATA_DIR`-Override), `config` (`Config`
laden/erzeugen/validieren, `AIWM_*`-Overrides über injizierte Lookup-Funktion —
kein Prozess-Env in Tests), `logging` (rotierende Datei + stderr, Filter-Validierung),
`app` (`App::load` = Ordner + Config; `bootstrap_process` = + Logging + Startlog).

Neues Bin **`aiwm-cored`**: headless Core, installiert Shutdown-Handler
(ctrl-c/break/close/shutdown auf Windows; SIGINT/SIGTERM sonst) **vor** dem
Ready-Banner, wartet, beendet sauber.

Tauri-Host ruft `bootstrap_process()` im Start, hält `App` + Log-Guard als State;
`about`-Command liefert jetzt die Config-Zusammenfassung.

Tests: **23 Unit + 1 Integration** (`daemon_shutdown.rs` startet `aiwm-cored`,
schickt `CTRL_BREAK_EVENT`, prüft Exit 0 + „shutdown complete" + angelegte
Artefakte). Ganze `check.ps1` grün.

Zusatz zum Plan: `AIWM_DATA_DIR` überschreibt den Datenordner (portable Installs +
Tests) — war nötig, damit der Integrationstest nicht das echte `%APPDATA%` anfasst.

### WP-2 — Ergebnis (abgeschlossen)

`core/migrations/0001_init.sql` = komplettes Schema v1 (7 Tabellen, `STRICT`,
FK-Constraints). `core::db`:

- `Database` — `sqlx`-Pool, `connect` (on-disk, WAL, `foreign_keys=ON`,
  `create_if_missing`) + `connect_in_memory` (für Tests, 1 gepinnte Connection),
  eingebettete Migrationen via `sqlx::migrate!`.
- `SettingsRepo` — `get` / `set` (upsert) / `set_if_absent` (Seeding) / `all`.
- Weitere Repos (`runtimes` → WP-4, `jobs` → WP-5, `models` → Phase-2-Importer)
  landen mit ihrem Consumer, nicht spekulativ.

`App` hält jetzt `db: Database`; `App::load` ist async, öffnet die DB und seedet
`schema_version` + `first_run_at` idempotent. `aiwm-cored` und der Tauri-Host
(`RunEvent::Exit`) schließen den Pool sauber — verifiziert: kein `-wal`/`-shm`
nach sauberem Beenden.

Entscheidung: `config.toml` bleibt einzige Quelle der Startkonfig; die
`settings`-Tabelle nur für app-verwaltete Marker (siehe §4).

Kein compile-time Query-Checking (`sqlx::query!`) — Runtime-Queries, um den
`DATABASE_URL` / `cargo sqlx prepare`-Umweg zu sparen. Kann später via CI-Schritt
nachgezogen werden ([TODO.md](TODO.md)).

Tests: **33 Unit + 1 Integration**. Ganze `check.ps1` grün.

### WP-3 — Ergebnis (abgeschlossen)

`core::telemetry` (Ordnermodul):

- `SystemTelemetry` — Snapshot: `gpu: GpuStatus` (`Available(GpuInfo)` /
  `Unavailable { reason }`, serde-tagged), `host: HostStatus` (RAM + CPU
  gesamt/pro Kern), `captured_at_ms`.
- `gpu.rs` — `GpuSource`-Trait; `NvmlSource` (Device 0: Name, VRAM, Util, Temp,
  Top-VRAM-Prozesse pid+MB); `real_source()` fällt bei fehlendem NVML sauber auf
  `NoGpu` zurück. Nicht-kritische Metriken (Util/Temp/Prozesse) → 0 statt Fehler.
- `host.rs` — `HostSource`-Trait; `SysinfoHost` (RAM + CPU via sysinfo,
  `system`-Feature only).
- `Sampler` — Hintergrund-Task, 1 Hz, `watch<SystemTelemetry>`; `latest()` immer
  verfügbar (erster Sample synchron vor dem Spawn), `subscribe()` für Streams,
  Task wird bei Drop abgebrochen. Quellen injizierbar (`spawn_with`) → Mock-Tests.

`App` hält `telemetry: Sampler` (in `App::load` gestartet). `aiwm-cored` druckt
die erste Messung in den Ready-Banner.

Verifiziert auf der Maschine:
`NVIDIA GeForce RTX 4080 SUPER 1594/16376 MB VRAM, 39% util, 43°C |
RAM 15577/31967 MB | CPU 5%` — deckt sich mit `nvidia-smi`.

Tests: **40 Unit + 1 Integration**. Ganze `check.ps1` grün.

### WP-4 — Ergebnis (abgeschlossen)

`core::runtime` (Ordnermodul):

- `RuntimeAdapter` (async-trait): `id`, `kind`, `spawn_spec`, `health`,
  `load_model(model_id, vram_mb)`, `unload_model`, `loaded_models`,
  `vram_used_mb`. Typen: `RuntimeKind`, `Health`, `SpawnSpec`.
- `job.rs` — `JobObject`: RAII-Wrapper um ein Windows Job Object mit
  `KILL_ON_JOB_CLOSE` (via `win32job`, **kein eigenes `unsafe`**). Non-Windows:
  No-op.
- `supervisor.rs` — `RuntimeSupervisor`: spawnt den Child (tokio, `kill_on_drop`)
  ins Job Object, Monitor-Task per `tokio::select!` (Stop-Signal vs. `child.wait()`),
  Auto-Restart mit gedeckelter Exponential-Backoff (`MAX_RESTARTS=10`,
  200 ms … 30 s). `stop()` / Drop beenden den Prozessbaum zuverlässig.
- `fake.rs` — `FakeRuntimeAdapter` + `FakeConfig`: kein echter Prozess,
  konfigurierbar (Health, Load-Delay, Load-Fehler pro Modell); zählt Aufrufe.
  Basis für die Scheduler-Tests in WP-5.

Tests: **48 Unit + 1 Integration**. Die kritischen (Windows-only):
`dropping_the_job_kills_assigned_processes`,
`dropping_the_supervisor_kills_the_process`,
`crashed_process_is_restarted_with_backoff` — alle grün.

Der `core`-Crate hat jetzt **null `unsafe`** im Produktivcode.

Offen für später: CREATE_SUSPENDED + Resume gegen das (winzige) Race-Fenster
zwischen `CreateProcess` und `AssignProcessToJobObject` ([TODO.md](TODO.md)).

### WP-5 — Ergebnis (abgeschlossen)

- `orchestrator::state` — `JobState` (9 Zustände) + explizite Übergangstabelle
  (`can_transition_to` / `ensure_transition`); alles nicht Gelistete ist
  `CoreError::InvalidJobTransition`.
- `db::jobs` — `JobRepo`: `insert` / `get` / `list(filter)` / `next_runnable`
  (FIFO über queued+blocked) / `set_state` (validiert Übergang, schreibt Event) /
  `append_event` / `events` / **`recover_interrupted`** (Crash-Replay: alles
  mid-flight → failed).
- `runtime::RuntimeRegistry` — geteilte Adapter-Map, VRAM-Summe,
  `runtime_with_model`.
- `scheduler` — `Scheduler`-Trait (`plan` + `pin`/`unpin`/`is_pinned`);
  `HybridScheduler`: `frei = Budget − Treiber-Overhead − Headroom − Σ(geladen)`,
  Decision `RunNow | LoadThenRun | EvictThenLoad{victim} | Blocked{reason}`,
  gepinnte (Agent-)Modelle werden nie evakuiert. **Szenariomatrix**
  (ADR-003) als Tabellen-Tests.
- `orchestrator::engine` — `JobEngine`: `submit` / `run_next` (treibt einen Job
  bis zum Ruhezustand) / `recover`. Job-Body ist in WP-5 ein No-op
  (Fake-Adapter „lädt"). Agent-Sessions pinnen ihr Modell; `EvictThenLoad`
  entlädt das Opfer real (über Registry) und lädt neu.

Schema-Änderung: `jobs.runtime_id` / `jobs.model_id` sind **ohne FK-Constraint**
(Job-Historie ist append-only und soll das Entfernen von Modellen/Runtimes
überleben). Migration 0001 angepasst (noch keine Release-DB).

`App::load` ruft jetzt `recover_interrupted()` beim Start.

Tests: **75 Unit + 1 Integration**. Ganze `check.ps1` grün.

### WP-6 — Ergebnis (abgeschlossen)

- `api::handlers` — transport-agnostische Funktionen (`about`, `telemetry`,
  `settings` / `set_setting`, `list_jobs`, `job_events`, `submit_job`,
  `runtimes`, `recent_logs`). Beide Transporte serialisieren **dieselben**
  Funktions-Rückgaben.
- `api::http` — axum-Router auf `127.0.0.1` (ADR-008): `GET /about /telemetry
  /settings /jobs /jobs/{id} /runtimes /logs`, `PUT /settings/{key}`,
  `POST /jobs`, `GET /ws` (Telemetrie-Stream, 1 Hz). `CoreError` →
  400 (Config/Transition) / 500.
- `api::{ApiServer, Services, spawn}` — bindet den Server + startet die
  **JobEngine-Run-Loop** (`run_next` mit 250 ms Idle-Poll); `Services`-Drop
  stoppt beides.
- `App` hält jetzt `runtimes`, `scheduler` (VRAM-Budget = `config.vram_budget_mb`
  > 0, sonst GPU-Total, sonst 8192), `jobs: JobEngine`. `bootstrap_process`
  liefert `Arc<App>`.
- `aiwm-cored` + Tauri-Host starten `api::spawn`; der Tauri-Host spiegelt
  `about` / `get_telemetry` / `get_settings` / `list_jobs` als
  `#[tauri::command]`s über dieselben `handlers`.
- `Config.vram_budget_mb` (Default 0 = auto).

Verifiziert:
- `about_is_identical_over_the_handler_and_http` — Handler-Ausgabe ==
  HTTP-Body (byte-identisch)
- `server_binds_loopback_only`, WS-Stream liefert Telemetrie-Frames,
  Job-Loop leert die Queue, 400 bei leerem Setting-Key
- Live: `aiwm-cored` und Tauri-Host beantworten `GET /about` auf
  `127.0.0.1:48160`, `vram_budget_mb=16376` (auto vom 4080S)

Tests: **81 Unit + 1 Integration**. Ganze `check.ps1` grün.

### WP-7 — Ergebnis (abgeschlossen)

UI (`ui/src/`, organisiert nach Feature):

- `lib/ipc.ts` — typisierte `invoke`-Wrapper + Typen (spiegeln die Core-DTOs).
- `lib/hooks.ts` — `useTelemetry` (Seed via `get_telemetry`, dann `telemetry`-Event
  vom Host, 1 Hz Push), `useJobs`/`useRuntimes`/`useLogs` (Polling), `useAbout`.
- `components/Meter` — Balken mit last-abhängiger Farbe (ok/warn/crit),
  compositor-freundlich (`transform: scaleX`).
- `features/dashboard` — GPU-Karte (VRAM-Meter, Util, Temp, Budget), Host-Karte
  (RAM/CPU), Job-Tabelle mit „No jobs yet"-Zustand, vier deaktivierte
  Capability-Buttons.
- `features/diagnostics` — Environment-Key/Value, Runtime-Tabelle,
  Log-Tail (monospace, auto-scroll).
- `styles/tokens.css` — Design-Tokens, hell **und** dunkel
  (`prefers-color-scheme`, kein Dark-by-default-Zwang). Bewusst zurückhaltend;
  visuelle Richtung wird zu Phase-2-Start geschärft.

Host (`src-tauri`): neue Commands `get_runtimes` / `get_recent_logs`;
`.setup()` pusht jede Telemetrie-Messung als `telemetry`-Event an die WebView.

Verifiziert: `pnpm tauri dev` zeigt das Fenster mit **echten Live-Werten**
(RTX 4080 SUPER 1.1/16.0 GB VRAM, 15 % Util, 42 °C; RAM 14.9/31.2 GB; CPU 10 %).
`typecheck` / `lint` / `build` grün.

Offen für später: `eslint-plugin-react-hooks` in die UI-Lint-Config
([TODO.md](TODO.md)).

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
