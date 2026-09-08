# Architecture Decision Records

Format je Eintrag: Entscheidung · Kontext · Alternativen · Warum verworfen ·
Konsequenzen · Status.

---

## ADR-001 — Desktop-Stack: Tauri 2 + Rust-Core + Python-Sidecar

**Status:** Entschieden (mit dem Projektinhaber, Discovery-Phase).

**Kontext:** Die App ist überwiegend Orchestrierung: Kindprozesse überwachen,
GPU/RAM monitoren, Jobs schedulen, lokale DB, lokale API. Dazu eine schlanke UI.
Teile des AI-Ökosystems (HF-Hub-Client, Modell-Datei-Inspektion, Agent-CLIs)
leben in Python.

**Alternativen:**
- *Electron + Node/TS:* schnellste Entwicklung, ein JS-Ökosystem. Verworfen:
  großer RAM/Disk-Footprint für ein Dauerläufer-Tool, schwächeres
  Prozess-Lifecycle unter Windows, Python trotzdem als Subprozess nötig.
- *Python + FastAPI + lokale Web-UI:* native Ökosystem-Nähe. Verworfen:
  schwaches OS-/Tray-/Prozess-Handling, Windows-Packaging unangenehm, GC-Pausen
  im Monitoring, schwächere Typsicherheit im kritischen Kern.
- *Go-Core + Web-UI:* gute Prozesskontrolle, klein. Verworfen: schwächere
  GPU-/ML-Bindings, weniger Fit als Rust für die Trait-lastige Adapter-Architektur,
  ebenfalls Mehr-Sprachen-Stack.

**Konsequenzen:**
- (+) Kleines Bundle, geringer Idle-Verbrauch, robustes Kill-Verhalten (Job Objects),
  typsicherer Kern.
- (−) Drei Sprachen (Rust/TS/Python). Rust-Lernkurve. Gegenmaßnahme: dünner
  Sidecar, eine versionierte RPC-Grenze, scharfe Modulgrenzen.

---

## ADR-002 — Runtime-Verhältnis: Manage-first (mit Attach-Fallback)

**Status:** Entschieden (mit dem Projektinhaber).

**Kontext:** Das Tool soll Discovery/Install/Update/Removal von Runtimes zentral
übernehmen, damit der Nutzer nicht mehr zwischen Websites/Tools wechselt.

**Alternativen:**
- *Attach-first:* nur vorhandene Installationen erkennen/steuern. Geringstes
  Risiko. Als **Fallback** übernommen, nicht als Primärmodus.
- *Reine Orchestrierung:* nie installieren. Verworfen: widerspricht dem
  Kernnutzen „nicht mehr fünf Programme".

**Konsequenzen:**
- (+) Ein Ort für alles, konsistente Versionen.
- (−) Wir erben Environment-/CUDA-/Versionskomplexität (Risiko R2/R6).
  **Bindende Konsequenz:** pro Runtime genau *eine* kuratierte, getestete Version
  und *eine* Installationsart. Keine frei konfigurierbare Umgebung. „Repair"-Pfad
  der die venv sauber neu baut.

---

## ADR-003 — VRAM-Strategie: Hybrid-Scheduler, session-aware

**Status:** Entschieden (mit dem Projektinhaber).

**Kontext:** 16 GB VRAM, mehrere Modalitäten. Weder „alles resident" noch „immer
nur eins" allein ist gut.

**Entscheidung:** Ein resident Slot für das aktive Modell + Job-Queue für den
Rest + automatisches unload/reload für **nicht gepinnte** Modelle. Modelle mit
aktiver Agent-Session sind gepinnt und werden nie automatisch evakuiert; bei
VRAM-Konflikt entscheidet der Nutzer per Klartext-Dialog.

**Alternativen:**
- *Immer resident, manuell:* schnellste Wiederholung, aber ständige OOM-Fehler
  und Nutzer trägt alle Entscheidungen. Verworfen als Default.
