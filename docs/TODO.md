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
  (`core::link`, ADR-007). **Offen bleibt** die Verprobung von `Junction` gegen
  einen realen Konsumenten: ComfyUI war es nicht — der Store liegt auf `E:`, die
  ComfyUI-Install unter `%LOCALAPPDATA%` (`C:`), und NTFS-Junctions überspannen
  keine Volumes. ComfyUI liest den Store stattdessen über
  `extra_model_paths.yaml` (`LinkStrategy::ExtraPath`, 3.3 / ADR-019). Der
  nächste Junction-Kandidat wäre ein LM-Studio-artiger Konsument auf derselben
  Volume.
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
- ~~`LlamaServerOptions` über die Settings-UI konfigurierbar machen~~ → ✅ 2.7
  (`[llama]` in `config.toml`, `GET`/`PUT /config`, ADR-017). Offen: die Optionen
  live anwenden statt neustart-pflichtig (`Mutex<LlamaServerOptions>` +
  `set_options` auf dem Adapter); ebenso `store_path` / `vram_budget_mb` /
  `log_filter` (Letzteres braucht einen `tracing`-`reload::Handle`)
- ~~Phase-2-Abschluss: durchgehender End-to-End-Test Modell-Wechsel~~ → ✅
  `core/tests/model_swap.rs`
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
- ~~Modell-Research (Bild-Modelle, Quant, VRAM)~~ → ✅ in [PHASE_3_PLAN.md](PHASE_3_PLAN.md)
  §Research (SDXL/Flux/SD3.5/Qwen für 16 GB). Video-Modelle → vor Phase 4.
- ~~ComfyUI: minimale Custom-Node-Menge~~ → ✅ PHASE_3_PLAN.md: genau
  `city96/ComfyUI-GGUF` (Slice 3.2b), Rest bewusst später
- ~~`uv`-verwaltete venv-Strategie + „Repair"~~ → ✅ 3.2a (`comfyui::install`,
  ADR-018). Offen: Cleanup von `runtimes/comfyui/{<alter-tag>,uv-cache,python}`
  beim Versions-Bump; freien Speicherplatz vor dem torch-Download prüfen
- ComfyUI cu130-torch: GPU-**Treiber**-Kompatibilität auf der echten 4080 Super
  verifizieren — **weiterhin offen**: alle Bild-Smokes (3.4–3.6) fahren gegen
  die Fake-ComfyUI, nicht die echte CUDA-Laufzeit. Sobald die echte ComfyUI ein
  Bild rendert prüfen; fällt es aus → cu128/cu126-Pin.
- **Flux-Graph gegen die echte ComfyUI verproben** (mit dem cu130-Check
  zusammen): (a) dass `DualCLIPLoaderGGUF`s `get_full_path("clip", …)` unsere
  `text_encoders`-Ordner-Konfig aus `extra_model_paths.yaml` findet
  (ComfyUIs `map_legacy` sollte `clip`→`text_encoders` mappen — geprüft im Code,
  nicht live); (b) dass `UnetLoaderGGUF` das GGUF in `diffusion_models/` sieht;
  (c) Steps/Scheduler/Guidance-Defaults an einem echten Flux-Render kalibrieren.
- Flux-Companion-Auflösung (3.6) nutzt eine Namens-Heuristik (`t5` / `clip`).
  Robuster wäre ein `.safetensors`-Header-Check (Tensor-Namen verraten T5 vs
  CLIP-L eindeutig) — hängt an der ohnehin vertagten Header-Inspektion. Bis
  dahin: klappt für die kuratierten Dateinamen, kann bei exotischen Umbenennungen
  daneben greifen.
- `.safetensors`-Header-Inspektion (3.3 vertagt): der JSON-Header am Dateianfang
  trägt Tensor-Namen/Shapes/dtype — daraus ließen sich Arch-Familie (SDXL/Flux/
  SD3.5), Precision (fp16/fp8) und ein besserer VRAM-Estimate ableiten, statt
  Namens-Heuristik + „Dateigröße + Familie-Headroom". Braucht einen bounded
  Reader wie beim GGUF-Header.
- ~~`import_model`: Bild-Modelle bekommen keine `model_roles`~~ → ✅ 3.4:
  `ModelKind::default_role()` (`Checkpoint`/`DiffusionModel` → `base_diffusion`).
  VAE/LoRA/Text-Encoder bekommen noch keine — nachziehen, wenn 3.6 eine
  Pipeline baut, die sie per Rolle auflöst (aktuell nur `base_diffusion`).
- ComfyDirs (3.3): der Store-Pfad landet über `aiwm-model-paths.yaml` erst beim
  **nächsten** ComfyUI-Start in der laufenden Runtime. Bei laufendem Server nach
  einer Store-Pfad-Änderung wäre ein Neu-Schreiben + `POST /free` oder ein
  Server-Neustart sauberer — für den MVP ok (Settings sagt „Neustart nötig").
- Bild-Job (3.4/3.5): MVP pollt `/history` (750 ms) + die UI pollt `jobDetail`
  (700 ms). `/ws`-Fortschritt (`progress` / `executing` / `executed`) an die UI
  streamen → echter Fortschrittsbalken statt nur „preparing…". Auch:
  SDXL-Refiner-Pass, Batch-Größe > 1.
- Image-UI (3.5): Galerie hat keine Paginierung / kein Löschen / keinen
  Download-Button, kein Bild-Zoom/Lightbox, keine Prompt-History, keine
  Style-Presets. Der Download-Button ist heikel im Tauri-Sandbox (`<a download>`
  inert) — bräuchte einen `save`-Dialog-Command. Alles eigene kleine Slices bei
  Bedarf.
- Output-Retention (3.7 zeigt nur einen Hinweis): automatisches Aufräumen /
  Größenlimit / „Ordner öffnen"-Button für `<local_root>/outputs`. Auch: die
  Bild-Bytes werden von `GET /jobs/{id}/output` am Stück gelesen — für Video
  später auf Streaming umstellen.
- ComfyUI-Optionen (3.7): nur `vram_mode` ist exponiert. Weitere sinnvolle
  Flags (`--reserve-vram`, `--fast`, `--use-split-cross-attention`) + ein
  „extra args"-Feld könnten dazu, wenn echte Flux-Läufe zeigen was fehlt.
  `vram_mode` ändert die **laufende** Runtime nicht (Neustart) — ein
  `ComfyUiAdapter::set_options` + Server-Neustart wäre der saubere Live-Weg,
  analog zum offenen `LlamaServerOptions`-Live-Apply.
- `GET /jobs/{id}/output` (3.5) liest die ganze Datei in den RAM und serviert
  sie am Stück — ok für einzelne SDXL-PNGs (~1–3 MB); bei großen Bildern /
  Video später auf `tokio_util::io::ReaderStream` + `Content-Length` umstellen.

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
