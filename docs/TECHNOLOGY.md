# Technologievergleich

Stand: September 2026. Modell-/Versionsdetails sind zum Implementierungszeitpunkt
neu zu verifizieren (siehe Brief 10.24). Bewertungskriterien laut Brief:
Offline, Cloud nötig, Account nötig, Daten lokal, Open Source — gleichrangig zu
Performance/Qualität/UX.

---

## 1. LLM-Runtime

| Kriterium | llama.cpp / llama-server | Ollama | LM Studio |
|---|---|---|---|
| Offline (Betrieb) | ✓ | ✓ | ✓ |
| Cloud nötig | ✗ | ✗ (Registry-Pull optional) | ✗ |
| Account nötig | ✗ | ✗ | ✗ |
| Daten lokal | ✓ | ✓ | ✓ |
| Open Source | ✓ (MIT) | ✓ (MIT) | ✗ (proprietär, gratis) |
| OpenAI-kompatible API | ✓ | ✓ (+ eigene API) | ✓ |
| Modell-Speicherung | freie GGUF-Datei | content-addressed Blob-Store (kopiert) | HF-Cache-Layout |
| Modell-Sharing mit ComfyUI/anderen | ✓ via Junction | ✗ | ✓ via Junction |
| Steuerbarkeit (Prozess, Flags, KV-Quant, Offload) | hoch | mittel (Modelfile) | niedrig (GUI-zentriert) |
| Headless / scriptbar | ✓ | ✓ | eingeschränkt |
| Multi-GPU / Feintuning der Perf | ✓ | mittel | mittel |
| Reife der Model-Mgmt-Features | niedrig (neu) | hoch | hoch |

**Empfehlung:** **llama.cpp / llama-server als primäre LLM-Runtime.** Gründe:
echtes Modell-Sharing (ein GGUF, viele Konsumenten), volle Kontrolle über
KV-Cache-Quantisierung und CPU-Offload (auf 16 GB entscheidend), sauber als
Kindprozess steuerbar, MIT. Nachteil: wir bauen etwas Modell-Verwaltung selbst —
das wollen wir ohnehin (kanonischer Store).

**Ollama als optionaler Zweitadapter** für Nutzer, die dessen Ökosystem/Modelfiles
wollen; Duplikate werden transparent gemacht. **LM Studio** vorerst nicht
integrieren (proprietär, GUI-zentriert, geringer Mehrwert neben llama.cpp).

> Offene Entscheidung A aus [ANALYSIS.md](ANALYSIS.md): hiermit vorgeschlagen zugunsten llama.cpp.

---

## 2. Bild- & Video-Runtime

| Kriterium | ComfyUI | Automatic1111 / Forge | InvokeAI | SD.Next |
|---|---|---|---|---|
| Offline | ✓ | ✓ | ✓ | ✓ |
| Cloud/Account nötig | ✗ | ✗ | ✗ | ✗ |
| Open Source | ✓ (GPL-3) | ✓ | ✓ (Apache) | ✓ |
| API für Automatisierung | ✓ stark (`/prompt`, WS) | ✓ (begrenzt) | ✓ | ✓ |
| Neue Modelle zuerst | ✓ (schnellste Adoption) | langsamer | langsam | mittel |
| Workflow-Flexibilität | sehr hoch (Node-Graph) | niedrig | mittel | mittel |
| VRAM-Effizienz / Offload | ✓ (gut steuerbar) | mittel | mittel | ✓ |
| Risiko: Custom Nodes = beliebiger Code | **ja** | gering | gering | gering |

**Empfehlung:** **ComfyUI** — De-facto-Standard, beste API, schnellste
Modell-Unterstützung, gut auf 16 GB tunebar. Wir kapseln es vollständig hinter
unseren Pipelines; der Nutzer sieht nie den Node-Graph. **Custom Nodes werden
nie automatisch installiert** — nur eine von uns getestete, festgepinnte Menge.

---

## 3. Desktop-Stack

