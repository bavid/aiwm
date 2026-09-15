# TODO / Parkplatz

Zurückgestellte Punkte aus Analyse und Planung. Nicht im aktuellen Sprint —
hierher, damit nichts verloren geht.

## Vor Phase 2
- Visuelle Designrichtung für die UI festlegen (Typo, Palette, Layout-Charakter) —
  bewusst *nicht* Dark-Mode-by-default; siehe web/design-quality-Regeln
- ~~llama.cpp: gepinnte Version + Bezugsquelle des Windows-CUDA-Builds~~ →
  ✅ 2.2b (ADR-014). ~~Alte `runtimes/llamacpp/<build>/` beim Versions-Bump
  aufräumen~~ → ✅ `install::cleanup_old_builds` (best-effort, läuft nach jedem
  erfolgreichen `install()`). Offen: freien Speicherplatz vor dem Download
  prüfen (Brief 10.16 → Phase-6-Download-Manager)
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
  `log_filter` (Letzteres braucht einen `tracing`-`reload::Handle`) /
  `[paths].{outputs,runtimes,cache}_path` (ADR-026, neustart-pflichtig weil
  `runtimes_dir` schon beim `LlamaCppAdapter`/`ComfyUiAdapter`-Constructor
  gebraucht wird)
- ~~Phase-2-Abschluss: durchgehender End-to-End-Test Modell-Wechsel~~ → ✅
  `core/tests/model_swap.rs`
- `core::compat` (2.6): die 650-MB-Overhead-Konstante + die grobe KV-Reserve
  (`160 MB / 1K ctx`) gegen echte `nvidia-smi`-Messungen kalibrieren (Phase 6,
  [BENCHMARKS.md](BENCHMARKS.md)). Auch: KV-Cache-Quantisierung (`-ctk`/`-ctv`
  q8/q4) senkt den Bedarf — als Option erwägen
- ~~Blockierte Jobs automatisch neu einreihen, wenn VRAM frei wird ohne dass
  ein anderer Job evictet~~ → **geprüft (2026-09-13), kein echter Bug**:
  `run_job_loop` pollt ohnehin ungebremst (`JOB_LOOP_IDLE` 250ms,
  `JOB_LOOP_BLOCKED_BACKOFF` nur 3s nach einem `Blocked`-Ergebnis, `api/mod.rs`),
  `next_runnable` + `HybridScheduler::plan()` sind zustandslos und werten einen
  `blocked`-Job bei jedem Tick frisch neu aus (kein "stecken bleiben" auf
  Engine-/Scheduler-Ebene, s. `blocked_job_stays_blocked_and_is_picked_up_again`
  in `orchestrator/engine.rs`) — im schlimmsten Fall also ≤3s Verzögerung, kein
  permanenter Stillstand. Zwei der drei genannten Auslöser existieren zudem noch
  gar nicht als erreichbarer Pfad: weder "Runtime manuell stoppen" noch
  `unload_model` sind über HTTP/Tauri/UI aufrufbar (nur intern bei
  Neustart/Install). Nur "Agent-Session-Ende" ist real erreichbar
  (`AgentSessions::stop`/`drain_events`) und funktioniert bereits über das
  Polling. Ein `wake_blocked()` würde also zwei nicht existierende Call-Sites
  bedienen und die dritte nur von ≤3s auf ~sofort verkürzen — aufgehoben bis
  eins der beiden fehlenden API-Surfaces (Runtime-Stop, `unload_model`) real
  gebaut wird.
- Chat: WebSocket/SSE-Token-Stream an die UI statt `jobs.result`-Polling
  (ADR-015 — MVP pollt); Multi-Turn-Verlauf, System-Prompt + Sampling-Parameter
- Ein langer Chat-Job blockiert die Job-Schleife (Single-Slot-Prämisse ADR-003) —
  ok für den MVP; bei paralleler Bild+LLM-Nutzung neu bewerten
- Cancel während des Modell-Loads: aktuell nur an Schritt-Grenzen (2.4b). Echtes
  Abbrechen mitten im `load_model` bräuchte ein Signal in `RuntimeSupervisor` /
  `await_healthy`
- ~~UI-Lint: `eslint-plugin-react-hooks` (+ `-react-refresh`) in
  `ui/eslint.config.js` aufnehmen~~ → ✅ (5.2.0 `recommended` — bewusst nicht
  die 6.x/7.x-Linie, die zusätzlich ~15 experimentelle "React Compiler"-Regeln
  mitbringt; `react-refresh/only-export-components` als `warn`, Standard-Vite-
  Template-Setup). Deckte 2 reale `exhaustive-deps`-Fälle in `lib/hooks.ts`
  (bewusstes `key`-statt-`fetcher`/`params`-Pattern — beide mit begründetem
  `eslint-disable-next-line` versehen, sonst hätte es den Poll-/Debounce-
  Intervall bei jedem Render neu gestartet) und einen echten Fix in
  `Models.tsx` (`tags` jetzt selbst `useMemo`-gewrappt) auf.

