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
  Größenlimit / „Ordner öffnen"-Button für `<local_root>/outputs`. Mit Video
  (4.1) dringlicher — Clips sind groß (4.5 hat den Retention-Hinweis).
- **Video-Job gegen die echte ComfyUI verproben (4.0):** (a) `SaveVideo` +
  `av>=17` sind wirklich in der Installer-venv; (b) der reale `/history`-
  Output-Key für Video (`videos` / `images` / `gifs` — `collect_media` prüft
  alle drei, aber der echte Name ist ungeprüft); (c) `CLIPLoader type="wan"` +
  `UNETLoader weight_dtype="default"` finden die `video/diffusion_models/`- und
  `text_encoders/`-Ordner aus `extra_model_paths.yaml`; (d) Wan-Shift / Sampler
  (`uni_pc`/`simple`) / Steps an einem echten 5B-Render kalibrieren; (e) das
  1800-s-Timeout gegen echte Clip-Zeiten prüfen; (f) **Bild→Video (4.2):** dass
  `LoadImage` den nach `<base>/input/` kopierten Frame findet und
  `WanImageToVideo.start_image` ihn ohne `clip_vision` akzeptiert (5B TI2V);
  ob der Startframe vorab auf die Zielauflösung skaliert werden muss; (g)
  **LTX-Template (4.4):** `CLIPLoader type="ltxv"` mit dem fp8-t5 lädt,
  `LTXVScheduler` → `SamplerCustom` produziert Sigmas/Latents wie erwartet,
  `LTXVImgToVideo` mit Startframe, LTX-Settings (max_shift/base_shift/terminal,
  Steps, CFG) an einem echten 2B-Render kalibrieren.
- **LTX-Frame-Grid (4.4):** LTX-Video will `(frames-1) % 8 == 0`, `capability::
  video` klemmt aber universell auf Wans `4k+1`. Die LTX-Nodes runden intern ab
  (weniger Frames als angefragt). Optionen: pro Rezept snappen (bräuchte den
  Recipe schon in `from_params` — hat nur die Params, nicht das Modell), oder
  universell auf `8k+1` (Superset von `4k+1`, kostet Wan etwas Granularität).
- **LTX-2 / LTX-2.3 (19B/22B)** bleibt draußen (ADR-020): GGUF braucht
  gepatchte `ComfyUI-GGUF`-Loader (unveröffentlichter Commit) + `ComfyUI-KJNodes`
  → eigener ADR + Installer-Erweiterung, wenn der Weg stabil ist. Bringt Audio
  und höhere Qualität.
- **RAM-Warnung (4.5) ist eine Formel-Heuristik** — `Modellgröße + 6 GB` gegen
  `sysinfo::available_memory`, ein Schwellwert für alle Video-Modelle. Der echte
  Offload-Footprint hängt von `vram_mode`, Encoder-Größe, Auflösung/Länge ab —
  an echten Wan/LTX-Läufen (4.0) kalibrieren; evtl. pro Familie/Modell.
- **Output-Retention (4.5 macht sie nur sichtbar):** `about.outputs_bytes` +
  „reveal"-Knopf sind da, aber es gibt weiter **kein** automatisches Aufräumen,
  Größenlimit oder „X löschen"-Knopf. Eigene kleine Slice bei Bedarf (mit dem
  `GET /jobs/{id}/output`-Streaming zusammen).
- **`[comfyui] extra_args` (4.5)** wird roh an die ComfyUI-Kommandozeile
  angehängt (whitespace-gesplittet). Für den loopback-only-MVP + „Power-User"-
  Feld ok; die Werte sind ungefiltert. Kein Shell-Risiko (`SpawnSpec` übergibt
  Args einzeln, keine Shell), aber ein Tippfehler kann ComfyUI am Start hindern
  → `detail()` zeigt dann „starting…"/Crash-Meldung.