| Kriterium | Tauri 2 + Rust | Electron + Node/TS | Python + FastAPI + Web-UI | Go + Web-UI |
|---|---|---|---|---|
| Bundle-Größe | ~10–20 MB | ~150–250 MB | mittel (Python-Runtime) | klein |
| RAM idle | niedrig | hoch | mittel | niedrig |
| Prozess-Lifecycle / Kill unter Windows | ✓ stark (Job Objects) | ok | schwächer | ✓ |
| Nähe zum AI-Ökosystem | via Sidecar | via Sidecar | **nativ** | via Sidecar |
| GPU-Telemetrie (NVML-Bindings) | ✓ (`nvml-wrapper`) | ✓ (FFI/Addon) | ✓ (`pynvml`) | ✓ |
| Windows-Integration (Tray, Autostart, Installer) | ✓ | ✓ | schwächer | mittel |
| Entwicklungsgeschwindigkeit | mittel (Rust-Lernkurve) | hoch | hoch | mittel |
| Wartbarkeit / Typsicherheit Core | ✓ hoch | mittel | mittel | ✓ hoch |
| Ein-Sprachen-Stack | ✗ (Rust+TS+Py) | ✗ (TS+Py) | ~ (Py+JS) | ✗ (Go+TS+Py) |

**Entscheidung (ADR-001): Tauri 2 + Rust-Core + Python-Sidecar.** Begründung in
[DECISIONS.md](DECISIONS.md). Kernpunkt: Der Core ist überwiegend Orchestrierung,
Prozess-Aufsicht und Monitoring — Rusts Stärken. Python bleibt auf das begrenzt,
was nur in Python existiert.

**Wesentlicher Trade-off:** Drei Sprachen. Gegenmaßnahme: scharfe Grenzen
(Core ↔ Sidecar über eine kleine, versionierte RPC-Schnittstelle), Sidecar bleibt
dünn.

---

## 4. Frontend-Framework

| Kriterium | React + TS | Svelte 5 | SolidJS | Vue 3 |
|---|---|---|---|---|
| Ökosystem / Komponenten | größte | mittel | klein | groß |
| Bundle / Runtime-Overhead | mittel | klein | klein | mittel |
| Vertrautheit / Hiring von Beispielen | hoch | mittel | niedrig | mittel |
| Eignung für Live-Dashboards (Streams) | ✓ | ✓ | ✓ (fein-granular) | ✓ |

**Empfehlung:** **React + TypeScript + Vite.** Größtes Ökosystem, meiste
verfügbare UI-Bausteine, gut für Tauri dokumentiert. Overhead ist für eine
lokale Desktop-App unkritisch.

---

## 5. Datenhaltung

| Option | Bewertung |
|---|---|
| **SQLite** (Datei, via `sqlx`/`rusqlite`) | ✓ Empfehlung. Offline, null Setup, transaktional, reicht für Jahre. |
| Postgres | ✗ Overkill, Server-Prozess, kein Vorteil für Single-User lokal. |
| JSON-Dateien | ✗ nur für Config-Export; keine Abfragen, keine Transaktionen. |
| DuckDB | ~ interessant für spätere Benchmark-Analytik, nicht als Primär-DB. |

Große Binärdaten (Modelle, Bilder, Videos) liegen **im Dateisystem**, die DB
hält nur Metadaten + Pfade.

---

## 6. Agent-Runtimes (Phase 5)

| Kriterium | Hermes Agent (Nous Research) | OpenCode | aider | Continue |
|---|---|---|---|---|
| Offline / lokal | ✓ (Custom endpoint) | ✓ | ✓ | ✓ (Editor-Plugin) |
| Account / Cloud nötig | ✗ (Nous Portal nur optional, bezahlt) | ✗ | ✗ | ✗ |
| Open Source | ✓ (MIT) | ✓ (MIT) | ✓ (Apache) | ✓ (Apache) |
| Lokale Modelle | ✓ llama.cpp / vLLM / Ollama / SGLang | ✓ | ✓ | ✓ |
| Telemetrie | keine | keine | keine | opt-out |
| Stack | Python 3.11 + `uv`, **kein Docker nötig** | Node / TS | Python | TS (IDE) |
| Persistentes Memory / Skills | ✓ eingebaut (`~/.hermes/`, Memory + auto-Skills + Sub-Agents) | Session-History (SQLite) | Git-Historie | begrenzt |
| Sub-Agents mit eigenem Terminal | ✓ | ~ | ✗ | ✗ |
| Headless steuerbar durch unser Tool | ✓ (CLI, `hermes model` Custom endpoint) | ✓ (`opencode serve`) | ✓ (CLI/scriptbar) | schwächer (IDE-gebunden) |
| Windows-Eignung | ~ baut Kontext via `bash -l` → Git-Bash/WSL nötig (vor Phase 5 prüfen) | ✓ | ✓ | ✓ |
| Betriebsart | Terminal + Desktop-App | Terminal + `serve` + Desktop | Terminal | VS Code / JetBrains |

