# Phase 2 — MVP: Plattform-Kern

Erste echte Capability: **Chat** über eine lokale llama.cpp-Runtime. In Scheiben,
jede für sich testbar.

| Scheibe | Inhalt | Status |
|---|---|---|
| **2.1** | `ModelRepo` (CRUD + Rollen) · GGUF-Header-Inspektion · manueller Import in den kanonischen Store · API + UI-Tab „Models" | ✅ |
| **2.2a** | `LlamaCppAdapter`: Binär-Auflösung (Env / Managed-Ordner / PATH), ein `llama-server`-Prozess pro residentem Modell über `RuntimeSupervisor`, `/health`-Polling, `unload`, **Attach-Fallback** auf laufenden Port, nicht-streamendes `complete()` | ✅ |
| **2.2b** | llama.cpp-**Installer**: gepinnter CUDA-Build (`ggml-org/llama.cpp` Release-Assets), SHA-256-verifizierter Download, Doppel-Zip-Entpacken, `offline_mode`-Hard-Refusal, `RuntimeRepo`, `POST /runtimes/llamacpp/install` + UI-Knopf mit Fortschritt | ✅ |
| 2.3 | Link-Manager: kanonische Datei ↔ Runtime via NTFS-Junction (ADR-007); `model_links`-Tabelle | offen |
| 2.4 | Chat-Job: `job_type=chat`, Modell (explizit oder `Auto`) → Scheduler → llama-server laden → Prompt → Antwort streamen; `job_events` + Cancel | offen |
| 2.5 | UI: „Chat"-Capability-Button aktiv, einfache Prompt/Antwort-Oberfläche; Job-Fortschritt aus dem Event-Stream | offen |
| 2.6 | Kompatibilitäts-Check vor dem Laden (VRAM-Budget vs. `vram_estimate_mb` + KV-Cache-Schätzung), Klartext-Fehler | offen |
| 2.7 | Settings-UI (Store-Pfad, Offline-Schalter, Theme), Diagnostics erweitert | offen |

**Phase-2-DONE-Kriterium:** frisches Windows → App → llama.cpp wird eingerichtet
→ GGUF importieren → Chat-Job läuft → zweites Modell → Wechsel ohne manuelles
VRAM-Management → Netz trennen → läuft weiter.

---

## 2.1 — Ergebnis (abgeschlossen)

- `db::ModelRepo` — `insert` (mit Rollen, in einer Transaktion, dedupe+sort) /
  `get` / `list` (Rollen via zweite Query gejoined) / `find_by_sha256` /
  `find_by_path` / `delete` (FK-Cascade auf `model_roles`) / `mark_used` /
  `roles`. `models.file_path` ist UNIQUE.
- `model::gguf` — **eigener, bounded GGUF-Header-Reader** (kein Fremd-Crate für
  untrusted Binärdaten): Magic/Version, Metadaten-KVs (alle Typen, Arrays werden
  *übersprungen* statt materialisiert), Tensor-Infos für die exakte
  Parameterzahl. Alle Counts/Längen range-checked. Liefert `architecture`,
  `name`, `quantization` (aus `general.file_type`), `context_length`,
  `parameter_count`.
- `model::import` — `import_model(db, store_root, ImportRequest)`:
  validieren (`.gguf`) → SHA256 (streaming, `spawn_blocking`) → GGUF inspizieren →
  Dedupe über SHA256 (No-op wenn schon da) → Zielpfad
  `<store>/llm/<slug>/<originalname>` → move (oder copy bei `keep_original`,
  cross-volume-Fallback) → `ModelRepo::insert`. `vram_estimate_mb` =
  Dateigröße + 1 GB Headroom (klar als Schätzung markiert).
- API: `GET /models` (jetzt echt), `POST /models` (201 / 200 bei Dedupe);
  Tauri-Commands `list_models` / `import_model`.
- UI: Tab **„Models"** — Import-Formular (Pfad, Rollen-Chips, keep-original) +
  Model-Tabelle (Name, Arch, Quant, Params, Größe, Ctx, VRAM-Schätzung, Rollen).

Verifiziert:
- 16 neue Tests (6 ModelRepo, 5 GGUF-Parser inkl. truncated/garbage/implausible,
  4 Import inkl. move/copy/dedupe, 1 slugify).