## Vor Phase 3/4
- ~~Modell-Research (Bild-Modelle, Quant, VRAM)~~ → ✅ in [PHASE_3_PLAN.md](PHASE_3_PLAN.md)
  §Research (SDXL/Flux/SD3.5/Qwen für 16 GB). Video-Modelle → vor Phase 4.
- ~~ComfyUI: minimale Custom-Node-Menge~~ → ✅ PHASE_3_PLAN.md: genau
  `city96/ComfyUI-GGUF` (Slice 3.2b), Rest bewusst später
- ~~`uv`-verwaltete venv-Strategie + „Repair"~~ → ✅ 3.2a (`comfyui::install`,
  ADR-018). Offen: Cleanup von `runtimes/comfyui/{<alter-tag>,uv-cache,python}`
  beim Versions-Bump; freien Speicherplatz vor dem torch-Download prüfen
- ~~ComfyUI cu130-torch: GPU-**Treiber**-Kompatibilität auf der echten 4080
  Super verifizieren~~ → ✅ **verifiziert (2026-09-12)**: cu130/PyTorch 2.14
  bootet sauber gegen den echten Treiber, `Device: cuda:0 NVIDIA GeForce RTX
  4080 SUPER`. Dabei **einen echten Startup-Crash gefunden + behoben**:
  `--base-directory` (für die ADR-019-Datenablage-Trennung) leitet auch
  ComfyUIs `custom_nodes`-Suche dorthin um — die lagen dort nie, ComfyUI
  crashte beim Start (`os.listdir` auf einen nicht existenten Pfad), und selbst
  ohne Crash hätte `ComfyUI-GGUF` nie geladen. Fix: Junction von
  `<comfyui-data>/custom_nodes` auf die echte Install (`core::runtime::comfyui::
  launch::ComfyDirs::ensure`). Nach dem Fix: **erster echter End-to-End-Render
  dieses Projekts** — ein reales 256×256/9-Frame-Wan-Clip, echte MP4-Datei.
  Details: [MODELS.md](MODELS.md) / Commit `280018d`.
- **Flux selbst noch nicht real getestet** (unabhängig vom obigen Fix): Flux
  Q8_0 allein braucht laut Katalog-Daten ~13,7 GB, der komplette Stack (+T5+
  CLIP-L+VAE) ~17 GB — passt auf einer 16-GB-Karte nicht, mit oder ohne
  ComfyUIs eigenes `lowvram`/`novram`-Offloading ungetestet (AIWMs eigener
  Preflight-Check blockt den Job schon vor dem ersten ComfyUI-Request, bekommt
  also nie die Chance, das rauszufinden — siehe „Preflight-Gate ignoriert
  ComfyUIs vram_mode" unten). **Flux-Graph gegen die echte ComfyUI verproben**
  bleibt offen: (a) dass `DualCLIPLoaderGGUF`s `get_full_path("clip", …)`
  unsere `text_encoders`-Ordner-Konfig aus `extra_model_paths.yaml` findet
  (ComfyUIs `map_legacy` sollte `clip`→`text_encoders` mappen — geprüft im Code,
  nicht live); (b) dass `UnetLoaderGGUF` das GGUF in `diffusion_models/` sieht;
  (c) Steps/Scheduler/Guidance-Defaults an einem echten Flux-Render kalibrieren.
  Voraussetzung: entweder ein kleinerer Flux-Quant (Q4/Q5) im Katalog, oder das
  Preflight-Gate lässt ComfyUIs eigenes VRAM-Management ranprobieren.
- **Preflight-Gate ignoriert ComfyUIs `vram_mode`**: Settings dokumentiert
  „Low VRAM — offload aggressively (helps Flux on 16 GB)" als Feature, aber der
  Scheduler blockt einen zu großen Job schon VOR dem ersten ComfyUI-Request,
  unabhängig vom konfigurierten `vram_mode` — ComfyUIs eigenes Offloading
  bekommt nie eine Chance zu beweisen, dass es einen knapp-zu-großen Job doch
  schafft (langsamer, aber lauffähig). Ob das lohnt, hängt daran, wie gut
  ComfyUI-GGUFs `lowvram`-Pfad für GGUF-Diffusionsmodelle wirklich ist — nicht
  live geprüft.
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
- ~~Hermes `bash -l` / WSL2 / Adapter / Installer~~ → ✅ 5.4 (ADR-021): Adapter
  (`agent::hermes`, `hermes gateway` pro Session, erzwungene `config.yaml`,
  Bearer, `chat/stream`-SSE) + `uv`-Installer (`agent::hermes::install`) + UI-
  „Install Hermes"-Fluss stehen, gegen Fixtures verprobt. **Offen (manuell, wie
  Slice 4.0):** ein echter `hermes`-Lauf — fährt `#[ignore]`-Test
  `agent::hermes::install::tests::real_pinned_install` den Installer + `hermes
  postinstall` sauber durch? Stimmen die SSE-Event-Namen (`assistant.delta` vs.
  `message.delta`, `approval.request`, ob `/chat/stream` nach der Approval offen
  bleibt oder neu geöffnet werden muss)? Akzeptiert die echte Version die
  geratenen `config.yaml`-Sandbox-Keys (`security`/`permissions`/`tools`)? Ist
  `postinstall` (node/Browser/ffmpeg) den Umfang wert, oder wird Hermes
  „Advanced/optional"?