- *Strikte Queue (immer 1 Modell):* maximal stabil/einfach, aber langsamer
  Kontextwechsel Coding↔Bild. Bleibt als wählbarer „Safe Mode".

**Konsequenzen:**
- (+) Gute Balance aus UX/Stabilität.
- (−) Scheduler-Logik ist der komplexeste Kern-Teil → umfangreiche Tabellentests
  für die VRAM-/Session-Szenariomatrix. Model-Load-Zeiten (Sekunden bis
  Minuten) müssen im UI sichtbar sein.

---

## ADR-004 — MVP-Scope: Plattform-Kern zuerst

**Status:** Entschieden (mit dem Projektinhaber).

**Kontext:** Alternativen waren „Coding zuerst", „Bild zuerst", „alle drei dünn".

**Entscheidung:** MVP = Dashboard + Resource-Manager + Job-System + lokale
Model-Library + eine LLM-Runtime, mit *nur* Chat/Completion als erster Capability.
Kein Bild/Video/Agent, keine Online-Discovery, keine Benchmarks.

**Warum die anderen verworfen:** „Coding zuerst" würde Agent-Komplexität
(Memory, Sandbox, Langlauf) in den MVP ziehen. „Bild zuerst" zieht die
ComfyUI-Pipeline-Abstraktion vor. „Alle drei dünn" maximiert Integrationsrisiko.
Kern zuerst gibt ein stabiles, getestetes Fundament, auf das jede Capability
gleich aufsetzt.

**Konsequenzen:**
- (+) Sauberes Fundament, klare Contracts, jede spätere Capability ist „nur ein
  Adapter + Pipelines".
- (−) Längere Zeit bis zu sichtbarem AI-Nutzen. Gegenmaßnahme: „Chat" als kleine
  echte Capability im MVP, damit es nicht nur Monitoring ist.

---

## ADR-005 — Persistenz: SQLite + Dateisystem

**Status:** Entschieden — umgesetzt in WP-2 (`sqlx` + SQLite, Schema v1,
`core/migrations/`).

**Entscheidung:** Eine SQLite-DB unter `%APPDATA%\AIWorkstationManager\aiwm.db`
für alle Metadaten (Modelle, Jobs, Agents, Settings). WAL, `foreign_keys=ON`.
Große Binärdaten (Modelle, Outputs) im Dateisystem, DB hält nur Pfade + Hashes.
`job_events` ist append-only. Runtime-Queries (kein compile-time `sqlx::query!`),
siehe [TODO.md](TODO.md).

**Alternativen:** Postgres (Overkill, Server-Prozess), reine JSON-Dateien (keine
Abfragen/Transaktionen). DuckDB später ergänzend für Benchmark-Analytik denkbar.

---

## ADR-006 — Primäre LLM-Runtime: llama.cpp / llama-server