- **`lib/dev-mock.ts`:** beim Vite-Dev im Browser tauchen ein paar
  `transformCallback`-Konsolenfehler auf (StrictMode-Doppelmount vs.
  `@tauri-apps/api/event`-`listen()`). Rein kosmetisch, dev-only — die App
  rendert + funktioniert. `shouldMockEvents` ist an; ein sauberer Fix wäre
  `listen()` im `dev-mock` explizit zu handhaben.
- **Verwaiste `input/`-Kopien (4.2):** der `StagedFrame`-`Drop`-Guard räumt den
  Normalfall (Erfolg / Fehler / Cancel) auf, aber nicht einen harten Absturz
  mitten im Render. Ein Sweep von `<base>/input/*` beim Server-Start (oder ein
  Alters-Filter) wäre robuster. Solange die App läuft, ist es dicht.
- **`init_image` ist ein ungeprüfter Pfad (4.2):** jeder lesbare lokale
  Bildpfad wird nach ComfyUIs `input/` kopiert. Für den loopback-only-MVP
  (ADR-008, der Nutzer kontrolliert seine Maschine) ok; mit LAN-Zugriff /
  Agenten (Phase 5) braucht es eine Allowlist (Outputs-Ordner + Store) oder
  nur-Job-ID.
- **`generate_media` puffert die ganze Ausgabe im RAM** (`Vec<u8>` aus `/view`)
  — für einzelne PNGs ok, für einen mehr-MB-`.mp4` verschwenderisch. Auf
  Streaming (`reqwest` → `tokio::fs`) umstellen, zusammen mit dem
  `GET /jobs/{id}/output`-Streaming unten.
- **Video-Store-Vermischung (4.1):** Wans VAE + umt5-Encoder werden als
  `Vae` / `TextEncoder` importiert und landen unter
  `<store>/image/{vae,text_encoders}/`, nicht unter `<store>/video/`. ComfyUI
  findet sie per Dateiname über den gemergten Ordner-Key, also funktional egal —
  aber unsauber. Optionen: eigene `video/`-Unterordner + ein dritter YAML-Block,
  oder ein „für Video"-Flag beim Import.
- Video-`from_params` (4.1): harte Grenzen sind Backend-seitig geklemmt
  (`width`/`height` 128–1280 auf 16, `length` auf `4k+1` 5–121, `fps` 8–30,
  `steps` 1–60, `cfg` 1–15). ~~Die UI (4.3) muss dieselben Grenzen + eine
  „größer/länger = viel langsamer, kann OOM"-Warnung zeigen.~~ → ✅ 4.3
  (`Video.tsx` klemmt/snappt clientseitig, Warn-Notiz ab > 480p / 81 Frames).
- **Video-UI-Politur (4.3):** (a) echter Prozentbalken statt letzter
  Event-Zeile — braucht `/ws`-Fortschritt vom Core (`progress`/`executing`),
  gleiche Baustelle wie beim Bild (3.4/3.5); (b) echtes Poster-Frame (erstes
  Frame per ffmpeg/`av` extrahieren und als `poster=` setzen) statt
  `<video preload=metadata>`; (c) die Minuten-Schätzung ist eine grobe Formel
  (`frames × steps × pixel`) — nach 4.0 an echten Wan-Läufen kalibrieren;
  (d) Galerie: Lightbox, Download-Knopf (Tauri-`save`-Dialog), Retention /
  Löschen, Paginierung — wie bei der Bild-Galerie offen.
- **`lib/dev-mock.ts` (4.3)** ist minimal — nur die Kommandos, die die Studios
  brauchen. Wenn mehr Tabs im Browser getestet werden sollen, die fehlenden
  Kommandos ergänzen (es warnt in der Konsole bei unbehandelten).
