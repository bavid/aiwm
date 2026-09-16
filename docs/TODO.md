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
- ✅ **`core::compat` (2.6): `RUNTIME_OVERHEAD_MB` gegen eine echte
  `llama-server`-Ladung kalibriert** (2026-09-16): echter Chat-Job auf der
  echten RTX 4080 Super — Mistral-Small-3.2-24B-Instruct, IQ3_M-GGUF
  (10650964832 Bytes, 40 Layer/5120 Hidden/32 Heads/8 KV-Heads), 8192-Token
  Chat-Default-Kontext, `-ngl 999` (volles GPU-Offload). Per NVML gemessen:
  Idle-Desktop-Baseline 909 MB → Peak 12599 MB während der Ladung, also echtes
  Delta 11690 MB. Die alte 650-MB-Konstante sagte 12407 MB voraus (Gewichte
  10157 + KV 1600 + 650) — **717 MB zu konservativ**: Gewichte+KV allein
  (11757 MB) lagen schon über dem tatsächlichen Verbrauch, der reale feste
  Overhead war also nahe null, nicht 650 MB. Auf **350 MB** gesenkt (reale,
  positive Sicherheitsmarge in Höhe eines typischen CUDA-Kontexts, aber ohne
  den alten 717-MB-Puffer). Einzelner Realwert — das einzige zweite echte
  GGUF-Chat-Modell auf dieser Maschine (Qwen2.5-7B-Instruct, F16) passt mit
  ~15,3 GB gar nicht erst auf die 16-GB-Karte und lieferte daher keinen
  zweiten Datenpunkt. Test `real_mistral_load_pins_the_recalibrated_overhead_
  and_stays_above_measured_actual` pinnt beides (Schätzung bleibt ≥ real
  gemessen, Overshoot deutlich unter den alten 717 MB). **Nicht kalibriert:**
  `KV_ROUGH_MB_PER_1K_CTX` (die grobe `160 MB/1K ctx`-Reserve) — beide echten
  GGUF-Modelle auf dieser Maschine tragen vollständige Arch-Metadaten
  (`n_layers`/`n_embd`/`n_heads`/`n_kv_heads`) und trafen daher immer den
  präzisen `kv_bytes_per_token`-Pfad, nie den groben Fallback; der bräuchte
  ein echtes Modell mit unvollständigem GGUF-Header, das hier nicht verfügbar
  war. KV-Cache-Quantisierung (`-ctk`/`-ctv` q8/q4) weiterhin offen als Option.
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
- ~~Bild-Job (3.4/3.5): MVP pollt `/history` (750 ms) + die UI pollt `jobDetail`
  (700 ms). `/ws`-Fortschritt (`progress` / `executing` / `executed`) an die UI
  streamen → echter Fortschrittsbalken statt nur „preparing…".~~ → ✅
  `core::progress::ProgressHub` + `ComfyUiAdapter::generate_media` verbindet
  sich zusätzlich zum bestehenden `/history`-Poll (der weiter die eigentliche
  Fertigstellung erkennt) auf ComfyUIs eigenes `/ws?clientId=…` und published
  `progress`/`executing`-Events dorthin; `GET /ws/jobs/{id}` (real, kein
  Polling) streamt sie an die UI weiter, die per rohem `WebSocket` direkt auf
  den Loopback-Server verbindet (kein Tauri-IPC nötig, wie schon bei den
  `<img>`/`<video>`-Output-URLs). Bild **und** Video zeigen jetzt einen
  echten Fortschrittsbalken (Schritt/Gesamt, Prozent) statt nur der letzten
  Log-Zeile, wenn ComfyUI das Event liefert. Offen bleiben SDXL-Refiner-Pass
  und Batch-Größe > 1 (unverändert vertagt).
