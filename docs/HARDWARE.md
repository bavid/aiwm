# Hardware-Einschätzung

Ziel: realistische Erwartungen. Keine Empfehlungen, die auf dieser Maschine
praktisch unbrauchbar sind. Konkrete Modellnamen/-versionen sind zum
Implementierungszeitpunkt neu zu prüfen — die *Größenordnungen* hier sind stabil.

## System

| Komponente | Wert | Bedeutung fürs Projekt |
|---|---|---|
| RTX 4080 Super | 16 GB GDDR6X, ~736 GB/s, Ada (gute FP8/BF16) | Bestimmt die harte Obergrenze. FP8 nutzbar. |
| RAM | 32 GB DDR5 | **Der eigentliche Engpass** für Offloading (Video, große Bilder, langer Kontext). 64 GB würde Video/Longcontext spürbar verbessern. |
| Ryzen 7 7800X3D | 8C/16T, großer L3 | Stark genug für CPU-Offload einzelner Layer; nicht für Voll-CPU-Inferenz großer Modelle. |
| SSD `E:` | ~1,5 TB frei | Reicht für eine ernsthafte Bibliothek über mehrere Phasen. Store: `E:\AI\models`. |

Praktisch nutzbar sind meist **~14–15 GB VRAM** (Windows/Treiber/Desktop belegen
~1 GB, plus Headroom gegen OOM).

---

## LLM (Text / Coding)

### Was flüssig läuft

| Klasse | Beispiele (2026, zu verifizieren) | Quant | VRAM (Gewichte) | Kontext realistisch | Speed (grob) | Eignung |
|---|---|---|---|---|---|---|
| 7–9B dense | Qwen2.5-Coder 7B, Llama-3.1 8B | Q5–Q8 | 6–9 GB | 32–128k | 60–100+ tok/s | Autocomplete, schnelle Agents, gut mit Reserve für KV-Cache |
| 12–15B dense | Qwen2.5-Coder 14B, Phi-4 14B | Q4–Q6 | 9–12 GB | 32–64k+ | 30–55 tok/s | Solider Allrounder für Coding-Agents |
| ~20B MoE | gpt-oss-20b (MXFP4) | nativ | ~12–13 GB | 128k | schnell (nur ~3–4B aktiv) | Sehr gutes Preis/Leistung auf 16 GB |
| ~24B dense | Devstral Small (agentic Coding) | Q4_K_M | ~14–15 GB | 16–32k (knapp) | 15–25 tok/s | Passt, aber **kaum Headroom** für Bild/Video parallel oder großen KV-Cache |
| ~30B MoE | Qwen3-Coder-30B-A3B (3B aktiv, ~256k nativ) | IQ4/Q4 | ~13–18 GB (Grenzfall) | 64–256k mit KV-Quant + Offload | 20–40 tok/s | **Bester Coding-Kandidat**, aber genau an der Kante → Estimator muss ehrlich sein |

### Was grenzwertig ist

- **32B dense** (z. B. Qwen2.5 32B): Q4 ~19–20 GB → CPU-Offload nötig → ~8–15 tok/s.
  Für interaktives Coding zäh, für Batch ok.
- **Sehr großer Kontext (128k+) bei 14–24B dense**: nur mit KV-Cache-Quantisierung
  (Q8 oder Q4) und Flash-Attention. Ohne das läuft der VRAM über.

### Was nicht realistisch ist

- **70B**: Q4 ~40 GB. Nur mit IQ2 + massivem Offload lauffähig, dann langsam und
  qualitativ schwach. **Nicht empfehlen.**

### KV-Cache — die oft übersehene Größe

Der KV-Cache wächst mit `Kontextlänge × Layer × Heads`. Grobe Hausnummer: bei
einem 14B-Modell kann 128k-Kontext mehrere GB kosten — das konkurriert direkt mit
den Gewichten. Hebel: GQA-Modelle bevorzugen, KV-Cache auf Q8/Q4 quantisieren,
Flash-Attention, Kontext nur so groß wie nötig.

### Antwort auf „Agents lange autonom" (Brief Abschnitt 4)

Auf dieser Hardware **nicht** über ein riesiges Context-Window lösbar. Der
tragfähige Weg:

