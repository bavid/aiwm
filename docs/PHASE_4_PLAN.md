# Phase 4 — Video-Generierung (ComfyUI)

Dritte echte Capability: **Text→Video** und **Bild→Video** über dieselbe
gekapselte ComfyUI-Runtime wie Phase 3. Die Maschinerie ist da — Adapter,
`core::pipeline`, `capability`, das `/prompt → /history → /view`-Muster, die
Galerie, der Scheduler. Phase 4 fügt Video-Templates, den Videofluss durch die
Pipeline und ein Video-UI hinzu.

**Video ist langsam.** Ein 5-Sekunden-Clip in 480p dauert auf der 4080 Super
Minuten, nicht Sekunden. Die UI muss das *vorher* sagen (geschätzte Dauer,
Auflösung, Frame-Zahl) — sonst wirkt die App kaputt.

**Phase-4-DONE-Kriterium:** frisches Windows → App → ComfyUI wird eingerichtet →
Wan-2.2-5B (+ Encoder + VAE) importieren → „Generate Video" mit Prompt → ein
3–5-s-`.mp4` landet im Output-Ordner + Galerie (mit Prompt/Seed/Modell/Länge) →
**Bild→Video**: ein Bild aus der Galerie als Startframe wählen → Clip →
währenddessen die UI-Erwartungssteuerung (Dauer, „das dauert einige Minuten") →
Scheduler koordiniert Video ↔ LLM/Bild ums VRAM-Budget → Netz trennen → das
schon Installierte läuft weiter.

| Scheibe | Inhalt | Status |
|---|---|---|
| **4.0** | **Die echte ComfyUI verproben** (Phase-3-Rest): auf der echten Maschine installieren, cu130-torch-CUDA-Laufzeit prüfen, SDXL- **und** Flux-GGUF-Pipeline aus Phase 3 real durchrendern, `DualCLIPLoaderGGUF`/`UnetLoaderGGUF`-Ordner-Key-Auflösung gegen `extra_model_paths.yaml` bestätigen, `av`/`SaveVideo` verfügbar. Fällt cu130 aus → cu128/cu126-Pin. Ergebnis + etwaige Fixes dokumentieren. **Voraussetzung für alles Weitere.** | offen |
| 4.1 | `capability::video`: `job_type=video`, Params (prompt, negative, w/h, length in Frames, fps, steps, cfg, seed, model\|Auto über Rolle `base_video`); **feste Pipeline** `wan_ti2v` (`core::pipeline`) = `WanImageToVideo` (ohne `start_image` = T2V) → `KSampler` → `VAEDecode` → `CreateVideo` → `SaveVideo` (mp4/h264). `generate_image` → `generate_media` verallgemeinern (VIDEO-Output-Key in `/history`, `/view` für `.mp4`). `ModelKind::VideoModel` + Familie `wan`. Output nach `<outputs>/<job_id>.mp4` → `jobs.output_path`. VRAM-Schätzung pro Familie. Cancel via `/interrupt` | ✅ |
| 4.2 | **Bild→Video**: `init_image`-Param (Pfad oder Job-ID eines fertigen Bildes); der Startframe wird nach `<comfyui-data>/input/<job_id>.<ext>` kopiert und als `LoadImage` → `WanImageToVideo.start_image` verdrahtet. Ein Bild-Job-Output aus der Galerie als Quelle | ✅ |
| 4.3 | UI-Tab „Video" — Prompt/Negativ, Auflösung/Länge/fps/Steps/CFG/Seed, Model [Auto], optional „Start from image" (Galerie-Pick oder Pfad), „Generate"; **Erwartungssteuerung** (geschätzte Dauer + „das dauert Minuten", Fortschritt); `<video>`-Player auf `GET /jobs/{id}/output` (CSP `media-src`); Galerie der Video-Jobs (Poster = erstes Frame). Dashboard-Button „Generate Video" aktiv | ✅ |
| 4.4 | Zweites Template **LTX-2 / LTX 2.3 (GGUF)** über `ComfyUI-GGUF` (Node schon installiert); `docs/VIDEO_MODELS.md` (kuratierte Modelle: SHA256, HF-Quelle, Lizenz, Settings); Katalog-Einträge (`core::model::catalog`) | ✅ *(als **LTX-Video 0.9.5 2B**, Core-Nodes — ADR-020)* |
| 4.5 | Politur: Erwartungssteuerung verfeinern (Zeit-Schätzung kalibrieren), **RAM-Warnung** wenn das Offload-Budget kritisch wird, Retention-Hinweis (Videos sind groß), Settings (Wan/LTX-Optionen, `--reserve-vram`), Diagnostics; Scheduler: Video-Slot neben LLM/Bild-Slot verproben (`core/tests/*_swap.rs`-Erweiterung) | offen |

