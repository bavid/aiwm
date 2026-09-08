# Runtimes

Wie das Tool AI-Runtimes verwaltet. Aktueller Stand + geplante Integration.

## Abstraktion

`RuntimeAdapter` (in `core::runtime`) — jede Runtime implementiert:

| Methode | Zweck |
|---|---|
| `id` / `kind` | Identität |
| `spawn_spec` | wie der Prozess gestartet wird (oder `None`) |
| `health` | Zustand: `Unknown` / `Starting` / `Healthy` / `Unhealthy` |
| `load_model` / `unload_model` | Modell in der Runtime laden/entladen |
| `loaded_models` / `vram_used_mb` | VRAM-Buchhaltung für den Scheduler |

`RuntimeSupervisor` besitzt den OS-Prozess: Start ins **Windows Job Object**
(`KILL_ON_JOB_CLOSE`), Health-Monitor, Auto-Restart mit gedeckelter
Exponential-Backoff (`MAX_RESTARTS=10`, 200 ms … 30 s).

`RuntimeRegistry` — geteilte Adapter-Map, von `JobEngine` und `HybridScheduler`
genutzt.

## Status

| Runtime | Adapter | Stand |
|---|---|---|
| Fake | `FakeRuntimeAdapter` | ✅ vollständig (Tests) |
| llama.cpp / llama-server | `LlamaCppAdapter` | geplant Phase 2 — primäre LLM-Runtime (ADR-006) |
| ComfyUI | `ComfyUiAdapter` | geplant Phase 3 — Bild/Video, vollständig gekapselt |
| Ollama | `OllamaAdapter` | optional, Phase 3+ (Duplikate transparent, ADR-006) |
| LM Studio | — | vorerst nicht (proprietär, GUI-zentriert) |

## Manage-first (ADR-002)

Das Tool installiert/versioniert die Runtimes selbst — aber mit **einer** fest
kuratierten, getesteten Version pro Runtime und **einer** Installationsart. Kein
frei konfigurierbares Python-Environment. „Repair"-Pfad baut die venv sauber neu.

## Zu untersuchen vor Phase 2/3

- llama.cpp: gepinnte Version + Bezugsquelle des Windows-CUDA-Builds
- ComfyUI: minimale getestete Custom-Node-Menge (Custom Nodes = beliebiger Code)
- `uv`-verwaltete venv pro Runtime; gebündelte CUDA-Runtime statt System-CUDA
- Health-Endpunkte + Modell-Load/Unload-APIs je Runtime
- Shared Model Cache: welche Runtimes können dieselbe Datei via Junction nutzen
  (llama.cpp/LM Studio/ComfyUI ja; Ollama nein — siehe [ANALYSIS.md](ANALYSIS.md) §1.3)
