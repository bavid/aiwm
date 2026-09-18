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
- Für Phase 3 weiterhin offen: Assembly-Format-Priorität (HTML-Scroll zuerst
  vs. PDF/CBZ), ob/wie ein Reorder-Endpoint für die Timeline gebraucht wird,
  sobald Nutzer:innen wirklich Szenen nachträglich umsortieren wollen. Die
  IP-Adapter-Node-Pack-Auswahl (unten offen gelassen) ist jetzt entschieden
  und umgesetzt.

## Story Studio Phase 2 — Charakter-Konsistenz, erster Schnitt (2026-09-16)
Phase 1 (oben) hat jedes Charakter-/Location-/Szenenbild als unabhängigen
`job_type=image`-Job ohne jede Konsistenz-Maschinerie erzeugt — dieselbe
Figur sah in jedem Bild anders aus. Phase 2 verankert eine Generierung an
einem vorhandenen Referenzbild (`portrait_job_id`/`reference_job_id`), statt
unabhängig zu generieren, sobald eines existiert.

**Recherche zuerst** (Auftrag: "Recherche, keine Vermutung" — Repo-Quellen
real gelesen, nicht aus dem Gedächtnis):
- `cubiq/ComfyUI_IPAdapter_plus` (GPL-3.0, 6,1k Stars) ist der De-facto-
  Standard für SDXL-IP-Adapter — seit 2025-04-14 laut eigenem README
  offiziell "maintenance only", aber vollständig funktionsfähig und mit
  Abstand am meisten genutzt. Kein eigenes `requirements.txt` für die
  Basisfunktion (nur die FaceID-Varianten brauchen `insightface`, hier
  ungenutzt) — der Install braucht also nur den Node selbst, keinen
  zusätzlichen pip-Schritt.