- `llama-server`-Tool-Calling: `--jinja` emittiert OpenAI-`tool_calls`, OpenCode
  parst sie (5.0-Stub-Test). ✅ 5.1a: `LlamaServerOptions.jinja` (default an) +
  `chat_template` → `--jinja` / `--chat-template` im Spawn, `[llama]`-Config +
  Settings-Toggle. **Offen:** ein echter Coding-Modell-Lauf (Qwen2.5-Coder-GGUF)
  → verlässliche `tool_calls`? + KV-Cache nicht zu hart quantisieren
  (`-ctk q4_0` schadet) → **manueller Agent-Smoke** — Prozedur + Kalibrier-
  Checkliste in [AGENT_MODELS.md](AGENT_MODELS.md) (`curl` gegen die Loopback-API,
  echter Qwen2.5-Coder-GGUF + echtes `opencode`). Adapter/Subsystem/API sind
  gegen Fixtures verprobt; offen ist nur der echte Modell-Lauf (wie Slice 4.0).
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
  parallele Sessions bleiben Post-MVP. `open` nimmt `first_message` optional —
  Zwischenzustand `Starting` nur kurz sichtbar.
- 5.5a Crash-Recovery: ✅ umgesetzt — `AgentRepo::recover_orphaned()` failt beim
  Start jede nicht-terminale Session (Prozess-pro-Session → kein echtes Resume).
  Checkpoint-basiertes Resume nur relevant, falls je ein In-Prozess-Runtime-
  Modell kommt (5.5c, vertagt).
- Agent-Sandbox-Niveau: ✅ **umgesetzt in 5.2** (config-level) — erzwungene
  OpenCode-`permission` (`edit`/`write` auf Workspace, `external_directory`
  read-only, `bash` ask, Netz-Tools aus) + `SpawnSpec.env_remove`/`SCRUBBED_ENV`.
  Details in [SECURITY.md](SECURITY.md). Vertagt (opt-in, eigener ADR): echte
  FS-/Prozess-Isolation (WSL2/Container/AppContainer), **Toolset-Whitelisting pro
  Profil** (`SessionSpec.toolset` wird noch nicht durchgesetzt), per-Kommando-
  Bash-Deny-Muster.
- Backup/Restore-Umfang: ✅ **umgesetzt in 5.5b** (`core::backup`). Da alle
  Agent-Daten (Profile, Sessions, Transkripte) in `aiwm.db` liegen, ist das
  Archiv nur `aiwm.db`-Snapshot + `config.toml` + Modell-Manifest (SHA-256,
  **nicht** die Dateien) — kein Profil-Ordner nötig. Import staged nach
  `<data>/.pending-import/`, `App::load` tauscht beim Neustart (alt →
  `*.pre-import`). Settings-Karte „Backup & restore".
- 5.5c (vertagt, Post-MVP-Politur): Kontext-Kompaktierung anzeigen, „Agent
  pausieren?"-Fluss (R8), Diagnostics-Zeile pro Agent.

## Phase 6 (Model-Manager v2) — ✅ abgeschlossen (Scheiben 6.0–6.9)
Scheibenplan + Research + Ergebnisse: [PHASE_6_PLAN.md](PHASE_6_PLAN.md).
ADRs 022–025. 371 Lib / 57 integ / 5 pytest.
- ~~6.0-Spike~~ ✅ (ADR-022 / ADR-024) · ~~6.1 `core::registry`~~ ✅ ·
  ~~6.2 Discovery-UI~~ ✅ · ~~6.3 Kompat-Engine v2~~ ✅ (ADR-016-Zusatz) ·
  ~~6.4 Download-Manager~~ ✅ (ADR-023) · ~~6.5 `core::bench`~~ ✅ ·
  ~~6.6 benchmark-gestützte Auto-Auswahl~~ ✅ · ~~6.7 Upgrade-Check~~ ✅ (ADR-025) ·
  ~~6.8 Aufräum-Reports~~ ✅ · ~~6.9 Tags + Discovery-Verlauf + Rate-Limit-Backoff
  + `HF_TOKEN` + Registry-Diagnostics~~ ✅
