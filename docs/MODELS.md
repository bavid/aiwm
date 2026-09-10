# Models

Wie das Tool Modelle verwaltet. Stand + Plan.

## Abstraktionskette (Brief 10.22)

```
Model → Capability → Runtime → Pipeline → Job → Hardware
```

Ein Modell wird nicht direkt gewählt — der Nutzer wählt eine **Capability**, das
System schlägt passende Modelle/Pipelines vor.

## Datenmodell (Schema v1, seit WP-2)

`models` — id (uuid v7), publisher, name, family, format, quant, arch,
param_count, file_path (unique), sha256, size_bytes, ctx_max, vram_estimate_mb,
ram_estimate_mb, source, source_revision, imported_at, last_used_at, use_count,
`n_layers` / `n_embd` / `n_heads` / `n_kv_heads` (GGUF-Arch-Dims für die
VRAM-Schätzung, Migration 0004 / 2.6). `model_roles` (coding / chat / upscaler /
base_diffusion / …), `model_links` (pro Runtime: passthrough | extra_path |
junction | hardlink | copy).

## `ModelKind` — getypter Store (3.3 · Video 4.1)

Beim Import bestimmt `core::model::ModelKind` (Typ-Hint aus der UI oder aus der
Endung: `.gguf`→`chat`, `.safetensors`→`checkpoint`) das Ziel:

| `ModelKind` | Store-Unterordner | ComfyUI-`folder_paths`-Schlüssel | Rolle (auto) |
|---|---|---|---|
| `Chat` | `llm/<slug>/` | — (llama.cpp) | — (User wählt) |
| `Checkpoint` | `image/checkpoints/` | `checkpoints` | `base_diffusion` |
| `DiffusionModel` | `image/diffusion_models/` | `diffusion_models` | `base_diffusion` |
| `Vae` | `image/vae/` | `vae` | `vae` |
| `Lora` | `image/loras/` | `loras` | — |
| `TextEncoder` | `image/text_encoders/` | `text_encoders` | `text_encoder` |
| `VideoModel` | `video/diffusion_models/` | `diffusion_models` | `base_video` |

Der Video-Store (`<store>/video/`) bekommt einen eigenen zweiten Block
`aiwm_video:` in `extra_model_paths.yaml` (`base_path: <store>/video`). Wans
Begleiter (umt5-Encoder, VAE) werden weiterhin als `TextEncoder` / `Vae`
importiert und liegen unter `<store>/image/{text_encoders,vae}/` — ComfyUI merged
die Ordner-Keys und findet sie per Dateiname; die kosmetische Vermischung ist ein
TODO. Typ-Hint `video` beim Import routet in den Video-Store.

`import_model` nimmt `.gguf` **und** `.safetensors`; `.ckpt`/`.bin`/`.pt`/`.pth`
werden mit Klartext abgelehnt (Pickle kann beim Laden Code ausführen — erst nach
`.safetensors` konvertieren). GGUF-Header wird für `chat` geparst; der
**`.safetensors`-Header** (bounded reader, 6.3b) füllt **Param-Count +
Precision** (dominanter DTYPE, fp8-Varianten → `FP8`) + `arch` aus
`__metadata__` — ein nicht lesbarer Header failt den Import **nicht** (Fallback
auf die Namens-Heuristik). Bild-Modelle landen flach im Typ-Ordner (ComfyUI-
Konvention), Namens-Kollision → `-<hash8>`-Suffix.

Bild- und Video-Modelle bekommen die Rolle aus dem Typ
(`ModelKind::default_role`), so dass `Auto` das `base_diffusion`- bzw.
`base_video`-Modell findet und `capability::image` / `capability::video` die
Begleiter (`text_encoder` / `vae`) auflösen können (3.4/3.6/4.1). `family` aus
der Datei-Namens-Heuristik; **VRAM-Headroom pro Familie** (6.3b,
`media_headroom_mb`): Wan 6 GB · LTX/Flux/SD3 4 GB · SDXL 2 GB · sonst 2,5 GB —
dokumentierte Heuristik (`HARDWARE.md`-Bereiche), echte Kalibrierung wartet auf
den Hands-on-ComfyUI-Lauf (4.0).

## Katalog — „Known models" (3.6 · Video 4.4)

`core::model::catalog::KNOWN_MODELS` — eine kuratierte Liste mit HF-Quelle,
**SHA-256**, Größe, Lizenz. Bild: SDXL + der Flux-Stack (Diffusions-GGUF, T5,
CLIP-L, VAE). Video (4.4): der **Wan 2.2 TI2V-5B**-Stack (Modell + umt5 + VAE)
und **LTX-Video 0.9.5 2B** (Modell + VAE gebündelt, T5 = der Flux-T5-Eintrag).
`GET /models/known` / `list_known_models` speist den **„Known models"**-Abschnitt
im Models-Tab. Kein Download-Manager im MVP (Phase 6) — man lädt selbst und
importiert; bei SHA-256-Treffer stempelt `import_model`
`publisher`/`family`/`source_revision = catalog:<id>`. Details + empfohlene
Settings: [IMAGE_MODELS.md](IMAGE_MODELS.md) / [VIDEO_MODELS.md](VIDEO_MODELS.md).