**Entscheidung (ADR-010): Hermes Agent + OpenCode.**
- **Hermes Agent** deckt „Agent mit Gedächtnis / Langlauf / Sub-Agents" ab
  (Brief Abschnitte 4 + 18) — dieses Memory-/Skill-System müssen wir nicht selbst
  bauen. Unser Tool orchestriert und begrenzt es (Profil, Modell = lokaler
  llama-server, Pfad-Allowlist, Command-Approval).
- **OpenCode** als schlanker terminal-/`serve`-basierter Coding-Agent für kürzere
  Aufgaben.
- **aider** bleibt als scriptbarer Zweitkandidat notiert.

**Windows-Vorbehalt:** Hermes' `bash -l`-Kontext und die vLLM-Empfehlung sind
Linux-orientiert. Wir fahren Hermes gegen unseren llama-server-Endpoint; die
`bash`-Abhängigkeit ist vor Phase 5 auf der echten Maschine zu verifizieren
(Git-Bash mitliefern oder WSL2 als optionale Voraussetzung dokumentieren).

---

## 7. Modell-Quellen-APIs (für Model Manager v2, nicht MVP)

| Quelle | API | Download | Metadaten | Für Auto-Download geeignet? |
|---|---|---|---|---|
| Hugging Face Hub | ✓ REST + `huggingface_hub` | ✓ (Range, Resume, SHA256) | ✓ reichhaltig (Model Card, Tags, Dateien, Revisions) | ✓ (kuratiert: Repos mit `.gguf`/`.safetensors`) |
| Ollama Library | ✓ (Registry-Protokoll) | ✓ | mittel (Tags, Größen, Quant) | ✓ |
| GitHub Releases | ✓ REST | ✓ (Assets) | schwach (nur Release-Notes) | ~ (nur mit manueller Zuordnung) |
| Civitai | ✓ REST | ✓ | ✓ (Bild-Modelle, LoRAs) | ~ (Trust/Qualität variabel, später) |

**Reihenfolge:** HF Hub zuerst (GGUF für LLM), dann Ollama-Library, dann
ComfyUI-relevante HF-Repos für Diffusion. GitHub/Civitai später.

Details zu verifizieren (Brief 10.24): exakte Rate-Limits, ob HF-Token für
öffentliche Repos nötig (i. d. R. nicht), Revision-/Commit-Pinning, wie
Quantisierung zuverlässig aus Dateinamen + GGUF-Header gelesen wird.

---

## 8. GPU-Telemetrie

| Option | Bewertung |
|---|---|
| **NVML** via `nvml-wrapper` (Rust) | ✓ Empfehlung. VRAM, Auslastung, Temperatur, Prozesse, Takt. Kein `nvidia-smi`-Parsing. |
| `nvidia-smi` XML/CSV parsen | ✗ Fallback nur, fragil. |
| Windows PDH / WMI | ~ nur ergänzend für CPU/RAM (oder `sysinfo`-Crate). |

---

## 9. Zusammenfassung der Empfehlungen

| Bereich | Wahl | Status |
|---|---|---|
| LLM-Runtime | llama.cpp / llama-server (primär), Ollama optional | Entschieden (ADR-006) |
| Bild/Video-Runtime | ComfyUI, vollständig gekapselt | Vorschlag |
| Desktop | Tauri 2 + Rust | Entschieden (ADR-001) |
| Frontend | React + TS + Vite | Vorschlag |
| DB | SQLite | Vorschlag (ADR-005) |
| Telemetrie | NVML (`nvml-wrapper`) | Vorschlag |
| Agent-Runtimes | Hermes Agent + OpenCode | Entschieden (ADR-010) |
| Modell-Quellen | HF Hub → Ollama-Library → (später GitHub/Civitai) | Vorschlag, v2 |