- **Offen aus 6.9 (bewusst verschoben / fallen gelassen):**
  - Benannte Modell-Collections mit Reihenfolge/Notiz — freie Tags decken die
    Nutzergeschichte; nur bei echtem Bedarf (`model_collections`-Entität +
    eigene CRUD-Oberfläche)
  - `ETag`/`If-None-Match` beim Registry-Cache-Refresh — verworfen: `304` spart
    nur Bandbreite, nicht das Rate-Budget (der Call zählt so oder so)
  - `HF_TOKEN` live anwenden statt neustart-pflichtig (wie `[llama]`/`[models]`)
- **Bleibt für Phase 6 / 4.0:** die 6.3/6.5-Estimator-Konstanten
  (`RUNTIME_OVERHEAD_MB`, `KV_ROUGH_MB_PER_1K_CTX`, `SPEED_REF_TPS`,
  `media_headroom_mb`) gegen echte `nvidia-smi`-Peaks kalibrieren →
  [HARDWARE.md](HARDWARE.md), zusammen mit dem 4.0-ComfyUI-Smoke

### Post-6.9: kategorisierter Models-Tab + Datenablage (ADR-026)
- ✅ `scripts/start.ps1`/`install.ps1`, ADR-026 (Datenablage portabel per
  Default), Models-Tab „Recommended models" (Image/Video/Chat/Code-Reiter,
  `core::model::FEATURED_MODELS`, Fit-Verdicts). Details:
  [MODELS.md](MODELS.md) „Kategorisierter Katalog".
- ✅ **Behoben:** Download-getriggerte Importe übergaben `import_model` immer
  `roles: []` (die `downloads`-Tabelle hatte keine `roles`-Spalte) — unsichtbar
  für `pick_for_role`, fatal für eine `coding`-Empfehlung. **Migration
  `0009_download_roles`** (`downloads.roles`, Komma-Liste) + Durchreichen bis
  zum Worker; `db::ModelRepo::set_roles` + `PUT /models/{id}/roles` für die
  nachträgliche Korrektur (Model-Library: Toggle-Chips). Code-Reiter hat jetzt
  denselben Ein-Klick-Download wie Chat; „Show download options" zeigt jede
  gefundene Quant (nicht nur die kuratierte); „Import a model" akzeptiert einen
  Link als Alternative zum lokalen Pfad. Details: [MODELS.md](MODELS.md).

### Post-6.9b: Bild/Video-„Stacks" (Basismodell + Pflicht-Begleiter)
- ✅ `core::model::catalog::{ModelStack, MODEL_STACKS}` — 4 kuratierte Stacks
  (SDXL, Flux, Wan 2.2, LTX), referenzieren bestehende `KnownModel`-Einträge
  per `member_ids` (keine Datenduplikation). `GET /models/stacks` /
  `list_model_stacks`, Image/Video-Reiter zeigt `StackCard`s mit „Download
  entire stack"-Knopf. Details: [MODELS.md](MODELS.md) „Stacks".
- **Offen:** LoRA-Dateien sind noch nicht Teil eines Stacks oder überhaupt des
  Katalogs — `KNOWN_MODELS` hat aktuell keinen einzigen LoRA-Eintrag. Sobald
  kuratierte LoRAs (z. B. ein Flux-Style-LoRA) ins Sortiment kommen, bräuchte
  `ModelStack` optionale/nicht-Pflicht-Mitglieder (LoRAs sind nie
  Pflichtbestandteil eines Setups, anders als VAE/Text-Encoder) — aktuell
  behandelt `member_ids` jedes Mitglied als Pflicht.

### Post-6.9c (2026-09-12): E2E-Review gegen echte Hardware — Scheduler, VRAM-Schätzung, ComfyUI, Agents
User-Anfrage: „no more quick fixes. validate everything e2e and generate
actual files videos images test code" — der reale `aiwm-cored` lief gegen die
echten installierten Modelle/Runtimes (ComfyUI, llama.cpp, Hermes, OpenCode)
statt nur gegen die Fixture-Test-Suite; jeder dabei gefundene Bug wurde
root-caused behoben (Commit `280018d`, 411 lib + 58 integ + 5 pytest).
- ✅ **Queue-Starvation behoben**: `db::jobs::next_runnable()` wählte den
  global ältesten `queued`/`blocked`-Datensatz unabhängig vom State — ein
  einziger dauerhaft blockierter Job verhungerte jeden anderen Job für immer.
  Queued-Jobs gehen jetzt immer vor blocked; ein blockierter Job wird erst
  erneut versucht, wenn nichts anderes läuft.