- ~~Image-UI (3.5): Galerie hat keine Paginierung / kein Löschen / keinen
  Download-Button, kein Bild-Zoom/Lightbox, keine Prompt-History, keine
  Style-Presets. Der Download-Button ist heikel im Tauri-Sandbox (`<a download>`
  inert) — bräuchte einen `save`-Dialog-Command.~~ → ✅ Paginierung (24/Seite),
  ein `Lightbox`-Zoom (Esc/Backdrop/×, ◀/▶) und ein echter Download-Button
  (neuer `save_job_output`-Tauri-Command kopiert die Bytes, nachdem
  `@tauri-apps/plugin-dialog`s `save()` das Ziel gewählt hat — das Schreib-
  Gegenstück zum bestehenden `open()`-„Browse…"-Ladepfad) sind jetzt für
  **Bild und Video** da. Löschen gab es schon (`gallery__delete`).
  Prompt-History und Style-Presets bleiben offen — eigene kleine Slices bei
  Bedarf.
- Output-Retention (3.7 zeigt nur einen Hinweis): ~~automatisches Aufräumen /
  Größenlimit / „Ordner öffnen"-Button für `<local_root>/outputs`.~~ → ✅
  `core::cleanup::outputs` (Alters-Tage und/oder Gesamt-MB-Limit, beide `0` =
  aus) + `POST /outputs/cleanup` (liest `config.toml` frisch, kein Neustart
  nötig) + ein bewusst best-effort Sweep beim Start, wenn eine Policy gesetzt
  ist. Settings → „Generated media" hat die zwei Felder + einen „Clean up
  now"-Knopf (ausgegraut ohne gespeicherte Policy). **Bewusste Entscheidung**
  (dokumentiert in `cleanup::outputs`' Moduldoc): ein Sweep löscht nur die
  Datei, nie die Job-DB-Zeile — `job_output_path` behandelt eine fehlende
  Datei bereits als „kein Output" (identisch zu einer von Hand gelöschten
  Datei heute), das ist das einfachere/sicherere Verhalten als zusätzlich
  Zeilen unter einer evtl. offenen Detailansicht zu löschen.
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
  **Real gegengeprüft, aber nicht angepasst (2026-09-16):** echte Wan-2.2-
  TI2V-5B- und LTX-Video-2B-Renders auf der echten RTX 4080 Super liefen
  beide durch (128×128, 5/9 Frames, 4 Steps, ~30–48 s) und die Warnung feuerte
  korrekt bei echter RAM-Knappheit („~12454 MB short" / „~5953 MB short"). Die
  `VIDEO_RAM_SLACK_MB`-Konstante selbst ließ sich auf dieser Maschine aber
  **nicht sauber kalibrieren**: `sysinfo`/NVML messen System-weites RAM, und
  diese Dev-Maschine hatte parallel eine eigene Claude-Code-Session, Cargo-
  Builds, Browser etc. laufen — der `ram_used_mb`-Median schwankte um
  ±2 GB in 10-Sekunden-Fenstern allein durch fremde Prozesse, weit über dem
  Signal, das ein einzelnes Video-Modell beisteuert. Anders als beim VRAM
  (praktisch exklusiv durch AIWMs eigene Prozesse belegt, siehe die
  `core::compat`-Kalibrierung oben) lässt sich der Modell-eigene RAM-Anteil
  aus Gesamt-System-Telemetrie auf einer geteilten, ausgelasteten Maschine
  nicht verlässlich isolieren — die Konstante unverändert gelassen, statt sie
  ins Blaue zu raten. Bräuchte entweder eine ruhige Maschine ohne
  Fremdlast, oder Pro-Prozess-RSS-Messung (RSS von `python.exe`/ComfyUI statt
  `sysinfo::available_memory()`), um den echten Modell-Footprint sauber vom
  Rest des Systems zu trennen.
- ~~**Output-Retention (4.5 macht sie nur sichtbar):** `about.outputs_bytes` +
  „reveal"-Knopf sind da, aber es gibt weiter **kein** automatisches Aufräumen,
  Größenlimit oder „X löschen"-Knopf.~~ → ✅ siehe der Output-Retention-Eintrag
  weiter oben (`core::cleanup::outputs`, `POST /outputs/cleanup`, Settings-UI).
  Das `GET /jobs/{id}/output`-RAM-Streaming-Thema unten bleibt separat offen.
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
- **Video-UI-Politur (4.3):** ~~(a) echter Prozentbalken statt letzter
  Event-Zeile — braucht `/ws`-Fortschritt vom Core (`progress`/`executing`),
  gleiche Baustelle wie beim Bild (3.4/3.5);~~ → ✅ siehe den Bild-Job-Eintrag
  oben (`core::progress`, `GET /ws/jobs/{id}`) — gilt für Bild und Video
  gleichermaßen. (b) echtes Poster-Frame (erstes Frame per ffmpeg/`av`
  extrahieren und als `poster=` setzen) statt `<video preload=metadata>`
  bleibt offen; (c) die Minuten-Schätzung ist eine grobe Formel
  (`frames × steps × pixel`) — nach 4.0 an echten Wan-Läufen kalibrieren;
  ~~(d) Galerie: Lightbox, Download-Knopf (Tauri-`save`-Dialog), Retention /
  Löschen, Paginierung — wie bei der Bild-Galerie offen.~~ → ✅ siehe den
  Image-UI-Eintrag oben — `Lightbox`/Download/Paginierung sind für Bild
  **und** Video erledigt, Retention ist global pro `<outputs_dir>` (nicht
  pro Studio), Löschen gab es schon.
- **`lib/dev-mock.ts` (4.3)** ist minimal — nur die Kommandos, die die Studios
  brauchen. Wenn mehr Tabs im Browser getestet werden sollen, die fehlenden
  Kommandos ergänzen (es warnt in der Konsole bei unbehandelten).
- ComfyUI-Optionen (3.7): weitere sinnvolle Flags (`--fast`,
  `--use-split-cross-attention`) könnten dazu, wenn echte Flux-Läufe zeigen
  was fehlt. ~~`vram_mode` ändert die **laufende** Runtime nicht (Neustart) —
  ein `ComfyUiAdapter::set_options` + Server-Neustart wäre der saubere
  Live-Weg~~ → ✅ **`ComfyUiAdapter::set_options` (2026-09-16)**: ein
  `[comfyui]`-Config-Save löst jetzt einen echten, verwalteten Neustart aus
  (denselben `RuntimeSupervisor::stop` + `ensure_server_locked`-Pfad, den
  `load_model`/`stop` schon nutzen) statt nur `config.toml` zu überschreiben
  und auf den nächsten App-Neustart zu warten — ComfyUI selbst hat kein
  Live-Reconfigure (`--<mode>vram` & Co. werden einmalig beim Start gelesen,
  gegen den echten v0.34.0-Quellcode geprüft: `comfy/model_management.py`
  setzt `vram_state`/`set_vram_to` einmalig aus `cli_args.args`, keine
  HTTP-Route rührt sie danach an). Ein Server, den wir nur *attached* haben,
  wird nicht angefasst (nicht unser Prozess) — dort greifen die neuen
  Optionen erst beim nächsten selbst gestarteten Server. Dabei **einen
  echten, live-verifizierten Bug gefunden**: `VramMode::NormalVram` mappte
  auf `--normalvram` — dieses Flag **existiert nicht** in ComfyUI (weder
  v0.34.0 noch aktuell; die echte `vram_group` kennt nur `--gpu-only` /
  `--highvram` / `--lowvram` / `--novram` / `--cpu`). Ein echter Start mit
  `--normalvram` lässt `main.py` sofort mit `error: unrecognized arguments:
  --normalvram` abbrechen (gegen den echten, installierten v0.34.0 verifiziert)
  — hätte also jeden `NormalVram`-Start crash-loopen lassen, bis der
  Supervisor aufgibt. Gefixt: `NormalVram` gibt jetzt kein Flag mehr aus
  (= ComfyUIs eigener Default, wie `Auto`). `LlamaServerOptions`-Live-Apply
  (llama.cpp-Pendant) bleibt weiterhin offen — siehe „`LlamaServerOptions`
  über die Settings-UI konfigurierbar machen" oben; kein Analogie-Fix hier,
  da noch keine `set_options`-Grundlage auf der llama.cpp-Seite existiert.
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

## Story Studio (Phase 1 umgesetzt — 2026-09-16)
Charakter-/Szenen-basierter Story-Builder. Kurzfassung des Gesamtkonzepts:
Story (Setting/Ära, Art-Style, Prämisse) → Character (Traits/Backstory,
gelockte Referenz-Portrait-Job-ID für konsistentes Aussehen, Inventar,
Beziehungen) → Character-Log (append-only "Brain") → Location (eigenes
gelocktes Referenzbild) → Scene (Narrative + Dialogzeilen + Teilnehmer +
Location + Redline) → Scene-Image (mehrere pro Scene, eine kanonische) →
Assembly (Auswahl aus Scenes+Images fürs Bündeln/Export). Geplante Phasen:
1 Text+Bild-MVP → 2 IP-Adapter-Konsistenz → 3 Assembly/Export → 4
LoRA/ControlNet-Stretch.

**Phase 1 (text + plain image, keine Konsistenz-Maschinerie) ist gebaut:**
- Datenmodell komplett: `core/migrations/0013_stories.sql` +
  `core/src/db/stories/{mod,characters,npcs,locations,scenes,scene_images}.rs`
  — Story/Character/Npc/Location/Scene/SceneImage, normalisierte
  `character_relationships` und ein append-only `character_logs` (auto-append
  bei jedem neu hinzugefügten Scene-Teilnehmer, kein Duplikat beim Re-Save).
  Cascade-Deletes durchgängig; `scene_images` erzwingt "genau ein kanonisches
  Bild pro Scene" über einen partiellen Unique-Index. 37 neue lib-Tests.
- API-Wiring nach bestehendem Muster (dto.rs → handlers.rs → http.rs-Routen
  **und** Tauri-Commands in `src-tauri/src/lib.rs`, identisch zu
  Sessions/Documents): ~30 Handler, Scenes werden als `SceneDetailDto`
  (Scene + Participants + Dialogue + Images) zusammengesetzt, analog zu
  `job_detail`. Ein HTTP-Integrationstest deckt den vollen Flow ab
  (Story → Character → Scene → Character-Log-Auto-Append → Scene-Image →
  cascade-delete).
- Generierung nutzt die bestehende Image-Capability 1:1: `ui/src/features/
  stories/generateImage.ts` submitted einen normalen `job_type=image`-Job
  (gleiche Defaults wie Image-Tab), der Prompt kommt aus
  `storyPrompts.ts` (Story-Art-Style + Character/Location/Scene-Felder).
  Kein neuer Node, keine neue Capability.
- UI: neuer "Stories"-Tab (`ui/src/features/stories/`), folgt dem
  `hidden`-statt-Conditional-Mount-Muster aus `App.tsx`. `SectionNav`-Rail
  (Characters/World/Timeline/Assembly-Stub, Assembly bewusst leer für
  Phase 3), Timeline als vertikaler Scene-Card-Scroll mit Dialogzeilen als
  echtem UI-Overlay über dem Bild (nie ins Bild gebacken), persistentes
  Character-Sheet als Drawer (Portrait, Traits/Backstory editierbar,
  Inventar, Beziehungen, Character-Log) — aus jeder Section erreichbar,
  wie explizit gefordert. `dev-mock.ts` hat einen vollständigen In-Memory-
  Mock inkl. einer vorgefüllten Beispiel-Story für die Vorschau.
- Verifiziert: `cargo fmt`/`clippy --workspace --all-targets -D warnings`/
  `test --workspace` grün (Rust-Datenmodell + API); `tsc --noEmit`,
  `eslint .` und `vite build` grün (UI). Live im Browser (dev-mock)
  gegengeprüft: Story-Picker, Section-Rail, Timeline-Card mit
  Dialog-Overlay funktionieren wie vorgesehen.

**Bewusst NICHT gebaut (Phase-1-Scope-Cuts, siehe Auftrag):**
Charakter-Konsistenz (IP-Adapter/LoRA — Phase 2, braucht neuen ComfyUI-
Custom-Node-Pack), Assembly/Export-Logik (Phase 3, nur Stub-Sektion),
NPCs bewusst leichtgewichtig (kein volles Character-Schema), Szenen-
Reorder-UI/-Endpoint (Timeline ordnet sich aktuell nur nach Erstellreihenfolge
— `position` ist im Schema vorhanden, ein Reorder-Endpoint kann später ohne
Migration ergänzt werden), die "story-scene"-PromptAssistant-Erweiterung
(im Auftrag als "nice-to-have, not mandatory" markiert), der spätere
Workflow-Diagramm-View ("wer hat mit wem gesprochen") — das Datenmodell
(normalisierte `scene_participants`/`scene_dialogue_lines`/`position`)
unterstützt das aber bereits ohne Restrukturierung.

- **Offene Frage vom User (2026-09-14, weiterhin unbeantwortet):** lohnt
  sich eine bestehende Open-Source-Workflow-/Orchestrierungs-Engine für die
  mehrstufige Story-Pipeline (Charakter-Erstellung → Backstory → Scene →
  Dialog → Bild-Gen → Narration, mit Redo/Regenerate pro Schritt), statt
  das Schritt-für-Schritt-Sequencing selbst zu bauen? Noch nicht evaluiert;
  für Phase 1 nicht nötig gewesen (nur ein einzelner Image-Job pro Schritt).
- Für Phase 2/3 offen: IP-Adapter-Node-Pack-Auswahl, Assembly-Format-
  Priorität (HTML-Scroll zuerst vs. PDF/CBZ), ob/wie ein Reorder-Endpoint
  für die Timeline gebraucht wird, sobald Nutzer:innen wirklich Szenen
  nachträglich umsortieren wollen.

## Lokale KI-Trainings-Engine (Teilsystem 1 ✅ umgesetzt — 2026-09-16, Teilsystem 2 offen)
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
  oder soll die Dataset-Pipeline von Tag 1 an beide Zielarten bedienen? — für
  Teilsystem 1 irrelevant geworden (die Pipeline bedient beide Zielarten
  gleichermaßen, siehe unten), bleibt aber offen für Teilsystem 2.

### Teilsystem 1 — Dataset-Prep-Pipeline: ✅ umgesetzt (2026-09-16)

Vollständig gebaut und getestet, alle Gates grün (`cargo fmt`/`clippy -D
warnings`/`test` sauber, sidecar `pytest`/`ruff` sauber, UI `tsc`/`eslint`
sauber, live im Browser gegen den dev-mock verifiziert — Pipeline-Lauf,
Kuratier-Grid mit Caption-Edit/Exclude, Export alle im Screenshot bestätigt).

- **Ingest** (`core/src/capability/dataset/ingest.rs`): `walk_dataset_root`
  läuft rekursiv über den Root-Ordner; jeder *unmittelbare* Unterordner wird
  zum Tag; `.mp4` (Video) und `.png`/`.jpg`/`.jpeg`/`.webp` (bereits Frame)
  werden erkannt, deterministisch sortiert.
- **Frame-Extraktion** (`extract.rs`): `ffmpeg` (Auflösung analog
  `sidecar::resolve_uv` — PATH, dann WinGet-Links) sampled Video auf
  konfigurierbare fps (Default 1.5, Bereich 0.1–10). Ein Standbild braucht
  keine Extraktion — es ist bereits ein Frame (Nutzer-Prinzip "jede Art
  Daten"). `ffmpeg` selbst war noch keine Abhängigkeit irgendwo im Repo,
  ist aber auf dieser Maschine bereits vorhanden (WinGet) und wird auch von
  ComfyUIs eigenem `SaveVideo`-Node vorausgesetzt (siehe 4.0-Eintrag oben).
- **Qualitätsfilter** (`filter.rs`): Unschärfe via Varianz-des-Laplace
  (Standard-Technik, **von Hand implementiert statt über `imageproc::filter::
  filter3x3`** — dessen Output-Clamping auf den Ziel-Pixeltyp hätte negative
  Laplace-Werte auf 0 abgeschnitten und die Varianz systematisch verzerrt,
  siehe Modul-Dokkommentar); Near-Duplicate-Erkennung via `image_hasher`
  (gepflegter Nachfolger von `img_hash`, echtes Crate von crates.io, kein
  Selbstbau) mit Hamming-Distanz gegen den zuletzt behaltenen Frame pro
  Quelle (O(n), nicht O(n²) — Duplikate sind laut Videokohärenz ohnehin
  konsekutiv).
- **Captioning** (`caption.rs` + neues `sidecar/src/aiwm_sidecar/vision.py`):
  - **Florence-2** (Microsoft, **MIT-Lizenz**, 230M/770M Parameter — Lizenz
    und Parameterzahl live gegen die aktuelle Hugging-Face-Model-Card von
    `microsoft/Florence-2-large` verifiziert, nicht aus dem Training
    geraten) captioned jeden behaltenen Frame per Task-Prompt
    (`<DETAILED_CAPTION>`). Braucht `trust_remote_code=True` (eigene
    Modeling-Datei im HF-Repo) plus `timm`/`einops` (DaViT-Vision-Backbone) —
    neue Sidecar-Deps.
  - **Qwen2.5-VL-7B-Instruct** (Alibaba, **Apache-2.0**, 7B Parameter, echtes
    Multi-Image-Prompting bestätigt — ebenfalls live gegen die aktuelle
    Model-Card verifiziert) übernimmt die Eskalation: wirkt eine
    Florence-2-Caption unsicher (Heuristik unten), wird der Frame zusammen
    mit einem späteren Frame aus derselben Quelle (Default-Offset 5 behaltene
    Frames — der User nannte wörtlich "Frame X gegen X+5") neu captioned, mit
    der festen Frage "was passiert zwischen diesen beiden Bildern". Lädt
    standardmäßig **4-bit über `bitsandbytes`** (fp16 allein wären ~14 GB
    Gewichte — auf einer 16-GB-Karte neben allem anderen, was AIWM sonst
    lädt, nicht vertretbar); `quantization: "4bit"|"8bit"|"none"` bleibt ein
    Parameter für größere Karten. Nutzt `qwen_vl_utils.process_vision_info`
    (Alibabas eigenes Multi-Image-Chat-Template-Hilfsmittel).
  - **Eskalations-Heuristik** (`is_low_confidence_caption`, dokumentiert im
    Modul): eskaliert bei (1) auffällig kurzer Caption (< 4 Wörter), (2)
    Hedging-Sprache ("unclear", "hard to tell", …), oder (3) jedem N-ten
    Frame (Default 20) als periodisches Sicherheitsnetz gegen einen
    selbstsicher-aber-falschen Fall, den (1)/(2) nicht fangen. Eskalation
    gilt nur für Video-Frames (ein Standbild hat keinen zeitlichen Nachbarn).
  - Beide Modelle nur mit Fake-Doubles getestet (`FakeFlorence2`/`FakeQwenVl`
    in `sidecar/tests/test_vision_captioning.py`, exakt nach dem Muster von
    `FakeKokoro`/`FakeDia`) — **kein echter Multi-GB-Modell-Load in dieser
    Umgebung verifiziert** (kein GPU-Zugriff hier), die Implementierung folgt
    aber genau den dokumentierten Model-Card-Usage-Snippets.
- **Neuer Runtime-Typ** `VisionAdapter` (`core/src/runtime/vision.rs`,
  `RuntimeKind::Vision`): gleiche Sidecar-Client-Form wie `TtsAdapter`, aber
  **bewusst nicht** dessen Zero-VRAM-Annahme — `loaded_models()` trackt
  echte, anfragen-abhängige VRAM-Zahlen (Florence-2 immer, + Qwen bei
  Eskalation), damit der Scheduler bei Bedarf tatsächlich evicten kann.
- **Neuer Job-Typ** `dataset_prep` (`orchestrator::engine`): synthetische
  Modell-Id fürs Scheduler-Ledger (`DATASET_VISION_MODEL_ID`, gleiche
  Begründung wie beim Upscale-Job — kein einzelnes Library-`Model` dahinter),
  reale (anfragenabhängige) VRAM-Reservierung statt Fallback-Konstante.
- **Neue Tabelle** `dataset_frames` (Migration `0014_dataset_frames.sql` —
  **Namenskollision**: ein parallel arbeitender Story-Studio-Agent hat
  ebenfalls eine `0012_*.sql` angelegt; beim Zusammenführen der Branches muss
  eine der beiden auf `0013` umnummeriert werden, noch nicht gelöst),
  cascade-delete mit ihrem Job (gleiche Begründung wie `documents`/Sessions).
- **Kuratier-UI** (`ui/src/features/dataset/Dataset.tsx`, neuer "Dataset"-Tab):
  Root-Ordner-Picker + Pipeline-Parameter → Job-Start → Live-Fortschritt
  (Event-Zeile, Frame-Zähler) → Grid mit Thumbnail, Tag-Chip, editierbarer
  Caption (Freitext überschreibt die Auto-Caption, `caption_engine` wird
  dabei geleert — markiert "von Hand"), Exclude-Checkbox, Qwen-Badge bei
  eskalierten Frames. **Export**-Button schreibt die kuratierte Auswahl als
  `NNNN.png`/`NNNN.txt`-Paare (die Standard-Sidecar-Konvention, die
  kohya-ss/sd-scripts & Co. erwarten) in einen Zielordner — export-only,
  kein Trainingslauf.
- Neue HTTP-Routen (`api/http.rs`) + Tauri-Commands (`src-tauri/src/lib.rs`):
  `GET /jobs/{id}/dataset-frames`, `PUT .../dataset-frames/{frame_id}`,
  `GET .../dataset-frames/{frame_id}/image`, `POST /jobs/{id}/dataset-export`
  — Job-Start selbst braucht keine neue Route, `submit_job` mit
  `job_type=dataset_prep` reicht (generischer Mechanismus).
- **Noch offen / bewusst nicht gebaut:**
  - Kein `KNOWN_MODELS`-Katalogeintrag für Florence-2/Qwen2.5-VL (kein
    Ein-Klick-Download über den Models-Tab) — Import läuft über den
    bestehenden generischen "Ordner zeigen + Rolle zuweisen"-Weg
    (`vision_florence2`/`vision_qwen2_5_vl`-Rollen), analog zu Dias
    `dia_engine`/`dia_codec`. Ein Katalogeintrag ist ein sauberer Folge-Slice.
  - Kein echter End-to-End-Lauf mit echten Modellgewichten/echtem Video
    verifiziert (keine GPU/Testdaten in dieser Umgebung) — nur Unit-Tests
    (Rust, reale kleine PNGs für Blur/Hash-Tests) + Sidecar-Fake-Doubles +
    Browser-Verifikation gegen den dev-mock.
  - `VisionAdapter.unload_model` räumt nur die Rust-seitige Buchführung auf,
    nicht den Python-seitigen Modell-Cache im Sidecar-Prozess selbst (gleiche
    Lücke wie bei den anderen Sidecar-gestützten Adaptern) — reale
    GPU-Speicher-Freigabe bei Eviction ist ein Folge-Thema, sobald das an
    echter Hardware gemessen wird.

### Teilsystem 2 — Trainings-Orchestrator: geplant (2026-09-16), noch keine Implementierung

**Priorität entschieden (User, 2026-09-16):** Architektur von Anfang an
zielart-generisch (`TrainingTarget::Image | Video`), aber der erste
Implementierungs-Slice ist **Bild-LoRA** (SDXL/Flux) — reife Tooling-Lage,
niedrigeres Risiko. Video-LoRA (Wan/LTX) folgt als eigener Slice, sobald das
Harness für Bild einmal steht; die Tooling-Recherche dafür (Kandidaten
ungeprüft: z. B. `musubi-tuner` für Wan, oder ein diffusers-basiertes
Skript) ist noch offen und wird erst zu dem Zeitpunkt gemacht, nicht jetzt
spekulativ vorweggenommen.

**Warum kohya-ss/sd-scripts für den Bild-Pfad, nicht erst zu recherchieren:**
Teilsystem 1's Export-Format (`NNNN.png`+`NNNN.txt`-Paare) wurde **bereits
bewusst** nach genau dieser Konvention gebaut (siehe Export-Beschreibung
oben) — die Dataset-Pipeline ist also faktisch schon der erste halbe Schritt
zu einem kohya-ss-Trainingslauf, nicht nur zufällig kompatibel.

**Reales technisches Risiko, das die Architektur bestimmt (2026-09-16
gegen den echten Code geprüft):** `JobEngine::recover` (`core/src/
orchestrator/engine.rs:268`) markiert jeden beim App-Start noch
`running`-Job als **fehlgeschlagen** — es gibt aktuell keinerlei
Job-Resume nach einem Neustart. Für einen stundenlangen Trainingslauf ist
das inakzeptabel (ein Absturz/Update-Neustart darf nicht 3 Stunden GPU-Zeit
wegwerfen). AIWM hat dafür bereits **einen** Präzedenzfall: der externe
Agent-`Launcher` (OpenCode/Hermes) spawnt bewusst einen echten,
unabhängigen Terminal-Prozess, **nicht** Job-Object-supervised, gerade weil
der länger leben soll als die App selbst (siehe `launcher`-Modul-Doc). Ein
Trainingslauf sollte diesem Muster folgen — ein `TrainingRun` ist kein
`Job` im bestehenden Scheduler-Sinn, sondern ein eigenständiger,
detached Subprozess mit eigenem Fortschritts-Tracking (kohya-ss schreibt
eigene Checkpoints; AIWM muss nur den Fortschritt lesen/anzeigen, nicht die
Resume-Logik selbst bauen) — nicht der bestehende `JobEngine`-Pfad mit
seiner Restart=Fail-Semantik.

**Architektur-Skizze (Plan, nicht gebaut):**
- Neuer `TrainingAdapter` nach dem bestehenden `RuntimeAdapter`-Muster
  (install/health/status), aber mit eigenem, isoliertem `uv`-venv für
  kohya-ss/sd-scripts — getrennt vom Sidecar-venv, aus genau dem Grund, aus
  dem der Sidecar selbst schon isoliert ist (potenziell andere
  Torch-Version/Dependency-Baum als die Haupt-Sidecar-Umgebung).
  Install folgt dem etablierten idempotenten Marker-Muster
  (`runtime::comfyui::install` als Vorbild).
- `TrainingRun` als eigene DB-Tabelle (Status/Fortschritt/Zielart/
  Basis-Checkpoint/Ergebnis-Datei-Pfad), **nicht** in `jobs` — Grund siehe
  oben. Der Scheduler muss trotzdem von der VRAM-Reservierung wissen
  (ein laufendes Training blockiert GPU-Kapazität wie jeder andere
  Runtime), auch ohne über den Job-Mechanismus zu laufen.
- UI-Einstieg: ein "LoRA trainieren"-Button direkt im Dataset-Tab nach dem
  Export (natürlicher nächster Schritt in der bestehenden Kuratier-UI),
  mit sinnvollen Defaults für Rank/Alpha/Lernrate/Epochen, damit ein Laie
  nicht jeden Trainings-Parameter verstehen muss, bevor er startet.
- Nach Abschluss: Ergebnis-LoRA automatisch ins bestehende Modell-System
  importieren (gleicher Import-Pfad wie ein manuell heruntergeladenes
  LoRA), sofort in Image/Video zum Testen wählbar.

**Ausdrücklich noch offen, bewusst nicht vorab entschieden:**
Video-LoRA-Tooling-Wahl (siehe oben); ob/wie ein laufendes Training in der
Diagnostics-/Runtimes-Übersicht neben ComfyUI/llama.cpp erscheint; wie
Abbruch eines laufenden Trainings sauber funktioniert (kohya-ss selbst
beenden vs. nur den Fortschritt ignorieren). **Weiterhin keine
Implementierung** — laut `brainstorming`-Skill erst vollständiges
Design/Spec + User-Freigabe, dann `writing-plans`, bevor hier Code
entsteht; diese Notiz ist der Zwischenstand dieses Prozesses, kein
fertiges Spec.

## ComfyUI Workflow-Engine (geplant, 2026-09-16 — noch keine Implementierung)

**Ziel laut User:** bessere Bild-/Video-Ausgabequalität, nicht nur mehr
Optionen. **Priorität entschieden (User, 2026-09-16):** erst eine
wiederverwendbare Workflow-Template-Schicht bauen, dann die eigentlichen
Qualitäts-Features als Templates darauf.

**Problem, das die Template-Schicht löst:** Jede Generierungs-"Lane"
(`checkpoint_txt2img`, `flux_txt2img`, `flux2_klein_txt2img`,
`flux2_klein_edit`, Upscale separat) ist heute eine von Hand geschriebene
Rust-Funktion, die den ComfyUI-Graphen Node für Node neu zusammenbaut
(`core/src/pipeline/mod.rs`). Eine neue Lane = eine neue ~100-Zeilen-
Funktion + eigene Tests; ein neues Qualitäts-Feature (Hi-Res-Fix,
ControlNet, Face-Restore) müsste in jede bestehende Lane einzeln von Hand
verdrahtet werden — die Wartungskosten multiplizieren sich mit jeder
Kombination. Schon heute duplizieren sich Checkpoint-Load/Prompt-Encode/
KSampler-Verdrahtung leicht abgewandelt über vier Funktionen hinweg — eine
reale, bereits sichtbare Redundanz, unabhängig von neuen Features.

**Inspiration, keine Kopie (siehe unten):** Ein paralleles lokales
AI-Studio-Projekt (`E:\locally-uncensored`, AGPL-3.0 — Idee übernommen,
kein Code kopiert) hat einen Workflow-Graphen + eine "Parameter-Map" —
ein fester Satz generischer Achsen (Prompt, Seed, Steps, CFG, Größe,
Sampler, Input-Bild, Frames/FPS, Model-Loader), auf `{Node-Id,
Input-Key}`-Paare in einem konkreten Graphen abgebildet. Eine neue Lane
wird dort zur Konfigurations-Aktion (Graph importieren + Felder mappen),
nicht zu neuem Pipeline-Code. AIWM soll **nicht** roh importierte
Workflow-JSON-Dateien aus dem ComfyUI-Editor unterstützen — das würde genau
die Node-Graph-Komplexität zurückbringen, die die App bewusst vor dem
Nutzer versteckt (siehe Story Studio: "keine neuen Nodes, keine neue
Capability" für Bildgenerierung). Was portiert wird, ist die **Idee**:
Graph-Bau in kleine, benannte, komponierbare, einzeln getestete Fragmente
zerlegen (Checkpoint laden, Prompt encodieren, KSampler-Pass,
Latent-Upscale, ControlNet anwenden, Face-Restore-Pass, LoRA-Kette — Letzteres
existiert als `splice_loras` bereits genau in dieser Form) statt als
Rust-Idee 1:1 aus einer anderen Codebase übernommen.

**Architektur-Skizze (Plan, nicht gebaut):**
- **Phase A — Fragment-Schicht:** bestehende Graph-Funktionen refactorn,
  sodass sie aus wiederverwendbaren Fragment-Buildern komponiert werden
  statt jede ihr eigenes Boilerplate zu duplizieren. Ein "Recipe" = eine
  geordnete Fragment-Liste + ein Parameter-Struct; das Zusammensetzen
  erzeugt das finale Graph-JSON — genau das Muster, das `splice_loras`
  heute schon für die eine Fragment-Art (LoRA-Kette) vormacht, nur
  verallgemeinert.
- **Phase B — Qualitäts-Rezepte als Fragmente**, in dieser Reihenfolge
  (steigender Aufwand):
  1. **Hi-Res-Fix / Mehrpass-Refinement** (niedrig auflösend generieren →
     Latent-Upscale → zweiter Low-Denoise-KSampler-Pass) — kein neues
     externes Modell nötig, gut verstandene SDXL/Flux-Technik, guter
     erster Test ob sich die Fragment-Schicht lohnt.
  2. **Face-Restoration** als Post-Process — ein dediziertes, kleines
     Modell/Node, überschaubarer Umfang, gutes Aufwand/Nutzen-Verhältnis.
  3. **ControlNet/Region-Conditioning** (Referenzbild-gesteuerte Pose/
     Tiefe/Kanten-Konditionierung) — braucht einen neuen Modell-Download-
     Weg (ControlNet-Checkpoints) und Preprocessor-Nodes, größter Aufwand,
     zuletzt.

**Ausdrücklich noch offen, bewusst nicht vorab entschieden:** welches
ControlNet-Preprocessor-Node-Pack (braucht echte Recherche, gleiche
Disziplin wie jede bisherige Custom-Node-Entscheidung diese Session); ob
Hi-Res-Fix ein Per-Generation-Toggle oder eine globale Qualitätsstufe wird;
der reale VRAM-/Zeit-Mehrkosten eines zweiten KSampler-Passes auf einer
16-GB-Karte (an echter Hardware messen, nicht annehmen — gleiche Disziplin
wie die VRAM-Kalibrierung oben); ob Face-Restore/ControlNet gleich gut für
SDXL und Flux funktionieren oder pro Familie unterschiedliche Node-Packs
brauchen (gleiche SDXL-vs-Flux-Reifegrad-Frage wie bei Story Studios
IP-Adapter-Arbeit). **Keine Implementierung** — wie beim Trainings-
Orchestrator erst vollständiges Design/Spec + User-Freigabe vor Code.

## Ideen aus `locally-uncensored` (recherchiert 2026-09-16, kein Code übernommen)

Ein paralleles lokales AI-Studio-Projekt (`E:\locally-uncensored`, AGPL-3.0)
wurde auf UX-/Architektur-Ideen durchsucht ("Ideen/Layouts klauen, nie
Code" — AGPL-Lizenz macht Code-Kopieren ohnehin heikel). Ergebnis unten;
**LoRA-Stack-UI wird direkt umgesetzt** (siehe eigener Abschnitt), der Rest
ist Backlog für spätere Slices, absteigend nach Aufwand geordnet:

- **LoRA-Stack mit Pro-Item-Stärke-Regler** — ✅ wird jetzt gebaut, siehe
  unten. Direkter, bereits heute existierender Lücke: `core::pipeline`
  unterstützt eine LoRA-Kette mit Pro-LoRA-Stärke seit längerem
  (`LoraSpec`/`splice_loras`), aber kein UI-Tab verdrahtet das je.
- **Hardware-Fit-Badge im Discover-Tab** — Modelle nach VRAM-Fit in Tiers
  einsortieren (passt/knapp/zu groß) statt nur einer binären Warnung;
  kleiner, in sich geschlossener Slice.
- **Kompaktions-Records für lange Chats** — statt eines einzigen
  "Zusammenfassung ab Index N", mehrere stapelbare Kompaktions-Einträge,
  jeweils an die **ID** der letzten ersetzten Nachricht verankert (überlebt
  Edits/Deletes), mit Vorher/Nachher-Tokenzahl. AIWM hat aktuell keinerlei
  Context-Folding für lange Chats — echte Lücke, aber ein eigener,
  nicht-trivialer Slice (Chat-Verlauf-Datenmodell betroffen).
- **Cross-Session-Memory-Store** — ein von der einzelnen Chat-Session
  getrennter, persistenter Fakten-Speicher (typisiert, Keyword-Retrieval),
  den jede neue Session mit einbezieht. Eigener Slice, braucht eigenes
  Datenmodell + Retrieval-Strategie-Entscheidung.
- **Personas** — einfache Presets (Name/Icon/System-Prompt), global aktiv +
  Override pro Chat. Klein genug für einen direkten Port, wenn Chat mal
  wieder dran ist.
- **Chatbot-Import (ChatGPT/Claude/Gemini-Export) → RAG statt Chat-Verlauf**
  — importierter Fremd-Verlauf wird als durchsuchbarer Kontext behandelt,
  nicht als eigene Chat-Session. Braucht AIWMs Dokument-RAG-Pipeline als
  Ziel, die schon existiert (`documents`-Tabelle) — realistischer Slice,
  sobald Import gewünscht ist.
- **Angedockte Galerie/3-Spalten-Layout** — ein einklappbares
  Galerie-Panel neben dem Generierungs-Canvas (statt eines separaten
  Result-Panels) bzw. ein 3-Spalten-Layout für den Agents-Tab
  (Transkript + Datei-Baum + Vorschau + Plan/Approve-Karte). Reines
  Layout-Refactoring, kein neues Backend nötig.
- **A/B-Vergleichsmodus im Chat** — zwei Modelle parallel streamen lassen,
  eigene Token/Zeit/Durchsatz-Stats pro Spalte. Eigener Slice.
- **Inline-Freigabe-Leiste + geschichtetes Permission-Modell für Agents**
  — Tool-Aufrufe nicht-modal direkt im Transkript freigeben/ablehnen, mit
  Kategorie-Filter → Pro-Session-Override → Pro-Aufruf-Freigabe als drei
  Schichten. Passt zum bestehenden Agents-Tab, eigener Slice.
- **Backup/Restore-Trias gegen Updater-Datenverlust** — debounced +
  Intervall + `beforeunload`-Backup kombiniert. Nur relevant, falls AIWM
  je Datenverlust durch den eigenen Update-Mechanismus beobachtet — aktuell
  kein bekanntes Problem, daher niedrige Priorität.
- **Telefon-/Tunnel-Fernzugriff** — **verworfen**, widerspricht AIWMs
  Loopback-only/Offline-first-Grundsatz (ADR-008/009) direkt.

## LoRA-Stack-UI (2026-09-16, in Arbeit)

Schließt die oben genannte Lücke direkt: `core::pipeline::LoraSpec`/
`splice_loras` existieren bereits und werden von `checkpoint_txt2img`/
`flux_txt2img` genutzt, aber kein UI-Tab (`Image.tsx`/`Video.tsx`) exponiert
das je — ein Nutzer kann heute keine LoRA auswählen, geschweige denn
mehrere stapeln. Baut eine Stack-UI (aktive LoRAs als Liste mit
Pro-Item-Stärke-Regler 0–2, Default-Stärke 0.8, Rescan-Button gegen den
Modell-Store, ausgeblendet für Checkpoint-Familien ohne LoRA-Seam) nach dem
in `locally-uncensored` beobachteten (nicht kopierten) Muster.

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
  Keys (`resize_type.scale` / `resize_type.width`/`height`) wurden aus
  ComfyUIs `_io.py`/`execution.py`-Quellcode hergeleitet, dann gegen den
  echten Quellcode gegengeprüft, und schließlich **live verifiziert**
  (2026-09-15): Node über die echte `install()` in die reale, bereits
  bestehende ComfyUI-Installation nachinstalliert (nur das fehlende Stück,
  `uv`/Source/GGUF-Node waren schon da — der eigentliche Auslöser war ein
  echter Laufzeitfehler "Node 'RTXVideoSuperResolution' not found", weil
  die Installation vor dem RTX-Feature entstand), `nvvfx` importiert
  sauber im echten venv, dann ein echter End-to-End-Lauf über `aiwm-cored`
  gegen die echte API: ein echtes 256×256-SDXL-Bild generiert, per
  `job_type=upscale` auf 2× skaliert, Ergebnis tatsächlich 512×512 PNG
  (294 KB) — kein Fehler, keine Annahme. Nicht zu verwechseln mit "DLSS 5": das ist eine noch nicht
  final released Game-Rendering-Technologie (GTC 2026 angekündigt), deren
  populärste GitHub-Repos "geleakte" NVIDIA-Binaries per DLL-Injection in
  beliebige Spiele patchen — bewusst nicht angefasst (Sicherheits-/
  Lizenzrisiko, und architektonisch eh nicht für Batch-Datei-Upscaling
  gedacht, sondern fürs Echtzeit-Rendering einer Game-Engine).
