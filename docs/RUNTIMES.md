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
| ComfyUI | `ComfyUiAdapter` | ✅ Adapter (3.1) + Installer (3.2) + getypter Store (3.3) + Bild-Job (3.4) + UI/Galerie (3.5) + Flux-Template (3.6) + Politur (3.7) + **Video-Job** (`wan_ti2v`, `generate_media`, 4.1) + **Bild→Video** (4.2) + **Video-UI** (4.3) + **LTX-Template** (`ltx_video`, `VideoRecipe`, 4.4). Offen: echte ComfyUI verproben (4.0) |
| Ollama | `OllamaAdapter` | optional, Phase 3+ (Duplikate transparent, ADR-006) |
| LM Studio | — | vorerst nicht (proprietär, GUI-zentriert); wenn doch, `strategy_for` → `Junction` |

### `LlamaCppAdapter` (Stand 2.4a)

- **Ein `llama-server`-Prozess pro residentem Modell.** `llama-server` bedient
  genau ein Modell; der Adapter startet/stoppt ihn passend zum Scheduler-Slot.
- **Inferenz:** `complete()` (nicht-streamend, `/completion`) und
  `stream_completion(prompt, max_tokens, tx)` (streamend, `/v1/chat/completions`
  — Chat-Vorlage vom Modell). Der Chat-Job (`capability::chat`) nutzt den Stream
  und schreibt die Antwort progressiv in `jobs.result`.
- **Binär-Auflösung:** `AIWM_LLAMACPP_PATH` → Managed-Install unter
  `<data_dir>/runtimes/llamacpp/` → `PATH`. Fehlt alles: `detail() = "not
  installed"`, `load_model` liefert einen Klartextfehler.
- **Start:** `-m <datei> --host 127.0.0.1 --port <frei> --no-webui -ngl 999
  -c <ctx> --flash-attn on` (Defaults in `LlamaServerOptions`, später über
  Settings-UI). `-c` = `compat::effective_ctx(model.ctx_max)` =
  `min(ctx_max, 8192)`, wenn `ctx_size` nicht explizit gesetzt ist — so allokiert
  `llama-server` genau den Kontext, gegen den der Scheduler geplant hat (2.6),
  statt den vollen trainierten (128K → OOM). Health-Gate: `GET /health` bis `200`
  oder Timeout/`GaveUp`.
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
- **Modell-Zugriff (ADR-007, 2.3):** `core::link::strategy_for("llamacpp", …)` =
  `Passthrough` — llama-server liest die kanonische Store-Datei direkt (`-m`).
  Kein Junction/Kopie nötig; der Import verzeichnet den Zusammenhang in
  `model_links` (`GET /models` → `runtimes: ["llamacpp"]`).

### `ComfyUiAdapter` (Stand 3.3)

- **Ein langlebiger Server für alle Bild-Modelle** (nicht ein Prozess pro Modell
  wie llama.cpp), lazy beim ersten `load_model` gestartet. `unload` =
  `POST /free` (Server bleibt oben), nicht Stop. Details + Begründung: ADR-018.
- **Modell-Zugriff (ADR-019):** kein Junction. Der Store ist getypt —
  `<store>/image/{checkpoints,diffusion_models,vae,loras,text_encoders}/` — und
  `ComfyDirs::ensure()` schreibt bei **jedem** Server-Start
  `<comfyui-data>/aiwm-model-paths.yaml` (`base_path: <store>/image` + die fünf
  Ordner-Mappings). `build_spawn_spec` hängt `--extra-model-paths-config <yaml>`
  an. Ein geänderter Store-Pfad greift also beim nächsten Start ohne weiteres
  Zutun. Grund für die Plan-Abweichung: NTFS-Junctions überspannen keine Volumes
  (Store `E:`, ComfyUI-Install `C:`), ComfyUI hat `extra_model_paths.yaml` als
  First-Class-Feature.
- **Installer (3.2):** `runtime::comfyui::install`. `uv` (0.12.11) + gepinnte
  Quelle (`v0.34.0`) + `uv venv` (Python 3.13) + torch `cu130` +
  `requirements.txt` + der eine Custom Node `city96/ComfyUI-GGUF` (Commit
  gepinnt). Die `uv`-Schritte laufen hinter dem `CmdRunner`-Trait (Fake im Test);
  Downloads über das geteilte `runtime::download`. `POST
  /runtimes/comfyui/install` (202) startet im Hintergrund; Fortschritt in `GET
  /runtimes` → `detail`. `offline_mode` = Hard-Refusal. Idempotent (`venv_python`
  + `main.py` + `<node>/__init__.py`).