- ✅ **VRAM-Schätzung**: `media_headroom_mb` (Sampler-Aktivierungen fürs
  *Basis*-Modell) wurde für JEDEN Medien-Kind angewandt — ein 235-MB-CLIP-L
  landete bei ~2,8 GB geschätztem Bedarf. Jetzt nur für Checkpoint/Diffusion/
  Video-Kinds; die 5 schon importierten Begleitdateien in der DB korrigiert.
  Zusätzlich: der Scheduler-Bedarf für einen Bild/Video-Job war ein fixer
  Wert pro Modell — ein winziger 256×256/9-Frame-Testclip brauchte laut
  Schätzung genauso viel wie das größtmögliche Rendering und wurde entsprechend
  ebenso oft blockiert. Skaliert jetzt den Headroom-Anteil mit den echten
  Pixel-/Frame-Zahlen des Jobs.
- ✅ **Stack-Fit berücksichtigt jetzt die Summe**: `GET /models/stacks` gab
  pro Mitglied einen Fit zurück, die UI nahm den schlechtesten einzelnen Wert
  — jedes Flux-Mitglied für sich sieht grün/gelb aus, aber ein Render braucht
  alle vier gleichzeitig im VRAM (~17 GB auf einer 16-GB-Karte). Neues
  `ModelStackDto.fit` = Verdict über die **Summe** aller Mitgliedsgrößen.
- ✅ **ComfyUI-Absturz beim ersten echten Start behoben** — siehe „Vor Phase
  3/4" oben (cu130-Eintrag): `custom_nodes` fehlte unter `--base-directory`,
  gefixt per Junction. **Erster erfolgreicher echter Render** (Wan-Clip, MP4)
  dieses Projekts.
- ✅ **Hermes/OpenCode — erste echte Läufe, mehrere reale Bugs gefunden**:
  fehlendes `aiohttp` verhinderte Hermes' API-Server-Start (Timeout beim
  Session-Öffnen); die echte Hermes-API verschachtelt `session.id` anders als
  angenommen; Agent-Sessions liefen mit dem Chat-Kontext-Limit (8192 Token)
  statt dem vollen Modell-Kontext (ein Tool-Calling-System-Prompt allein
  braucht 20–25k+); und der eigentliche Grund hinter dem mitten im Lauf
  abgebrochenen Chess-Test: beide Adapter nutzten denselben HTTP-Client mit
  30-Sekunden-Timeout für ihren langlebigen SSE-Stream — jeder Turn über 30 s
  killte den Stream, was die Drain-Loop als „Runtime tot" las und das Modell
  mitten in der Generierung freigab. Jeder Adapter nutzt jetzt einen
  separaten, ungebremsten Client fürs Streaming.
- **Weiterhin offen (nicht code-fixbar ohne größere Arbeit):** Hermes' eigene
  Mindestanforderung von 64.000 Token Kontext — sowohl fürs Hauptmodell als
  auch für sein „Auxiliary Compression Model" — ist unabhängig vom obigen Fix;
  unsere aktuellen Modelle (Qwen2.5-*-14B, nativ 32K) erreichen das nicht.
  Bräuchte entweder ein Modell mit echtem ≥64K-Kontext oder eine echte
  YaRN/RoPE-Scaling-Unterstützung in `LlamaServerOptions` (existiert noch
  nicht). OpenCode hat diese Anforderung nicht und funktioniert nach dem
  Kontext-Fix.
- **Models-Tab-Politur**: „Import a model" stand vor dem Katalog — ein
  Erstnutzer sah ein Pfad-Feld, bevor ihm gesagt wurde, was er installieren
  soll. Katalog steht jetzt zuerst. „Set import type" (aus Katalog/Discover)
  änderte ein Dropdown weiter unten ohne jedes sichtbare Feedback — scrollt
  jetzt dorthin und blinkt kurz auf.

