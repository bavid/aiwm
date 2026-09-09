# Video models — kuratierte Liste

Die Modelle, die das Tool für `job_type=video` kennt. Wie bei den Bild-Modellen
hat der MVP **keinen Download-Manager** (Phase 6) — man lädt die Dateien selbst
von Hugging Face und importiert sie über den **Models**-Tab mit dem passenden
Typ. Der Importer erkennt eine Datei an ihrer **SHA-256** und stempelt dann
Publisher/Familie automatisch (`models.source_revision = catalog:<id>`).

Die Liste lebt als Code in [`core::model::catalog`](../core/src/model/catalog.rs)
(`GET /models/known` / Tauri-Command `list_known_models`); der **Known
models**-Abschnitt im Models-Tab zeigt sie mit „Copy link".

Hardware-Kontext: RTX 4080 Super, 16 GB VRAM, 32 GB RAM. **Video ist langsam** —
ein 480p-Clip dauert Minuten, nicht Sekunden. Siehe [HARDWARE.md](HARDWARE.md).

> **Noch nicht an der echten ComfyUI verprobt.** Alle Node-Namen, Settings und
> VRAM-Zahlen sind recherchiert (Slice 4.0 steht aus). Fixes fließen hierher
> zurück.

---

## Wan 2.2 TI2V-5B — Default

**Ein** Modell für Text→Video *und* Bild→Video, native ComfyUI-Nodes, Apache-2.0
(kommerziell nutzbar). Pipeline: `wan_ti2v` — `UNETLoader` + `CLIPLoader
type="wan"` + `VAELoader` → `ModelSamplingSD3` (Shift 8) → `WanImageToVideo` →
`KSampler` (`uni_pc`/`simple`) → `VAEDecode` → `CreateVideo` → `SaveVideo`.

Braucht **drei** Dateien aus
[`Comfy-Org/Wan_2.2_ComfyUI_Repackaged`](https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged)
(`split_files/…`):

| Datei | Größe | Import-Typ | SHA-256 |
|---|---|---|---|
| `wan2.2_ti2v_5B_fp16.safetensors` | 10,0 GB | **Video model** | `456f901338bd9eadbded3828b819109a9b68e8a525ca5cf8d0049a69fcfeca1e` |
| `umt5_xxl_fp8_e4m3fn_scaled.safetensors` | 6,74 GB | **Text encoder / CLIP** | `c3355d30191f1f066b26d93fba017ae9809dce6c627dda5f6a66eaa651204f68` |
| `wan2.2_vae.safetensors` | 1,41 GB | **VAE** | `e40321bd36b9709991dae2530eb4ac303dd168276980d3e9bc4b6e2b75fed156` |

Lizenz: Apache-2.0. Der umt5-Encoder wird beim Sampling auf die CPU ausgelagert.

**Empfohlene Settings:** 832×480 (oder 480×832), **81 Frames @ 24 fps** (~3,4 s),
25–30 Steps, CFG ≈ 5. Frame-Zahl muss `(n-1) % 4 == 0` sein (die App klemmt auf
`4k+1`). 720p geht, ist aber deutlich langsamer und näher an der VRAM-Kante.

**Peak-VRAM (recherchiert):** Gewichte 10 GB + VAE-Decode + Latents ≈ 12–14 GB;
der Encoder (6,7 GB) ist transient. Auf 16 GB komfortabel für 480p.

---

## LTX-Video 0.9.5 (2B) — zweites Template (4.4, ADR-020)

Schneller und leichter als Wan, native Core-Nodes, **kein Custom Node**. Ein
einzelnes `.safetensors` trägt **Model + VAE** — es braucht nur einen
T5-Encoder. Pipeline: `ltx_video` — `CheckpointLoaderSimple` + `CLIPLoader
type="ltxv"` → `LTXVConditioning` → `EmptyLTXVLatentVideo` (bzw.
`LTXVImgToVideo` mit Startframe) → `LTXVScheduler` → `SamplerCustom`
(`KSamplerSelect euler`) → `VAEDecode` → `CreateVideo` → `SaveVideo`.

| Datei | Größe | Import-Typ | SHA-256 |
|---|---|---|---|
| `ltx-video-2b-v0.9.5.safetensors` | 6,34 GB | **Video model** | `720d15c9f19f7d0f6b2a92bbbc34410e2cfb2f6856a100b38f734fbf973d4adf` |

Quelle: [`Lightricks/LTX-Video`](https://huggingface.co/Lightricks/LTX-Video) ·
Lizenz: **LTXV License** (OpenRAIL-M-artig — kommerzielle Bedingungen im Repo
prüfen).

**T5-Encoder:** LTX nutzt einen normalen `t5xxl` (kein umt5). Der bereits für
Flux kuratierte **`t5xxl_fp8_e4m3fn.safetensors`** (4,89 GB, Apache-2.0,
[`comfyanonymous/flux_text_encoders`](https://huggingface.co/comfyanonymous/flux_text_encoders))
funktioniert mit `CLIPLoader type="ltxv"` — als **Text encoder / CLIP**
importieren.

**Empfohlene Settings:** 768×512, **49–97 Frames @ 24 fps**, 25–30 Steps,
CFG ≈ 3–5, Sampler `euler`. LTX braucht **lange, beschreibende Prompts** — kurze
Prompts kosten spürbar Qualität. Frame-Zahl bei LTX eigentlich `(n-1) % 8 == 0`;
die App klemmt auf `4k+1`, LTX rundet intern ab (weniger Frames als angefragt) —
für 4.0 zu kalibrieren.

**Companion-Auflösung:** `capability::video` wählt das Template über
`model.family` (`wan` / `ltx`) und löst die Begleiter per Rolle + Name auf:
Wan → `umt5…` + `wan…`-VAE; LTX → ein `t5…` (nicht `umt5`). Fehlt etwas, schlägt
der Job mit Klartext fehl.

---

## Bewusst (noch) nicht

- **LTX-2 / LTX-2.3 (19B/22B)** — GGUF braucht gepatchte Loader + KJNodes
  (ADR-002-Konflikt); fp8 passt nicht in 16 GB. Post-MVP, wenn der GGUF-Node-Weg
  stabil ist.
- **Wan 2.2 A14B (14B MoE)** — 24 GB+ für gute Auflösungen.
- **HunyuanVideo** — 60 GB+ / stark quantisiert.
- **Audio** (LTX-2 kann es), **Frame-Interpolation**, **Video-Upscale** — eigene
  spätere Slices.