Nach dem MVP (txt2vid + img2vid): **Frame-Interpolation** (RIFE / Wan-eigene
Interpolation) und **Video-Upscale** — eigene Capabilities mit eigenen Modellen,
als spätere Slices.

---

## Research (2026-09)

### Die Landschaft hat sich seit dem ROADMAP-Entwurf verschoben

| Modell | VRAM (16 GB) | Bewertung |
|---|---|---|
| **Wan 2.2 TI2V-5B** | fp16 ~10 GB Gewichte, ~12–14 GB Peak; GGUF Q8 ~5 GB | **Native ComfyUI-Unterstützung**, **ein** Modell für T2V *und* I2V (TI2V), 720p @ 24 fps, reift schnell. **→ Default & erstes Template.** |
| **LTX-2 / LTX 2.3** (2026) | Full-precision 32 GB+, **GGUF** Q4/Q8 für ≤ 24 GB | schneller, neuer, kann Audio; GGUF über `ComfyUI-GGUF`. Höhere Komplexität → **zweites Template (4.4)**. |
| Wan 2.2 A14B (MoE, 14B) | 24 GB+ für gute 480/720p | zu groß für komfortable 16 GB → **übersprungen** (evtl. „Advanced/langsam" später). |
| HunyuanVideo | 60 GB+ / stark quantisiert | großes Ökosystem, aber schwerer auf 16 GB als Wan 5B → Phase 4.x/6. |

Quellen: [ComfyUI Wan 2.2 Native Workflow](https://docs.comfy.org/tutorials/video/wan/wan2_2) ·
[LTX-2 GGUF Guide 2026](https://dev.to/gary_yan_86eb77d35e0070f5/how-to-install-and-configure-ltx-2-gguf-models-in-comfyui-complete-2026-guide-1d3m) ·
[Local Text-to-Video Low VRAM 2026](https://localaimaster.com/blog/local-text-to-video-low-vram) ·
[Wan 2.2 TI2V-5B on RTX 3090](https://smeltcore.com/recipes/wan-2-2-ti2v-5b-on-rtx-3090-720p-text-image-to-video-in-comfyui/).

### Wan 2.2 TI2V-5B — die Dateien

| Datei | Größe | Ordner (ComfyUI) | Quelle |
|---|---|---|---|
| `wan2.2_ti2v_5B_fp16.safetensors` | 9,3 GB | `diffusion_models/` | [`Comfy-Org/Wan_2.2_ComfyUI_Repackaged`](https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged) |
| `umt5_xxl_fp8_e4m3fn_scaled.safetensors` | 6,3 GB | `text_encoders/` | (dito) — wird beim Sampling auf die CPU ausgelagert |
| `wan2.2_vae.safetensors` | 1,3 GB | `vae/` | (dito) |
| GGUF-Alternative | Q8 ~5 GB … Q4 ~3 GB | `diffusion_models/` | [`QuantStack/Wan2.2-TI2V-5B-GGUF`](https://huggingface.co/QuantStack/Wan2.2-TI2V-5B-GGUF) |

Alle drei Basis-Dateien liegen in einem **ungated** Comfy-Org-Repo. Der
`Comfy-Org`-Repackaged-Weg ist der Standard und passt exakt in unseren getypten
Store (`diffusion_models` / `text_encoders` / `vae`).

### ComfyUI kann Video jetzt nativ

- **Nodes:** `WanImageToVideo` (`{positive, negative, vae, width=832, height=480,
  length=81, batch_size=1, start_image?}` → `(pos, neg, latent)` — dasselbe Node
  für T2V *und* I2V), `CLIPLoader type="wan"` für den umt5-Encoder,
  `ModelSamplingSD3` (Shift ~8 für 5B), `KSampler`, `VAEDecode`, **`CreateVideo`**
  (`images + fps → VIDEO`), **`SaveVideo`** (`VIDEO → mp4/mkv/webm`, h264/av1).
- **Kein VideoHelperSuite nötig.** `SaveVideo` ist ein Core-Node; `av>=17.0.0`
  (PyAV bringt ffmpeg mit) steht **schon in ComfyUIs `requirements.txt` (v0.34.0)**
  — der 3.2-Installer zieht es also bereits.
- `SaveVideo` schreibt in `--output-directory` (unser `<outputs>/`) und meldet
  das Ergebnis im `/history` — der genaue Output-Key (`videos` / `images` /
  `gifs`) muss gegen die echte ComfyUI verifiziert werden (4.0). Das
  `/view`-Abholen bleibt gleich (`type=output`, jetzt `.mp4`).

### Zeit- und VRAM-Realität (4080 Super, 16 GB)

- Wan 5B, 480p, ~81 Frames (~3,4 s), ~25 Steps: **grob 2–5 Minuten** pro Clip.
  720p / mehr Frames: deutlich länger, näher an der VRAM-Kante.
- Peak-VRAM 5B fp16: Gewichte 9,3 GB + VAE-Decode + Latents ≈ 12–14 GB. Der
  umt5-Encoder (6,3 GB) ist transient (CPU-Offload). ComfyUIs „smart memory"
  managt den Tausch — auf 16 GB komfortabel für 480p, knapp für 720p.
- **RAM zählt:** der Offload schiebt den Encoder + Teile des Modells in den
  System-RAM. 32 GB RAM ist das Minimum; bei langen Clips / 720p kann es eng
  werden → **RAM-Warnung** (4.5).
- **`IMAGE_GENERATE_TIMEOUT` (600 s) ist für Video zu knapp** — der Video-Pfad
  braucht ein eigenes, großzügigeres Limit (z. B. 30 min) oder ein
  fortschrittsbasiertes.

### Wiederverwendung aus Phase 3

- `ComfyUiAdapter` + `generate_image`-Poll-Schleife → zu `generate_media`
  verallgemeinern (nur der Output-Key + die Extension unterscheiden sich).
- `core::pipeline::Recipe` → um `WanTi2v` (und später `Ltx`) erweitern.
- `capability::image::resolve_flux_companions` → das Muster für Wans Encoder +
  VAE (`resolve_video_companions`).
- `import_model` + `ModelKind` + `catalog` + Galerie + `GET /jobs/{id}/output`:
  alles trägt, nur Video-Kinds/Familien + `media-src` in der CSP kommen dazu.

---

## Offene Entscheidungen (Empfehlung → deine Freigabe)

**A — Erstes Video-Modell.**
→ **Empfehlung: Wan 2.2 TI2V-5B, `Auto`-Default.** Native ComfyUI-Nodes, ein
Modell für Text→Video *und* Bild→Video, passt in 16 GB, ungated. LTX-2 als
zweites Template (4.4), wenn Wan steht — exakt das Muster SDXL→Flux aus Phase 3.

**B — Modell-Format.**
→ **Empfehlung: `.safetensors` fp16 als Default** (`Comfy-Org`-Repackaged,
`Load Diffusion Model`), **GGUF optional** über den schon installierten
`ComfyUI-GGUF`-Node für knappes VRAM / lange Clips. Der Katalog listet beide.
(Wan 5B fp16 passt — anders als Flux 12B, wo GGUF nötig war.)

**C — Output-Container.**
→ **Empfehlung: MP4 / H.264.** Universell im WebView abspielbar (`<video>`),
`SaveVideo` nativ, `av` schon gebündelt. WebM/AV1 als spätere Option. Kein
GIF (schlechte Qualität/Größe).

**D — Bild→Video-Eingabe.**
→ **Empfehlung: der Startframe kommt aus einem fertigen Bild-Job (Galerie-Pick)
oder einem Dateipfad.** `capability::video` kopiert ihn nach
`<comfyui-data>/input/<job_id>.<ext>` (ComfyUIs `LoadImage` liest aus `input/`)
und verdrahtet `LoadImage → WanImageToVideo.start_image`. Kein
`--input-directory`-Umbau. Aufräumen der `input/`-Kopien beim Job-Ende.

**E — Längen-/Auflösungs-Grenzen in der UI.**
→ **Empfehlung: harte Obergrenzen** — 5 s (≈ 121 Frames @ 24 fps), max. 720p —
mit klarer „länger/größer = viel langsamer, kann OOM"-Warnung. Default
480p / ~81 Frames / 24 fps. Der Backend klemmt zusätzlich.

**F — 4.0 ist Voraussetzung, nicht optional.**
→ Phase 3 lief komplett gegen `aiwm-fake-comfy`. Video kann man **nicht** gegen
eine Fake-ComfyUI sinnvoll bauen — die Node-Namen, der `/history`-Output-Key
und die Zeit/VRAM-Realität müssen an der echten Runtime stimmen. **4.0 zuerst**,
notfalls an deiner echten Maschine (Installation + ein SDXL-Render + ein
Flux-Render + `SaveVideo`-Verfügbarkeit). Fixes aus 4.0 fließen in Phase 3 zurück.

---

## 4.1 — Ergebnis (abgeschlossen)

Dritte Capability steht — **Text→Video** durch dieselbe gekapselte ComfyUI wie
Bild. Gebaut gegen `aiwm-fake-comfy` + die recherchierten Node-Namen (Variante 1);
die Verprobung gegen die echte ComfyUI bleibt 4.0.

- **`capability::video`** (neu): `VideoRequest::from_params` — nur `prompt` ist
  Pflicht; `width`/`height` runden auf 16 und klemmen `[128, 1280]`, `length`
  snappt auf Wans `4k + 1`-Raster (`round_video_length`, `[5, 121]`), `fps`
  `[8, 30]`, `steps` `[1, 60]`, `cfg` `[1.0, 15.0]`, `seed` explizit `≥ 0` sonst
  zufällig. `apply_to` schreibt die aufgelösten Werte über `job.params` zurück
  (Seed wird reproduzierbar, Galerie hat konkrete Zahlen). `run()` baut den
  Wan-Workflow, ruft `generate_media`, schreibt `<outputs>/<job_id>.mp4`.
  `VideoOutcome { Done(VideoDone) | Cancelled }`.
- **`capability::media`** (neu): gemeinsame Helfer, damit `image` und `video`
  sich nicht doppeln — `comfy_err`, `file_name`, `str_param`, `round_to`,
  `random_seed`/`resolve_seed` (`SEED_CEILING = 1 << 53`), `write_output`
  (async, `create_dir_all` + `write`). `capability::image` darauf umgestellt,
  sein lokaler Seed-/Datei-Kram entfällt.
- **`core::pipeline::wan_ti2v`** (neu): fester Graph `UNETLoader` +
  `CLIPLoader type="wan"` + `VAELoader` → `CLIPTextEncode`×2 →
  `ModelSamplingSD3` (Shift 8) → `WanImageToVideo` (ohne `start_image` = T2V) →
  `KSampler` (`uni_pc` / `simple`, denoise 1) → `VAEDecode` → `CreateVideo`
  (fps) → `SaveVideo` (mp4). `start_image` (4.2) hängt optional ein `LoadImage`
  ein. `VideoInputs` / `WanModels` als Eingabe-Structs.
- **`ComfyUiAdapter`**: `generate_image` → **`generate_media(workflow, cancel,
  timeout)`** verallgemeinert — `GeneratedImage` → `GeneratedMedia { bytes,
  extension }`, Timeout als Parameter (Bild 600 s, Video 1800 s), Extension aus
  dem Dateinamen (`rsplit_once('.')`, Default `png`). `ComfyClient`: `ImageRef`
  → `MediaRef`, `collect_media` prüft `outputs.<node>.{images,videos,gifs}`,
  `PromptOutcome::Done(Vec<MediaRef>)`.
- **`ModelKind::VideoModel`** (neu): Rolle `base_video`, Store-Unterordner
  `video/diffusion_models/`, `comfy_folder()` → `diffusion_models`.
  `import`: `media_family` erkennt `wan` / `ltx` (Familie + `HEAVY`-Headroom),
  `model_type: "video"` routet in den Video-Store. `extra_model_paths.yaml`
  bekommt einen zweiten Block `aiwm_video:` (`base_path: <store>/video`).
- **`JobEngine`**: `resolve_comfyui_target` (war `resolve_image_target`) bedient
  `image` **und** `video` — für Video `("base_video", "no video model in the
  library — import Wan 2.2 5B …")`, VRAM-Fallback `VIDEO_VRAM_FALLBACK_MB =
  11 264` für Familie `wan`/`ltx`. `try_drive`-Zweig `job_type == "video"`:
  Params auflösen → `set_params` (Seed pinnen) → `video::run` → bei `Done`
  `output_path` + Event „video ready — …", bei `Cancelled` → `Cancelled`-State.
- **`GET /jobs/{id}/output`**: Content-Type-Map um `mp4` → `video/mp4`,
  `webm` → `video/webm` erweitert.
- **`aiwm-fake-comfy`**: scannt den Prompt-Graphen nach `SaveVideo` / `SaveImage`,
  meldet `videos` (mp4) bzw. `images` (png) im `/history`, `/view` liefert
  `TINY_MP4` (32-Byte ftyp-Stub) für `.mp4`. Neue Flags unverändert.

Verifiziert:
- **+14 Unit-Tests** (`video` 7, `media` 3, `pipeline` 2 — Wan-Graph T2V + I2V,
  `kind`/`import` je 1) → **239 Lib-Tests** grün. **+3 Integrationstests**
  `tests/video_job.rs` (`#[cfg(windows)]`): Auto-Video-Job rendert eine `.mp4`
  und schreibt die Datei; Video-Job ohne Encoder → `Failed` mit „umt5" +
  „Models tab"; Auto-Video ohne Video-Modell → „no video model" → **29
  Integrationstests**. `check.ps1` grün (Clippy `-D warnings`, `tsc`, `eslint`,
  `ruff`, `pytest`).
- **Live** (`aiwm-cored` + Fake-ComfyUI, `smoke_41.sh`): Wan-Stack importiert
  (`family=wan`, `roles=[base_video]` nach `models/video/diffusion_models/`;
  umt5 → `text_encoder`, `wan2.2_vae` → `vae`). Video-Job (`length: 40`) →
  `auto-selected` → Events „rendering 512×288 video, 41 frames @ 24 fps
  (~1.7s) …" + „Wan — encoder … VAE … this takes several minutes" + „video
  ready" → `completed`; `params.length == 41` (auf `4k+1` gesnappt), `seed`
  gepinnt. `GET /jobs/<id>/output` → `content-type: video/mp4`, ftyp-Bytes.
  `aiwm-model-paths.yaml` trägt den `aiwm_video:`-Block. Video-Job ohne Encoder
  → `failed`: „Wan needs the umt5 text encoder — import umt5_xxl_… as „Text
  encoder / CLIP" on the Models tab".

Bewusst **nicht** in 4.1: Bild→Video / `start_image` (4.2), Video-UI-Tab (4.3),
LTX-Template + Katalog-Einträge (4.4), Streaming großer Clips (`generate_media`
puffert noch komplett im RAM — TODO), die echte ComfyUI (4.0 — `SaveVideo`,
`av`, der reale `/history`-Output-Key, Zeit/VRAM-Kalibrierung). Wans VAE + umt5
liegen weiterhin unter `<store>/image/{vae,text_encoders}/` (ComfyUI findet sie
per Dateiname über den gemergten Ordner-Key) — kosmetisch, TODO.

---

## 4.2 — Ergebnis (abgeschlossen)

Bild→Video über denselben `wan_ti2v`-Graphen — `WanImageToVideo` nimmt einen
Startframe entgegen (das Node macht T2V *und* I2V). Weiter gegen `aiwm-fake-comfy`.

- **`VideoRequest.init_image: Option<String>`** — aus `params["init_image"]`
  (getrimmt, leer → `None`). `from_params` extrahiert nur den Rohwert;
  `apply_to` schreibt ihn unverändert zurück (Galerie in 4.3 zeigt die Quelle).
- **`capability::video`**: neu `resolve_start_frame(db, spec)` — `spec` ist
  entweder die **Job-ID** eines fertigen Bild-Jobs (`db.jobs().get` → dessen
  `output_path`) oder ein **Dateipfad**. Muss eine existierende
  `.png`/`.jpg`/`.jpeg`/`.webp`-Datei sein, sonst Klartext-Fehler
  („start frame not found" / „must be a … image"). `stage_start_frame` kopiert
  sie nach `<comfyui input>/<job_id>.<ext>`; der bare Dateiname geht als
  `start_image` in `wan_ti2v` (→ `LoadImage` → `WanImageToVideo.start_image`).
- **`StagedFrame`-Guard**: ein `Drop`-Typ entfernt die `input/`-Kopie, sobald
  `run` zurückkehrt (Erfolg, Fehler *oder* Cancel). Die Kopie wird **vor** dem
  „takes several minutes"-Event angelegt, damit ein schlechter `init_image`
  sofort fehlschlägt. Event „image→video — start frame from <spec>".
- **`ComfyUiAdapter::input_dir()`** (+ `ComfyDirs::input()`) — `<base>/input/`,
  öffentlich, damit `capability::video` dort ablegen kann.
- **`aiwm-fake-comfy`**: parst jetzt `--base-directory`, scannt den Graphen nach
  `LoadImage` und lässt `/history` **fehlschlagen**, wenn die referenzierte
  Datei nicht unter `<base>/input/` liegt — der Integrationstest beweist damit,
  dass das Staging wirklich am richtigen Ort landet.
- Engine unverändert: der `try_drive`-`video`-Zweig (`from_params` → `apply_to`
  → `set_params` → `video::run`) trägt `init_image` schon durch.

Verifiziert:
- **+3 Unit-Tests** (`video` — `init_image`-Parse/Roundtrip, `checked_frame`
  Ablehnungen, `resolve_start_frame` Pfad + Job-ohne-Output) → **242 Lib-Tests**.
  **+3 Integrationstests** `tests/video_job.rs`: Startframe aus einem **Pfad**
  (staged, gerendert, Kopie danach entfernt, Event „start frame"); Startframe
  aus einem **fertigen Bild-Job** (Bild-Job zuerst, dann seine ID); fehlender
  Startframe → `Failed` „start frame not found", **vor** dem Render → **32
  Integrationstests**. `check.ps1` grün.
- **Live** (`smoke_42.sh`): (A) `init_image` = Pfad → mp4, `params.init_image`
  gepinnt, `<comfyui-data>/input/<job>.png` nach dem Render weg. (B) `init_image`
  = Bild-Job-ID → mp4, Event nennt die Job-ID. (C) `init_image` = `Z:/nope/…`
  → `failed` „start frame not found", kein „takes several minutes".

Bewusst **nicht** in 4.2: Video-UI mit Galerie-Pick (4.3), Startframe-Skalierung
auf die Zielauflösung (ComfyUIs `WanImageToVideo` klemmt selbst), `clip_vision`
für stärkere Bild-Treue (5B TI2V braucht es nicht; ggf. später als Option),
Aufräumen verwaister `input/`-Kopien nach einem Absturz (der Guard deckt den
Normalfall; ein Start-Sweep wäre robuster — TODO), Pfad-Allowlist für
`init_image` (loopback-only MVP, ADR-008 — TODO).

---

## 4.3 — Ergebnis (abgeschlossen)

UI-Tab **„Video"** — spiegelt den Image-Tab (gleiches Layout, Formular, Galerie),
plus Erwartungssteuerung und `<video>`-Player.

- **`features/video/Video.tsx`** (`VideoStudio`): Sidebar-Formular (Prompt,
  Negativ, Größe-Presets Landscape/Portrait/Square/HD-720p, W/H/Frames/FPS/
  Steps/CFG, Seed, Model [Auto über `base_video`]), Result-Panel, Galerie.
  Frontend-Klemmung spiegelt `capability::video` (Dim auf 16 / 128–1280,
  Frames auf Wans `4k+1` / 5–121, FPS 8–30, Steps 1–60, CFG 1–15).
- **Erwartungssteuerung**: eine Notiz unter dem Formular nennt die Cliplänge
  (`frames / fps`) und eine **grobe** Minuten-Spanne (skaliert mit Frames ×
  Steps × Pixeln); ab > 480p / > 81 Frames wird sie zur Warnung
  („viel langsamer, kann OOM"). „Video ist langsam — Minuten, nicht Sekunden."
- **Fortschritt**: das Result-Panel pollt `job_detail` (900 ms) und zeigt die
  letzte `rendering …` / `image→video …` / `takes several minutes`-Event-Zeile
  + „This can take several minutes." (kein Prozentbalken — der Core liefert noch
  keinen; TODO 4.5).
- **`<video>`-Player**: `GET /jobs/{id}/output` (`controls preload="metadata"`).
  CSP in `tauri.conf.json` um `media-src 'self' http://127.0.0.1:* http://localhost:*`
  erweitert. Galerie-Kacheln = `<video muted preload="metadata">` (WebView2
  zeigt das erste Frame) + „▶"-Overlay.
- **Bild→Video im UI**: ein `<fieldset>` „Start from an image" — Dropdown der
  fertigen Bild-Jobs (Galerie-Pick, sendet die Job-ID) **oder** ein
  Pfad-Feld (gewinnt, wenn gefüllt). Bei Auswahl eine Mini-Vorschau des
  Startframes. `params.init_image` geht mit.
- **`ipc.ts`**: `VideoParams`-Interface, `jobOutputUrl` (generisch; `imageOutputUrl`
  bleibt als Alias), `ModelType` um `"video"` erweitert. **`Models.tsx`**:
  Import-Typ „Video model" (`.safetensors` / `.gguf`). **`App.tsx`**: „Video"-Tab
  zwischen Image und Models. **`Dashboard.tsx`**: „Generate Video" → Tab,
  „ready".
- **`components/NumField.tsx`** (neu): das Zahlen-Eingabefeld aus `Image.tsx`
  herausgezogen (jetzt von Image + Video geteilt).
- **`lib/dev-mock.ts`** (neu, dev-only): ein gecannter Tauri-IPC-Bridge
  (`@tauri-apps/api/mocks`), den `main.tsx` per dynamischem Import nur unter
  `import.meta.env.DEV` **und** ohne Tauri-Shell lädt (im Prod-Build toter
  Code — Bundle unverändert 52 Module). Damit lassen sich die Studios ohne
  laufenden Core im Browser ansehen.

Verifiziert:
- `pnpm typecheck` + `pnpm lint` sauber, `pnpm build` grün (kein Größen-Zuwachs).
  Rust unverändert (**242 Lib + 32 Integ**).
- **Visuell** (Vite-Dev + `dev-mock`, Browser): Video-Tab rendert; „Generate"
  → Result zeigt `running` + die `rendering 832×480 video …`-Zeile + „This can
  take several minutes." + Meta (Prompt/Model/Clip); nach `completed` ein
  `<video controls>` auf `127.0.0.1:<port>/jobs/<id>/output`; Galerie-Kachel =
  `<video muted>`. HD-720p-Preset schaltet die Warn-Notiz + skaliert die
  Schätzung (2–8 → 6–19 min). „Start from an image" zeigt die Thumbnail-
  Vorschau und sendet die Job-ID. Models-Tab bietet „Video model" als
  Import-Typ.

Bewusst **nicht** in 4.3: echter Prozentbalken (Core hat noch keinen `/ws`-
Fortschritt — 4.5), echtes Poster-Frame (erstes Frame per ffmpeg extrahieren —
`<video preload=metadata>` reicht im MVP), Range-Requests für `<video>`-Seeking
bei langen Clips (TODO, hängt am `GET /jobs/{id}/output`-Streaming), Lightbox /
Download / Retention-Aufräumen der Galerie (eigene kleine Slices bei Bedarf),
Registry der Größe-Presets.

---

## 4.4 — Ergebnis (abgeschlossen) · **Plan-Abweichung, siehe ADR-020**

Zweites Video-Template. Der Plan nannte LTX-2 GGUF über den installierten
`ComfyUI-GGUF`-Node — die Recherche zeigt, dass LTX-2/2.3 dafür gepatchte Loader
+ KJNodes braucht und selbst als GGUF kaum in 16 GB passt. Gewählt:
**LTX-Video 0.9.5 (2B)** als einzelnes `.safetensors`, **nur Core-ComfyUI-Nodes**
(ADR-020). Weiter gegen `aiwm-fake-comfy`.

- **`core::pipeline`**: neu `ltx_video(i, m)` — `CheckpointLoaderSimple`
  (Model + VAE) + `CLIPLoader type="ltxv"` → `CLIPTextEncode`×2 →
  `LTXVConditioning` (frame_rate) → `EmptyLTXVLatentVideo` → `LTXVScheduler`
  (max_shift 2.05 / base_shift 0.95 / terminal 0.1) → `KSamplerSelect euler` →
  **`SamplerCustom`** (Sigmas vom Scheduler, kein `KSampler`) → `VAEDecode` →
  `CreateVideo` → `SaveVideo`. Mit Startframe: `EmptyLTXVLatentVideo` wird durch
  `LoadImage` + `LTXVImgToVideo` ersetzt (produziert Conditioning + Start-Latent,
  wie `WanImageToVideo`). Graph aus dem offiziellen
  `Comfy-Org/workflow_templates`-JSON verifiziert.
- **`VideoRecipe { Wan, Ltx }`** + `for_family` (`"ltx"` → Ltx, sonst Wan) —
  spiegelt `Recipe::for_family` (Bild). `LtxModels { checkpoint, t5 }`.
- **`capability::video::run`** umgebaut: baut `(workflow, companion_note)` je
  Rezept. Wan → `resolve_wan_companions` (umt5 + Wan-VAE). LTX →
  `resolve_ltx_encoder` (ein `t5…`, **nicht** `umt5`; VAE steckt im Checkpoint).
  Fehlt der T5 → „LTX-Video needs a T5 text encoder — import t5xxl_… on the
  Models tab". `name_is_t5` = enthält „t5", nicht „umt5".
- **`core::model::catalog`**: +4 Einträge (die ersten Video-Einträge) —
  `wan22-ti2v-5b`, `wan-umt5-xxl-fp8`, `wan22-vae`, `ltx-video-2b-095`. Echte
  HF-SHA-256 (gegen die HF-LFS-Pointer geprüft). `KNOWN_MODELS` jetzt 11
  Einträge; `GET /models/known` liefert sie mit; „Known models"-Panel zeigt sie
  (LTX-T5 = der schon gelistete `t5xxl-fp8`).
- **`docs/VIDEO_MODELS.md`** (neu): Wan-Stack + LTX, SHA-256, HF-Quelle, Lizenz,
  Settings, Companion-Auflösung, „bewusst nicht". Katalog-Doc-Kommentar +
  `docs/MODELS.md` verweisen darauf.
- **`wan_ti2v`**: `SaveVideo` bekommt `codec: "auto"` (matcht die echte
  Node-Signatur; fake-comfy ignoriert es) — Konsistenz mit `ltx_video`.

Verifiziert:
- **+4 Unit-Tests** (`pipeline` — `VideoRecipe`-Wahl, LTX-Graph T2V + I2V;
  `capability::video` — `resolve_ltx_encoder` T5-nicht-umt5) → **246 Lib-Tests**.
  **+2 Integrationstests** `tests/video_job.rs`: `family "ltx"` → LTX-Template,
  nur T5 nötig, rendert `.mp4`, Event „LTX-Video — T5 encoder …"; LTX ohne T5
  (nur umt5 da) → `Failed` „T5 text encoder" → **34 Integrationstests**.
  `check.ps1` grün.
- **Live** (`smoke_44.sh`): `GET /models/known` trägt die 4 Video-Einträge
  (64-Hex-SHAs, `ltx-video-2b-095` family `ltx`). LTX-Checkpoint importiert nach
  `models/video/diffusion_models/`. Video-Job (`length: 49`) → `auto-selected
  ltx-video-2b-v0.9.5` → Event „LTX-Video — T5 encoder „t5xxl_fp8…", VAE from
  the checkpoint" → `completed`, `.mp4` über `/jobs/<id>/output` (`video/mp4`).
  LTX-Job mit nur einem umt5 → `failed`: „LTX-Video needs a T5 text encoder …".

Bewusst **nicht** in 4.4: LTX-2 / 2.3 (19B/22B — GGUF-Node-Patch + KJNodes,
ADR-020), LTX-13B (zu groß), LTX-2-Audio, der `(frames-1) % 8`-Grid für LTX
(die App klemmt auf `4k+1`, LTX rundet intern ab — für 4.0), UI-Änderungen (der
Video-Tab von 4.3 dispatcht schon über `base_video`, egal welche Familie),
Frame-Interpolation / Upscale (spätere Slices), echte ComfyUI (4.0).

---

## Bewusst nicht in Phase 4

- **Frame-Interpolation / Video-Upscale** — eigene Capabilities (RIFE,
  Upscale-Modelle), eigene Slices *nach* dem txt2vid/img2vid-MVP.
- **Wan 14B / HunyuanVideo** — zu schwer für komfortable 16 GB (evtl.
  „Advanced/langsam" später).
- **Audio** (LTX-2 kann es) — erst wenn das Video-Grundgerüst steht.
- **Video-zu-Video / ControlNet-Video / VACE / Kamerasteuerung** — Post-MVP.
- **Batch / mehrere Clips pro Job**, Prompt-Travel, Loop-Erkennung.
- **Online-Modell-Discovery / Auto-Download** (Phase 6).