### Post-6.9b: VRAM scheduling was blind to other GPU applications, plus UI polish
User report: "all picture and video generation fails due to block or fail" — traced to
`HybridScheduler` planning purely against the config/GPU-total budget minus its own
bookkeeping, with **no live check of the GPU's actual free VRAM**. On a machine where
other applications (browser, games, desktop compositor) already hold several GB, the
scheduler either wrongly reported `Blocked` (bad bookkeeping) or wrongly said
`LoadThenRun` and then hit a real CUDA out-of-memory `failed` — both symptoms the user
saw, from the same root cause. **Also clarified:** "blocked" has never been a content
filter — grepped the whole codebase, there is no NSFW/safety-checker/censorship code
anywhere; `Blocked` is purely `core::scheduler::Decision` (not enough VRAM right now).
- ✅ `scheduler::PlanRequest` gained `live_free_vram_mb: Option<u64>`, populated by
  `JobEngine` from the live NVML telemetry reading (`Sampler`, already wired to the
  engine for `bench`). `HybridScheduler::effective_free_mb` caps the budget-derived
  free figure by this live reading, and the eviction path now only evicts a resident
  model when doing so would **actually** close the gap (`free + victim_vram >=
  needed`) rather than evicting for nothing. `blocked_reason` names the live free MB
  and points at "other running applications" when that's the actual constraint. +3
  scheduler unit tests.
- ✅ **UI**: a `blocked` job isn't running — it just waits for room, and can wait
  forever if none frees up. Image/Video/Chat all showed a bare `"blocked…"` spinner
  with no explanation, and the Generate/Send button stayed disabled until the tab was
  switched away and back (which only *looked* like a fix — the stuck job was still
  sitting there server-side). Fixed: the Result/turn panel now shows `error_text`
  (the human-readable reason) for a blocked job, and the submit button only disables
  on a genuinely *running* job — a blocked one no longer locks the form.
- ✅ **Image/Video gallery overlap**: `.image__form { position: sticky; top: 0 }`
  visually overlapped the Gallery card once the page scrolled past the form (grid
  track sizing edge case, confirmed by injecting extra gallery items and scrolling in
  the browser preview). Removed the sticky positioning.
- ✅ **Chat has no history** — `Chat.tsx`'s `turns` was pure local component state,
  wiped every time the tab unmounted (`App.tsx` conditionally renders `{tab ===
  "chat" && <Chat />}`) or the app restarted. Past chat turns were already sitting in
  the `jobs` table (`job_type: "chat"`, `result` progressively updated) exactly like
  Image/Video's Gallery — just never read back. Added a one-time hydration effect
  that rebuilds `turns` from `useJobs()` on mount, same data source the Image/Video
  galleries already used.
- ✅ **Video "start frame" needed a typed path** — added a native file-picker
  ("Browse…") next to the path field via `@tauri-apps/plugin-dialog` (new dependency,
  JS + `tauri-plugin-dialog` Rust crate + `dialog:default` capability — mirrors the
  existing `tauri-plugin-opener` wiring). Chose a picker over real OS drag-and-drop:
  Tauri's native drag-drop is window-scoped (needs hit-testing against the target
  element and `dragDropEnabled` config), while a picker is the simpler, robust option
  the user also explicitly named as acceptable. The manual path field stays for power
  users / scripted paths. Image capability has no img2img input yet, so nothing to
  wire there.
- +3 lib tests → **399 lib + 58 integ + 5 pytest**. check.ps1 green; browser smoke
  (dev-mock): chat turn survives a tab switch, Image gallery no longer hides behind
  the form after scrolling, Browse… button no-ops cleanly outside Tauri, no new
  console errors on any tab.

## Story Studio (noch nicht begonnen)
Charakter-/Szenen-basierter Story-Builder (User: "lets plan actuall a friggin
lot" → ausführliches Konzept + interaktiver Mockup-Artifact entstanden, aber
**noch kein Code, kein Doc im Repo** — dieser Eintrag ist der einzige
Repo-seitige Anker, damit das Konzept nicht mit dem Chat-Verlauf verloren
geht). Kurzfassung: Story (Setting/Ära, Art-Style, Prämisse) → Character
(Traits/Backstory, gelockte Referenz-Portrait-Job-ID für konsistentes
Aussehen, Inventar, Beziehungen) → Character-Log (append-only "Brain" —
Adventures/Steps/Inventory/Personen) → Location (eigenes gelocktes
Referenzbild) → Scene (Narrative + Dialogzeilen + Teilnehmer + Location +
Redline) → Scene-Image (mehrere pro Scene, eine kanonische) → Assembly
(Auswahl aus Scenes+Images fürs Bündeln/Export). Konsistenz-Mechanismus:
IP-Adapter (Referenzbild-Konditionierung, kein Training) als MVP, LoRA pro
Charakter später — beides noch nicht in AIWM installiert (neuer ComfyUI-
Custom-Node-Pack nötig). Dialog immer UI-Overlay-Text, nie ins Bild gebacken
(explizite Anforderung — KI-Text in Bildern ist unzuverlässig). Geplante
Phasen: 1 Text+Bild-MVP → 2 IP-Adapter-Konsistenz → 3 Assembly/Export → 4
LoRA/ControlNet-Stretch. UI-Idee: neuer "Stories"-Tab, SectionNav-Rail
(Characters/World/Timeline/Assembly), Timeline als Haupt-Scroll-Fläche mit
Scene-Cards, persistentes Character-Sheet als Drawer.
- **Offene Frage vom User (2026-09-14):** lohnt sich eine bestehende
  Open-Source-Workflow-/Orchestrierungs-Engine für die mehrstufige
  Story-Pipeline (Charakter-Erstellung → Backstory → Scene → Dialog →
  Bild-Gen → Narration, mit Redo/Regenerate pro Schritt), statt das
  Schritt-für-Schritt-Sequencing selbst zu bauen? Noch nicht evaluiert.
  Kandidaten zu prüfen, wenn's soweit ist: etwas Leichtgewichtiges das sich
  in einen Rust/Tauri-Prozess einbetten lässt (kein separater Server/Node
  wie n8n) — z. B. ein reiner State-Machine-/DAG-Executor als Rust-Crate,
  oder ComfyUI selbst als Graph-Engine zweckentfremden (Story-Schritte als
  Nodes) — beides ungeprüft, nur Erstgedanken.