1. **Modellwahl:** MoE mit nativ langem Kontext (Qwen3-Coder-30B-A3B) oder ein
   effizientes 14B — beide lassen Raum für KV-Cache.
2. **Context-Engineering im Tool:** Kompaktierung alter Turns, Tool-Output-Kürzung,
   Repository-Retrieval statt Voll-Dump, Sub-Agents mit frischem Kontext,
   periodische Checkpoints.
3. **Scheduler:** aktives Modell „pinnen", nicht mitten in der Session evakuieren.

Das ist Software-Arbeit im Tool, nicht primär eine Hardware- oder Modellfrage.

---

## Bildgenerierung

| Aufgabe | Modelle (zu verifizieren) | VRAM | Zeit/Bild (grob) | Bewertung |
|---|---|---|---|---|
| Text→Image (schnell) | SDXL, SD 3.5 Medium | 6–10 GB | 2–8 s | Sehr komfortabel |
| Text→Image (Qualität) | Flux.1 dev/schnell (FP8) | ~12 GB | 15–40 s | Komfortabel, klarer Sweet Spot |
| Text→Image (Text-Rendering, 20B) | Qwen-Image (FP8 ~16 GB / GGUF Q4–Q5) | 12–16 GB | 30–70 s | Passt; GGUF-Quant für mehr Headroom |
| Image→Image / Inpaint / Outpaint | SDXL-/Flux-Inpaint | wie oben | wie oben | Gut |
| ControlNet / LoRA | zu SDXL/Flux | +1–3 GB | +wenig | Gut auf 16 GB |
| Upscale (Standard) | 4x-ESRGAN-Varianten, RealESRGAN | 1–3 GB | Sekunden | Trivial |
| Upscale (Diffusion, Qualität) | SUPIR o. ä. | hoch, Offload | Minuten | Nur mit Tiling/Offload, langsam |
| Face-Restore | GFPGAN / CodeFormer | 1–2 GB | Sekunden | Trivial |
| Hintergrund entfernen | RMBG / BiRefNet | 1–2 GB | Sekunden | Trivial |
| Segmentierung (für Objekt-/Personen-Entfernung) | SAM 2 | 2–4 GB | Sekunden | Gut; danach Inpaint |

**Fazit Bild:** die hardwarefreundlichste Domäne. Nahezu alles Sinnvolle läuft gut.

---

## Videogenerierung

Ehrliche Erwartung: **2–5 Sekunden Clip, 480–720p, 2–10 Minuten Rechenzeit**,
starkes Offloading. Kein Video-Studio.

| Modell (zu verifizieren) | Variante | VRAM | Realistisch auf dieser Maschine? |
|---|---|---|---|
| Wan 2.2 (5B, TI2V) | nativ/FP8 | 8–12 GB | ✓ Primär-Empfehlung. 480–720p, kurze Clips. |
| Wan 2.2 (14B) | GGUF Q4–Q5 + Offload | ~15–16 GB + viel RAM | 🟡 Grenzwertig. 32 GB RAM → Swapping-Risiko. Mit 64 GB deutlich besser. |
| LTX-Video (distilled FP8) | FP8 | ~12–16 GB | ✓ Schnellste Option, dafür geringere Detailtreue. |
| HunyuanVideo (voll) | — | >24 GB | ✗ Nicht auf 16 GB. |
| Frame-Interpolation | RIFE / FILM | 1–2 GB | ✓ Trivial. |
| Video-Upscale | per Frame ESRGAN | 2–3 GB | ✓ Funktioniert, langsam (pro Frame). |

**Empfehlung:** Video ab Phase 4 mit **Wan 2.2 5B** und **LTX distilled** als
getestete Optionen. 14B nur als „Advanced, langsam"-Pfad. RAM-Upgrade auf 64 GB
als Hardware-Empfehlung dokumentieren.

---

## Image-Enhancement / Editing

