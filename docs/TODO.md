# TODO / Parkplatz

Zurückgestellte Punkte aus Analyse und Planung. Nicht im aktuellen Sprint —
hierher, damit nichts verloren geht.

## Vor Phase 2
- Visuelle Designrichtung für die UI festlegen (Typo, Palette, Layout-Charakter) —
  bewusst *nicht* Dark-Mode-by-default; siehe web/design-quality-Regeln
- ~~llama.cpp: gepinnte Version + Bezugsquelle des Windows-CUDA-Builds~~ →
  ✅ 2.2b (ADR-014). Offen: freien Speicherplatz vor dem Download prüfen
  (Brief 10.16 → Phase-6-Download-Manager); alte `runtimes/llamacpp/<build>/`
  beim Versions-Bump aufräumen
- ~~Windows: Junction vs. Hardlink für Modell-Dateien testen~~ → ✅ 2.3
  (`core::link`, ADR-007). Offen: echtes Junctionen in einen Runtime-Ordner
  gegen einen realen Konsumenten (ComfyUI, Phase 3)
- sqlx: optional compile-time Query-Checking (`sqlx::query!`) via
  `cargo sqlx prepare` + `.sqlx/` im Repo + CI-Schritt `--check` (aktuell
  Runtime-Queries)
- RuntimeSupervisor: CREATE_SUSPENDED + Resume, um das Race-Fenster zwischen
  `CreateProcess` und `AssignProcessToJobObject` zu schließen (aktuell: sofortige
  Zuweisung, für llama-server/ComfyUI vernachlässigbar)
- `LlamaCppAdapter`: freien Port aus dem `llama-server`-stdout lesen statt
  Bind-and-Drop (schließt das Port-Race); braucht stdout-Capture im Supervisor
- `LlamaCppAdapter`: Router-Mode evaluieren (ein Server, `/models` + `?autoload=`)
  als Alternative zum Prozess-pro-Modell — spart Modellwechsel-Latenz
- `LlamaServerOptions` über die Settings-UI konfigurierbar machen (2.7):
  `-ngl`, `-c` (überschreibt `compat::effective_ctx`), `--flash-attn`,
  Load-Timeout
- `core::compat` (2.6): die 650-MB-Overhead-Konstante + die grobe KV-Reserve
  (`160 MB / 1K ctx`) gegen echte `nvidia-smi`-Messungen kalibrieren (Phase 6,
  [BENCHMARKS.md](BENCHMARKS.md)). Auch: KV-Cache-Quantisierung (`-ctk`/`-ctv`
  q8/q4) senkt den Bedarf — als Option erwägen
- Blockierte Jobs automatisch neu einreihen, wenn VRAM frei wird ohne dass ein
  anderer Job evictet: Runtime-Stop, `unload_model` über die API, Agent-Session-
  Ende. Aktuell ruht ein `blocked`-Job bis Cancel / anderer Job evictet; ein
  `JobEngine::wake_blocked()` + Aufrufer an diesen Stellen wäre der saubere Weg
- Chat: WebSocket/SSE-Token-Stream an die UI statt `jobs.result`-Polling
  (ADR-015 — MVP pollt); Multi-Turn-Verlauf, System-Prompt + Sampling-Parameter
- Ein langer Chat-Job blockiert die Job-Schleife (Single-Slot-Prämisse ADR-003) —
  ok für den MVP; bei paralleler Bild+LLM-Nutzung neu bewerten
- Cancel während des Modell-Loads: aktuell nur an Schritt-Grenzen (2.4b). Echtes
  Abbrechen mitten im `load_model` bräuchte ein Signal in `RuntimeSupervisor` /
  `await_healthy`
- UI-Lint: `eslint-plugin-react-hooks` (+ `-react-refresh`) in `ui/eslint.config.js`
  aufnehmen (aktuell nur js + typescript-eslint recommended)

## Vor Phase 3/4
- Dedizierte Modell-Research-Aufgabe (aktuelle Bild-/Video-Modelle, Quant, VRAM)
- ComfyUI: minimale getestete Custom-Node-Menge definieren + festpinnen
- `uv`-verwaltete venv-Strategie pro Runtime + „Repair"-Funktion

## Vor Phase 5 (Agents)
- Hermes Agent auf der echten Windows-Maschine: `bash -l`-Abhängigkeit
  verifizieren (Git-Bash mitliefern oder WSL2 dokumentieren)
- Agent-Sandbox-Niveau festlegen (Pfad-Allowlist + Command-Approval reicht für
  Start; echte FS-/Prozess-Isolation später/optional)
- Backup/Restore-Konzept inkl. `~/.hermes/`

## Vor Phase 6 (Model-Manager v2)
- Spike: HF-Hub- + Ollama-Registry-API real testen (Rate-Limits, Token-Pflicht,
  Revision-Pinning, Quant-Erkennung) → Ergebnisse in RUNTIMES.md / MODELS.md
- Kompatibilitäts-/VRAM-Estimator: Formel + Kalibrierung gegen echte Messungen
- Quality-Score-Gewichtung definieren (extern gepflegte Benchmarks + lokale
  Speed/VRAM/Stabilität)
- Best-of-N-Modellauswahl: Bewertungskriterium pro Capability

## Offen / später zu entscheiden
- App-Selbst-Update offline (manueller Installer + Signaturprüfung angenommen)
- Parallele Jobs: Policy verfeinern (klein-LLM + Upscale gleichzeitig)
- 2. GPU zukünftig: `GpuId` im Datenmodell vorsehen, Multi-GPU-Scheduling später
- LAN-/Remote-Zugriff (opt-in, mit Auth) — frühestens nach Phase 6
- Relighting, Generative Fill, Video-Restoration
- Plugin-/Adapter-Plattform für Dritt-Runtimes (erst wenn interne Adapter stabil)