Für **Agent-Sessions** (Phase 5) braucht es ein Chat-GGUF mit der zusätzlichen
Rolle **`coding`** und verlässlichem Tool-Calling (`llama-server --jinja`).
Kandidaten + Import + Smoke: [AGENT_MODELS.md](AGENT_MODELS.md). Noch nicht im
`KNOWN_MODELS`-Katalog (Phase 6).

## Status

| Feature | Stand |
|---|---|
| Schema + Tabellen | ✅ WP-2 (+ `jobs.result`, Migration 0002, 2.4a) |
| `ModelRepo` (CRUD, Rollen, `find_by_*`, `pick_for_role`) | ✅ 2.1 / 2.4a |
| Manueller Modell-Import (GGUF **oder** `.safetensors` wählen → getypter Store) | ✅ 2.1 · `.safetensors` + `ModelKind`-Routing + Pickle-Ablehnung 3.3 |
| `Auto`-Modellwahl (Rolle → zuletzt/meist genutzt → Name) | ✅ 2.4a `chat` (ADR-015) · 3.4 `base_diffusion` für `job_type=image`; benchmark-gestützt erst Phase 6 |
| Kanonischer Store + Link-Manager | ✅ Store 2.1 · `core::link` 2.3 (Passthrough/Junction/Hardlink/Copy, `model_links`, ADR-007) · `ExtraPath` 3.3 (ADR-019). GGUF → llama.cpp = `passthrough`; Bild → ComfyUI = `extra_path` |
| VRAM-Fit-Schätzung vor dem Load (`core::compat`) | ✅ 2.6 (ADR-016): Gewichte + KV-Cache aus GGUF-Arch-Dims + flacher Overhead, geschätzt für `min(ctx_max, 8192)`; Scheduler plant dagegen; passt es nicht → `blocked` mit Klartext. + 6.3 **`compat::verdict`** → 🟢/🟡/🔴 mit Begründung (VRAM-Budget + freier RAM für Offload) für Discovery/Upgrade-Check. Konstanten-Kalibrierung → 4.0 |
| Kompatibilitäts-Engine (🟢/🟡/🔴 vor Download) | ✅ 6.3 `compat::verdict` in der „Discover"-Dateiliste + `.safetensors`-Header-Reader (Param-Count/Precision) im Import |
| Kuratierter „Known models"-Katalog (SHA-256, HF-Quelle, Lizenz) | ✅ 3.6 (`core::model::catalog`, `GET /models/known`); Auto-Download → Phase 6 |
| Online-Discovery (HF Hub, Ollama-Library) | ✅ 6.1 `core::registry` + 6.2 „Discover"-Panel im Models-Tab (`GET /registry/{search,models/{id}}`, Fit-Ampel via `core::compat`, „Copy link"); Ollama später |
| Download-Manager (Queue, Resume, Verify, Speicherplan) | Phase 6 (6.4) |
| Dedup-/Unused-/Versions-Reports | Phase 6 |
| Benchmark-gestützte Auto-Auswahl | Phase 6 (braucht [BENCHMARKS.md](BENCHMARKS.md)) |

## Hardware-Realität

Welche Modellgrößen/Quantisierungen auf der RTX 4080 Super (16 GB) sinnvoll sind:
[HARDWARE.md](HARDWARE.md). Kurz: 7–14B komfortabel, ~24–30B an der Kante,
70B unrealistisch. Video: Kurzclips 480–720p.

## Phase 6 — Model-Manager v2

Scheibenplan + API-Research: [PHASE_6_PLAN.md](PHASE_6_PLAN.md). `core::registry`
(HF-Hub-Quellen-Adapter, `ModelSource`-Trait), Download-Manager, `core::bench`
(lokale Mikro-Benchmarks), Discovery-UI, Upgrade-Check, Aufräum-Reports.
**Kein automatischer Download aus unbekannten Quellen.**

**6.0-Spike ✅ (ADR-022 / ADR-024)** — HF-Hub-API live geprüft:
- `GET /api/models?…&expand[]=gguf&expand[]=safetensors&expand[]=gated&…` —
  `expand[]` geht **auch auf dem Listen-Endpoint**; ein Call bringt Param-Count,
  `context_length`, Precision (`safetensors.parameters`-DTYPE-Key), Lizenz,
  `base_model`, `lastModified` — **ohne Download**.
- **`filter=base_model:<owner/repo>`** findet alle Quant-Re-Uploads + Abkömmlinge
  eines Basismodells → Kern des Upgrade-Checks.
- **Verify-SHA-256 vorab:** `GET /api/models/{id}/tree/{rev}?recursive=true` →
  `lfs.oid` (nicht `xetHash`). Split-GGUFs als Set.
- Gated-Repos: Metadaten + `/tree` ohne Token (200); nur `/resolve/` braucht
  Lizenz+Token. Rate-Limit anon 500 API-Calls / 5 min / IP.
- **Remote-Quant-Erkennung aus dem Dateinamen** (`q4_k_m`, `q8_0`, `fp16`, …) —
  die HF-`gguf`-Metadaten tragen `general.file_type` nicht. Lokal:
  `gguf::ftype_name` (schon da).
- **Ollama:** kein Such-API (OCI-Manifest nur für bekannte Namen) → best-effort-
  Adapter, später.