- **Bild-Job (3.4):** `ComfyClient` bekam `submit_prompt` (`POST /prompt`),
  `history` (`GET /history/{id}`), `view` (`GET /view`); `interrupt` ist der
  Cancel-Hook. `generate_media(workflow, cancel, timeout)` (bis 4.1
  `generate_image`) reiht die API-Format-Graph ein, pollt `/history` alle
  750 ms, holt die Ausgabe via `/view` — spiegelt
  `LlamaCppAdapter::stream_completion`. Der `JobEngine`-Body (`job_type=image`)
  schreibt sie nach `<outputs>/<job_id>.png` und setzt `jobs.output_path`;
  `Auto` wählt das `base_diffusion`-Modell. Die 600-s-Deckelung (Bild) lässt
  einen Job eher fehlschlagen als hängen. Die UI holt das Bild über
  `GET /jobs/{id}/output` (3.5).
- **Video-Job (4.1):** derselbe Weg. `generate_media` ist über `image` **und**
  `video` verallgemeinert — `GeneratedMedia { bytes, extension }`, Timeout als
  Parameter (Video 1800 s), `collect_media` prüft `outputs.<node>.{images,
  videos,gifs}`, Extension aus dem Dateinamen. Der `JobEngine`-Body
  (`job_type=video`) baut `pipeline::wan_ti2v`, schreibt `<outputs>/<job_id>.mp4`,
  Event „video ready — …". `Auto` wählt das `base_video`-Modell,
  `capability::video` löst umt5-Encoder + Wan-VAE über Rolle + Namen auf. Cancel
  = `POST /interrupt`. `capability::media` bündelt die Seed-/Datei-Helfer für
  Bild + Video.
- **Bild→Video (4.2):** der `init_image`-Param ist eine Job-ID (Output eines
  fertigen Bild-Jobs) oder ein Bildpfad. `capability::video::resolve_start_frame`
  löst ihn zu einer existierenden `.png`/`.jpg`/`.webp`-Datei auf,
  `stage_start_frame` kopiert sie nach `ComfyUiAdapter::input_dir()` (=
  `<base>/input/`) als `<job_id>.<ext>`; der bare Name geht als `start_image` in
  `wan_ti2v` → `LoadImage` → `WanImageToVideo.start_image`. Ein `StagedFrame`-
  `Drop`-Guard löscht die Kopie, sobald der Body zurückkehrt (Erfolg / Fehler /
  Cancel). Angelegt **vor** dem „takes several minutes"-Event, damit ein
  ungültiger Frame sofort fehlschlägt.
- **Templates (`core::pipeline`, 3.4/3.6/4.1):** `Recipe::for_family` wählt
  `checkpoint_txt2img` (SDXL & Co, Core-Nodes) oder `flux_txt2img`
  (`UnetLoaderGGUF` + `DualCLIPLoaderGGUF` + `VAELoader` + `FluxGuidance`,
  SD3-Latent, Sampler bei CFG 1). Für Flux löst `capability::image` die drei
  Begleiter (T5 / CLIP-L / VAE) über Rolle + Namen auf; fehlt einer → Klartext-
  Fehler. **Video:** `VideoRecipe::for_family` wählt `wan_ti2v` (`UNETLoader` +
  `CLIPLoader type="wan"` + `VAELoader` → `ModelSamplingSD3` Shift 8 →
  `WanImageToVideo` → `KSampler` `uni_pc`/`simple`) oder `ltx_video` (LTX-Video
  0.9.5 — `CheckpointLoaderSimple` + `CLIPLoader type="ltxv"` → `LTXVConditioning`
  → `EmptyLTXVLatentVideo` bzw. `LTXVImgToVideo` → `LTXVScheduler` →
  `SamplerCustom`), beide → `VAEDecode` → `CreateVideo` → `SaveVideo`.
  `capability::video` löst die Begleiter pro Rezept auf (Wan: umt5 + Wan-VAE ·
  LTX: nur ein t5, VAE im Checkpoint). Feste Templates, keine user-editierbare
  Registry (PHASE_3_PLAN §C).
- **Optionen (`[comfyui]`, 3.7 · 4.5):** `ComfyOptions { vram_mode, extra_args }`
  — `vram_mode` (`auto` / `highvram` / `normalvram` / `lowvram` / `novram`) wird
  zum `--<mode>vram`-Flag beim Spawn. **4.5:** `reserve_vram_mb` (→
  `--reserve-vram <GB>`, `0` = aus, Cap 8 GB) und `extra_args` (roher String,
  whitespace-gesplittet) kommen dazu — `ComfyConfig::to_options()` baut die
  Arg-Liste. Settings-UI, `config.toml`, neustart-pflichtig (ADR-017).
  `detail()` zeigt bei laufendem Server die ComfyUI-Version + den Modus.
- **RAM-Vorabwarnung (4.5):** `capability::video` liest vor dem Render einmal
  `sysinfo::available_memory`; liegt es unter Modellgröße + 6 GB Offload-Slack,
  ein `Warn`-Event (ComfyUI spillt Encoder + Modell ins RAM). Nicht blockierend —
  Heuristik, an echten Läufen zu kalibrieren.

## Link-Manager (`core::link`, ADR-007)

`materialize(canonical_file, dest, strategy) -> PathBuf` macht die kanonische
Datei für eine Runtime erreichbar und gibt den zu ladenden Pfad zurück:

| Strategy | Wie | Für |
|---|---|---|
| `Passthrough` | nichts (kanonischer Pfad) | llama.cpp (`-m <store-datei>`) |
| `ExtraPath` | nichts (No-Op); der Store-Ordner steht in ComfyUIs `extra_model_paths.yaml` | ComfyUI (ADR-019) |
| `Junction` | NTFS-Directory-Reparse-Point (`junction`-Crate, kein Admin, gleiche Volume) | LM Studio o. Ä. (noch kein realer Konsument) |
| `Hardlink` | `std::fs::hard_link`, nur gleiche Volume | Sonderfälle |
| `Copy` | echte Kopie | Ollama (content-addressed Store) |

`strategy_for(runtime_id, _)`: `"llamacpp"` → `Passthrough`, `"comfyui"` →
`ExtraPath`, `"ollama"` → `Copy`, alles andere → `Junction`.

`dematerialize` hebt den Link auf, ohne die Store-Datei zu berühren. Alle
Operationen idempotent. Aktiv: `Passthrough` (Phase 2) + `ExtraPath` (3.3).
`Junction`/`Hardlink`/`Copy` sind gebaut + getestet, aber noch ohne realen
Konsumenten — die Junction-Verprobung gegen eine echte Runtime bleibt offen
(ComfyUI war wegen der Volume-Grenze nicht der richtige Kandidat).

## Manage-first (ADR-002)

Das Tool installiert/versioniert die Runtimes selbst — aber mit **einer** fest
kuratierten, getesteten Version pro Runtime und **einer** Installationsart. Kein
frei konfigurierbares Python-Environment. „Repair"-Pfad baut die venv sauber neu.

## Kompatibilitäts-Check vor dem Load (`core::compat`, ADR-016)

`estimate(&ModelDims, ctx)` schätzt vor dem `load_model` den VRAM-Bedarf:
Gewichte (= Datei) + KV-Cache (fp16, aus GGUF-Arch-Dims `n_layers` / `n_embd` /
`n_heads` / `n_kv_heads`; grobe Reserve wenn die fehlen) + flacher Overhead
(650 MB). Der `JobEngine` plant den Scheduler gegen `total_mb`; passt es nicht,
geht der Job auf `blocked` mit Klartext-`error_text`
(„not enough VRAM for … : weights … + KV cache … @ 8K ctx + … overhead — …"),
**ohne** `llama-server` zu starten. Kalibrierung der Konstanten gegen echte
`nvidia-smi`-Messungen → Phase 6.

## Modell-Quellen (Phase 6, `core::registry`, ADR-022)

6.0-Spike (Live-Probes 2026-09):

- **Hugging Face Hub** = primäre Quelle. Read-API anonym nutzbar (500 API-Calls
  / 5 min / IP), `expand[]` auch auf `GET /api/models`, `filter=base_model:<id>`
  findet Abkömmlinge, `GET /api/models/{id}/tree/{rev}?recursive=true` →
  `lfs.oid` = die **SHA-256** für `download_verified` (nicht `xetHash`).
  Gated-Repos sind durchsuch-/inspizierbar; nur `/resolve/` braucht Lizenz +
  `HF_TOKEN`. Offline-Fallback = TTL-JSON-Cache + `ETag`/`If-None-Match`.
- **Ollama-Library** = **kein Such-API**. `registry.ollama.ai/v2/library/<m>/manifests/<tag>`
  (OCI) liefert `layers[].{digest: "sha256:…", size}` nur für einen bekannten
  Namen. → best-effort-Adapter hinter `ModelSource`, niedrige Priorität.

## Zu untersuchen vor Phase 2/3

- ~~llama.cpp: gepinnte Version + Bezugsquelle des Windows-CUDA-Builds~~ →
  ✅ umgesetzt (2.2b, ADR-014)
- ~~ComfyUI: minimale getestete Custom-Node-Menge~~ → ✅ genau einer,
  `city96/ComfyUI-GGUF`, Commit gepinnt (3.2b)
- ~~`uv`-verwaltete venv pro Runtime; gebündelte CUDA-Runtime statt System-CUDA~~
  → ✅ ComfyUI-Installer (3.2): `uv venv` + torch `cu130`. cu130-Treiber-Bedarf
  auf einer frischen Maschine gegen den echten Server verproben → 3.4
- Health-Endpunkte + Modell-Load/Unload-APIs je Runtime — llama-server: `/health`,
  `/props`, `/completion`, `/v1/chat/completions`; **Router-Mode** (ein Server,
  mehrere Modelle, `?autoload=`) neu — als spätere Optimierung notiert
- ~~Shared Model Cache: welche Runtimes können dieselbe Datei via Junction
  nutzen~~ → ComfyUI liest den Store über `extra_model_paths.yaml` (ADR-019),
  nicht via Junction (Volume-Grenze). Junction bleibt für LM-Studio-artige
  Konsumenten reserviert. Ollama: Kopie (siehe [ANALYSIS.md](ANALYSIS.md) §1.3)
