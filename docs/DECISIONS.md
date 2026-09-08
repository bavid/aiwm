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

**Status:** Entschieden — `core::link` umgesetzt in 2.3. `models` / `model_roles`
seit WP-2; `model_links` seit 2.3 live (Migration `0003` machte `runtime_id` zur
Soft-Ref).

**Entscheidung:** Eine kanonische Datei pro Modell im Store (`E:\AI\models\`).
`core::link::materialize` verknüpft sie pro Runtime nach `LinkStrategy`:
- **`Passthrough`** — die Runtime nimmt einen absoluten Pfad (llama.cpp). Nichts
  im Dateisystem, nur ein `model_links`-Eintrag als „nutzbar von".
- **`Junction`** — NTFS-Directory-Reparse-Point (`junction`-Crate, kein Admin,
  gleiche Volume) für Runtimes, die einen Ordnerbaum scannen (ComfyUI, LM Studio).
- **`Hardlink`** — Datei-Hardlink, nur gleiche Volume.
- **`Copy`** — echte Kopie; unvermeidbar für Ollamas content-addressed Store.

`strategy_for(runtime_id, format)` wählt: `llamacpp` → Passthrough, `ollama` →
Copy, sonst → Junction. Duplikate durch `Copy` weist der Dedup-Report (Phase 6)
aus.

**Konsequenz Phase 2:** nur `Passthrough` ist aktiv (llama.cpp). Junction/Copy
sind gebaut + getestet (inkl. echter Junction-Roundtrip unter Windows) und
stehen für Phase 3 bereit — kein spekulativer ungetesteter Code.

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

## ADR-015 — Chat-Capability: `/v1/chat/completions`-Streaming, `Auto` per Rolle, Ergebnis in der DB

**Status:** Entschieden — umgesetzt (Streaming/Auto 2.4a, Cancel 2.4b).

**Entscheidung:**
1. **Endpoint:** streamendes `POST /v1/chat/completions` (OpenAI-kompatibel), **nicht**
   `/completion`. llama-server wendet die im GGUF hinterlegte Chat-Vorlage an —
   kein manuelles Prompt-Templating pro Modellfamilie.
2. **`Auto`-Modellwahl:** ein Job ohne explizites Modell nimmt das beste Modell
   mit passender Rolle (`chat`) — Heuristik: zuletzt genutzt → meist genutzt →
   Name. Kein Benchmark/Quality-Score (Phase 6). Kein Modell mit Rolle ⇒
   Klartext-Fehler „import a .gguf first".
3. **Streaming-Transport zur UI:** die Antwort wird progressiv in `jobs.result`
   geschrieben (Flush ~200 ms); die UI pollt `GET /jobs/{id}` (wie alles andere
   im MVP). **Kein** WebSocket-Token-Stream — später als Verfeinerung.
4. **Job-Body-Ort:** `capability::chat` (neues Modul). Der `orchestrator` treibt
   Zustandsmaschine + Scheduler; jede Capability besitzt ihren `Running`-Body.
   Der Engine hält dafür den typisierten `Arc<LlamaCppAdapter>` (wie
   `scheduler` / `jobs` in `App`) — kein Downcast von `dyn RuntimeAdapter`.
5. **Cancel (2.4b):** per-Job `watch<bool>` im `JobEngine`, an Checkpoints
   geprüft; im Stream ein `select!` — Cancel dropt die `reqwest`-Antwort, was den
   Server ebenfalls stoppt. `Queued`/`Blocked` werden direkt auf `Cancelled`
   gesetzt. Kein Abbruch mitten im Modell-Load (das wäre ein größerer Umbau am
   `RuntimeSupervisor`) — nur an den Schritt-Grenzen.

**Alternativen:**
- *`/completion` mit rohem Prompt:* ohne Chat-Vorlage schlechte Antworten bei
  Instruct-Modellen; `--jinja` + manuelles Templating ist fragil. Verworfen.
- *WS/SSE-Stream an die UI:* echtes Token-für-Token, aber neue Transport-
  Infrastruktur. Die Chat-UI (2.5) pollt `GET /jobs/{id}` alle 350 ms und zeigt
  `job.result` wachsen — reicht für den MVP, ist persistent (Reload-fest) und
  nutzt denselben Weg wie alles andere. Verfeinerung notiert ([TODO.md](TODO.md)).
- *`generate_text` auf dem `RuntimeAdapter`-Trait:* verfrühte Abstraktion für
  genau eine Text-Runtime. Kommt, wenn eine zweite existiert.

**Konsequenzen:**
- (+) Erste echte Capability, end-to-end getestet (Fixture + echtes SmolLM2).
- (−) Ein langer Chat-Job blockiert die Job-Schleife (Single-Slot-Prämisse,
  ADR-003 — akzeptiert). Token-Zähler kommt aus `timings.predicted_n`
  (llama.cpp-Erweiterung), Fallback `usage.completion_tokens`.

---

## ADR-016 — VRAM-Fit-Schätzung vor dem Modell-Load (`core::compat`)

**Status:** Entschieden — umgesetzt (2.6).

**Kontext:** Bis 2.5 plante der Scheduler gegen `vram_estimate_mb = Dateigröße +
1 GB`. Das ignoriert den KV-Cache (wächst linear mit Kontext × Modell-Tiefe/
-Breite) — und `llama-server` allokiert ohne `-c` den vollen trainierten Kontext
(oft 128K), dessen KV-Cache allein die 16-GB-Karte sprengt. Ergebnis: der teure
Modell-Load lief los und OOM-te erst dort, ohne Klartext.

**Entscheidung:**
1. **Eigene Schätzung, drei Teile:** `weights` (= Datei) + `kv_cache` + `overhead`.
   - **KV-Cache** aus GGUF-Arch-Metadaten (`block_count`, `embedding_length`,
     `attention.head_count[_kv]` — in 2.6 zu `models`-Spalten + Parser ergänzt):
     `4 · n_layers · (n_embd/n_heads) · n_kv_heads · ctx` Bytes (fp16, K+V; GQA
     über `n_kv_heads`). Fehlen die Dims → grobe Reserve `160 MB / 1K ctx`,
     sichtbar als `kv_is_rough`.
   - **Overhead** = **flache 650 MB** (CUDA-Context, cuBLAS, Compute-Graph). Keine
     Kalibrierung gegen echte Messungen — das ist eine Phase-6-Aufgabe
     ([BENCHMARKS.md](BENCHMARKS.md)).
2. **Effektiver Kontext:** `min(ctx_max, 8192)`. Chat braucht selten den vollen
   trainierten Kontext; die Schätzung *und* das `-c` von `llama-server` nutzen
   denselben Wert, damit Schätzung = Allokation. Settings-UI überschreibt (2.7).
3. **Bewusst konservativ:** lieber über-reservieren und einreihen als laden und
   abstürzen.
4. **`blocked` ist ein Ruhezustand:** passt es nicht, geht der Job mit
   Klartext-`error_text` auf `blocked` — **kein** Modell-Load. Er bleibt
   „runnable", aber die Schleife backt 3 s zurück und `try_drive` prüft einen
   bereits blockierten Job still (kein Zustands-/Event-Churn). Weiter geht es erst,
   wenn echt VRAM frei wird (anderer Job evicted, Runtime-Stop, Session-Ende).

**Alternativen:**
- *Fremd-Crate / llama.cpp fragen:* llama.cpp hat kein Vorab-„passt das?"-API
  ohne Load. Ein Fremd-Parser für die Schätzung wäre mehr Abhängigkeit für eine
  Heuristik, die ohnehin kalibriert werden muss.
- *Einfach `llama-server` laden lassen und den OOM abfangen:* der Load kostet
  Sekunden + reale VRAM-Allokation + evtl. Treiber-Reset; der Klartext-Grund fehlt.
- *Kontext = `ctx_max`:* korrekt für „max", aber sprengt bei 128K-Modellen die
  Karte für einen normalen Chat. Deckelung ist die pragmatische Wahl.

**Konsequenzen:**
- (+) Der Block-Fall ist *vor* dem teuren Load sichtbar, mit Aufschlüsselung
  („weights 8.4 GB + KV cache 2.7 GB @ 8K ctx + 0.6 GB overhead").
- (+) Zweites-Modell-Wechsel plant jetzt gegen realistische Zahlen.
- (−) Die 650-MB-Konstante + die grobe KV-Reserve sind geschätzt, nicht gemessen —
  bis zur Phase-6-Kalibrierung kann die Schätzung daneben liegen (bewusst eher zu
  hoch).
- (−) Blockierte Jobs werden noch nicht automatisch neu eingereiht, wenn eine
  Runtime stoppt / eine Session endet — nur beim Evict durch einen anderen Job
  oder manuell ([TODO.md](TODO.md)).

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