| Aufgabe | Pipeline (Skizze) | Hardware |
|---|---|---|
| General Enhancement | Denoise → mild Upscale → Sharpen | leicht |
| Face / Skin | Face-Detect → CodeFormer/GFPGAN → (optional Inpaint für Haut) | leicht |
| Muttermal / kleine Störungen entfernen | SAM2-Maske → Inpaint (SDXL/Flux) | mittel |
| Objekt / Person entfernen | SAM2-Maske → Inpaint/Outpaint, ggf. mehrere Passes | mittel |
| Upscale 2x–4x | RealESRGAN / 4x-Modell, Tiling bei großen Bildern | leicht–mittel |
| Restore (alt/verrauscht) | Denoise → Face-Restore → Upscale | mittel |
| Relighting | dediziertes Relight-Modell (zu evaluieren) | mittel–hoch |

**Fazit:** Enhancement ist gut machbar und wäre der Bereich mit dem schnellsten
„Wow" — wird aber laut Scope-Entscheidung erst nach dem Plattform-Kern gebaut.

---

## Speicherplatz (Größenordnungen)

| Artefakt | Grobe Größe |
|---|---|
| 1 Coding-LLM (14–30B, Q4 GGUF) | 8–20 GB |
| 1 Diffusion-Basismodell (Flux/Qwen-Image) | 12–24 GB |
| Video-Modell (Wan 2.2) | 10–30 GB |
| Aux (VAE, Text-Encoder, ControlNet, Upscaler, SAM) | 5–15 GB gesamt |
| ComfyUI + venv + Torch (CUDA) | 8–12 GB |
| llama.cpp + CUDA-Runtime | 1–3 GB |

Eine ernstzunehmende lokale AI-Bibliothek belegt schnell **200–400 GB**. Bei
~1,5 TB frei auf `E:` ist das mehrere Phasen lang unkritisch; der Model-Manager
prüft trotzdem freien Platz vor jedem Download und liefert Dedup/Unused-Reports
(Brief 21).

---

## Estimator-Konstanten (Stand 6.3 — noch nicht gemessen kalibriert)

`core::compat` schätzt den VRAM-Bedarf **vor** dem Load; `core::model::import`
schätzt Bild-/Video-Modelle. Die Konstanten sind aus den Größenordnungen oben
abgeleitet, **nicht** aus `nvidia-smi`-Messungen — echte Kalibrierung wartet auf
den Hands-on-ComfyUI-Lauf (4.0) + den manuellen Agent-Smoke.

| Konstante | Wert | Bedeutung |
|---|---|---|
| `RUNTIME_OVERHEAD_MB` | 650 | CUDA-Kontext + cuBLAS-Workspace + Compute-Graph pro Modell (`llama-server`) |
| `KV_ROUGH_MB_PER_1K_CTX` | 160 | KV-Cache-Reserve pro 1K Token, wenn die GGUF-Arch-Dims fehlen (dense-13B-nah, GQA deutlich darunter) |
| `FIT_TIGHT_PCT` | 85 % | ab hier ist ein Fit „gelb" statt „grün" — kein Puffer für längeren Kontext / ein zweites Modell |
| `OFFLOAD_RAM_RESERVE_MB` | 4096 | RAM für OS + Page-Cache, das beim Layer-Offload frei bleiben muss |
| `media_headroom_mb` | Wan 6144 · LTX/Flux/SD3 4096 · SDXL 2048 · sonst 2560 | Sampler-Aktivierungen + VAE-Decode + Compute-Buffer über die Gewichts-Größe |

Kalibrierungs-Plan: bei 4.0 pro getestetem Modell `nvidia-smi`-Peak während
Load + Sampling loggen, gegen `estimate().total_mb` halten, die Flat-Konstanten
nachziehen; Ergebnis-Tabelle hier ergänzen.

## Hardware-Empfehlungen (optional)

| Upgrade | Nutzen | Priorität |
|---|---|---|
| RAM 32 → 64 GB DDR5 | Video (14B), großer Kontext, große Bild-Batches, weniger Swapping | **hoch**, wenn Video/Longcontext wichtig |
| Große schnelle NVMe (2–4 TB) nur für `E:\AI\models` | Modell-Bibliothek ohne ständiges Aufräumen | mittel |
| GPU 16 → 24 GB (z. B. später) | 32B dense flüssig, Video 14B komfortabel, Coding-Modell + Bild parallel | mittel (teuer) |
