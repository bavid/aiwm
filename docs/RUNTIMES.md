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
| llama.cpp / llama-server | `LlamaCppAdapter` | ✅ Adapter (2.2a) + Installer (2.2b): Download/Verify/Entpacken des gepinnten CUDA-Builds |
| ComfyUI | `ComfyUiAdapter` | geplant Phase 3 — Bild/Video, vollständig gekapselt |
| Ollama | `OllamaAdapter` | optional, Phase 3+ (Duplikate transparent, ADR-006) |
| LM Studio | — | vorerst nicht (proprietär, GUI-zentriert) |

### `LlamaCppAdapter` (Stand 2.2a)

- **Ein `llama-server`-Prozess pro residentem Modell.** `llama-server` bedient
  genau ein Modell; der Adapter startet/stoppt ihn passend zum Scheduler-Slot.
- **Binär-Auflösung:** `AIWM_LLAMACPP_PATH` → Managed-Install unter
  `<data_dir>/runtimes/llamacpp/` → `PATH`. Fehlt alles: `detail() = "not
  installed"`, `load_model` liefert einen Klartextfehler.
- **Start:** `-m <datei> --host 127.0.0.1 --port <frei> --no-webui -ngl 999
  --flash-attn on` (Defaults in `LlamaServerOptions`, später über Settings-UI).
  Health-Gate: `GET /health` bis `200` oder Timeout/`GaveUp`.
- **Attach-Fallback (ADR-002):** `attach(port, model_id, vram)` probt `/health`,
  gleicht die Modelldatei über `/props` ab, adoptiert den Server **ohne** seine
  Lebensdauer zu übernehmen (`unload` tötet ihn nicht).
- **Installer (2.2b):** `runtime::llamacpp::install`. Gepinnt: `b10855`, zwei
  Assets von `ggml-org/llama.cpp` (`llama-…-bin-win-cuda-12.4-x64.zip` +
  `cudart-…-12.4-x64.zip`), SHA-256 + Größe fest im Code (Werte aus dem
  `digest`-Feld der Releases-API). Streaming-Download mit mitlaufendem Hash →
  Mismatch = Abbruch; `zip`-Entpacken (flach) in
  `%LOCALAPPDATA%\…\runtimes\llamacpp\b10855\`. `offline_mode` = Hard-Refusal.
  Idempotent. `POST /runtimes/llamacpp/install` (202) startet es im Hintergrund;
  Fortschritt in `GET /runtimes` → `detail`. `RuntimeRepo` hält Version + Zustand.
- **Version-Bump:** Tag + beide Digests in `install::PINNED_ARCHIVES` ändern.
  Digests holt man mit `gh api repos/ggml-org/llama.cpp/releases/tags/<tag> --jq
  '.assets[] | select(.name|test("win-cuda-12.4-x64")) | {name, digest}'`.

## Manage-first (ADR-002)

Das Tool installiert/versioniert die Runtimes selbst — aber mit **einer** fest
kuratierten, getesteten Version pro Runtime und **einer** Installationsart. Kein
frei konfigurierbares Python-Environment. „Repair"-Pfad baut die venv sauber neu.

## Zu untersuchen vor Phase 2/3

- ~~llama.cpp: gepinnte Version + Bezugsquelle des Windows-CUDA-Builds~~ →
  ✅ umgesetzt (2.2b, ADR-014)
- ComfyUI: minimale getestete Custom-Node-Menge (Custom Nodes = beliebiger Code)
- `uv`-verwaltete venv pro Runtime; gebündelte CUDA-Runtime statt System-CUDA
- Health-Endpunkte + Modell-Load/Unload-APIs je Runtime — llama-server: `/health`,
  `/props`, `/completion`, `/v1/chat/completions`; **Router-Mode** (ein Server,
  mehrere Modelle, `?autoload=`) neu — als spätere Optimierung notiert
- Shared Model Cache: welche Runtimes können dieselbe Datei via Junction nutzen
  (llama.cpp/LM Studio/ComfyUI ja; Ollama nein — siehe [ANALYSIS.md](ANALYSIS.md) §1.3)