- Vor Implementierung offen (aus dem Konzeptions-Gespräch, unbeantwortet):
  Phase-1-Umfang (Text-first vs. Konsistenz-von-Tag-1), NPC-Gewicht (volles
  Character vs. leichtgewichtig), Export-Priorität (HTML-Scroll zuerst vs.
  PDF/CBZ), Reaktion auf das Mockup-Layout.

## Lokale KI-Trainings-Engine (noch nicht begonnen, in Brainstorming)
User-Leitprinzip (2026-09-15, wörtlich wichtig): AIWM soll ein **lokales
All-in-one-Tool** bleiben — eigenes Training auf **eigenen Daten, jeder Art**
soll genauso leicht zugänglich sein wie die bestehenden Image/Video/Chat-
Features, nicht ein Fremdkörper. D.h. dieses Feature ist kein einmaliger
"Video-zu-LoRA"-Sonderfall, sondern der erste Schnitt einer generischeren
"bring your own dataset, train locally"-Fähigkeit — Architektur entsprechend
offen halten (nicht hart auf Video verdrahten), auch wenn der erste Schnitt
bei Video anfängt.

Ursprüngliche Anforderung (User, roh): Pfad zu einem Ordnerbaum voller
Videomaterial (`E:/Data/<VieleOrdner>/*.mp4`, Ordnername = Art-Style/Artist)
→ automatisch zu Trainingsdaten für ein bestehendes Checkpoint/LoRA
(Bild- oder Video-Generierung) aufbereiten, dann **ein Klick** = Trainingslauf
(Stunden), danach das neue Modell testen. Explizite Nutzer-Beobachtungen:
Einzelbilder sind bessere Trainingsdaten als Rohvideo; Unschärfe/Duplikate
schaden; das Tool soll das automatisch selbst erkennen und bereinigen;
ein lokales Vision-Modell soll pro Frame beschriften ("was ist da zu sehen");
wenn das Modell sich bei einem Frame unsicher ist, soll es Frame X gegen
Frame X+5 vergleichen (zeitlicher Kontext), um zu verstehen was gerade
passiert; der Nutzer soll die Auto-Captions "von Zeit zu Zeit" manuell
korrigieren können, bevor das Training losläuft.

**Erkannt: das ist mindestens zwei große, größtenteils unabhängige
Teilsysteme** (Skill-Vorgabe: bei Multi-Subsystem-Anfragen erst zerlegen,
nicht direkt in Detailfragen einsteigen) — getrennt spezifizieren, aber so
bauen dass Teilsystem 1 auch ohne Teilsystem 2 nützlich ist:
1. **Dataset-Prep-Pipeline**: Ordner einlesen → Video-Decode/Frame-Extraktion
   (ffmpeg) → Schärfe-/Duplikat-Filter → lokales VLM captioned jedes Frame →
   bei Unsicherheit Vergleich Frame X vs. X+N für zeitlichen Kontext →
   Kuratier-UI zum manuellen Nachbessern der Captions/Auswahl. Braucht einen
   **neuen Runtime-Typ** (Vision-Language-Model, existiert in AIWM noch gar
   nicht — Kandidaten ungeprüft: Florence-2, JoyCaption, Qwen2-VL, LLaVA;
   welches lokal auf 16 GB gut läuft ist offene Recherche).
