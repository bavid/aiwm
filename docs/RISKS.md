# Risikoregister

Skala: **Hoch** = kann das Projekt zum Scheitern bringen · **Mittel** = kostet
spürbar Zeit/Qualität · **Niedrig** = beobachten.

---

## R1 — Scope-Kollaps *(Hoch)*

Der Brief beschreibt Jahre an Arbeit. Solo/klein ist das Hauptrisiko, nie fertig
zu werden oder ein halbfertiges Monster zu bauen.

**Gegenmaßnahmen:** Harte Phasengrenzen. MVP ohne AI-Capability. Jede Phase muss
für sich nutzbar sein. Feature-Wünsche landen in `TODO.md`, nicht im aktuellen
Sprint. Nach jeder Phase ehrlicher DONE-Bericht + Go/No-Go.

## R2 — „Wir bauen doch Stability Matrix" *(Hoch)*

Manage-first + Auto-Alles ist genau die Klasse Tool, deren Komplexität den Nutzer
frustriert hat. Risiko: dieselbe Komplexität, nur mit unserem Namen drauf.

**Gegenmaßnahmen:** Radikale Kuratierung — eine getestete Runtime-Version, keine
freie Env-Konfiguration, feste Pipelines. Advanced-Optionen strikt hinter einem
zweiten Layer. Erfolgskriterium regelmäßig prüfen: „fühlt es sich einfacher an als
Ollama + ComfyUI einzeln?"

## R3 — VRAM-Realität auf 16 GB *(Hoch)*

Mehrere gewünschte Szenarien (großer Coding-Kontext + parallele Bildgenerierung;
Video 14B; 32B dense flüssig) sind auf 16 GB / 32 GB RAM nicht komfortabel.
Enttäuschungsrisiko.

**Gegenmaßnahmen:** [HARDWARE.md](HARDWARE.md) als Erwartungsanker. Estimator rechnet
konservativ und *zeigt* die Rechnung. UI kommuniziert Grenzen proaktiv statt Jobs
scheitern zu lassen. RAM-Upgrade-Empfehlung (64 GB) klar dokumentiert.

## R4 — Modell-Sharing mit Ollama unmöglich *(Mittel)*

„Ein Modell, eine Datei" scheitert an Ollamas Blob-Store. Bei Ollama im Stack
gibt es Duplikate.

**Gegenmaßnahmen:** llama.cpp als primäre LLM-Runtime (Entscheidung A). Ollama
optional, Duplikate transparent im Dedup-Report. Erwartung im UI ehrlich rahmen.

## R5 — ComfyUI Custom Nodes = Code-Ausführung *(Mittel/Sicherheit)*

Viele Bild-/Video-Workflows brauchen Custom Nodes = beliebiger Python-Code aus
dem Netz.

**Gegenmaßnahmen:** Nur eine von uns getestete, festgepinnte Node-Menge.
Nie automatische Node-Installation. Wenn ein Workflow eine unbekannte Node
braucht → explizite Nutzer-Bestätigung mit Quelle. `.safetensors`/`.gguf` vor
Pickle bevorzugen.

## R6 — Python-Environment-Hölle *(Mittel)*

ComfyUI + Torch + CUDA + Custom Nodes ist notorisch für Versionskonflikte. Wenn
wir das „managen", erben wir die Brüche.

**Gegenmaßnahmen:** `uv`-verwaltete, festgepinnte venv pro Runtime. Gebündelte
CUDA-Runtime, kein System-CUDA. „Repair"-Funktion die die venv sauber neu
aufbaut. venv-Zustand als Health-Check.

## R7 — Drei-Sprachen-Stack *(Mittel)*

Rust + TypeScript + Python. Kontextwechsel-Kosten, Build-Komplexität, Onboarding.

**Gegenmaßnahmen:** Sidecar bewusst dünn halten. Eine kleine versionierte
RPC-Grenze Core ↔ Sidecar. Kein Python im Core-Pfad wo Rust reicht. Klare
Ordnergrenzen.

## R8 — Auto-Unload bricht laufende Agent-Session *(Mittel)*

Naives VRAM-Scheduling evakuiert das Modell eines aktiven Agents.

**Gegenmaßnahmen:** Session-Awareness im Scheduler (ADR-003). „Pinned"-Modelle.
Konflikt → Nutzer fragen, nicht still evakuieren. Tabellentests für die
Szenario-Matrix.

## R9 — Modell-Landschaft driftet schnell *(Mittel)*

Zwischen Planung und Bau ändern sich beste Modelle/Versionen/APIs. Content-Farm-
Quellen sind teils ungenau/halluziniert.

**Gegenmaßnahmen:** Modell-Auswahl nicht hart verdrahten — Rollen + Metadaten +
Estimator. Primärquellen (offizielle Repos, Model Cards, GitHub) zum Bauzeitpunkt.
Dedizierte Modell-Research-Aufgabe vor Phase 3/4/6 (Brief 10.24, 31, 38).

## R10 — HF-/Registry-API-Annahmen falsch *(Mittel, v2)*

Rate-Limits, Token-Pflicht, Revision-Handling, Quant-Erkennung können anders sein
als angenommen.

**Gegenmaßnahmen:** **Scheibe 6.0** (`PHASE_6_PLAN.md`) = expliziter Spike: HF-
Hub-API + Ollama real testen, Ergebnisse in `RUNTIMES.md`/`MODELS.md`. Erste
Befunde (09/26): Anon-Limit 500 API-Calls/5 min/IP reicht; `lfs.oid` liefert die
Verify-SHA-256 vorab; Ollama hat kein offizielles Such-API. Download-Manager mit
Backoff + Resume + `RateLimit`-Header-Auswertung.

## R11 — Windows-Prozess-/Firewall-Verhalten *(Niedrig/Mittel)*

Verwaiste Kindprozesse, Firewall-Dialoge, Pfad-/Locking-Eigenheiten, Junction-
Rechte (Admin nötig?).

**Gegenmaßnahmen:** Job Objects. Alle Runtimes auf `127.0.0.1`. Früh auf der
echten Maschine testen. Junction (nicht Symlink — braucht keine Admin-Rechte für
Verzeichnisse; für Dateien ggf. Hardlink prüfen).

## R12 — Objektiver Quality-Score nicht lokal machbar *(Niedrig)*

Der Brief will nicht-subjektive Scores. Lokal gibt es keinen billigen Qualitäts-
Benchmark.

**Gegenmaßnahmen:** Score = extern gepflegte Benchmarks (online) + lokal gemessene
Speed/VRAM/Stabilität, als gewichtete Heuristik gekennzeichnet. Kein Anspruch auf
Objektivität der „Qualitäts"-Achse.

## R13 — App-Update offline *(Niedrig)*

Offline-first App braucht trotzdem einen Update-Weg.

**Gegenmaßnahmen:** Manueller Installer-Download + Signaturprüfung. Kein
Auto-Update-Zwang, kein Telemetrie-Callback.

## R14 — Datenabfluss bei „privaten Fotos" *(Niedrig, Design-kritisch)*

Enhancement arbeitet mit privaten Fotos/Videos. Versehentlicher Cloud-Call wäre
ein Vertrauensbruch.

**Gegenmaßnahmen:** Cloud strikt opt-in pro Aktion mit Bestätigungsdialog.
Offline-Modus blockt jeden externen Call hart. Kein Provider im Default-Pfad.