- Für Flux gibt es kein vergleichbar aktives/reifes Äquivalent
  (`XLabs-AI/x-flux-comfyui`: letzter Push Ende 2024; `cubiq`s PuLID-Repos:
  ebenfalls "maintenance only"). **Aber**: FLUX.2 [klein] ist bereits ein
  Edit-trainiertes Modell — Phase 1 nutzt dafür schon ComfyUIs Core-Node
  `ReferenceLatent` (`flux2_klein_edit`) — und genau dieser Mechanismus
  liefert "Kontext-Style" Referenz-Konditionierung für eine *neue*
  Generierung ganz ohne neuen Custom-Node oder neue Gewichte. Plain
  FLUX.1-dev ist dagegen *nicht* Edit-trainiert — `ReferenceLatent` würde
  dort nichts bewirken (steht wörtlich im Node-Docstring: "for an edit
  model"). Das reale Äquivalent für FLUX.1 wäre Flux Redux
  (`StyleModelApply`/`CLIPVisionEncode`, ebenfalls Core-Nodes, aber neue
  Gewichte nötig) — bewusst nicht in diesem Schnitt, siehe unten.

**Umgesetzt:**
- SDXL-Familie: `checkpoint_ipadapter_txt2img` (`core/src/pipeline/mod.rs`)
  — `CLIPVisionLoader` + `IPAdapterModelLoader` (explizite Dateinamen, nicht
  der namens-Pattern-abhängige `IPAdapterUnifiedLoader`) + `LoadImage` →
  `IPAdapterAdvanced`, vor den Sampler geschaltet; LoRAs patchen zuerst,
  IPAdapter danach.
- FLUX.2 [klein]: `flux2_klein_reference_txt2img` (GGUF) und
  `flux2_klein_reference_txt2img_safetensors` (safetensors) — beide nötig,
  ein echter Bug wurde beim Live-Smoke-Test gefunden und gefixt: die
  safetensors-Datei über den GGUF-only-Loader zu schicken scheitert am
  `/prompt`-Endpoint mit `unet_name: '...' not in [...]`
  (`UnetLoaderGGUF` listet nur `.gguf`-Dateien).
- Zwei neue `ModelKind`-Varianten (`ClipVision`, `IpAdapter`) — volle
  Library-Bürger wie jede andere Modellart (eigener Store-Unterordner,
  eigene `extra_model_paths.yaml`-Zeile, eigene Rolle, jetzt auch im
  Models-Tab-Importer wählbar).
- Neuer Custom-Node-Install (`core/src/runtime/comfyui/install.rs`):
  `ComfyUI_IPAdapter_plus`, gleiches idempotentes Pinned-Archiv-Muster wie
  GGUF/RTX, echt heruntergeladen und mit selbst berechnetem SHA-256 gepinnt.
- UI: Character Sheet zeigt ein "Consistency-anchored"-Badge sobald ein
  Portrait existiert; Regenerate-Buttons (Character/Location/Solo-Charakter-
  Szene) heißen dann "… (anchored)". Szenen mit mehreren Teilnehmern ankern
  bewusst NICHT (ein Referenzbild kann nicht mehrere Charaktere gleichzeitig
  führen — bräuchte mehrere IPAdapter-Durchläufe, Phase-3+-Erweiterung).

**Echt Ende-zu-Ende gegengeprüft** (nicht nur Unit-Tests) via `aiwm-cored`
gegen die reale ComfyUI-Installation, echte heruntergeladene Gewichte
(`CLIP-ViT-H-14-laion2B-s32B-b79K.safetensors`, ~2,5 GB;
`ip-adapter-plus_sdxl_vit-h.safetensors`, ~848 MB — beide mit selbst
berechnetem SHA-256 importiert) und das reale SDXL-Base-Checkpoint:
derselbe Prompt/Seed einmal ohne und einmal mit Referenzbild gerendert
zeigt sichtbar konsistentere Haar-/Augenfarbe und Kapuzen-Zustand als zwei
unabhängige Phase-1-Renderings desselben Prompts. Gleicher Vergleich für
FLUX.2 [klein] wiederholt (unabhängig vs. referenz-verankert, andere Szene,
gleicher Charakter) — deutlich konsistentere Haarfarbe/Sommersprossen.
Test-Jobs/-Bilder danach wieder gelöscht, Node-Install und importierte
Gewichte bleiben (echte, wiederverwendbare Library-Einträge).

`cargo fmt`/`clippy --workspace --all-targets -D warnings`/
`test --workspace` grün (749 lib-Tests, 2 absichtlich ignorierte
Netzwerktests). `pnpm typecheck`/`lint`/`build` grün. UI live gegen
dev-mock verifiziert (Character Sheet mit/ohne Portrait — Badge und
"(anchored)"-Label erscheinen korrekt nur wenn ein Portrait existiert).

**Bewusst NICHT gebaut (Phase-2-Scope-Cuts, dokumentiert statt
stillschweigend weggelassen):**
- Plain FLUX.1-dev bleibt ohne Charakter-Konsistenz — `ReferenceLatent`
  wirkt dort nicht (nicht Edit-trainiert). Nächster Schritt wäre Flux Redux
  (`flux1-redux-dev.safetensors` + SigLIP-Vision-Encoder, beides neue
  Downloads, aber nur Core-ComfyUI-Nodes — kein neuer Custom-Node-Pack).
- Mehrfach-Charakter-Szenen ankern an keinen Teilnehmer (s.o.) — bräuchte
  verkettete IPAdapter-Anwendungen, eine pro Charakter, mit eigener Masken-/
  Regionslogik.
- FaceID-/PuLID-artige, gesichtsspezifische Konditionierung (stärkerer
  Identitäts-Lock als der hier verwendete "plus"-Subject-Transfer) nicht
  evaluiert — eigener Recherche-Aufwand (InsightFace-Abhängigkeit, eigene
  LoRA pro Modell).
- Kein Gewicht/Stärke-Slider in der UI (nur ein Server-Default von 0,8) —
  die Story-Studio-Oberfläche hat bewusst keine Bildparameter-Feinsteuerung,
  wie schon in Phase 1.

## Lokale KI-Trainings-Engine (Teilsystem 1 ✅ umgesetzt inkl. Plan-1-Erweiterungen — 2026-09-16, Teilsystem 2 ✅ umgesetzt — 2026-09-17)
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

### Teilsystem 1 — Erweiterungen (Plan 1 "Dataset Extensions"): ✅ umgesetzt (2026-09-16)

Spec: `docs/superpowers/specs/2026-09-16-training-orchestrator-design.md`
(Abschnitte 3, 3A, 3C, 3D, 3E, 4B, 5); Plan:
`docs/superpowers/plans/2026-09-16-training-orchestrator-plan-1-dataset-extensions.md`
(14 Tasks, alle grün). Alle Gates sauber: `cargo fmt --check`/`clippy
--workspace --all-targets -D warnings` sauber, `cargo test --workspace`
**909 passed / 0 failed / 4 ignored** (37 Test-Binaries), sidecar `ruff`
sauber + `pytest` **89 passed**, UI `tsc`/`eslint`/`vite build` sauber, jede
UI-Task zusätzlich live im Browser gegen den dev-mock verifiziert.

- **Datasets sind jetzt Objekte** (Migration `0015_datasets.sql`): neue
  Tabellen `datasets` (Name, `mode` `frames|clips`, `source_root`,
  `trigger_word`, `prep_job_id`, `export_dir`), `dataset_concepts` und
  `frame_concepts`. `dataset_frames` wurde dafür neu aufgebaut statt
  ge-`ALTER`t (SQLite kann die FK-Action einer bestehenden Spalte nicht
  ändern): `job_id` ist nullable mit `ON DELETE SET NULL`, der Cascade hängt
  jetzt am Dataset. **Ein Datensatz überlebt damit seinen Prep-Job** — er ist
  das dauerhafte Objekt, auf das ein Trainingslauf aus Teilsystem 2 zeigt,
  wiederverwendbar über viele Läufe. Die in der 0014-Notiz oben erwähnte
  Migrations-Nummernkollision ist damit erledigt (`0014` = `dataset_frames`,
  `0015` = `datasets`).
- **Verworfene Frames bleiben stehen, mit Begründung**: `rejection_reason`
  (`''` = behalten) statt stiller Löschung. Das Kuratier-Grid zeigt sie
  hinter Filter-Chips pro Grund mit Zähler, und **Restore** ("doch behalten")
  macht ein automatisches Urteil rückgängig — der Nutzer behält das letzte
  Wort über jede Heuristik.
- **Filterstufe C** (`capability/dataset/filter.rs`) über Blur/Duplikat
  hinaus, alles billig und ohne ML: **tote Frames** (flach, nahezu schwarz
  oder weiß — Blenden, Leerbilder), **Übergänge** (ein *bereits unscharfer*
  Frame wird gezielt als `transition` statt als `blur` geführt, wenn er per
  Perceptual-Hash zusätzlich weit von Vorgänger *und* Nachfolger entfernt
  liegt — die Signatur eines Schnitt-Schmierers/Cross-Fades; Unschärfe allein
  bleibt Unschärfe, ein scharfer Frame wird nie zum Übergang), und eine
  **Diversitäts-Obergrenze pro Clip** (Farthest-Point-Auswahl auf denselben
  Hashes, damit ein langer statischer Clip nicht hunderte fast identische
  Frames in den Datensatz kippt). Gründe insgesamt: `black`, `transition`,
  `blur`, `duplicate`, `cap`, `unusable`.
- **Captioning ist optional** und im Prep-Formular abschaltbar (ohne
  installierten Captioner bleibt es zwangsweise aus). Die UI erklärt die
  Konsequenz wörtlich: *"Recommended for style LoRAs: what is described stays
  controllable, what is not becomes part of the style."* — bzw. ohne
  Captioner: alles Wiederkehrende fließt in das Trigger-Wort.
- **Captioner-Registry** (`capability/dataset/captioner.rs`), Auswahl im
  Formular, Installationsstatus aus der Modell-Library:
  - **Florence-2** (`florence2`, MIT, Prosa, unterstützt die
    X-vs-X+N-Eskalation) — wie gehabt über den Vision-Sidecar.
  - **WD EVA02 Tagger v3** (`wd-eva02-tagger-v3`, Apache-2.0, Danbooru-Tags,
    keine Eskalation — ein Tagger hat keinen Satz, dessen Sicherheit man
    beurteilen könnte) — 0.3B ONNX-Klassifikator, läuft per `onnxruntime`
    **auf der CPU** im Sidecar (`vram_mb: 0`, kein Torch beteiligt).
    `required_files` = `model.onnx` + `selected_tags.csv`, damit ein
    einzelner CSV-Import nicht fälschlich als installierter Tagger zählt.
    **Einmal echt verifiziert**: Gewichte gegen den Katalog-Pin gehasht und
    ein realer Tagging-Lauf durchgeführt — der einzige echte Modell-Lauf in
    Plan 1, alles andere läuft über Fake-Doubles.
  - **JoyCaption: bewusst verschoben.** Der Weg dahin wäre llama.cpp mit
    `--mmproj` (Multimodal-Projector); ob die im Repo verwendete
    llama.cpp-Version das in der benötigten Form unterstützt, ist **noch
    nicht geprüft** — offener Punkt, keine Attrappe gebaut.
- **Konzepte + geführter "Learn"-Modus** (Spec 3A/4B): ein Konzept ist eine
  benannte Sache, die der LoRA lernen soll (`token` ohne Vorbedeutung im
  Basismodell, z. B. `kenji_xy`, plus optionale Beschreibung). Frames werden
  ihm über `frame_concepts` zugeordnet — **nie** in die Caption-Spalte
  denormalisiert. Die UI warnt inline, wenn ein Token ein gewöhnliches Wort
  ist (Client-Spiegel von `compose::token_warning`) und wenn ein Konzept
  unter 20 Beispiele hat. Der **Learn**-Modus (`LearnSets.tsx`, umschaltbar
  gegen das Grid) führt durch Sets von maximal 30 behaltenen Frames
  (Gruppierung "nach Clip" bzw. grob "nach Ähnlichkeit"), mit
  Mehrfachauswahl (Klick, Shift-Bereich, Alle/Invertieren/Leeren),
  Tastatursteuerung (←/→ Set, A, I, Esc, Enter = zuweisen) und einer
  Konzept-Übersicht mit Zählern/Warnungen.
- **Clip-Modus** (`capability/dataset/clip.rs` + Grid): in `mode = clips`
  bleibt **jedes Quellvideo ein Objekt** — eine Zeile mit der per `ffprobe`
  gelesenen Dauer und einem **Vorschau-Standbild** als `frame_path`, das
  Video selbst als `source_path`. Nicht lesbare oder zu kurze Clips landen
  als `unusable` (mit Badge, ohne Vorschaubild) statt still zu verschwinden;
  ein fehlgeschlagenes Vorschaubild kippt nicht den ganzen Lauf. Der Kurator
  setzt pro Clip **Start/Ende (s)** (leer = ganzer Clip, Validierung in der
  Karte: Start < Ende, beide ≥ 0 und ≤ Dauer), gespeichert als
  `clip_start_secs`/`clip_end_secs`; der Export **trimmt per ffmpeg-Stream-
  Copy** (kein Re-Encode). Fehlt ffmpeg, bricht der Export ab, statt
  stillschweigend das ungeschnittene Material auszuliefern.
- **Komponierter Export** (`compose.rs` + `export.rs`): die Caption wird erst
  beim Export aus getrennt gespeicherten Teilen zusammengesetzt —
  Dataset-Trigger, Konzept-Token (+ Beschreibung), eigene Auto-/Hand-Caption
  — in wählbarer **Reihenfolge** (`tags_first` für Anime/SDXL, `prose_first`
  für FLUX.2). Ausgabe weiterhin `NNNN.<ext>` + `NNNN.txt`-Paare.
- **API-/Tauri-/`ipc.ts`-Oberfläche** (neu neben den bestehenden
  Job-Routen): `GET /captioners`; `GET /datasets`; `GET|PUT|DELETE
  /datasets/{id}`; `GET /datasets/{id}/frames`; `GET
  /datasets/{id}/frames/{frame_id}/image`; `GET
  /datasets/{id}/frame-concepts`; `GET|POST /datasets/{id}/concepts`; `POST
  /datasets/{id}/export`; `PUT|DELETE /concepts/{id}`; `POST|DELETE
  /concepts/{id}/frames`. Jeweils mit Tauri-Command-Spiegel und typisiertem
  `ui/src/lib/ipc.ts`-Binding; `PUT .../dataset-frames/{frame_id}` nimmt die
  Clip-Grenzen dreiwertig entgegen (Schlüssel fehlt = unverändert,
  `null` = auf die natürliche Grenze zurücksetzen).

**Noch offen / bewusst nicht gebaut (Plan 1):**
- **Ähnlichkeits-Gruppierung im Learn-Modus ist nur ein grober Proxy**
  (Tag + Quelle auf dem Client): die Perceptual-Hashes werden nicht in die
  UI geschickt. Dafür braucht es eine **gespeicherte Hash-Spalte** —
  verschoben auf Plan 2; serverseitige Gruppierung ist der Folge-Slice.
- **JoyCaption** als dritter Captioner (siehe oben — llama.cpp-`--mmproj`-
  Prüfung steht noch aus).
- **Wan-Clip-Training selbst**: der Clip-Modus produziert getrimmte Clips,
  aber es gibt keinen Trainingslauf dafür — das ist Teilsystem 2.
- **Sidecar-Cache-Key-Normalisierung**: die drei Engine-Caches
  (`_florence2_cache`, `_qwen_vl_cache`, `_wd_tagger_cache`) schlüsseln auf
  den rohen `model_dir`-String ohne Pfad-Normalisierung — ein
  Trailing-Slash, abweichende Groß-/Kleinschreibung oder ein Symlink lädt
  dasselbe Modell ein zweites Mal.
- **Tagger-Klassifikationslogik** liegt in `tag_frame` statt auf der
  Engine-Klasse selbst.
- **Decode-once**: die Filter-Verdrahtung öffnet jedes Frame dreimal
  (`is_blurry` / `is_dead_frame` / `phash_of`) statt einmal zu dekodieren und
  das Bild zu teilen.
- **`pipeline::filter_groups` dekodiert synchron im async `run`**
  (vorbestehend) — bei Bedarf in `tokio::task::spawn_blocking` wickeln,
  sobald echte Läufe zeigen, dass es zählt.
- **`export_dataset_for_job` sucht das Dataset über `list()` + `find`** —
  ein `DatasetRepo::find_by_prep_job` lohnt sich, sobald die Dataset-Zahl
  wächst.
- **Der Export kopiert/trimmt Medien sequenziell.**
- **UI-Folgepunkte** (aus den Task-12/13-Reviews): das Frame-Grid ist **nicht
  virtualisiert** (bei `PAGE_SIZE` 60 unkritisch, bei Datensätzen mit
  tausenden Frames erneut ansehen — `FrameCard` ist inzwischen memoisiert und
  `usePolled.refetch` stabil, die Virtualisierung fehlt weiterhin); die
  Prep-/Export-/Konzept-Formulare sind `<div>`s statt semantischer
  `<form onSubmit>` (kein Enter-zum-Absenden); `ui/eslint.config.js` hat kein
  `eslint-plugin-jsx-a11y` (hätte die `alt=""`-/unbeschriftete-Input-Funde
  automatisch gefangen).

### Teilsystem 2 — Trainings-Orchestrator: ✅ umgesetzt (2026-09-17)

Gebaut, getestet und **auf dieser Maschine mit echten Gewichten und echten
Daten durchgelaufen** — kein Fake-Trainer, kein Trockenlauf. Der erste
vollständige FLUX.2-[klein]-4B-LoRA-Lauf steht unten mit den gemessenen
Zahlen; alles was hier als Zahl steht, stammt aus diesem Lauf und nicht aus
einer Schätzung.

Alle Gates grün: `cargo fmt --all -- --check` sauber, `cargo clippy
--workspace --all-targets -- -D warnings` sauber, `cargo test --workspace`
**1046 passed / 0 failed / 6 ignored**, sidecar `ruff check .` sauber und
`pytest` **89 passed**, UI `tsc --noEmit` / `eslint .` / `vite build` sauber.

**Architektur-Entscheidung gegenüber dem Plan geändert:** nicht
kohya-ss/sd-scripts, sondern **`ostris/ai-toolkit`** (fest gepinnter Commit
`e65c4d0fb69251e692390574c49873297dc4bae5`). Grund: ai-toolkit unterstützt
FLUX.2 [klein] 4B/9B, SDXL und Wan 2.2 aus einer einzigen Config-Form heraus,
kohya-ss zum Zeitpunkt der Recherche kein FLUX.2. Teilsystem 1's
Export-Format (`NNNN.png`+`NNNN.txt`) passt unverändert auch hier.

- **Runtime-Adapter + Installer** (`core/src/runtime/training/{mod,install}.rs`):
  eigenes, isoliertes `uv`-venv unter `E:\AI\data\runtimes\ai-toolkit\`
  (Python 3.12), Quell-Archiv über SHA-256 + Größe verifiziert
  (`6d4c67fa…e5`, 35.740.120 B), danach `torch==2.13.0`/`torchvision==0.28.0`/
  `torchaudio==2.11.0` vom cu130-Index **vor** `requirements.txt`, damit der
  CUDA-Build gewinnt. Abschluss-Marker `.installed-<commit>`, also ist ein
  "Reparieren" ein einziges Löschen und ein Commit-Bump erzwingt den Neubau.
  Installation lief hier in einem Durchgang durch; `POST /training/probe`
  meldet danach `{"torch_version":"2.13.0+cu130","cuda":true,
  "vram_total_mb":16375}`.
- **Profil-Registry** (`core/src/training/profile.rs`): vier Familien
  (FLUX.2 klein 4B/9B, SDXL, Wan 2.2 TI2V 5B) mit ai-toolkit-Arch-Id,
  VRAM-Strategie, Preset-Startwerten (fast/balanced/thorough) und — neu —
  `measured`, den *gemessenen* Sekunden/Schritt und dem VRAM-Peak eines
  echten Laufs. `measured` ist `None`, solange niemand die Familie hier
  wirklich trainiert hat; geraten wird nichts.
- **Basis-Gewichts-Manifest** (`core/src/training/bases.rs`): pro Familie
  Repo, `hf download`-Selektor und — sobald einmal echt heruntergeladen —
  Größe und lokal berechnete SHA-256 jeder Datei, die der Trainer öffnet.
  `verify_base_dir` prüft beim Start nur Größen (billig) und beim Registrieren
  einmal alle Hashes (vollständig). Der Befehl, den der Preflight zum Kopieren
  anzeigt, wird serverseitig aus genau diesem Manifest gebaut, kann also nicht
  von dem abweichen, was der Runner danach erwartet.
- **YAML-Rendering** (`core/src/training/config.rs`): erzeugt ai-toolkits
  Job-Config. **Lektion PyYAML-Floats:** serde_yaml schreibt kleine Zahlen in
  wissenschaftlicher Kurzform (`lr: 1e-4`), und PyYAMLs 1.1-Resolver liest das
  als **String**, nicht als Float — der Lauf wäre mit einer Lernrate vom Typ
  `str` gestartet. `normalize_yaml_floats` schreibt betroffene Skalare
  deshalb in eine Form zurück, die PyYAML sicher als Zahl erkennt.
- **Log-Parser** (`core/src/training/progress.rs`): liest tqdm-Bar, Loss, LR,
  ETA und die Lifecycle-Marker aus dem Trainings-Log. `ai-toolkit` zeichnet
  die Bar mit `\r` an Ort und Stelle neu, ein Chunk enthält also viele
  veraltete Bars und ggf. ein angeschnittenes Fragment.
- **Detached-Start** (`core/src/training/process.rs` + `launcher::spawn`):
  der Trainer ist bewusst **kein** Job-Object-Kind der App, sondern ein
  eigenständiger Prozess, der einen App-Neustart überlebt — sonst würfe ein
  Update mitten im Lauf Stunden GPU-Zeit weg. **Lektion:** genau deshalb kann
  die App ihn nicht über ein Handle beenden; Abbruch läuft über
  `taskkill /PID <pid> /T /F`, und "lebt der noch?" wird über die PID plus den
  erwarteten Image-Namen beantwortet, nicht über ein Handle.
- **Runner mit CAS-Zustandsautomat** (`core/src/training/runner*.rs`):
  Preflight (Trainer da? Basis-Gewichte vollständig *und* unversehrt?
  Datensatz exportiert? genug VRAM? genug Platz?), Start, Poll, Pause/Resume/
  Abbruch, Wiederaufnahme nach App-Neustart. Jeder Zustandswechsel ist ein
  Compare-and-Swap, damit Poller und Nutzerklick nicht gegeneinander
  schreiben.
- **Fake-Trainer** (`core/src/bin/aiwm-fake-trainer.rs`): spricht denselben
  Log-Dialekt, für den vollständigen Lebenszyklus-Test ohne GPU.
- **API/Tauri/ipc + Training-Tab**: Profile, Status, Installation, Probe,
  Läufe, Pause/Resume/Abbruch, Sample-Bilder; Einstieg zusätzlich direkt aus
  dem Dataset-Tab.

**Der erste echte Lauf (2026-09-17, RTX 4080 Super 16 GB):**

- Material: 54 echte Renders → Dataset-Prep behielt **50** nach Unschärfe-/
  Duplikat-Filter, Export als 50 `NNNN.png`+`NNNN.txt`-Paare nach
  `E:\AI\data\training\datasets\myrenders-v1`. Ohne Captioner, Captions sind
  also nur das Trigger-Wort `myrender_xy`.
- Lauf: Preset **Fast**, 600 Schritte, Rank 16, LR 1e-4, 768 px, Batch 1,
  qfloat8 + quantisierter Text-Encoder, adamw8bit, flowmatch, EMA 0.99.
- **600/600 Schritte in 16 min 55 s** = **1,69 s/Schritt** (ein zweiter Lauf
  derselben Config: 16 min 30 s = 1,65 s/Schritt — die Zahlen sind stabil).
- **VRAM-Peak 12.340 MB von 16.376 MB**, davon ~1.256 MB schon vor dem Lauf
  vom Desktop belegt — der Lauf selbst brauchte also ~**11.084 MB**.
  GPU-Temperatur max. 77 °C. Das passt in die 12.288 MB, die das 4B-Profil
  vorab freihaben will; ein Test hält diese Zusage jetzt fest (und schlägt
  auch an, wenn die Reserve unnötig weit darüber liegt).
- Gesamt-Wanduhr inkl. Modell-Laden, Latent-Cache und vier Sample-Runden:
  **20 min 18 s** (13:05:23 → 13:25:41). Der allererste Lauf brauchte länger,
  weil ai-toolkit erst den 8-GB-Text-Encoder ziehen musste.
- Ergebnis: **`state: completed`**, LoRA mit 46.223.656 B (Rank 16)
  automatisch als Bibliothekseintrag importiert
  (`source: training:<run-id>`, Rolle `lora`, Familie `flux2`),
  Zwischenstände bei Schritt 200/400, acht Sample-Bilder bei 0/200/400/600 ×
  2 Prompts.
- Erste drei tqdm-Zeilen dieses Laufs (sie pinnen den Parser in einem Test):
  ```text
  myrender-v2:   0%|          | 0/600 [00:00<?, ?it/s]
  myrender-v2:   0%|          | 0/600 [00:10<?, ?it/s, lr: 1.0e-04 loss: 6.468e-01]
  myrender-v2:   0%|          | 1/600 [00:10<1:42:50, 10.30s/it, lr: 1.0e-04 loss: 6.468e-01]
  ```

**Vier Fehler, die nur ein echter Lauf finden konnte — alle behoben:**

1. **Der Trainer liest *eine* Datei, nicht das diffusers-Layout.** Das
   Manifest pinnte die fünf Dateien des diffusers-Snapshots. ai-toolkits
   `Flux2Model.load_model` macht aber
   `load_file(os.path.join(name_or_path, "flux-2-klein-base-4b.safetensors"))`
   — genau die Einzeldatei, die der Download-Befehl vorher als "Duplikat, das
   der Trainer nie öffnet" **ausgeschlossen** hatte. Text-Encoder (`Qwen/Qwen3-4B`,
   8 GB) und VAE (`ai-toolkit/flux2_vae`) holt der Trainer sich zur Laufzeit
   selbst aus ganz anderen Repos. Der erste Start starb an
   `FileNotFoundError: … flux2-klein-4b` — mit allen fünf gepinnten Dateien
   vorhanden und geprüft. **Nebenwirkung:** der Download schrumpft von 23 GB
   auf 7,75 GB. **Und:** der Trainer braucht beim ersten Lauf einer Familie
   Netz, auch wenn die Basis-Gewichte lokal liegen.
2. **Erfolg wurde nie gemeldet.** Der Runner wartete auf ai-toolkits
   ` - 1 completed job`. In diesem Commit ruft `run.py` `print_end_message`
   **nur im `except`-Zweig** auf — ein Job, der einfach gelingt, kehrt
   schweigend aus `main` zurück. Der erste Lauf trainierte alle 600 Schritte,
   schrieb Checkpoint und Samples, beendete sich mit 0 — und wurde als
   `interrupted` ohne LoRA verbucht. Jetzt gilt "die eigene Fortschrittsbar hat
   `N/N` erreicht" als Erfolgsnachweis.
3. **Der finale Checkpoint war unsichtbar.** Zwischenstände heißen
   `<name>_<step:09>.safetensors`, der **letzte** aber schlicht
   `<name>.safetensors`. Der Parser verlangte die nummerierte Form, hätte also
   den Stand von Schritt 400 als Ergebnis importiert.
4. **Alle Sample-Bilder waren unsichtbar.** ai-toolkit schreibt
   `<time>__<step:09>_<count>.<ext>` mit **zwei** Unterstrichen; der Parser
   zerlegte von links und las den leeren String dazwischen. `latest_samples`
   kam leer zurück und die Sample-Route antwortete 404, bei acht Bildern auf
   der Platte. (Die Bilder sind übrigens **JPEG**, nicht PNG.)

Zusätzlich fiel auf, dass der Fortschritts-Parser die tqdm-Bars *anderer*
Phasen (Quantisieren, Latent-Cache, Sample-Generierung) als Trainingsschritte
mitzählte — der Lauf meldete „Schritt 34", während das Log noch bei den
Baseline-Samples stand. Der Parser akzeptiert jetzt nur noch die Bar, die auf
den Namen des Laufs hört.

**9B-Versuch: nicht durchführbar (2026-09-17).** `black-forest-labs/FLUX.2-klein-base-9B`
ist **zugangsbeschränkt**: `hf download` antwortet
`Access denied. This repository requires approval.` Ohne Zustimmung zur Lizenz
auf der Modellseite und einen HF-Token mit Freigabe kommen die Gewichte nicht
herunter — das kann die App nicht umgehen. Das 9B-Profil sagt das jetzt im
`license_note`, damit niemand erst nach 18 GB Download darauf stößt. Ob 9B auf
16 GB mit `low_vram` + `layer_offloading` läuft, bleibt damit **unverifiziert**.

**Bewusst offen geblieben:**

- **Loss-Verlauf**: die UI zeigt nur den letzten Loss; für eine Sparkline
  fehlt ein `loss_history`-Feld im DTO.
- **Platz-Preflight** prüft serverseitig, hat aber keine eigene UI-Zeile.
- **Wan-2.2-Clips-Profil** weiterhin ungeprüft — kein echter Lauf.
- **JoyCaption** als Captioner weiterhin offen (siehe Teilsystem 1).
- **Diagnostics**: ein laufendes Training erscheint noch nicht als eigene
  GPU-Zeile neben ComfyUI/llama.cpp.
- **`find_for_model`-Heuristik** ist zu großzügig: `GET /training/profiles`
  listet u. a. `flux2-vae`, `t5xxl_fp8_e4m3fn` und `wan2.2_vae` als
  „trainierbare" Zielmodelle, weil nur Familie/Name/Parameterzahl geprüft
  werden und nicht die Rolle.
- **Basis-Gewichte laden** bleibt Handarbeit über die `hf`-CLI: die HTTP-API
  hat **keine** Route, die einen beliebigen Ordner als Verzeichnis-Modell
  registriert (die einzige vorhandene ist fest auf den Colibri-Chat-Katalog
  verdrahtet und kann `family` nicht setzen). Bis der Models-Tab ein „Ordner
  registrieren" bekommt, macht das die `#[ignore]`-Harness
  `core/tests/register_training_base.rs`.
- **LoRA-Wirkung ungeprüft**: ein Bild-Job mit dem trainierten LoRA
  (`myrender-v2.safetensors` @ 0.80) lief sauber durch und schrieb ein
  gültiges 768×768-PNG (`E:\AI\data\outputs\01a0af20-….png`, 1.004.788 B) —
  aber gegen die installierte **9B**-fp8-Variante, weil kein
  4B-Inferenz-Checkpoint installiert ist. ComfyUIs `LoraLoader` scheitert
  nicht an Keys, die nicht passen, er überspringt sie; **ob das auf 4B
  trainierte LoRA auf dem 9B-Modell überhaupt greift, ist damit nicht
  belegt**. Für einen echten Wirkungstest fehlt ein FLUX.2-[klein]-**4B**-
  Inferenz-Checkpoint in der Bibliothek.
- **Import-Pfad mit gemischten Trennzeichen**: die importierte LoRA landet
  unter `E:\AI\models\image/loras\myrender-v2.safetensors` — funktioniert
  unter Windows, sieht aber in Logs und UI falsch aus.
- **`reached_total` lebt nur im Speicher**: stirbt die App zwischen „Bar hat
  `N/N` erreicht" und dem Ende des Trainer-Prozesses, ist der Erfolgsnachweis
  weg und der Lauf wird nach dem Neustart als `interrupted` verbucht (dasselbe
  galt vorher für den Completion-Marker).

## ComfyUI Workflow-Engine — ✅ Phase A + Hi-Res-Fix umgesetzt (2026-09-17)

Plan: `docs/superpowers/plans/2026-09-17-comfyui-workflow-engine-plan-3.md`,
Design + Entscheidungstabelle:
`docs/superpowers/specs/2026-09-17-comfyui-workflow-engine-design.md`.

**Inspiration, keine Kopie:** Die Idee einer Fragment-Schicht stammt aus einem
parallelen lokalen AI-Studio-Projekt (`E:\locally-uncensored`, AGPL-3.0);
übernommen wurde ausschließlich das *Muster* „Graph-Bau in kleine, benannte,
einzeln getestete Fragmente zerlegen", kein Code. Roh importierte
Workflow-JSONs aus dem ComfyUI-Editor bleiben bewusst außen vor — das brächte
genau die Node-Graph-Komplexität zurück, die die App vor dem Nutzer versteckt.

**Phase A — Fragment-Schicht:**
- `core/src/pipeline/graph.rs`: `Graph`, `OwnedLink`, `NextId`, `Dim` — der
  Graph-Builder, auf dem alles andere sitzt.
- `core/src/pipeline/fragments/{loaders,conditioning,latent,sampling,output,loras,ipadapter,reference,video,upscale,hires}.rs`
  — je ein benanntes, einzeln getestetes Fragment.
- `core/src/pipeline/recipes/{image,story,video,upscale}.rs` mit fester
  Node-Id-Karte (`recipes::ids`: LoRA 90–94, Hi-Res 40–44).
- `core/src/pipeline/mod.rs` schrumpft von 2187 auf 386 Zeilen und ist nur noch
  Typen + öffentliche Fassade.
- **Byte-identisch bewiesen:** 16 Golden-Fixtures (`core/tests/fixtures/graphs/`)
  wurden *vor* dem Refactor aus den handgeschriebenen Graphen erzeugt; die
  komponierten Rezepte liefern exakt dieselben JSONs. Dazu 2 neue
  `*_hires.json`-Fixtures, die es vorher nicht geben konnte.

**Phase B.1 — Hi-Res-Fix:**
- `HiresFix { scale_by 1,25–2,0, denoise 0,2–0,7, steps 4–60 (Default: halbe
  First-Pass-Steps), upscale_method }` — geklemmt schon an der Param-Grenze
  (`ImageRequest::from_params`), die Rezepte verdrahten ungeprüft weiter.
- KSampler-Familie (SDXL-Checkpoint, FLUX.1 GGUF, FLUX.2 [klein] safetensors):
  `"40"` `LatentUpscaleBy` → `"41"` `KSampler` mit `denoise`.
- FLUX.2 [klein] GGUF (kein `KSampler`): `"42"` `Flux2Scheduler`
  (`steps = round(steps/denoise)`) → `"43"` `SplitSigmasDenoise` → `"44"`
  `SamplerCustomAdvanced`. Dadurch heißt `hires.steps` in **jeder** Familie
  dasselbe: tatsächlich ausgeführte Schritte.
- **Eine** Größenformel für Graph *und* Capability-Ebene:
  `pipeline::latent_upscaled_px` = `round(px/8·s)·8` (`LatentUpscaleBy` rundet
  im Latent, nicht in Pixeln — 1000 px × 1,25 sind 1248, nicht 1250). Die in
  `output_width`/`output_height` zurückgeschriebene Größe kann deshalb nicht
  von der echten Ausgabe abweichen.
- VRAM-Headroom skaliert mit den Pixeln des **zweiten** Passes
  (`ImageRequest::final_size()` → `media_vram_mb`, `orchestrator/engine.rs`).
- API: `params.hires`; Edit- und Referenz-Renders (Story Studio) ignorieren den
  Block bewusst — ein Edit hat kein eigenes Latent, und ein Referenz-Render
  tauscht Auflösung gegen Charakter-Konsistenz.
- Beweis am real gesendeten Graphen: `/__test/last_graph_node_types` im
  `aiwm-fake-comfy` + `core/tests/image_job.rs`.
- UI: Toggle im Image-Tab (`ui/src/features/image/HiresFixField.tsx`) — nicht im
  Story Studio, nicht im Edit-Modus.

### Gemessen (2026-09-17, RTX 4080 SUPER 16 GB)

Echter `aiwm-cored` aus dem Worktree gegen das von der App verwaltete ComfyUI
(`E:\AI\data\runtimes\comfyui`), keine Fixtures. Prompt „a lighthouse on a rocky
coast at golden hour, dramatic clouds, highly detailed", Negativ „blurry,
lowres", 1024×1024, 25 Steps im ersten Pass; Hi-Res mit denoise 0,45, 12
ausgeführten Schritten, `nearest-exact`. Jede Zeile ist der Median aus ≥ 3
Läufen mit **je eigenem Seed** — ComfyUI cached Node-Ausgaben, ein wiederholter
Seed liefert dasselbe Bild in ~1,5 s zurück, ohne zu rendern. VRAM-Spitze =
Maximum aus `nvidia-smi` mit 5 Messungen/s über den ganzen Job, **inklusive**
~1,4 GB Desktop-Grundlast und des residenten Modells (also Karten-Gesamtbelegung,
nicht der Anteil eines Passes).

| Modell | Auflösung | Hi-Res | Wandzeit warm | VRAM-Spitze | Ausgabe | Ergebnis/Anmerkung |
|---|---|---|---|---|---|---|
| SDXL base 1.0 (safetensors, cfg 7, euler/normal) | 1024×1024 | aus | **5,7 s** (3×5,7) | 10 573 MB | 1024×1024 | Referenzlauf |
| SDXL base 1.0 | 1024×1024 | 1,5× | **12,6 s** (12,0–13,2) | 14 005 MB | 1536×1536 | +6,9 s, Faktor 2,2. Sichtbar mehr Fels-/Gischtdetail; das Web-/Gittermuster, das das Basisbild im Wasser zeigt, verschwindet. Komposition bleibt erhalten. |
| SDXL base 1.0 | 1024×1024 | 2,0× | **19,8 s** (18,0–21,1) | 14 701 MB | 2048×2048 | **Passt in 16 GB** — kein OOM, keine Planer-Absage, ~1,7 GB Luft. Aber: Bei 2,0× + denoise 0,45 verschiebt der zweite Pass die Komposition sichtbar (Leuchtturm wandert und schrumpft) — Detailgewinn ja, „nur schärfer" nein. |
| FLUX.2 [klein] 9B fp8mixed (safetensors → KSampler-Familie, guidance 4) | 1024×1024 | aus | **15,6 s** (3×15,6) | 13 075 MB | 1024×1024 | Referenzlauf |
| FLUX.2 [klein] 9B fp8mixed | 1024×1024 | 1,5× | **34,4 s** (33,0–35,8) | 14 095 MB | 1536×1536 | +18,8 s, Faktor 2,2. Deutlich mehr Textur in Fels, Gischt und Wolken bei praktisch identischer Komposition — bestes Qualität/Zeit-Verhältnis der Messreihe. |
| FLUX.2 [klein] **GGUF** (`SamplerCustomAdvanced`-Zweig) | — | — | — | — | — | **Nicht messbar** — es ist kein GGUF-klein-Modell installiert. Dieser Zweig ist ausschließlich fixture-bewiesen (`flux2_klein_txt2img_hires.json`). |

Belegbilder (gitignored, außerhalb des Repos versioniert):
`E:\AI\.smoke-hires\` — `sdxl-seed202-base-1024.png` /
`sdxl-seed202-hires1.5x-1536.png`, `sdxl-seed523-base-1024.png` /
`sdxl-seed523-hires2.0x-2048.png`, `klein9b-seed302-base-1024.png` /
`klein9b-seed302-hires1.5x-1536.png` (jedes Paar mit identischem Seed: das
Basisbild *ist* der erste Pass des Hi-Res-Laufs) plus drei
Seite-an-Seite-Ausschnitte `cmp-*.png`. Die Auflösungen oben sind aus den
PNG-Headern gelesen, nicht aus den Job-Params.

**Kaltstart (einmalig, nicht in der Tabelle):** Der erste Bildjob einer Sitzung
startet zusätzlich den ComfyUI-Server und lädt das Modell — SDXL 1024²
ohne Hi-Res: **45,6 s**. Der erste klein-Job danach (Server läuft, SDXL wird
verdrängt, klein + Qwen-Text-Encoder werden geladen): **23,3 s**. Läufe, die
mitten in einer Sitzung zwischen SDXL und klein hin- und herschalten, kosten
24–41 s statt der warmen 15,6 s — das Umladen dominiert, nicht das Sampling.

**Ehrliche Lücken der Messung:**
- **FLUX.2 [klein] 1,5× ist am VRAM-Limit und wird situativ abgelehnt.** Der
  Planer verrechnet `13 092 MB` (Import-Schätzung) als `10 532 MB` Gewichte +
  `2 560 MB` Headroom, und skaliert den Headroom mit 2,25 (1,5² Pixel) →
  **16 292 MB** gegen ein Budget von 16 376 MB, also 99,5 %. Ist klein bereits
  geladen oder die Karte sonst leer, läuft der Job (siehe Tabelle); liegt noch
  SDXL im Speicher, kommt der Job gar nicht erst zum Rendern, sondern wird
  `blocked` — wörtlich: „not enough VRAM for flux-2-klein-9b-fp8mixed: 16292 MB
  needed, but this GPU only has 6176 MB usable in total — this model doesn't fit
  this card no matter what else is running. Try a smaller quant/model." Der Text
  ist in dieser Situation außerdem irreführend: die Karte *hat* das Modell
  gerade eben noch getragen, nur nicht zusätzlich zum residenten SDXL. Beides
  ist ein Kalibrierungs-/Meldungs-Thema für später, keine Hi-Res-Fix-Regression.
- **Der GGUF-klein-Zweig ist ungemessen** (kein Modell installiert), siehe
  Tabellenzeile.
- **Die 2,0×-Zeile misst „passt", nicht „ist gut":** technisch bestanden,
  gestalterisch ist 2,0× bei denoise 0,45 schon eine Neuinterpretation. Wer
  wirklich nur schärfen will, bleibt bei 1,25×–1,5× oder senkt denoise.

**Offen / später:**
- **Face-Restoration** als Post-Process-Fragment (Phase B.2, unverändert
  eingeplant).
- **ControlNet / Region-Conditioning** (Phase B.3) — braucht weiterhin die
  echte Recherche, welches Preprocessor-Node-Pack genommen wird; bewusst noch
  nicht entschieden.
- **Globale Qualitätsstufen** statt (oder zusätzlich zu) dem Per-Generation-
  Toggle: heute ist Hi-Res-Fix ausschließlich ein Schalter pro Bild.
- **Hi-Res-Fix für Video** — die Video-Rezepte (`wan_ti2v`, `ltx_video`) haben
  keine Hi-Res-Naht; das temporale Latent zweimal zu sampeln ist auf 16 GB
  nicht ohne eigene Messreihe zu haben.
- **Sampler/Scheduler pro Pass** — der zweite Pass erbt heute Sampler und
  Scheduler des ersten; getrennte Wahl wäre ein eigener Slice.
- **Rundungs-Gleichstand Python vs. Rust:** ComfyUI rundet die Latent-Größe mit
  Pythons `round()` (Ties zur geraden Zahl), `latent_upscaled_px` mit Rusts
  `f64::round` (Ties von der Null weg). Die beiden weichen nur ab, wenn ein
  Gleichstand auf einem ungeraden Ergebnis landet (Latent-Breite 162,5: Python
  162, Rust 163 → 8 px Unterschied). Nichts pinnt das Python-Verhalten heute
  fest; die Rust-Semantik bleibt bewusst stehen, siehe Doc-Kommentar an
  `pipeline::latent_upscaled_px`.

## Ideen aus `locally-uncensored` (recherchiert 2026-09-16, kein Code übernommen)

Ein paralleles lokales AI-Studio-Projekt (`E:\locally-uncensored`, AGPL-3.0)
wurde auf UX-/Architektur-Ideen durchsucht ("Ideen/Layouts klauen, nie
Code" — AGPL-Lizenz macht Code-Kopieren ohnehin heikel). Ergebnis unten;
**LoRA-Stack-UI wird direkt umgesetzt** (siehe eigener Abschnitt), der Rest
ist Backlog für spätere Slices, absteigend nach Aufwand geordnet:

- **LoRA-Stack mit Pro-Item-Stärke-Regler** — ✅ umgesetzt, siehe eigener
  Abschnitt unten. `core::pipeline` unterstützt eine LoRA-Kette mit
  Pro-LoRA-Stärke seit längerem (`LoraSpec`/`apply_loras`); die UI-Seite
  fehlte bzw. traf die Spec nicht genau.
- **Hardware-Fit-Badge im Discover-Tab** — ✅ umgesetzt (2026-09-18). Statt
  des farbigen Punkts mit `title=`-Tooltip jetzt ein beschriftetes
  Tier-Badge (`Fits` / `Tight` / `Too big` / `Unknown`, je mit eigener Form,
  VRAM-Schätzung und der Begründung des Cores als aufklappbarer, per
  Tastatur und Screenreader erreichbarer Text) — geteilt von Discover,
  Katalog und Upgrade-Check. Die Dateiliste eines aufgelösten Modells wird
  nach Tier sortiert (stabil innerhalb eines Tiers), zeigt eine
  Tier-Zusammenfassung ("3 fit · 2 tight · 4 too big for this GPU") und
  einen Filter "Hide files that won't fit" (aus by default, nennt beim
  Filtern immer die Zahl der ausgeblendeten Zeilen). Kein Core-Change nötig:
  `core::compat` liefert `FitVerdict` samt Begründung längst, es fehlte nur
  die Darstellung; ein "Too big"-File bleibt weiterhin herunterladbar — das
  Badge informiert, es blockiert nicht.
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
- **Personas** — ✅ umgesetzt (2026-09-18). Presets aus Name, Icon (ein Emoji)
  und System-Prompt: neue Tabelle `personas` plus `sessions.persona_mode`
  (`inherit` · `none` · `persona`) und `sessions.persona_id` (Migration `0018`,
  rein additiv), global aktive Persona als Settings-Schlüssel
  `chat.active_persona_id`. **Aufgelöst wird serverseitig beim Job-Start**, nicht
  im Client: Session-Override schlägt die globale Wahl, `none` heißt ausdrücklich
  „keine", sonst gilt die globale. Eine verwaiste Id heilt sich unterwegs selbst
  (bedingtes Clear, damit eine gleichzeitig getroffene Wahl nicht überschrieben
  wird; heilt der Heal keine Zeile, wurde die Session gerade umgehängt und die
  frisch gelesene Zeile gewinnt) — ein Chat scheitert nie an einer gelöschten
  Persona. Der System-Prompt geht **unverändert** als System-Nachricht vor die
  User-Nachricht (das Tool filtert nichts; Grenzen nur technisch: Name 1–60
  Zeichen, Icon ≤ 64 Bytes, Prompt 1–8000 Zeichen); **ohne** Persona ist der
  Request byte-identisch zu vorher (angenagelter Test bleibt grün). Der Job
  schreibt `persona: {id, name, icon}` in seine Params zurück und setzt ein
  Info-Event `persona: <icon> <name>`, damit der Verlauf auch nach Umbenennen
  oder Löschen zeigt, wer geantwortet hat. UI: Persona-Chip mit Herkunft
  („global" / „dieser Chat"), Menü und Verwalten-Dialog (Vorlagen, Bearbeiten,
  Löschen mit Bestätigung) im Chat-Tab, Persona-Marke an jeder Antwort. Gilt für
  llama.cpp- **und** Colibri-Chats (beide Clients tragen die System-Nachricht);
  Agents, Story Studio, Benchmarks bleiben unberührt. Der Prompt-Assistent
  (Image/Video/Voice, „talk through what you want") bleibt **bewusst
  persona-frei**: seine Antwort wird auf `PROMPT:`/`NEGATIVE:`-Zeilen geparst und
  er wird aus Tabs abgeschickt, für die niemand eine Stimme gewählt hat — Jobs
  mit `assistant_for` überspringen die Auflösung ganz (keine System-Nachricht,
  keine `persona`-Params, kein Event). Routen siehe
  [DEV_SETUP.md](DEV_SETUP.md).
  **Echter Lauf (2026-09-18, RTX 4080 SUPER, Mistral-Small-3.2-24B IQ3_M,
  dreimal dieselbe Frage „In one sentence, what is a compiler?"):** mit globaler
  Persona „🏴‍☠️ Pirate" (`origin: global`) → „*Arrr, a compiler be the scurvy
  dog that turns me code into machine language, savvy? Arr!*"; mit
  Session-Override `none` (`origin: none`) → „*A compiler is a program that
  translates code written in a high-level programming language into machine code
  for execution.*"; ohne Persona überhaupt (globaler Schlüssel geleert, Session
  zurück auf `inherit`) → **wortgleich** dieselbe Antwort wie mit `none`, und
  ohne `persona` in den Job-Params und ohne Persona-Event. Die Persona
  anschließend gelöscht, während der Chat per `mode: persona` auf sie zeigte →
  `GET /sessions` zeigt ihn sofort wieder auf `inherit`, der globale Schlüssel
  ist leer, und der vierte Chat lief normal durch.
  **Offen:** mehrstufiger Gesprächsverlauf (der Chat schickt weiterhin nur die
  aktuelle Nachricht — siehe „Kompaktions-Records für lange Chats" oben),
  Personas für Agents/Story Studio, Import/Export, Variablen/Platzhalter im
  Prompt, `eslint-plugin-jsx-a11y` für die neuen Dialoge.
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

## LoRA-Stack-UI — ✅ umgesetzt (2026-09-16)

Schließt die oben genannte Lücke: `core::pipeline`s `LoraSpec`/`apply_loras`
(interner Name der Funktion, die den Doc-Text oben `splice_loras` nennt)
existieren seit längerem und werden von **jedem** Bild- und Video-Rezept
genutzt — `checkpoint_txt2img`, `flux_txt2img`, `flux2_klein_txt2img`,
`flux2_klein_txt2img_safetensors`, `flux2_klein_edit`, `wan_ti2v`,
`ltx_video` nehmen alle einen `loras: &[LoraSpec]`-Parameter. Genauso war
die Job-Param-Ebene (`ImageRequest`/`VideoRequest` in
`core/src/capability/{image,video}.rs`, `parse_loras`/`resolve_loras` in
`capability/media.rs`) bereits vollständig verdrahtet, inklusive eigener
Unit-Tests für Parsing/Auflösung. Die **einzige** echte Lücke war die UI:
`ui/src/components/LoraPicker.tsx` existierte zwar schon (aus einer
früheren Session/Slice) und war in `Image.tsx`/`Video.tsx` eingehängt, traf
aber die hier verlangte Spec nicht exakt — dieser Slice bringt sie in Deckung
und liefert den bis dahin fehlenden Beweis-Test:

- **Default-Stärke auf 0.8 umgestellt** (vorher 1.0) — passend zum Wert, den
  `core::pipeline`s eigene Tests durchgängig als Beispiel nutzen
  (`add-detail-xl.safetensors @ 0.8`, `flux-2-realistic-detail @ 0.8`).
- **Regler-Bereich auf 0–2 umgestellt** (vorher −2–2) und von einem
  Zahlenfeld auf einen echten `<input type="range">`-Slider umgestellt —
  gleiches Muster wie der Speed-Regler im Voice-Tab
  (`Voice.tsx`/`voiceform__field input[type="range"]`), inklusive
  Live-Wertanzeige `Name — 0.80`.
- **Bleibt sichtbar bei null importierten LoRAs**, mit Hinweistext statt
  komplett zu verschwinden (vorher: `return null` bei leerer Liste) —
  mirrort das an anderer Stelle beobachtete "leer, aber mit
  Ordner-Hinweis"-Muster (z. B. "No image checkpoint yet — import an SDXL
  `.safetensors`…").
- **Ausblenden für Familien ohne LoRA-Seam**: `FAMILIES_WITHOUT_LORA_SEAM` in
  `LoraPicker.tsx` ist ein explizites (aktuell **leeres**) Array statt eines
  stillschweigenden "immer sichtbar" — geprüft gegen `core/src/pipeline/
  mod.rs`: **jedes** heute ausgelieferte Rezept (SDXL/einfacher Checkpoint,
  FLUX.1, FLUX.2 [klein] GGUF/safetensors/Edit, Wan 2.2, LTX-Video) hat
  eine LoRA-Naht. Es gibt also aktuell **keine** Familie, die die Sektion
  verstecken müsste — anders als in der Aufgabenstellung vermutet, aber
  ehrlich im Code dokumentiert statt stillschweigend weggelassen, falls
  künftig ein Rezept ohne `loras`-Parameter dazukommt.
- **Kein Rescan-Button gebaut**: `useModels()` pollt bereits alle 3 s
  (`ui/src/lib/hooks.ts`) — eine mitten in der Session importierte LoRA
  taucht ohne App-Neustart und ohne manuellen Rescan von selbst auf; ein
  zusätzlicher Button wäre reine Redundanz gewesen.
- **`ui/src/lib/dev-mock.ts`** um ein Wan-Family-LoRA-Fake
  (`m-lora-wan-motion`, Name identisch zum Fixture-Namen aus
  `core::pipeline`s eigenen Tests, `wan-motion.safetensors`) ergänzt, damit
  der Video-Tab im Dev-Preview eine familien-passende LoRA zum Stapeln hat
  (SDXL-/Flux-/FLUX.2-LoRAs existierten dort bereits).
- **Neuer End-to-End-Beweis-Test**
  (`an_image_job_with_a_lora_splices_a_loraloader_into_the_real_graph` in
  `core/tests/image_job.rs`): submittet einen echten Job mit
  `params.loras = [{ model_id, strength: 0.65 }]` durch die reale
  `JobEngine`, und prüft **nicht nur**, dass der Job `Completed` erreicht,
  sondern fragt danach den tatsächlich von ComfyUI empfangenen Graphen ab
  — dafür wurde `aiwm-fake-comfy` (das Test-Fixture) um einen
  Introspektions-Endpunkt `GET /__test/last_lora_chain` erweitert, der die
  `LoraLoader`-Kette des zuletzt eingereichten Graphen (Datei + Stärke, in
  Ketten-Reihenfolge) zurückgibt. Damit ist die komplette Kette
  UI-Param-Shape → `parse_loras` → `resolve_loras` → `pipeline::
  checkpoint_txt2img`'s `LoraLoader`-Node einmal end-to-end bewiesen, nicht
  nur stückweise per Unit-Test (die es für jedes einzelne Glied schon vorher
  gab).

**Gates**: `cargo fmt --all -- --check` sauber, `cargo clippy --workspace
--all-targets -- -D warnings` sauber (0 Warnungen), `cargo test --workspace`
sauber — **826 bestanden, 0 fehlgeschlagen, 2 ignoriert** (netzwerkabhängige
Hugging-Face-Tests). UI: `pnpm typecheck`/`pnpm lint`/`pnpm build` sauber.
Live gegen `dev-mock` im Browser verifiziert (eigener Dev-Server aus diesem
Worktree heraus gestartet, da der zuvor laufende `ui-dev`-Server auf einen
anderen Checkout zeigte): zwei LoRAs gleichzeitig aktiv mit unterschiedlicher
Stärke (0.80 / 1.55) im Image-Tab, Entfernen einer LoRA per Checkbox
bestätigt (Regler verschwindet, andere LoRA bleibt unverändert), gleiches
Verhalten im Video-Tab mit der neuen Wan-LoRA bestätigt.

## Benchmark-Tab — ✅ umgesetzt (2026-09-17)

Erfüllt den User-Wunsch „Wie viele Tokens/s produziere ich mit Modell X?":
Modell wählen, vordefinierten Test starten, Ausgabe messen. `job_type=bench`
(`core/src/bench/mod.rs`) lief schon seit 6.5 als Job über die JobEngine gegen
llama.cpp und lieferte Gen-/Prefill-tok/s, Kaltstart-Ladezeit und einen Score —
gefehlt haben die **versionierten Test-Sets**, die **feste Ausgabelänge**, die
**Pro-Prompt-Aufschlüsselung** und ein **eigener Tab**. Alles vier ist da:

- **Versionierte Suiten** (`core/src/bench/suites.rs`): `chat-v1` (erklären /
  zusammenfassen / E-Mail höflich umschreiben, 256 Tokens) und `coding-v1`
  (Funktion aus Spec schreiben / Bug finden und fixen / Code erklären +
  Testfälle vorschlagen, 384 Tokens). Selbst geschriebene englische Prompts,
  `-vN`-Suffix ist Teil der Id — ein geänderter Prompt wird `-v2`, damit alte
  Zeilen in der Historie vergleichbar bleiben.
- **Feste Ausgabelänge, deterministisch** (`GenerationOptions::fixed_length()`
  in `core/src/runtime/llamacpp/client.rs`): `ignore_eos`, `temperature 0`,
  `seed 0`, `cache_prompt false`. **Das war der eigentliche Fund dieses
  Slices:** ohne diese vier Felder hat ein Pass auf dem *echten* llama-server
  **2 Tokens** gemessen (das Modell hörte nach der Höflichkeitsfloskel auf) und
  einen **vollständig gecachten Prefill** — also eine Prefill-Rate, die nichts
  mehr mit Prompt-Verarbeitung zu tun hatte. Gegen `aiwm-fake-llama` war das
  nie aufgefallen, weil das Fixture immer bis zum Cap generiert. Der
  Kontrast ist unten gemessen: derselbe resident geladene Mistral liefert im
  Quick-Test **79 tok/s Prefill**, in der Suite **2369–2464 tok/s**.
- **Pro-Prompt-Detail + Pro-Prompt-Stabilität**: `BenchReport.detail`
  (`PromptResult` je Prompt mit `tokens`/`max_tokens`/gen/prefill) und
  `stability_score` = **Mittel der Pro-Prompt-Stabilitäten**. Über alle
  Prompts gepoolt hätte „Code ist langsamer als Prosa" als Jitter gezählt;
  innerhalb eines Prompts sind die Pässe wirklich vergleichbar.
- **Frühe Suite-Validierung**: eine unbekannte Suite lässt den Job sofort
  mit Klartext scheitern, statt erst nach dem Modell-Load.
- **Migration `0017`** (`suite`, `detail_json` — additiv, nullable) +
  `list_all(suite, limit)` in `core/src/db/bench.rs`.
- **API**: `GET /bench/suites`, `POST /models/{id}/benchmark` mit optionalem
  Body `{suite, runs}` (leerer Body = der alte Quick-Test),
  `GET /benchmarks/history?suite=&limit=` (Limit auf `1..=200` geklemmt).
- **UI**: `ui/src/features/benchmark/` — Container + Formular (Modell, Suite
  mit ausklappbarer Prompt-Liste, 1–5 Pässe pro Prompt), Live-Panel
  (`pass k / total` aus den Job-Events), Result-Karte (tok/s zuerst, dann
  Prefill / Ladezeit / VRAM-Spitze / Stabilität / Pro-Prompt-Tabelle, Warnung
  wenn ein Prompt vor dem Cap aufgehört hat), Vergleichstabelle (neuester Lauf
  pro Modell für die gewählte Suite, Balken via `scaleX`) und Pro-Modell-
  Historie. Die reinen Helfer liegen in `benchmark-utils.ts`.

**Gemessen (2026-09-17, RTX 4080 SUPER 16 GB, echter `aiwm-cored` + echter
llama-server `b10855`, Leerlauf-VRAM 705 MB):**

| Modell | Suite | tok/s | Prefill tok/s | Ladezeit | VRAM-Spitze | Stabilität | Pässe | Tokens/Prompt | Anmerkung |
|---|---|---|---|---|---|---|---|---|---|
| Mistral-Small-3.2-24B-Instruct-2506 ultra-uncensored-heretic, IQ3_M (10,7 GB) | `chat-v1` | **56,35** | 2464 | 6090 ms (kalt) | 12406 MB | 0,9995 | 6 (3 × 2) | 256/256, 256/256, 256/256 | Wall 37 s inkl. Kaltstart |
| dito | `coding-v1` | **56,32** | 2369 | — (resident) | 12405 MB | 0,9997 | 6 (3 × 2) | 384/384, 384/384, 384/384 | Wall 45 s |
| dito | — (Quick-Test, leerer Body) | **56,66** | **79** | — (resident) | 12415 MB | 0,9991 | 3 | 128 (Cap), EOS-terminiert, kein Detail | Wall 8 s; Prefill bricht ein, weil der Quick-Test `cache_prompt` anlässt |
| Qwen2.5-7B-Instruct **F16** (15,2 GB) | `chat-v1`, `coding-v1`, Quick-Test | — | — | — | — | — | 0 | — | Vom VRAM-Planer abgelehnt (Wortlaut unten) |

**Kern-Check bestanden:** auf dem echten llama-server hat **jeder** der zwölf
Suite-Detail-Einträge `tokens == max_tokens`, und **keine** `notes` enthält
„stopped early" — genau das, was `fixed_length()` garantieren soll. Die
`notes` lauten `"cold load; suite chat-v1, 6 pass(es) averaged"` bzw.
`"model already resident (load time not measured); suite coding-v1, 6 pass(es)
averaged"`.

**Qwen2.5 7B F16 wird abgelehnt** — nicht umgangen, sondern so protokolliert.
Wortlaut aus `jobs.error_text` (erster Versuch, GPU im Leerlauf):

> not enough VRAM for Qwen2.5 7B Instruct: needs ~15.0 GB (weights 14.2 GB + KV
> cache 0.4 GB @ 8K ctx + 0.3 GB overhead) — 15329 MB needed, but this GPU only
> has 14840 MB usable in total — this model doesn't fit this card no matter what
> else is running. Try a smaller quant/model.. Free VRAM by closing the resident
> model, or import a smaller quant / lower the context.

Die F16-Variante passt also auf diese Karte grundsätzlich nicht — ein
Q4/Q5-Quant desselben Modells wäre die Lösung, ist aber nicht installiert.
`E:\AI\models\llm\downloaded-7b{,-cb7e76e9}\qwen.Q4_K_M.gguf` sind 40-kB-Stubs
aus einem Download-Test, keine echten Modelle; deshalb steht in der Tabelle nur
**ein** real gemessenes Modell.

**Inspiration (Ideen, kein Code kopiert):** `ggml-org/llama.cpp` →
`llama-bench` für die Grundidee „feste Länge, pp und tg getrennt messen" (genau
das macht `fixed_length()`). `Aider-AI/aider` („polyglot benchmark") und
`openai/human-eval` / `bigcode-project/bigcode-evaluation-harness` bleiben
Vorbilder für eine **spätere** Korrektheits-Phase, nicht für diesen Slice —
hier wird kein generierter Code ausgeführt oder bewertet.

**Bewusst nicht enthalten:** Qualitäts-Benchmarks mit Judge-Modell,
Bild-/Video-Benchmarks, Netz-Leaderboards. Die App bleibt offline-first
(ADR-024: keine Qualitäts-Achse).

**Offen / später:**
- **Coding-Korrektheit mit Sandbox** — generierten Code wirklich ausführen und
  Tests laufen lassen (HumanEval-/Aider-Zuschnitt). Braucht eine eigene Spec
  *und* eine Sicherheitsentscheidung: ein Modell erzeugt beliebigen Code, der
  darf nicht ungeschützt auf dem Rechner des Users laufen.
- **Andere Runtimes als llama.cpp** — `benchmark_model` lehnt heute alles ab,
  was nicht GGUF ist; Hermes/Colibri/ComfyUI haben keine vergleichbare
  Token-Telemetrie.
- **Batch-Size- und Kontext-Sweeps** (llama-bench misst pp/tg je Batchgröße;
  die App fährt nur eine Konfiguration).
- **Export** der Historie (CSV/JSON) für Vergleiche außerhalb der App.
- **UI-Testrunner für die reinen Helfer** in `benchmark-utils.ts` — es gibt im
  `ui/`-Paket noch kein Testframework, die Helfer sind nur über die Live-Smoke
  abgedeckt.
- **Der Quick-Test in der Model Library nutzt weiter die alte Methode**
  (EOS-terminiert, Default-Sampling, Prompt-Cache an). Er bleibt absichtlich
  so — er ist der schnelle „läuft das Modell überhaupt und wie schnell
  ungefähr"-Test, und seine Historie soll vergleichbar bleiben. Die Score-Chip
  in der Model Library kann deshalb eine Suite-Zeile **oder** eine
  Quick-Test-Zeile zeigen; welche, hängt nur daran, was zuletzt lief.
- **Der Tauri-IPC-Pfad für Suite-Läufe ist nur über den Dev-Mock abgedeckt**
  (das HTTP-Zwilling hat Tests) — ein Smoke-Schritt, der prüft, dass ein aus
  dem Tab gestarteter Lauf ein nicht-NULL `suite` speichert, ist noch offen.

## PRIO 1 — offene Bugs (Stand 2026-09-18)

- **[PRIO 1] Dataset-Pipeline findet Videos direkt im Root-Ordner nicht.**
  Fehler beim Start mit einem Ordner, der nur `.mp4`-Dateien enthält:
  `runtime error [dataset]: no videos or images found under D:\Data\Test
  (expected tag subfolders containing .mp4/.png/.jpg/.jpeg/.webp files)`.
  **Ursache (geprüft, 2026-09-18):** `walk_dataset_root`
  (`core/src/capability/dataset/ingest.rs:62`) sammelt nur die *unmittelbaren
  Unterordner* (`min_depth(1).max_depth(1)`, nur `is_dir()`) und liest Dateien
  ausschließlich darin; Dateien direkt im Root werden nie betrachtet.
  `D:\Data\Test` enthält genau eine Datei `20250111-_1.mp4` (~154 MB) ohne
  Unterordner → leere Liste → dieser Fehler. Erwartung des Nutzers: "Ordner mit
  Videos auswählen" muss einfach funktionieren.
  **Fix-Richtung:** Dateien im Root zusätzlich aufnehmen, Tag = Name des
  Root-Ordners (oder ein neutraler Tag, im Formular überschreibbar);
  Unterordner-als-Tag bleibt für strukturierte Datasets. Groß-/Kleinschreibung
  der Endung prüfen (`.MP4`). Die Fehlermeldung und der Hinweistext im
  Dataset-Formular müssen das neue Verhalten erklären. Tests: Root nur mit
  Dateien, Root gemischt mit Unterordnern, leerer Root.
- **Test `daemon_shutdown::boots_and_shuts_down_cleanly_on_ctrl_break` scheitert,
  solange die Desktop-App läuft** (niedrige Prio, blockiert aber den Push, weil
  der Pre-Push-Hook alle Tests fährt): der Test startet `aiwm-cored` auf dem
  festen Port 48160; hält `aiwm-tauri` diesen Port, beendet sich der
  Test-Daemon vor seinem Ready-Banner (`core/tests/daemon_shutdown.rs:58`).
  Beobachtet 2026-09-18 beim Push eines reinen Doku-Commits. Fix-Richtung: der
  Test gibt per `AIWM_CORE_API_PORT` einen freien Port mit, ohne die
  Config-Default-Tests zu stören, die genau diese Variable ungesetzt erwarten.

## Dataset-Kuratierung & Trainings-Werkzeuge (Backlog, User-Wunsch 2026-09-18)

Aus dem ersten praktischen Blick auf den Trainings-Workflow ("100 Videos rein,
Frames sortieren, Training starten"). Heute gibt es pro Frame nur eine
Exclude-Checkbox im Kuratier-Grid (`ui/src/features/dataset/Dataset.tsx`) —
für hunderte bis tausende Frames aus Videos ist das zu langsam.

- **Schnelles Sortieren im Kuratier-Grid: zwei Spalten + Drag & Drop** — links
  "Behalten", rechts "Aussortiert" (unscharf / wird nicht genutzt).
  Mehrfachauswahl wie im Windows-Explorer: Rahmen mit der Maus aufziehen
  (Rubber-Band-Auswahl), Strg/Shift-Klick, **"Alle auswählen"**; die Auswahl
  per Drag & Drop in die jeweils andere Spalte ziehen. Frames, die der
  Qualitätsfilter (`filter.rs`: Unschärfe/Duplikat) schon aussortiert hat,
  starten rechts, damit man sie mit einem Zug zurückholen kann. Tastatur-
  Äquivalent nötig (Auswahl + Taste zum Verschieben), nicht nur Maus.
- **Frames wirklich löschen** — neben "aussortieren" (bleibt auf Platte, fließt
  nur nicht in den Export) eine Löschen-Option für ausgewählte Frames, mit
  Bestätigung. Klären: Löschen nur die extrahierten Frame-Dateien im
  Dataset-Ordner, **nie** die Quellvideos des Nutzers.
- **Datasets löschen** — ein ganzes Dataset (DB-Zeilen, extrahierte Frames,
  Export) aus dem Dataset-Tab entfernen, mit Bestätigung; Quellordner bleibt
  unangetastet. Laufende/abhängige Trainings-Runs vorher prüfen (ein Run, der
  den Export nutzt, darf nicht still kaputtgehen).
- **Trainings-Werkzeuge im Discover-/Models-Tab statt manuellem Import** —
  heute sagt der Dataset-Tab: "No captioner installed — import Florence-2 or
  the WD tagger on the Models tab. Without one, everything recurring in your
  frames flows into the trigger word." Der Nutzer will nicht manuell
  importieren. Stattdessen: eine eigene Kategorie "Training & Beschriftung"
  (Captioner Florence-2 / WD EVA02 Tagger v3 / Qwen2.5-VL, später weitere
  Hilfsmodelle/Addons fürs Training und für Bildbeschreibung) mit Ein-Klick-
  Download über den bestehenden Download-Weg (Hash-Prüfung, keine geratenen
  SHA-256). Der Hinweis im Dataset-Tab bekommt einen direkten
  "Empfohlenen Captioner installieren"-Button. Knüpft an den offenen Punkt
  "kein `KNOWN_MODELS`-Katalogeintrag für Florence-2/Qwen2.5-VL" in
  Teilsystem 1 an.

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