**Status:** Entschieden (Projektinhaber, Entscheidung A = „ja").

**Entscheidung:** llama-server als primäre LLM-Runtime; Ollama optionaler
Zweitadapter; LM Studio vorerst nicht.

**Begründung:** Echtes Modell-Sharing (ein GGUF, viele Konsumenten via Junction),
volle Kontrolle über KV-Cache-Quantisierung und CPU-Offload (auf 16 GB
entscheidend), sauber als Kindprozess steuerbar, MIT-Lizenz, headless.
Ollamas content-addressed Store erzwingt Kopien (Duplikate) und verbirgt Flags.

**Konsequenz:** Wir bauen etwas mehr Modell-Verwaltung selbst — deckt sich mit
dem ohnehin geplanten kanonischen Store.

---

## ADR-007 — Modell-Storage: kanonischer Store + Runtime-Links

**Status:** Vorgeschlagen (Schema-Tabellen `models` / `model_links` existieren
seit WP-2; der Link-Manager selbst kommt mit dem Phase-2-Modell-Importer).

**Entscheidung:** Eine kanonische Datei pro Modell im Store (`E:\AI\models\`).
Ein Link-Manager verknüpft sie pro Runtime: NTFS-Junction (llama.cpp, LM Studio,
ComfyUI) oder, wo unvermeidbar, Kopie/Import (Ollama). Duplikate durch Import
werden im Dedup-Report ausgewiesen.

---

## ADR-008 — Netzwerk-Exposure: nur `127.0.0.1`

**Status:** Entschieden — umgesetzt in WP-6 (`api::spawn` bindet
`SocketAddr::from((Ipv4Addr::LOCALHOST, port))`; Test `server_binds_loopback_only`).

**Entscheidung:** Core-API und alle verwalteten Runtimes binden ausschließlich
Loopback. Kein LAN-Listener, kein Remote-Zugriff im MVP. LAN/Remote frühestens
Phase 6, opt-in, mit Auth.

---

## ADR-009 — Offline-first als Querschnitts-Contract

**Status:** Entschieden (Querschnitts-Prinzip; teils umgesetzt: Telemetrie
degradiert ohne GPU, `Config.offline_mode` existiert). Online-Cache-Fallbacks
werden pro Feature ab Phase 6 fällig.

**Entscheidung:** Jede Funktion, die Online-Daten nutzt, braucht einen lokalen
Cache-Fallback und degradiert ohne Netz sauber (kein Fehler-Abbruch). Ein
globaler Offline-Modus (`offline_mode` in `config.toml`) blockt jeden externen
Call hart. Kein Cloud-Provider im Default-Pfad; Cloud ist pro Aktion zu
bestätigen.

---

## ADR-010 — Agent-Runtimes (Phase 5): Hermes Agent + OpenCode

**Status:** Entschieden (Projektinhaber, Entscheidung E).

**Kontext:** „Hermes" im Brief meint **Hermes Agent von Nous Research**
(github.com/NousResearch/hermes-agent, MIT, Release Feb 2026). Es ist kein reiner
Coding-Agent, sondern eine allgemeine Agent-Plattform mit *persistentem Memory*,
selbst erzeugten Skills, CLI, Sub-Agents mit eigenem Terminal/Python-RPC — deckt
also viele Wünsche aus Brief-Abschnitten 4 und 18 direkt ab.

**Entscheidung:** Zwei Adapter in Phase 5.
- **Hermes Agent** als „Agent mit Gedächtnis / Langlauf" — Python 3.11 + `uv`,
  läuft nativ ohne Docker, spricht llama.cpp/vLLM/Ollama über „Custom endpoint",
  speichert alles in `~/.hermes/` (Memory, Sessions, Skills), keine Telemetrie.
- **OpenCode** als schlanker terminal-/`serve`-basierter Coding-Agent.

**Passt zur Vision:** beide MIT, beide voll offline mit lokalem Endpoint, keine
Account-/Cloud-Pflicht (Hermes' bezahlte Tiers betreffen nur das optionale Nous
Portal), alle Daten lokal.

**Konsequenzen / offene Punkte für Phase 5:**
- (+) Hermes' Memory-/Skill-/Sub-Agent-System müssen wir nicht selbst bauen —
  unser Tool orchestriert und begrenzt es (Profil, Modell, Pfad-Allowlist,
  Command-Approval).
- (−) **Windows-Risiko:** Hermes baut seinen Session-Kontext über `bash -l` —
  braucht Git-Bash oder WSL. Vor Phase 5 auf der echten Maschine verifizieren.
- (−) Hermes empfiehlt vLLM für „fully on-premise"; vLLM ist unter Windows
  nativ problematisch → wir fahren Hermes gegen unseren **llama-server**-Endpoint
  (deckt sich mit ADR-006).
- `~/.hermes/` ist von unserem kanonischen Store getrennt — Memory-Speicherort
  im Backup-Konzept (Phase 5) berücksichtigen.

---

## ADR-011 — Lizenz: privat, keine kommerzielle Nutzung

**Status:** Entschieden (Projektinhaber, Entscheidung F).

**Entscheidung:** Das Tool ist ein privates Projekt ohne kommerzielle Nutzung.
Keine Veröffentlichung als Open-Source-Produkt geplant. Quellcode-Repo bleibt
privat; falls später doch geteilt, dann unter einer Non-Commercial-Lizenz
(z. B. PolyForm Noncommercial) — nicht jetzt entscheiden.

**Konsequenz:** Abhängigkeiten trotzdem auf Lizenzverträglichkeit prüfen
(GPL-Runtimes wie ComfyUI werden als *separate Prozesse* aufgerufen, nicht
gelinkt → unkritisch). Keine Lizenz-/Marken-Themen bei rein privater Nutzung.

---

## ADR-012 — Modell-Speicherort & Kapazität

**Status:** Entschieden (Projektinhaber, Entscheidung G).

**Entscheidung:** Kanonischer Store unter `E:\AI\models\`. Auf `E:` sind
~1,5 TB frei — genug für eine ernsthafte Bibliothek über mehrere Phasen. Der
Model-Manager prüft trotzdem freien Platz vor jedem Download (Brief 10.16) und
liefert Unused-/Dedup-Reports.

---

## ADR-013 — Phase-1-Bibliotheken

**Status:** Entschieden — im Verlauf von WP-1…WP-8 gewählt, hier festgehalten.

| Zweck | Wahl | Warum |
|---|---|---|
| Async-Runtime | `tokio` | Standard; `process` / `signal` / `sync` gebraucht |
| DB | `sqlx` 0.9 (SQLite, bundled) | async, eingebettete Migrationen, kein System-SQLite; Runtime-Queries (kein `DATABASE_URL`) |
| GPU-Telemetrie | `nvml-wrapper` | direkte NVML-Bindings statt `nvidia-smi`-Parsing |
| Host-Telemetrie | `sysinfo` (`system`-Feature) | RAM/CPU, schmaler Feature-Satz |
| Windows Job Object | `win32job` | sichere API, **kein** handgeschriebenes `unsafe` |
| Async-Traits | `async-trait` | `dyn RuntimeAdapter` / `dyn Scheduler` |
| HTTP/WS-Server | `axum` 0.8 (`ws`) | tokio-nativ, schlank; teilt Handler mit den Tauri-Commands |
| Logging | `tracing` + `tracing-appender` | strukturiert, rotierende Datei |
| Fehler | `thiserror` (Bibliothek), `anyhow` (Bin-Ränder) | ecc/rust-Regeln |
| Sidecar-Runner | `uv run` | reproduzierbare venv; `resolve_uv()` findet uv auch ohne PATH |
| HTTP-Client | `reqwest` (`default-features = false`, `json` + `native-tls` + `http2`) | Loopback für Runtimes (ADR-008), **HTTPS** für den llama.cpp-Release-Download (ADR-014). `native-tls` = Schannel auf dem Windows-Target: kein OpenSSL, keine gebündelten CA-Roots, nutzt den OS-Trust-Store. Ab Phase 2 reguläre Abhängigkeit (vorher dev-only, ohne TLS). |
| ZIP-Entpacken | `zip` 8 (`default-features = false`, `deflate`) | Runtime-Archive entpacken; `spawn_blocking`, `enclosed_name()` gegen Zip-Slip |

**Verworfen:** `reqwest` mit `rustls` (0.13 kennt nur das Feature `rustls`, das
einen Crypto-Provider mitzieht; `native-tls`/Schannel ist auf dem Windows-Target
schlanker), compile-time `sqlx::query!` (Setup-Reibung, → TODO),
Electron/Node-Backend (ADR-001), Postgres (ADR-005).

---

## ADR-014 — llama.cpp-Integration: ein Server pro Modell + verifizierter Pin-Download

**Status:** Entschieden — umgesetzt (Adapter 2.2a, Installer 2.2b).

**Kontext:** `llama-server` bedient genau **ein** Modell pro Prozess. Der
Hybrid-Scheduler (ADR-003) hält einen residenten LLM-Slot. Manage-first (ADR-002)
verlangt, dass das Tool die Runtime selbst beschafft.

**Entscheidung:**
1. **Ein `llama-server`-Kindprozess pro residentem Modell**, verwaltet vom
   `RuntimeSupervisor`. Modellwechsel = Prozess-Neustart (Sekunden). Passt 1:1
   zum Scheduler-Slot; einfaches, robustes Lifecycle.
2. **Attach-Fallback** adoptiert einen vom Nutzer gestarteten Server, ohne seine
   Lebensdauer zu übernehmen.
3. **Pin-Quelle:** Release-Assets von `ggml-org/llama.cpp`, Build-Tag +
   SHA-256 + Größe **fest im Code** (`install::PINNED_ARCHIVES`). Die Digests
   stammen aus dem `digest`-Feld der GitHub-Releases-API; im Code (nicht zur
   Laufzeit geholt) sind sie MITM-fest und offline-deterministisch. Gepinnt:
   **CUDA 12.4**-Windows-Build + `cudart`-Zip (gebündelte CUDA-Runtime, kein
   System-CUDA). Beide entpacken flach in einen Ordner.

**Installer (2.2b):** Streaming-Download mit mitlaufendem SHA-256, Größen-Check,
`zip`-Entpacken in `spawn_blocking` (Zip-Slip-Schutz über `enclosed_name()`).
`offline_mode` → Hard-Refusal. Idempotent. Fortschritt über `install_state()` →
`detail()` in die UI. Ziel: `%LOCALAPPDATA%\…\runtimes\llamacpp\<build>\`
(nicht Roaming — kann 1 GB+ sein).

**Alternativen:**
- *Router-Mode* (ein Server, dynamisches Laden mehrerer Modelle, neu in
  llama.cpp): spart Neustartzeit, aber jung/weniger erprobt und passt schlechter
  zum residenten Ein-Slot-Modell. Als spätere Optimierung notiert.
- *Drittanbieter-CUDA-Builds* (`ai-dock` u. a.): zusätzliche Vertrauensgrenze;
  offizielle Assets sind inzwischen vollständig (inkl. CUDA), also verworfen.
- *Selbst kompilieren:* CUDA-Toolchain-Zwang auf der Nutzermaschine — verworfen.

**Konsequenzen:**
- (+) Kleiner, testbarer Adapter; Prozess-Isolation; deterministische Version;
  verifizierter, reproduzierbarer Download (live: 645 MB, beide Digests OK,
  `llama-server.exe --version` läuft = CUDA-DLLs laden).
- (−) Sekunden Latenz beim Modellwechsel (im UI sichtbar machen). Der freie Port
  wird per Bind-and-Drop reserviert (winziges Race) — später ggf. aus dem
  `llama-server`-stdout lesen ([TODO.md](TODO.md)).
- (−) Version-Bump = Code-Änderung (Tag + zwei Digests). Bewusst: ein kuratierter
  Build (ADR-002), Digests im Code sind sicherer als API-geholte.

---

## Offene Entscheidungen

| # | Frage | Status |
|---|---|---|
| A | Primäre LLM-Runtime | ✅ llama.cpp (ADR-006) |
| B | Manage-first ohne freie Env-Konfiguration | ✅ Ja (ADR-002) |
| C | UI-Sprache | ✅ Default: Englisch, i18n-fähig |
| D | Netzwerk-Exposure | ✅ nur Loopback (ADR-008) |
| E | Agent-Runtimes | ✅ Hermes Agent + OpenCode (ADR-010) |
| F | Tool-Lizenz | ✅ privat, non-commercial (ADR-011) |
| G | SSD-Kapazität / Store-Pfad | ✅ `E:\AI\models`, ~1,5 TB frei (ADR-012) |

Keine blockierenden offenen Entscheidungen mehr für Phase 1. llama.cpp-Integration
+ CUDA-Build-Quelle: ✅ (ADR-014). Weiter offen: visuelle UI-Designrichtung
(siehe [TODO.md](TODO.md)).