- Live: `POST /models` mit einer handgebauten Mini-GGUF → korrekt geparst
  (`llama` / `Q4_K_M` / param_count aus Tensoren summiert / ctx 4096), Datei in
  den Store verschoben, `GET /models` liefert sie zurück.

Zusatz: `rust-version` von 1.80 auf **1.85** angehoben (`ErrorKind::CrossesDevices`).

Offen für 2.2+: `safetensors`-Import, GGUF-Metadaten für Multimodal/Embedding,
publisher aus HF-Repo-Struktur, RAM-Schätzung.

---

## 2.2a — Ergebnis (abgeschlossen)

- **`core::runtime::llamacpp`** (neues Submodul: `mod.rs` = Adapter/State,
  `client.rs` = HTTP, `launch.rs` = Binär-Auflösung + `SpawnSpec`):
  - `LlamaCppAdapter` implementiert `RuntimeAdapter`. `llama-server` bedient
    genau ein Modell pro Prozess → der Adapter fährt **einen** überwachten
    Server für das residente Modell. Ein zweites Modell zu laden tauscht das
    residente aus (auch wenn der Scheduler dachte, beide passen).
  - **Binär-Auflösung** (`launch::resolve_server_bin`): `AIWM_LLAMACPP_PATH` →
    `<data_dir>/runtimes/llamacpp/[<build>/][bin/]llama-server.exe` → `PATH`.
    Kein Treffer ⇒ Adapter registriert sich trotzdem, meldet aber
    „not installed".
  - `load_model`: Modelldatei über `ModelRepo` auflösen → freien Loopback-Port
    reservieren → `SpawnSpec` bauen (`-m … --host 127.0.0.1 --port … --no-webui
    -ngl 999 --flash-attn on`) → `RuntimeSupervisor::start` → `GET /health`
    pollen bis `200` (oder `SupervisorState::GaveUp` / Timeout).
  - `unload_model`: Supervisor stoppen (nur eigene Prozesse; **Attached** wird
    bloß vergessen). `health()`: `Unknown` (idle) / `Starting` (lädt) / Probe
    des Ports. `loaded_models` / `vram_used_mb` für den Scheduler.
  - **`attach(port, model_id, vram)`** (ADR-002-Fallback): probt `/health`,
    Best-Effort-Abgleich der Modelldatei über `GET /props`; adoptiert den Server
    ohne seine Lebensdauer zu übernehmen.
  - `complete(prompt, max_tokens)`: nicht-streamendes `POST /completion` — für
    den End-to-End-Beweis; Streaming + Chat-Template folgen in 2.4.
  - `detail()` (neue optionale `RuntimeAdapter`-Methode, default `None`): eine
    Klartext-Statuszeile („not installed" / „installed · idle" /
    „serving <Name> on :<Port>").
- **`LlamaClient`** (`client.rs`): `reqwest` (kein TLS, Loopback), `/health`
  (200→Healthy, 503→Starting, sonst→Unhealthy), `/completion`, `/props`.
- **`reqwest`** von dev-dep zu regulärer Workspace-Abhängigkeit befördert
  (`default-features = false`, `json`).
- **`aiwm-fake-llama`** (`core/src/bin/`): Test-Fixture, ein minimaler
  `llama-server`-Ersatz (`/health` mit optionaler Ladeverzögerung, `/props`,
  `/completion`) — damit der Spawn-/Health-/Serve-/Stop-Zyklus ohne 600-MB-
  Download getestet wird.
- **Wiring**: `App::load` registriert `LlamaCppAdapter::discover`.
  `RuntimeStatusDto` + `detail`. UI: Diagnostics zeigt die Statuszeile.
  `AppPaths::runtimes_dir()`.

Verifiziert:
- 22 neue Tests: 6 `LlamaClient` (In-Process-axum-Mocks), 6 `launch` (SpawnSpec-
  Flags, Resolver-Reihenfolge, Port), 6 Adapter (`mod.rs`) + 4 Integrationstests
  `tests/llamacpp_adapter.rs` gegen den echten Fixture-Subprozess: load →
  `/health` → `/completion` → unload; Reload = No-op; zweites Modell tauscht;
  fehlende Datei sauber abgewiesen. `check.ps1` grün.
- Live (`aiwm-cored`, Fake-Binär via `AIWM_LLAMACPP_PATH`): Mini-GGUF importiert,
  Chat-Job abgesetzt → Job-Engine → Scheduler → Adapter startet echten
  Subprozess auf Port, Job `queued → running → completed`, `GET /runtimes`
  zeigt `health: healthy` / `serving Smoke Coder 1B on :55890`. Bei
  cored-Shutdown wird der Subprozess über das Job Object mitgetötet.

Bewusst **nicht** in 2.2a: der Download/Installer (→ 2.2b), Streaming (→ 2.4),
Router-Mode (ein Server, mehrere Modelle — als spätere Optimierung notiert),
Port-Ermittlung aus `llama-server`-stdout statt Bind-and-Drop (→ [TODO.md](TODO.md)).

---

## 2.2b — Ergebnis (abgeschlossen)

- **`runtime::llamacpp::install`**: `install(runtimes_dir, offline, on_progress)`
  → `install_from(base_url, archives, …)` (auf die Archivliste + URL
  parametrisiert, damit End-to-End gegen einen lokalen Server testbar).
  - Gepinnt: `PINNED_BUILD = "b10855"`, zwei Assets von `ggml-org/llama.cpp`
    (`llama-…-bin-win-cuda-12.4-x64.zip` + `cudart-…-12.4-x64.zip`), **SHA-256 +
    Größe fest im Code** (Werte aus dem `digest`-Feld der GitHub-Releases-API).
  - Streaming-Download (`reqwest` + `.chunk()`), SHA-256 mitlaufend; Mismatch
    oder falsche Größe → Datei gelöscht, Fehler. Entpacken (`zip`-Crate, in
    `spawn_blocking`, `enclosed_name()` gegen Zip-Slip). Beide Zips **flach** in
    `<runtimes_dir>/llamacpp/b10855/` → `llama-server.exe` neben seinen CUDA-DLLs.
  - Idempotent (fertige Installation wird unverändert zurückgegeben);
    `offline_mode` → Hard-Refusal (auch im Handler).
- **`db::RuntimeRepo`** (`runtimes`-Tabelle, WP-4 nachgeholt): `get` / `all` /
  `upsert` / `set_state(id,kind,state,err)` / `record_install` / `mark_health`.
  Der Installer schreibt `installing` → `stopped`+Version bzw. `error`+Meldung.
- **Adapter**: `BinSource::{Fixed, Scan}` — im Betrieb wird bei **jedem** Aufruf
  neu aufgelöst, damit eine frische Installation ohne Neustart greift.
  `install_state()` (`Idle` / `Running{phase,done,total}` / `Failed`), von
  `detail()` als „downloading llama.cpp — 42%" / „extracting…" / „setup failed: …"
  gerendert. `App` hält den Adapter zusätzlich typisiert (`app.llama`).
- **API/UI**: `POST /runtimes/llamacpp/install` (202/200) + Tauri-Command
  `install_llamacpp`. Diagnostics: „Set up llama.cpp"-Knopf, der bei „not
  installed" / „setup failed" erscheint und den Live-Fortschritt zeigt.
- **`AppPaths`**: `root` (Roaming `%APPDATA%`, Config+DB) vs. `local_root`
  (`%LOCALAPPDATA%`, `runtimes_dir()`) getrennt — 1-GB-Runtime-Installs sollen
  nicht roamen. `AIWM_DATA_DIR` kollabiert beide.
- **`reqwest`** braucht jetzt TLS für den GitHub-Download → Feature `native-tls`
  (Schannel auf Windows, kein OpenSSL, keine gebündelten CA-Roots). ADR-013 aktualisiert.

Verifiziert:
- 12 neue Tests (5 `RuntimeRepo`, 6 `install` inkl. Voll-Pipeline gegen lokalen
  Server + Zip-Bau/-Entpacken + Hash-Reject + Offline-Refusal + Idempotenz,
  2 API-Handler, +2 `AppPaths`). `check.ps1` grün.
- **Live** (`aiwm-cored`, `POST /runtimes/llamacpp/install`): 645 MB in 22 s
  heruntergeladen, beide SHA-256 verifiziert (kein Mismatch), 55 Dateien / 1,1 GB
  flach entpackt; `detail` lief `14% → … → 94% → installed · idle`; die echte
  `llama-server.exe --version` läuft (`build 10855`) — Binär **und** CUDA-DLLs
  laden. Adapter erkennt die Installation ohne cored-Neustart.

Offen für 2.2b+: freien Speicherplatz vor dem Download prüfen (Brief 10.16 →
Phase 6 Download-Manager); Cleanup alter Build-Verzeichnisse beim Versions-Bump.
