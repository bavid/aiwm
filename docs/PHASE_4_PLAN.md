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
| 4.1 | `capability::video`: `job_type=video`, Params (prompt, negative, w/h, length in Frames, fps, steps, cfg, seed, model\|Auto über Rolle `base_video`); **feste Pipeline** `wan_ti2v` (`core::pipeline`) = `WanImageToVideo` (ohne `start_image` = T2V) → `KSampler` → `VAEDecode` → `CreateVideo` → `SaveVideo` (mp4/h264). `generate_image` → `generate_media` verallgemeinern (VIDEO-Output-Key in `/history`, `/view` für `.mp4`). `ModelKind::VideoModel` + Familie `wan`. Output nach `<outputs>/<job_id>.mp4` → `jobs.output_path`. VRAM-Schätzung pro Familie. Cancel via `/interrupt` | offen |
| 4.2 | **Bild→Video**: `init_image`-Param (Pfad oder Job-ID eines fertigen Bildes); der Startframe wird nach `<comfyui-data>/input/<job_id>.<ext>` kopiert und als `LoadImage` → `WanImageToVideo.start_image` verdrahtet. Ein Bild-Job-Output aus der Galerie als Quelle | offen |
| 4.3 | UI-Tab „Video" — Prompt/Negativ, Auflösung/Länge/fps/Steps/CFG/Seed, Model [Auto], optional „Start from image" (Galerie-Pick oder Pfad), „Generate"; **Erwartungssteuerung** (geschätzte Dauer + „das dauert Minuten", Fortschritt); `<video>`-Player auf `GET /jobs/{id}/output` (CSP `media-src`); Galerie der Video-Jobs (Poster = erstes Frame). Dashboard-Button „Generate Video" aktiv | offen |
| 4.4 | Zweites Template **LTX-2 / LTX 2.3 (GGUF)** über `ComfyUI-GGUF` (Node schon installiert); `docs/VIDEO_MODELS.md` (kuratierte Modelle: SHA256, HF-Quelle, Lizenz, Settings); Katalog-Einträge (`core::model::catalog`) | offen |
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

## Bewusst nicht in Phase 4

- **Frame-Interpolation / Video-Upscale** — eigene Capabilities (RIFE,
  Upscale-Modelle), eigene Slices *nach* dem txt2vid/img2vid-MVP.
- **Wan 14B / HunyuanVideo** — zu schwer für komfortable 16 GB (evtl.
  „Advanced/langsam" später).
- **Audio** (LTX-2 kann es) — erst wenn das Video-Grundgerüst steht.
- **Video-zu-Video / ControlNet-Video / VACE / Kamerasteuerung** — Post-MVP.
- **Batch / mehrere Clips pro Job**, Prompt-Travel, Loop-Erkennung.
- **Online-Modell-Discovery / Auto-Download** (Phase 6).
