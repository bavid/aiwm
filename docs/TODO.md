# TODO / Parkplatz

Zurückgestellte Punkte aus Analyse und Planung. Nicht im aktuellen Sprint —
hierher, damit nichts verloren geht.

## Vor Phase 2
- Visuelle Designrichtung für die UI festlegen (Typo, Palette, Layout-Charakter) —
  bewusst *nicht* Dark-Mode-by-default; siehe web/design-quality-Regeln
- llama.cpp: gepinnte Version + Bezugsquelle des Windows-CUDA-Builds klären
- Windows: Junction vs. Hardlink für Modell-Dateien testen (Rechte, Verhalten)

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
