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

**Status:** Entschieden — umgesetzt (Streaming/Auto 2.4a, Cancel 2.4b); die
`Auto`-Heuristik in Punkt 2 wurde in **Slice 6.6** benchmark-gestützt (siehe
„Zusatz" unten).

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

**Zusatz — Slice 6.6 (benchmark-gestützte `Auto`-Auswahl):** Punkt 2 wird um
`core::select::pick_for_role(db, role, vram_budget_mb, pref)` erweitert (nutzt
`ModelRepo::for_role_with_benchmark` → jeder Kandidat mit seinem neuesten
`benchmarks`-Eintrag aus 6.5). Reihenfolge: **Fit zuerst** (ein Modell, dessen
`vram_estimate_mb` ≤ Budget, schlägt eins, das nicht passt), **dann Score**
(preference-gewichtet aus den gemessenen `gen_tps` / `stability` + Params als
Heft-Proxy — **keine Qualitäts-Achse**, ADR-024), **dann Nutzung**
(`last_used_at` / `use_count` / Name). **Ohne Benchmark-Daten** ist es exakt die
alte Regel. `[models].auto_preference` = `balanced` (Default) / `fast` (mehr
Speed-Gewicht) / `quality` (mehr Heft-Gewicht). Betrifft `chat`, `coding`,
`base_diffusion`, `base_video`; `vae` / `text_encoder` bleiben auf der schlichten
`ModelRepo::pick_for_role`. Startup-Config (Restart nötig), verdrahtet über
`JobEngine::with_auto_preference` + `AgentSessions::with_auto_preference`;
`Scheduler` + `CodingRuntime` bekamen `budget_mb()`.

---

## ADR-016 — VRAM-Fit-Schätzung vor dem Modell-Load (`core::compat`)

**Status:** Entschieden — umgesetzt (2.6); verlängert in 6.3 (`FitVerdict` +
`.safetensors`-Header, siehe „Zusatz" unten).

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

**Zusatz — Slice 6.3 (Kompatibilitäts-Engine v2):**
- **`compat::verdict(dims, ctx, vram_budget_mb, free_ram_mb) -> FitVerdict`**
  wickelt `estimate()` in eine Klartext-Ampel `Green | Yellow{reason} |
  Red{reason} | Unknown` — für **Discovery** (6.2) und den **Upgrade-Check**
  (6.7): „würde dieses Modell hier laufen?" **vor** dem Download. Green < 85 %
  Budget; Yellow wenn eng; über Budget → Yellow wenn der freie System-RAM den
  Überhang + 4 GB OS-Reserve trägt (Offload, langsam), sonst Red.
- **`.safetensors`-Header-Reader** (`core::model::safetensors`, der seit 3.3
  vertagte TODO): bounded Reader → Param-Count + dominante Precision +
  `__metadata__`; im Import verdrahtet, ein nicht lesbarer Header failt nicht.
- **`media_headroom_mb(family)`** ersetzt die `+2,5 GB`-Namens-Faustregel:
  Wan 6 GB · LTX/Flux/SD3 4 GB · SDXL 2 GB · sonst 2,5 GB.
- **Der `HybridScheduler` bleibt die Laufzeit-Instanz** für „passt es *jetzt*"
  (Block/Allow + `describe()`-Grund). `verdict` ist advisory, es fasst die
  Scheduler-Entscheidung nicht an. Alle Konstanten +
  Kalibrierungs-Plan: [HARDWARE.md](HARDWARE.md).

---

## ADR-017 — Settings: `config.toml` bleibt die Wahrheit, Offline lebt

**Status:** Entschieden — umgesetzt (2.7).

**Kontext:** Bis 2.6 war `config.toml` reine Startkonfiguration ohne UI; die
`settings`-Tabelle ist App-State (Schema-Version, First-Run). Die Settings-UI
braucht einen Ort, editierbare Felder und eine Aussage, wann eine Änderung greift.

**Entscheidung:**
1. **Eine Quelle:** `config.toml` bleibt die Wahrheit für Startkonfiguration. Die
   Settings-UI liest und schreibt genau diese Datei (`GET`/`PUT /config`), **nicht**
   die `settings`-Tabelle. `AIWM_*`-Env-Overrides bleiben Vorrang beim Start, aber
   der Settings-Lese-Pfad (`Config::read_from`) zeigt die Datei ohne Overlay — ein
   Override verschluckt nie still einen gespeicherten Wert.
2. **Neustart-pflichtig ist der Normalfall, und die UI sagt es.** `store_path`,
   `vram_budget_mb`, `log_filter`, `core_api_port`, die `[llama]`- und (ab 3.7)
   die `[comfyui]`-Optionen werden beim Start gelesen; eine Änderung braucht
   einen App-Neustart. Jedes Feld trägt das Label („restart to apply" / „applies
   on the next model load"). `[comfyui].vram_mode` → `--<mode>vram`-Flag beim
   ComfyUI-Spawn.
3. **Ausnahme Offline:** `offline_mode` ist ein Sicherheits-Schalter (ADR-009) —
   ein neustart-pflichtiger Sicherheitsschalter ist schlechte UX. `App` hält ein
   `Arc<AtomicBool>` (`App::offline()`), aus der Config geseedet; die drei
   Ausgangs-Call-Sites lesen die Live-Zahl. `PUT /config` flippt sie sofort
   **und** persistiert.
4. **Theme ist reine UI:** `localStorage` + `data-theme` auf `<html>`, nie an den
   Core. „System" folgt `prefers-color-scheme`.
5. **`core_api_port` / `log_filter` bleiben Datei-only** — Power-User-Territorium,
   Port-Wechsel ist riskant (die App verbindet sich darauf), Log-Filter-Reload
   bräuchte einen `tracing`-`reload::Handle`.

**Alternativen:**
- *Alles in die `settings`-Tabelle:* dann zwei Quellen für Startkonfiguration,
  Migrations-Kopfschmerz, und die Datei (die der Nutzer auch von Hand editiert)
  wäre nicht mehr autoritativ. Verworfen.
- *Alles hot-reloadbar machen:* `Config` hinter `RwLock`, jeder `&App`-Handler
  müsste neu lesen, `HybridScheduler`/`LlamaCppAdapter` bräuchten Interior
  Mutability. Zu viel Umbau für Felder, die man einmal setzt. Nur Offline lohnt
  den `AtomicBool`.

**Konsequenzen:**
- (+) Ein Ort, ein Format; die Datei bleibt von Hand editierbar.
- (+) Offline schaltet ohne Neustart — der Schalter fühlt sich wie ein Schalter an.
- (−) Für Store-Pfad / VRAM-Budget / llama-Optionen muss der Nutzer die App neu
  starten. Für ein Desktop-Tool akzeptabel; die UI ist explizit.

---

## ADR-018 — ComfyUI-Integration: ein Server, viele Modelle, lazy + `/free`

**Status:** Entschieden — Adapter umgesetzt (3.1); Installer folgt in 3.2.

**Kontext:** ComfyUI ist die Bild-/Diffusion-Runtime (Phase 3). Anders als
`llama-server` (ein Prozess bedient genau ein Modell) ist ComfyUI **ein
langlebiger Server, der Modelle on-demand pro Workflow lädt und cacht**. Das
Muster von ADR-014 („ein `llama-server` pro residentem Modell, Swap = Neustart")
passt hier nicht.

**Entscheidung:**
1. **Ein überwachter Kindprozess + HTTP/WS-Client** (`ComfyUiAdapter`), wie bei
   llama-server. ComfyUI hört per Default auf `127.0.0.1` (ADR-008). Start-Flags:
   `--listen 127.0.0.1 --port <frei> --base-directory <d> --output-directory <o>
   --disable-auto-launch --dont-print-server`.
2. **Lazy Start, dauerhaft oben.** Der Server startet beim ersten `load_model`
   (nicht bei Registrierung — Python + torch-Import kostet 10–30 s) und bleibt
   bis zum App-Ende laufen. `unload_model` **stoppt ihn nicht** — es ruft nur
   `POST /free` (Modelle entladen). Ein Neustart wäre pro Modellwechsel zu teuer.
3. **`load_model`-Semantik:** Server sicherstellen + den einen VRAM-Slot (ADR-003)
   reservieren + das vorherige Modell via `/free` verdrängen. Der eigentliche
   Checkpoint wird von ComfyUI beim Workflow-Lauf geladen (`capability::image`,
   3.4) — der Adapter bucht nur den Platz.
4. **Health = `GET /system_stats`.** ComfyUI hat keinen `/health` und antwortet
   während des Boots nicht mit `503` (es lauscht schlicht noch nicht) — der
   Adapter trackt `Starting` selbst im `Server`-Slot.
5. **Attach-Fallback** (ADR-002) über `/system_stats`. `stop()` für einen
   expliziten Runtime-Neustart; Drop tut dasselbe (Job Object).
6. **Cancel = `POST /interrupt`** (kommt in 3.4, analog `stream.abort()` beim Chat).
7. **VRAM-Accounting** fürs Scheduling: die *deklarierte* Zahl aus `load_model`
   (wie bei llama.cpp). `/system_stats` liefert die reale VRAM nur für
   Diagnostics — sie enthält auch Fremdprozesse.

**Alternativen:**
- *Ein `llama-server`-artiges „ein Prozess pro Modell"*: ComfyUI unterstützt das
  nicht und der Prozess ist zu schwer zum ständigen Neustarten. Verworfen.
- *Server eager beim Start hochfahren*: zahlt den Python-Boot auch, wenn der
  Nutzer nie ein Bild generiert. Lazy ist besser.
- *`comfy-cli` als Laufzeit-Tool*: eigenes Pip-Tool, eigener Env-Zustand — wir
  kontrollieren die venv selbst (3.2), wie bei llama.cpp die Binärdatei.

**Installer (3.2) — eigene `uv`-getriebene Pipeline statt `comfy-cli`.**
- **Weg:** `uv` als einzelne verifizierte Static-Binary bootstrappen →
  ComfyUI-Quelle am gepinnten Tag (verifiziertes GitHub-`.zip`) →
  `city96/ComfyUI-GGUF` an einem gepinnten Commit (verifiziertes `.zip` →
  `custom_nodes/`) → `uv venv --python 3.13` (lädt Python) → `uv pip install
  torch … --index-url cu130` → `uv pip install -r requirements.txt` → `uv pip
  install -r <node>/requirements.txt`. Alles unter
  `<local_root>/runtimes/comfyui/`, „Repair" = löschen. Die `uv`-Subprozess-
  Schritte hinter einem `CmdRunner`-Trait, damit die Orchestrierung ohne echtes
  Python unit-testbar ist.
- **Torch:** kein exakter Versions-Pin. ComfyUIs eigenes `requirements.txt` pinnt
  torch auch nicht; die kuratierte Version ist „ComfyUI-Tag + cu130-Index +
  Python 3.13". `cu130` = ComfyUIs aktuelle Empfehlung für RTX 20+; braucht einen
  neueren NVIDIA-Treiber (Treiber-Kompatibilität wird bewiesen, wenn 3.4 ein Bild
  rendert — der `#[ignore]`-Smoke prüft nur, dass `import torch` in der venv geht).
- **`comfy-cli` verworfen als Laufzeit-*und* Installer-Tool:** es ist ein
  eigenes Pip-Paket mit eigenem Env-Zustand, eigenem Workspace-Layout und einer
  Wizard-Oberfläche. Für ein eingebettetes Control-Plane ist es eine zusätzliche
  bewegliche Schicht ohne Gewinn — die ~6 `uv`-Schritte selbst zu besitzen ist
  deterministischer und passt zu ADR-014 (llama.cpp: „wir kontrollieren den
  Install"). Als *Referenz* für die Schritte ist `comfy-cli` nützlich.
- **Source-Archiv-SHA:** GitHub publiziert für Source-Archive keinen Digest. Wir
  pinnen die selbst berechnete SHA-256; eine Regeneration auf GitHubs Seite
  surft als `SHA-256 mismatch` auf → von Hand prüfen, dann bumpen.

**Konsequenzen:**
- (+) Modellwechsel = ein `/free` + der nächste Workflow lädt neu — keine
  Server-Neustart-Latenz.
- (+) Derselbe Supervisor/Attach/Health-Baukasten wie Phase 2; dieselbe
  Download-/Verify-/Extract-Maschinerie (`runtime::download`) wie der llama.cpp-
  Installer.
- (−) Der ComfyUI-Prozess belegt Basis-RAM (~1–2 GB), solange die App läuft, auch
  im Leerlauf nach dem ersten Bild. Akzeptiert; ein „Runtime stoppen" kommt in 3.7.
- (−) Bis `capability::image` (3.4) reserviert `load_model` nur den Slot — die
  Verdrängung eines echten Checkpoints ist erst dann sichtbar.
- (−) Ohne torch-Pin kann ein künftiger torch-Release ComfyUI brechen — dann
  Pin nachziehen / ComfyUI-Tag bumpen.

---

## ADR-019 — Bild-Modelle: getypter Store + ComfyUI via `extra_model_paths.yaml` (statt Junction)

**Status:** Entschieden — umgesetzt (3.3). **Weicht bewusst von PHASE_3_PLAN §D ab.**

**Kontext:** Der Plan sah vor, die Store-Ordner per NTFS-Junction in ComfyUIs
`models/` zu spiegeln (die 2.3-`core::link`-Maschinerie). Zwei Probleme:
1. **Junctions können keine Volumes überspannen.** Der Store liegt auf `E:`
   (`E:\AI\models`, ADR-012), die ComfyUI-Installation unter `%LOCALAPPDATA%` =
   `C:`. Ein Junction von `C:` nach `E:` geht nicht; Symlinks bräuchten Admin /
   Developer Mode.
2. ComfyUI hat für genau diesen Fall ein **erstklassiges, dokumentiertes
   Feature**: `extra_model_paths.yaml` (per `--extra-model-paths-config`).

**Entscheidung:**
1. **Getypter Store:** `.gguf`-Chat-Modelle → `<store>/llm/<slug>/`. Bild-Modelle
   (`.safetensors`, und `.gguf` für quantisiertes Flux) → flache getypte Ordner
   `<store>/image/{checkpoints,diffusion_models,vae,loras,text_encoders}/`, 1:1
   auf ComfyUIs `folder_paths`-Namen.
2. **`ModelKind`** (`core::model`) — `Chat` / `Checkpoint` / `DiffusionModel` /
   `Vae` / `Lora` / `TextEncoder`. `import_model` nimmt einen optionalen
   `model_type`-Hint (sonst aus der Endung abgeleitet: `.gguf`→`chat`,
   `.safetensors`→`checkpoint`), validiert ihn gegen die Endung, routet in den
   Store-Ordner. Pickle-Formate (`.ckpt`/`.bin`/`.pt`) → Klartext-Ablehnung
   („convert to .safetensors first"). `.safetensors` wird **nicht** geparst
   (kein sicherer bounded Reader wie für GGUF; Header-Inspektion später).
3. **ComfyUI-Zugriff:** Der Adapter schreibt vor jedem Server-Start
   `<comfyui-data>/aiwm-model-paths.yaml` (`base_path: <store>/image`, alle fünf
   Ordner-Mappings) und startet mit `--extra-model-paths-config <die Datei>`.
   Kein Filesystem-Link. `model_links`-Strategie: neue Variante
   `LinkStrategy::ExtraPath` (`materialize` = No-Op wie `Passthrough`).
4. **VRAM-Schätzung** für Bild-Modelle: `Dateigröße + 2 GB` Headroom (grob;
   echte Familien-Zahlen mit `capability::image`, 3.4).

**Alternativen:**
- *Junction (Plan §D):* scheitert am Volume-Wechsel. Verworfen.
- *Store auf `C:` unter die ComfyUI-Installation legen:* widerspricht ADR-012
  (1,5 TB auf `E:`) und koppelt Store an Runtime-Version.
- *Symlink:* braucht Admin / Developer Mode — ADR-002 will „kein
  Sonderrechte-Setup".

**Konsequenzen:**
- (+) Funktioniert über Volumes, ohne Admin, ohne Junction-Reconciliation bei
  jedem Neustart. ComfyUIs eigener, getesteter Weg.
- (+) `core::link` bleibt für Runtimes *ohne* Config-Pfad-Feature (LM Studio) —
  `strategy_for` liefert dort weiter `Junction`.
- (−) Wer in ComfyUIs `models/`-Ordner schaut, sieht die Dateien nicht (nur die
  Config kennt sie). Kosmetisch.
- (−) Der TODO „echtes Junctionen gegen einen realen Konsumenten" bleibt offen —
  ComfyUI war nicht der richtige Konsument dafür.

---

## ADR-020 — Zweites Video-Template: LTX-Video 0.9.5 (2B), nicht LTX-2

**Status:** Entschieden — umgesetzt (4.4). **Weicht bewusst von PHASE_4_PLAN
§A / Scheibe 4.4 ab** („LTX-2 / LTX 2.3 GGUF über ComfyUI-GGUF").

**Kontext:** Der Plan nannte LTX-2 als zweites Template, GGUF über den schon
installierten `city96/ComfyUI-GGUF`-Node. Die Recherche (2026-09) zeigt drei
Blocker:
1. **LTX-2 / 2.3 ist groß.** 22B fp8 ≈ 22 GB, passt nicht in 16 GB. Nur GGUF
   Q4 (~12 GB) käme in Frage — und dann sehr knapp neben dem gemma-3-12B-Encoder.
2. **LTX-2-GGUF ist nicht im offiziellen `ComfyUI-GGUF`.** Der Loader braucht
   manuelle `loader.py`/`nodes.py`-Patches (unveröffentlichter Commit) **plus**
   `ComfyUI-KJNodes`. Das widerspricht ADR-002 („eine kuratierte, getestete
   Version, ein Custom Node").
3. **LTX-2.3 nativ** nutzt einen `.safetensors`-fp8-Checkpoint (kein GGUF),
   passt aber wegen (1) trotzdem nicht.

**Entscheidung:** Das zweite Video-Template ist **LTX-Video 0.9.5 (2B)** als
einzelnes `.safetensors` (`ltx-video-2b-v0.9.5.safetensors`, 6,3 GB, Model +
VAE gebündelt) + ein separater `t5xxl`-Encoder (`CLIPLoader type="ltxv"` — der
fp8-T5 aus dem Katalog reicht). **Alles Core-ComfyUI-Nodes**
(`CheckpointLoaderSimple`, `LTXVConditioning`, `EmptyLTXVLatentVideo` /
`LTXVImgToVideo`, `LTXVScheduler` → `SamplerCustom`, `CreateVideo`,
`SaveVideo`) — **kein neuer Custom Node**. `pipeline::VideoRecipe::for_family`
wählt: `family == "ltx"` → `ltx_video`, sonst `wan_ti2v`. `capability::video`
löst die Begleiter pro Rezept auf (Wan: umt5 + Wan-VAE; LTX: nur T5).

Das ist exakt das Muster SDXL→Flux aus Phase 3: ein zweites festes Template,
Familien-Dispatch, gleiche `VideoInputs`.

**Alternativen:**
- *LTX-2 19B GGUF (Plan):* neuer/gepatchter GGUF-Node + KJNodes → eigener ADR,
  verworfen für den MVP.
- *LTX-13B 0.9.8:* 28 GB / fp8 15 GB — zu groß für komfortable 16 GB.
- *Kein zweites Video-Template:* dann bleibt die `VideoRecipe`-Abstraktion
  ungetestet — der ganze Sinn von 4.4.

**Konsequenzen:**
- (+) Passt bequem in 16 GB, schneller als Wan 5B, kein Setup-Aufwand.
- (+) `VideoRecipe` + Familien-Dispatch bewiesen (2 Templates).
- (−) LTX-Video 0.9.x wants **`(frames-1) % 8 == 0`**; unser `4k+1`-Snap
  erfüllt das nicht immer — die LTX-Nodes runden intern ab (weniger Frames als
  gewünscht). Für 4.0 zu kalibrieren; TODO.
- (−) LTX-2 (Audio, höhere Qualität) bleibt außen vor bis der GGUF-Node-Weg
  stabil ist. Post-MVP.
- (−) Wie alles in Phase 4 (außer 4.0): gegen `aiwm-fake-comfy` + recherchierte
  Node-Namen gebaut, nicht an der echten ComfyUI verprobt.

---

## ADR-021 — Agent-Adapter: OpenCode zuerst, `llama-server --jinja`, Hermes ohne WSL-Zwang

**Status:** Entschieden — Slice 5.0 (Voraussetzungen verprobt). **Verfeinert
ADR-010** (Reihenfolge, Windows-Shell).

**Kontext:** ADR-010 nannte Hermes Agent als **Adapter 1**, OpenCode als 2, und
markierte Hermes' `bash -l`-Abhängigkeit als Windows-Risiko. Slice 5.0 hat beide
auf der echten Maschine verprobt (`node` 24, `uv` 0.12, Git-Bash, `wsl` da).

**Befunde:**
1. **OpenCode** (`opencode-ai` 1.18.30) = **eine self-contained `bin/opencode.exe`**
   (npm-Paket, native Windows). `opencode serve` läuft; `GET /doc` = OpenAPI 3.1.
   Config **komplett per `OPENCODE_CONFIG_CONTENT`-Env erzwingbar** (`GET /config`
   spiegelt sie). `enabled_providers: ["local"]` + `disabled_providers:
   ["opencode", …]` → nur unser lokaler Provider bleibt. Der Approval-Zyklus
   verprobt: `permission.asked`-Event (mit `always`-Muster-Vorschlag) → `GET
   /permission` → `POST /permission/{id}/reply`. CWD = Projekt/Workspace.
2. **Hermes Agent** (`hermes-agent` 0.19.0) = `uv pip install`, **~120 Python-
   Deps + `hermes postinstall` zieht `node` / Browser / `ripgrep` / `ffmpeg`** —
   deutlich schwerer. `hermes serve` (headless JSON-RPC/WS, `--skip-build`),
   `hermes -z` single-shot, `hermes acp`. **`bash -l` entschärft:** 0.19 hat
   native Windows- + Git-Bash-Pfad-Behandlung (MSYS); **kein WSL2-Zwang** — die
   mitgelieferte Git-Bash reicht.
3. **`llama-server --jinja`** emittiert Standard-OpenAI-`tool_calls` (aus dem im
   GGUF eingebetteten Chat-Template), nativ für Qwen 2.5 (Coder), Hermes 2/3,
   Llama 3.x, Functionary, Mistral Nemo. OpenCodes `@ai-sdk/openai-compatible`-
   Runtime parst genau dieses Format (Stub-Test bewiesen). Ohne `--jinja` kommen
   die Tool-Delimiter als Klartext durch.

**Entscheidung:**
- **OpenCode ist Adapter 1** (Slice 5.1) — leichter, Config erzwingbar, Approval-
  API sauber. **Hermes ist Adapter 2** (Slice 5.4). Dreht die ADR-010/ROADMAP-
  Reihenfolge um; „beide Adapter" bleibt.
- Der `LlamaCppAdapter` bekommt in 5.1 ein **`--jinja`-Flag** (+ optional
  `--chat-template`), wenn er ein Agent-/`coding`-Modell serviert.
- Erzwungene OpenCode-Config: custom Provider → `http://127.0.0.1:<llama-port>/v1`,
  `enabled_providers: ["local"]` + `disabled_providers: ["opencode", …]`,
  `permission: {bash: ask, edit: ask, webfetch: deny}`, `cwd` = Workspace.
- Hermes-Adapter mit gemanagtem `HERMES_HOME` unter `<data>/agents/hermes/` +
  erzwungener `config.yaml`; Git-Bash-Pfad aus der Umgebung.

**Konsequenzen:**
- (+) Erster funktionierender Agent kommt ohne das Hermes-Gewicht.
- (+) Tool-Calling-Risiko (ADR-010 offen) auf „echten Coding-Modell-Lauf" (5.1-
  Smoke) reduziert.
- (−) Hermes' 120 Deps + node/Browser/ripgrep/ffmpeg machen den Installer
  chunkig (wie der ComfyUI-Installer) — evtl. wird Hermes „Advanced/optional".
- (−) OpenCodes API ist groß (~180 Ops) und `v1`/`v2`-gemischt — wir binden nur
  ~8 Endpunkte, hand-geschrieben (kein Generated Client).
- (−) Beides nur Scratchpad-verprobt gegen Stubs; der echte agentische Loop mit
  einem lokalen Coding-Modell kommt in 5.1.

---

## ADR-022 — Model-Discovery: `core::registry`, Hugging Face Hub als primäre Quelle

**Status:** Entschieden — Slice 6.0 (Registry-Spike, echte API-Calls 2026-09);
in Slice 6.1 als `core::registry` umgesetzt (Trait + `HuggingFaceSource` +
`Registry`-Cache-Wrapper, gegen `aiwm-fake-hfhub` + einen `#[ignore]`-Live-Test).

**Kontext:** Phase 6 braucht Online-Modell-Suche. Der Brief (10.6/10.17) will
„One-Click" für beliebige Quellen; ANALYSIS rahmt das auf kuratierte Quellen +
assistierten Import (ADR bestätigt das). Frage: welche Quelle(n), welche API,
reicht anonym, wie sieht der Offline-Fallback aus.

**Befunde (Live-Probes gegen die echte API):**
1. **HF-Hub-API ist tragfähig und anonym nutzbar.**
   `GET /api/models?search=&filter=&pipeline_tag=&library=&author=&sort=&direction=-1&limit=&expand[]=…`.
   **`expand[]` funktioniert auch auf dem Listen-Endpoint** — ein Call liefert
   pro Treffer `gguf` (`total` = Param-Count, `architecture`, `context_length`,
   `chat_template`), `safetensors` (`parameters: { <DTYPE>: n }` → Precision +
   Param-Count **ohne Download**), `gated`, `lastModified`, `createdAt`,
   `downloadsAllTime`, `trendingScore`, `cardData`. `sort` u. a. `downloads`,
   `likes`, `likes7d`, `trendingScore`, `createdAt`, `lastModified`.
2. **Verify-SHA-256 steht vor dem Download fest.**
   `GET /api/models/{id}/tree/{rev}?recursive=true` → pro Datei
   `{ path, size, oid (git-sha1), lfs: { oid: "<sha256>", size }, xetHash }`.
   **`lfs.oid` ist die SHA-256** (auch auf Xet-Repos vorhanden); `xetHash` ist
   ein anderer Hash (Xet-Content-Addressing) — **nicht** verwenden. Split-GGUFs
   (`…-00001-of-00003.gguf`) als Set behandeln.
3. **Gated-Repos sind durchsuch- und inspizierbar** (`gated: "manual"|"auto"`);
   nur der Datei-Download (`/resolve/`) braucht akzeptierte Lizenz + Token. Die
   Discovery zeigt sie, der Download-Manager (6.4) muss `gated` erkennen und
   „Lizenz auf HF akzeptieren + Token setzen" sagen statt mitten im Download zu
   scheitern.
4. **Rate-Limits reichen locker.** `RateLimit: "api";r=499;t=98` /
   `RateLimit-Policy: "fixed window";"api";q=500;w=300` — **500 API-Calls / 5 min
   pro IP anonym**, 1 000 mit Free-Token. Ein Upgrade-Check ist ~1–3 Calls.
   `429` mit `RateLimit`-Header → Backoff auf `t`. **Kein `Cache-Control`**, aber
   schwacher **`ETag`** → `If-None-Match` beim Cache-Refresh (`304`).
5. **Ollama-Library hat kein Such-API.** `registry.ollama.ai/v2/library/<m>/manifests/<tag>`
   (OCI) liefert für einen **bekannten** Namen `layers[].{digest: "sha256:…", size}`
   — aber keine Suche/Liste. `ollama.com/search?format=json` ignoriert `format`
   und liefert HTML; Drittquellen (`ollamadb.dev`) sind brüchig / z. T. offline.
6. **Content-Farm-Spam ist real (R9).** `sort=trendingScore`/`likes7d` ist voll
   halluzinierter Namen („Qwen3.8-27B-…", aufgeblasene Downloads). **`filter=base_model:<id>`
   funktioniert** und ist der verlässliche Weg, echte Quant-Re-Uploads +
   Abkömmlinge eines bekannten Basismodells zu finden.

**Entscheidung:**
- **`core::registry::ModelSource`-Trait** (`search`, `details`) mit
  **`HuggingFaceSource` als einziger MVP-Implementierung**. Ollama-Library als
  späterer best-effort-Adapter hinter derselben Schnittstelle (HTML-Scrape,
  niedrige Priorität).
- **Nativer Rust-`reqwest`-Client, kein Python-Sidecar.** Der Plan-Entwurf
  (ARCHITECTURE §3.3) nannte `huggingface_hub` im Sidecar — verworfen: die API
  ist anonym, sind einfache GETs, und `lfs.oid` liefert den Verify-Hash direkt.
  Ein Sidecar-Roundtrip für drei GETs lohnt nicht. `runtime::download`
  (`download_verified`, `hex`) wird ohnehin nativ wiederverwendet.
- **Anonym per Default**; optionales `HF_TOKEN` (`[models]`-Config / Settings)
  nur für Gated-Repos / höhere Limits. **Nie Pflicht, nie im Backup-Export.**
- **SHA-256 = `lfs.oid`**, immer aus `/tree?recursive=true` geholt und an
  `download_verified` gereicht. `xetHash` wird ignoriert.
- **Offline-Fallback:** TTL-JSON-Cache unter `<data>/cache/registry/` (search-
  Queries + `details`), `offline_mode` → harte Ablehnung mit Klartext, sonst
  Cache + „stale seit …"-Marker. `ETag` beim Refresh.
- **Remote-Quant-Erkennung aus dem Dateinamen** (`q4_k_m`, `q8_0`, `fp16`,
  `bf16`, `iq4_xs`, …) — die HF-`gguf`-Metadaten tragen `general.file_type`
  **nicht**. Lokal bleibt es `gguf::ftype_name` (schon da).

**Konsequenzen:**
- (+) Eine Quelle, offizielle API, kein Python, Verify-Hash gratis.
- (+) `base_model:`-Filter + `expand[]` machen den Upgrade-Check (6.7) mit ~1–3
  Calls möglich.
- (−) Bild-/Video-Repos taggen `pipeline_tag`/`library_name` oft `null` —
  Discovery dort schwächer, muss auf `tags` + Familien-Namenssuche setzen.
- (−) Spam-Filterung nötig: Autor-Allowlist / `base_model`-Lineage gewichten,
  nicht nackte Popularität.
- (−) Ollama-Dedup (R4) bleibt Anzeige-only (Manifest nicht junction-bar).

---

## ADR-023 — Download-Manager: eine Queue, Range-Resume, Verify → `import_model`

**Status:** Entschieden — Slice 6.4 (**6.4a** `core::download` Kern, `f61af65`;
**6.4b** API / Tauri / „Download & import"-Knopf + Downloads-Liste, dieser Commit).

**Kontext:** Discovery (6.1/6.2) zeigt Remote-Modelle mit Datei-Liste, SHA-256
(`lfs.oid`) und Fit-Verdikt. Der Brief will „One-Click" — der Nutzer soll nicht
den Link kopieren, im Browser laden und den Pfad von Hand in den Import tippen.
Frage: eigener Job-Typ oder eigenes Subsystem, parallele Downloads oder nicht,
wie Fortschritt melden, wie mit Abbruch / Netzabriss / falschem Hash umgehen.

**Entscheidung:**
- **Eigenes Subsystem `core::download::DownloadManager`, kein `job`-Typ.** Ein
  Download belegt kein VRAM und darf nicht um den Scheduler-Modell-Slot
  konkurrieren; er hat einen anderen Lebenszyklus (resumebar, tag­elang pausierbar).
  Eigene Tabelle `downloads` (Migration `0006`), eigener Worker, **einmal** von
  `api::spawn` gestartet (neben der Job-Schleife), `Drop` bricht ihn ab.
- **Eine Queue, ein aktiver Slot.** `next_actionable()` nimmt genau einen
  `queued`/`running`-Eintrag (recovertes `running` zuerst, sonst `created_at ASC`).
  Kein Parallel-Download: eine Leitung, ein Fortschrittsbalken, keine
  Bandbreiten-Teilung; die Modelle sind groß (GB), seriell ist ehrlicher.
- **Staging → `import_model`.** Der Stream landet in
  `<local_root>/.downloads/<id>/<filename>` (nicht im Modell-Store — halb geladene
  Dateien haben da nichts verloren). Nach Verify übernimmt der bestehende
  `import_model`-Pfad (verschiebt in den Store, GGUF/`.safetensors`-Header,
  Rollen, `media_*`). Der Download-Manager dupliziert **nichts** vom Import.
- **HTTP-Range-Resume.** `transfer()` liest den On-Disk-Offset, schickt
  `Range: bytes=<offset>-` wenn > 0 → `206` anhängen · `200` neu · `416` schon
  komplett. Ein sauberes Ende **unter** der erwarteten Größe **oder** ein
  Stream-Fehler = Transport-Fehler → Retry (mit Resume) bis `MAX_RETRIES = 5`,
  dann `failed`. Die Zeile wird alle 400 ms neu gelesen → Pause / Cancel greift
  mitten im Stream.
- **Verify = voller Re-Hash** (blocking, `spawn_blocking`) gegen die SHA-256 aus
  6.1 + Größen-Check. Mismatch → Datei löschen + `failed` (kein Halb-Import).
  Ohne erwarteten Hash (`sha256: None`) wird nur die Größe geprüft.
- **Fortschritt ist ein gepolltes Feld, kein Event-Stream.** Der Plan-Entwurf
  nannte „Fortschritts-Events (`job_events`-Muster)" — verworfen: `bytes_done` /
  `state` auf der Zeile, alle 400 ms geflusht, die UI pollt `GET /downloads`
  (1,5 s). Ein Event-Kanal (SSE / Tauri-Event) für einen Zahlenwert lohnt nicht;
  `job_events` bleibt für die Job-Transkripte.
- **Recovery → `queued`, nicht `failed`.** Beim Start werden `running` + `verifying`
  auf `queued` zurückgesetzt (der Worker nimmt sie per Range wieder auf); `paused`
  bleibt `paused`. Jobs recovern zu `failed` — ein Download ist idempotent
  fortsetzbar, ein Job nicht.
- **`offline_mode` sperrt `enqueue` + `resume`** (ADR-009). Laufende Downloads
  laufen weiter (der Switch ist für neue ausgehende Calls).
- **Nicht one-click im MVP:** Split-GGUF-Sets (`…-00001-of-00003`) und Gated-Repos
  — der Knopf ist in `Discover.tsx` deaktiviert, mit Hinweis („Lizenz auf HF
  akzeptieren"). Multi-Datei-Sets + Token-Fluss kommen später (6.9).

**Konsequenzen:**
- (+) One-Click von der Discovery-Karte bis in den Store; Verify gratis aus 6.1.
- (+) Netzabriss ist unkritisch — Resume aus der Teil-Datei, über Neustart hinweg.
- (+) Kein Scheduler-Eingriff, kein VRAM-Konflikt; der Worker ist ein schlanker
  `tokio`-Loop mit `Notify`-Wakeup + 5-s-Idle-Poll.
- (−) Seriell: zwei Modelle laden dauert nacheinander. Bewusst.
- (−) **Speicherplanung fehlt noch** (freier Platz auf dem Store-Volume vs.
  Download-Größe, Klartext-Warnung *vor* dem Enqueue) — verschoben in die
  „Storage"-Ansicht (6.8). Aktuell scheitert ein zu großer Download erst beim
  Schreiben / Import.
- (−) Verify ist ein voller Re-Hash nach dem Download (I/O-Kosten ~einmal die
  Dateigröße lesen) — kein Streaming-Hash während des Transfers, weil Resume den
  Zwischenstand nicht mitführt.

---

## ADR-024 — Quality-Score: keine gebündelte externe Benchmark-Quelle im MVP

**Status:** Entschieden — Slice 6.0. **Verfeinert** die BENCHMARKS.md-Skizze und
R12.

**Kontext:** Der Brief (15/32/33) will benchmark-gestützte Auto-Auswahl + einen
„Overall Score". R12 hält fest: **kein billiger, lokaler, objektiver Qualitäts-
Benchmark**. Der Plan nahm an, extern gepflegte Leaderboards (SWE-bench o. Ä.)
online zu ziehen. Slice 6.0 hat die Quellenlage geprüft.

**Befunde:**
1. **Das HF Open LLM Leaderboard ist eingestellt** (v1 → Juni 2024 archiviert,
   v2 → **März 2025 abgeschaltet**). Kein kanonischer Nachfolger — HF setzt auf
   dezentrale „Community Evals" (`eval.yaml` pro Repo) + 200+ Community-
   Leaderboards.
2. **Verbleibende maschinenlesbare Quellen sind heterogen und cloud-lastig.**
   Das Aider-Polyglot-Leaderboard (`Aider-AI/aider` Repo, `polyglot_leaderboard.yml`,
   Apache-2.0) ist sauberes YAML — aber die Modellnamen sind Provider-API-Namen
   („gpt-4o-mini-2024-07-18"), **nicht** GGUF-Quant-Repo-IDs. SWE-bench-Daten
   liegen verstreut im `swe-bench/experiments`-Repo (kein einzelnes JSON).
   Artificial Analysis / LMArena / llm-stats: Lizenz + stabiles JSON unklar.
3. **Das Matching-Problem ist der eigentliche Blocker:** „Qwen2.5-Coder-7B-
   Instruct-**GGUF** @ Q4_K_M" auf eine Leaderboard-Zeile abzubilden ist
   unscharf und oft unmöglich (Quant ≠ Original, lokale kleine Modelle fehlen
   meist ganz).

**Entscheidung:**
- **Kein gebündeltes externes Leaderboard im MVP.** `core::bench` (6.5) misst
  **nur lokal**: tok/s (Prompt+Gen), Ladezeit, VRAM-Peak (NVML), RAM-Peak
  (sysinfo), Stabilität über N Läufe. Das ist der ehrliche, reproduzierbare
  Kern.
- **Der „Overall Score" ist eine offen deklarierte Heuristik** aus lokaler
  Performance + Fit (`core::compat`) + objektiven HF-Signalen (Downloads/Likes/
  Recency/`base_model`-Lineage) — **keine „Qualitäts"-Achse**, die wir nicht
  belegen können.
- **Externe Scores = opt-in, Post-6.5.** Falls später eine tragbare Quelle
  auftaucht (stabiles JSON, klare Lizenz, GGUF-Abdeckung, lösbares Matching),
  kommt sie als abschaltbarer Fetch mit „Heuristik/extern"-Kennzeichnung dazu.
  HF „Community Evals" (`eval.yaml` im Repo) ist der aussichtsreichste Kandidat,
  weil die Daten am Modell selbst hängen.
- **Upgrade-Check (6.7)** rankt entsprechend **ohne** Qualitäts-Score — nur
  Release-Datum, Downloads/Likes, Familien-Lineage, Fit; das lokale LLM
  markiert „Qualität nicht lokal verifizierbar".

**Konsequenzen:**
- (+) Keine Abhängigkeit von einer wackligen externen Quelle; nichts, was
  offline kaputtgeht.
- (+) `core::bench` ist klein und testbar (ein Job wie „Modell testen").
- (−) Die Auto-Auswahl (6.6) hat im MVP keine „Qualitäts"-Daten — sie bleibt
  Fit + lokale Perf + Nutzung. Regelbasiert bleibt der Default, wenn keine
  Bench-Daten da sind.
- (−) „Welches ist das *beste* Coding-Modell?" beantwortet das Tool nicht
  absolut — nur „das schnellste/größte, das bei dir passt und viel genutzt wird".

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
