# Image models — kuratierte Liste

Die Modelle, die das Tool für `job_type=image` kennt. Der MVP hat **keinen
Download-Manager** (Phase 6) — man lädt die Dateien selbst von Hugging Face und
importiert sie über den **Models**-Tab mit dem passenden Typ. Der Importer
erkennt eine Datei an ihrer **SHA-256** und stempelt dann Publisher/Familie
automatisch (`models.source_revision = catalog:<id>`).

Die Liste lebt als Code in [`core::model::catalog`](../core/src/model/catalog.rs)
(`GET /models/known` / Tauri-Command `list_known_models`); der **Known
models**-Abschnitt im Models-Tab zeigt sie mit „Copy link".

Hardware-Kontext: RTX 4080 Super, 16 GB VRAM. Siehe [HARDWARE.md](HARDWARE.md).

---

## SDXL — Default

| | |
|---|---|
| **Datei** | `sd_xl_base_1.0.safetensors` · 6,94 GB |
| **Typ beim Import** | Image checkpoint |
| **Quelle** | [`stabilityai/stable-diffusion-xl-base-1.0`](https://huggingface.co/stabilityai/stable-diffusion-xl-base-1.0) |
| **SHA-256** | `31e35c80fc4829d14f90153f4c74cd59c90b779f6afe05a74cd6120b893f7e5b` |
| **Lizenz** | CreativeML Open RAIL++-M (kommerziell erlaubt) |
| **VRAM** | ~8–10 GB beim Rendern |

Ein einzelnes `.safetensors` enthält Model + CLIP + VAE. Pipeline:
`checkpoint_txt2img` (ComfyUI-Default-Graph, keine Custom Nodes).

**Empfohlene Settings:** 1024×1024 (oder 832×1216 / 1216×832), 25–30 Steps,
CFG 6–8, Sampler `euler`, Scheduler `normal`. Negativ-Prompt wirkt.

---

## FLUX.1-dev — zweites Template (3.6)

Beste Prompt-Treue und In-Bild-Text, **nicht-kommerzielle** Lizenz (für dieses
Projekt ok, ADR-011). Läuft als **GGUF-Diffusionsmodell** über den gepinnten
`ComfyUI-GGUF`-Node (`UnetLoaderGGUF` + `DualCLIPLoaderGGUF` + `VAELoader`).
Braucht **vier** Dateien:

### 1. Diffusionsmodell (eine Quant-Stufe wählen)

| Datei | Größe | SHA-256 | Notiz |
|---|---|---|---|
| `flux1-dev-Q8_0.gguf` | 12,71 GB | `129032f32224bf7138f16e18673d8008ba5f84c1ec74063bf4511a8bb4cf553d` | beste Qualität, passt knapp in 16 GB |
| `flux1-dev-Q4_K_S.gguf` | 6,81 GB | `75bb19459b5240c9f373b5af527584af15b675867fa142efadf7478d6abbf62b` | kleiner, etwas Qualitätsverlust |

Quelle: [`city96/FLUX.1-dev-gguf`](https://huggingface.co/city96/FLUX.1-dev-gguf) ·
**Import-Typ: Diffusion model / UNet** · Lizenz: FLUX.1 [dev] Non-Commercial.

### 2. T5-XXL Text-Encoder (eine wählen)

| Datei | Größe | SHA-256 | Quelle |
|---|---|---|---|
| `t5xxl_fp8_e4m3fn.safetensors` | 4,89 GB | `7d330da4816157540d6bb7838bf63a0f02f573fc48ca4d8de34bb0cbfd514f09` | [`comfyanonymous/flux_text_encoders`](https://huggingface.co/comfyanonymous/flux_text_encoders) |
| `t5-v1_1-xxl-encoder-Q8_0.gguf` | 5,06 GB | `9ec60f6028534b7fe5af439fcb535d75a68592a9ca3fcdeb175ef89e3ee99825` | [`city96/t5-v1_1-xxl-encoder-gguf`](https://huggingface.co/city96/t5-v1_1-xxl-encoder-gguf) |

**Import-Typ: Text encoder / CLIP** · Lizenz: Apache-2.0. ComfyUI lädt den T5 auf
die CPU aus, sobald der Prompt kodiert ist — er belegt beim Sampling kein VRAM.

### 3. CLIP-L Text-Encoder

| Datei | Größe | SHA-256 |
|---|---|---|
| `clip_l.safetensors` | 0,25 GB | `660c6f5b1abae9dc498ac2d21e1347d2abdb0cf6c0c0c8576cd796491d9a6cdd` |

Quelle: [`comfyanonymous/flux_text_encoders`](https://huggingface.co/comfyanonymous/flux_text_encoders) ·
**Import-Typ: Text encoder / CLIP** · Lizenz: MIT.

### 4. VAE

| Datei | Größe | SHA-256 |
|---|---|---|
| `ae.safetensors` | 0,34 GB | `afc8e28272cd15db3919bacdb6918ce9c1ed22e96cb12c4d5ed0fba823529e38` |

Quelle: [`second-state/FLUX.1-dev-GGUF`](https://huggingface.co/second-state/FLUX.1-dev-GGUF)
(byte-identisch mit dem offiziellen BFL-VAE) · **Import-Typ: VAE** ·
Lizenz: FLUX.1 [dev] Non-Commercial.

### Companion-Auflösung

`capability::image` findet die drei Begleiter über die Rolle
(`text_encoder` / `vae`) und den Dateinamen: „t5" → T5, „clip" (ohne „t5") →
CLIP-L. Fehlt eine Datei, schlägt der Job mit einer Klartext-Meldung fehl
(„Flux needs a T5 text encoder — import …").

**Empfohlene Settings:** 1024×1024, 20 Steps, **Guidance ≈ 3–4** (die UI zeigt
„Guidance" statt „CFG", wenn ein Flux-Modell gewählt ist; der Sampler läuft
immer bei CFG 1). Sampler `euler`, Scheduler `simple`. Negativ-Prompt wirkt
**nicht** (Guidance-distilled).

**VRAM:** Q8 ~14–15 GB Peak beim Sampling (Unet + VAE-Decode + Compute-Buffer;
der T5 ist ausgelagert). Auf 16 GB knapp — der Scheduler verdrängt ein
residentes LLM.

---

## Bewusst nicht in der Liste

- **SD 3.5** — krummer VRAM-Bedarf (12–18 GB), kleines Ökosystem.
- **Qwen-Image**, **Flux Kontext**, **Flux Schnell** — Phase 6+.
- ControlNet / IPAdapter / Upscaler / LoRAs — eigene Slices, bewusst später.
- `.safetensors`-Flux (fp8, ohne GGUF) — der GGUF-Weg deckt 16 GB besser ab.