2. **Trainings-Orchestrator**: aus dem kuratierten Datensatz einen echten
   LoRA-/Fine-Tune-Lauf fahren (stundenlang), Fortschritt anzeigen, danach
   das Ergebnis ins bestehende Image/Video-Modell-System einhängen zum
   Testen. Braucht einen externen Trainings-Unterbau — für Bildmodelle
   (SDXL/Flux) existiert reife Tooling (z. B. kohya-ss/sd-scripts), für
   Video-Modelle (Wan/LTX) ist die Lage deutlich unreifer/unstandardisiert —
   beides noch nicht recherchiert/entschieden.
- **Offene Frage vom Assistenten an User (gestellt 2026-09-15, unbeantwortet):**
  Priorität zuerst auf Bild-LoRA-Training (reife Tooling-Lage) oder
  Video-LoRA-Training (Kernwunsch laut Beispiel, aber unreifere Tooling-Lage),
  oder soll die Dataset-Pipeline von Tag 1 an beide Zielarten bedienen?
- Noch nicht diskutiert: welches lokale VLM für Captioning, ob/wie das
  Vision-Runtime-Konzept sich in die bestehende `RuntimeAdapter`-Architektur
  einfügt (analog zu llama.cpp/ComfyUI/Colibri/TTS), Umfang der Kuratier-UI
  (reine Text-Edits vs. Bild-Grid mit Batch-Aktionen), ob Trainingsläufe
  durch den bestehenden Job-Scheduler laufen (Stunden-lange VRAM-Reservierung
  wäre ein neuer Anwendungsfall für den Scheduler) oder als eigener,
  scheduler-externer Prozess.
- **Ausdrücklich noch keine Implementierung** — laut `brainstorming`-Skill
  erst Design/Spec + User-Freigabe, dann `writing-plans`. Dieser Eintrag ist
  nur der Parkplatz-Anker, falls das Brainstorming-Gespräch unterbrochen wird.

## Offen / später zu entscheiden
- App-Selbst-Update offline (manueller Installer + Signaturprüfung angenommen)
- Parallele Jobs: Policy verfeinern (klein-LLM + Upscale gleichzeitig)
- 2. GPU zukünftig: `GpuId` im Datenmodell vorsehen, Multi-GPU-Scheduling später
- LAN-/Remote-Zugriff (opt-in, mit Auth) — frühestens nach Phase 6
- Relighting, Generative Fill, Video-Restoration
- Plugin-/Adapter-Plattform für Dritt-Runtimes (erst wenn interne Adapter stabil)
- ✅ **RTX Video Super Resolution als ComfyUI-Node — Upscale-Schritt für
  Image/Video-Tab umgesetzt** (2026-09-15): offizieller `Comfy-Org/
  Nvidia_RTX_Nodes_ComfyUI` (Apache-2.0) wird als zweiter Custom-Node-Pack
  installiert (analog `ComfyUI-GGUF`, gleicher `custom_nodes/`-Junction).
  Neues `job_type=upscale` läuft — wie Image/Video — als echter
  ComfyUI-Job durch die JobEngine (VRAM-Slot, Queueing), nicht synchron wie
  der Audio-Clean-Pass. UI: "Upscale"-Button auf der Result-Karte in
  Image/Video, sendet den fertigen Job als neue Quelle. Der Node nutzt
  ComfyUIs neueres V3-Schema (`io.DynamicCombo` für `resize_type`) statt der
  klassischen `NODE_CLASS_MAPPINGS`-Registrierung — die verschachtelten
  Keys (`resize_type.scale` / `resize_type.width`/`height`) wurden zunächst
  nur aus ComfyUIs `_io.py`/`execution.py`-Quellcode hergeleitet, dann aber
  **direkt gegen den echten `_io.py`-Quellcode am gepinnten Tag (v0.34.0)
  gegengeprüft** (`create_input_dict_v1` baut den Wire-Key explizit als
  `f"{parent_id}.{nested_id}"`) — die Form ist damit bestätigt, nur ein
  echter End-to-End-Render mit installiertem Node steht noch aus (gleiche
  Kategorie wie die offenen Flux/LTX-Live-Checks oben). Nicht zu verwechseln mit "DLSS 5": das ist eine noch nicht
  final released Game-Rendering-Technologie (GTC 2026 angekündigt), deren
  populärste GitHub-Repos "geleakte" NVIDIA-Binaries per DLL-Injection in
  beliebige Spiele patchen — bewusst nicht angefasst (Sicherheits-/
  Lizenzrisiko, und architektonisch eh nicht für Batch-Datei-Upscaling
  gedacht, sondern fürs Echtzeit-Rendering einer Game-Engine).