- ComfyUI-Optionen (3.7): nur `vram_mode` ist exponiert. Weitere sinnvolle
  Flags (`--reserve-vram`, `--fast`, `--use-split-cross-attention`) + ein
  „extra args"-Feld könnten dazu, wenn echte Flux-Läufe zeigen was fehlt.
  `vram_mode` ändert die **laufende** Runtime nicht (Neustart) — ein
  `ComfyUiAdapter::set_options` + Server-Neustart wäre der saubere Live-Weg,
  analog zum offenen `LlamaServerOptions`-Live-Apply.
- `GET /jobs/{id}/output` (3.5) liest die ganze Datei in den RAM und serviert
  sie am Stück — ok für einzelne SDXL-PNGs (~1–3 MB); mit Video (4.1, `mp4` /
  `webm` Content-Type ist da) auf `tokio_util::io::ReaderStream` +
  `Content-Length` + Range-Requests umstellen (der `<video>`-Player in 4.3 will
  seeken).

## Vor Phase 5 (Agents)
- ~~Agent-Research + Scheibenplan~~ → ✅ [PHASE_5_PLAN.md](PHASE_5_PLAN.md)
  (OpenCode + Hermes recherchiert, 5.0–5.5, offene Entscheidungen A–F).
- ~~Hermes `bash -l` / WSL2~~ → ✅ 5.0 (ADR-021): Hermes 0.19 hat native Windows-
  + Git-Bash-Behandlung, **kein WSL2-Zwang**. Offen: Hermes ist schwer (~120
  Deps + `hermes postinstall` = node/Browser/ripgrep/ffmpeg) — Installer-Umfang
  in 5.4 klären; evtl. wird Hermes „Advanced/optional".
- `llama-server`-Tool-Calling: `--jinja` emittiert OpenAI-`tool_calls`, OpenCode
  parst sie (5.0-Stub-Test). ✅ 5.1a: `LlamaServerOptions.jinja` (default an) +
  `chat_template` → `--jinja` / `--chat-template` im Spawn, `[llama]`-Config +
  Settings-Toggle. **Offen:** ein echter Coding-Modell-Lauf (Qwen2.5-Coder-GGUF)
  → verlässliche `tool_calls`? + KV-Cache nicht zu hart quantisieren
  (`-ctk q4_0` schadet) → **5.1cb-Smoke** (Adapter + Subsystem sind gegen
  Fixtures verprobt — `capability::agent` in 5.1ca, `LlamaCodingRuntime` platziert
  + pinnt; der echte Modell-Lauf hängt nur noch an der API/Tauri-Anbindung).
  `--jinja` global default-an ändert Chats das Template (embedded statt
  Heuristik — sollte besser sein; falls eine GGUF-Template kaputt ist:
  `jinja = false` + `chat_template` setzen).
- 5.1b OpenCode-Adapter: die exakten `GET /event`-Formen (`message.part.updated`
  `state`-Keys, `permission.asked` vs. `permission.updated`, `session.error`)
  stammen aus dem 5.0-Probe + Fixture — beim ersten echten `opencode`-Lauf in
  5.1cb gegenprüfen (wie der ComfyUI-`/history`-Output-Key in Phase 4). `interrupt`
  nutzt `POST /session/:id/abort` — Realverhalten (kommt ein `session.idle`?)
  offen.
- 5.1ca `AgentSessions`: MVP fährt **eine** Session gleichzeitig; mehrere
  parallele Sessions / Crash-Recovery aus `live_sessions()` = 5.5. `open` nimmt
  `first_message` optional — Zwischenzustand `Starting` nur kurz sichtbar.
- Agent-Sandbox-Niveau: ✅ festgelegt (Plan §B) — Pfad-Allowlist + Command-
  Approval + erzwungene Config; echte FS-/Prozess-Isolation opt-in + später
  (eigener ADR).
- Backup/Restore-Umfang: ✅ festgelegt (Plan §F) — `config.toml` + `aiwm.db` +
  `<data>/agents/<profil>/` + Modell-Manifest; **nicht** `~/.hermes/` des Users
  (eigenes gemanagtes Profil).

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
