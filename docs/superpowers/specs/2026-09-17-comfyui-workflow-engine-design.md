# ComfyUI Workflow Engine — Design (Phase A fragment layer + Phase B1 Hi-Res-Fix)

Status: written 2026-09-17 under the user's standing "work through the backlog" mandate; the
direction ("first a reusable template layer, then quality features as templates on it") was
decided by the user on 2026-09-16 (see `docs/TODO.md` → "ComfyUI Workflow-Engine").

## Ziel

Bessere Bild-/Video-Ausgabequalität mit weniger Wartungskosten. Heute ist jede Generierungs-
Lane eine handgeschriebene Rust-Funktion in `core/src/pipeline/mod.rs` (2 187 Zeilen), die den
ComfyUI-Graphen Node für Node baut; Checkpoint-Load, Prompt-Encode und Sampler-Verdrahtung
wiederholen sich leicht abgewandelt über acht Funktionen. Ein neues Qualitäts-Feature müsste
in jede Lane einzeln eingebaut werden.

## Entscheidungen

| Frage | Entscheidung |
|---|---|
| Rohe Workflow-JSON aus dem ComfyUI-Editor importieren? | **Nein.** Das brächte die Node-Graph-Komplexität zurück, die die App bewusst versteckt. Es werden Ideen aus `E:\locally-uncensored` (AGPL) übernommen, kein Code. |
| Was ist die Template-Schicht? | **Fragmente**: kleine, benannte, einzeln getestete Graph-Bausteine (Loader, Conditioning, Latent, Sampler-Pass, Latent-Upscale, Decode/Save, LoRA-Kette, IP-Adapter, Reference-Latent). Ein Rezept = geordnete Komposition von Fragmenten über einen typisierten Graph-Builder. |
| Verhalten der bestehenden Rezepte | **Byte-identisch** zum heutigen JSON (gleiche Node-Ids, gleiche Inputs). Grund: Tests, `aiwm-fake-comfy` und Story Studio verlassen sich auf Node-Ids; ein Refactor ohne sichtbare Änderung ist beweisbar (Golden-Fixtures, vor dem Umbau erzeugt). |
| Erstes Qualitäts-Rezept | **Hi-Res-Fix** (zweiter Low-Denoise-Pass nach Latent-Upscale): kein neues externes Modell, alle Nodes im ComfyUI-Kern, für SDXL/Checkpoint, FLUX.1, FLUX.2 [klein] (GGUF und safetensors). |
| Bedienung | Per-Generierung-Schalter im Image-Tab ("Hi-res fix" mit Skalierung 1.5×/2× und Denoise 0.3–0.6), Voreinstellung aus; kein globaler Qualitätsmodus (kann später darauf aufsetzen). |
| Messung | Echte Zahlen (Zeit, VRAM-Spitze) auf der 4080 Super für SDXL 1024→1536/2048 und klein 4B/9B, in `docs/TODO.md`; keine Annahmen. |
| Später (eigene Specs) | Face-Restore als Post-Process-Fragment; ControlNet/Region-Conditioning (braucht Model-Download-Weg + Preprocessor-Nodes, echte Recherche nötig). |

## Abschnitt 1 — Graph-Builder und Fragmente

`core::pipeline::graph::Graph`: ein dünner Wrapper um `serde_json::Map`, der Nodes unter
**expliziten Ids** anlegt (`g.node("4", "CheckpointLoaderSimple", json!({...}))`) und Links als
`Link { node: &str, slot: u32 }` typisiert (`link("4", 0)` → `["4", 0]`). Explizite Ids sind
Pflicht, damit die heutigen Graphen unverändert bleiben; ein `NextId`-Allokator existiert nur
für Fragmente, die neue Nodes einfügen (LoRA-Kette ab "90", Hi-Res-Fix ab "40").

Fragmente (`core::pipeline::fragments::*`), jeweils eine Funktion `fn(&mut Graph, …) -> Outputs`
mit einem kleinen Output-Struct (welche Links sie liefern):

- `loaders`: `checkpoint(g, id, file) -> Loaded { model, clip, vae }`, `flux_gguf(…)`,
  `flux2_klein_gguf(…)`, `flux2_klein_safetensors(…)`, `wan(…)`, `ltx(…)`.
- `conditioning`: `encode_pair(g, clip, pos, neg) -> Cond { positive, negative }`,
  `flux_guidance(g, cond, value)`, `zero_out(g, cond)`.
- `latent`: `empty(g, w, h)`, `empty_sd3(g, w, h)`, `empty_flux2(g, w, h)`,
  `upscale_by(g, id, latent, method, scale) -> Link`.
- `sampling`: `ksampler(g, id, model, cond, latent, SamplerParams { seed, steps, cfg,
  sampler, scheduler, denoise }) -> Link`, `custom_advanced(g, ids, model, cond, latent,
  CustomAdvancedParams { seed, steps, denoise, … }) -> Link` (die klein-GGUF-Kette
  `KSamplerSelect + Flux2Scheduler + RandomNoise + CFGGuider + SamplerCustomAdvanced`).
- `output`: `decode_and_save(g, decode_id, save_id, latent, vae, prefix)`.
- `loras`: die heutige `apply_loras` unverändert (Vorbild).
- `ipadapter`, `reference`: die heutigen Story-Studio-Fragmente, nur verschoben.

Rezepte (`core::pipeline::recipes::{image, video, upscale}`) komponieren daraus die heutigen
acht Bild-, zwei Video- und zwei Upscale-Graphen. Die öffentliche API von `core::pipeline`
(`Recipe`, `Txt2ImgInputs`, `checkpoint_txt2img`, …) bleibt erhalten (Re-Exports), sodass
`capability::{image,video,upscale}` unverändert kompilieren.

## Abschnitt 2 — Hi-Res-Fix

`Txt2ImgInputs.hires: Option<HiresFix { scale_by: f64, denoise: f64, steps: u32 }>` mit
Grenzen `scale_by ∈ [1.25, 2.0]`, `denoise ∈ [0.2, 0.7]`, `steps ∈ [4, 60]` (geklammert im
Request-Parser, nicht im Fragment). Fragment `hires_fix(g, first_pass_latent, model, cond,
vae?, …)`: `LatentUpscaleBy` (`upscale_method: "nearest-exact"`, `scale_by`) → zweiter
Sampler-Pass mit demselben Modell/Conditioning und `denoise` → der Decode hängt am zweiten
Pass. Pro Rezept-Familie:

- Checkpoint (SDXL): `KSampler` → `LatentUpscaleBy` → `KSampler(denoise)` → `VAEDecode`.
- FLUX.1 GGUF und klein-safetensors: identisch (beide nutzen `KSampler` bei CFG 1).
- klein GGUF: zweite `SamplerCustomAdvanced`-Kette mit eigenem `Flux2Scheduler`, dessen
  `denoise`-Input **gegen den ComfyUI-v0.34.0-Quellcode zu verifizieren** ist (gleiche
  Disziplin wie beim RTX-Node); wenn `Flux2Scheduler` keinen `denoise`-Input hat, wird die
  Sigma-Kette über `SplitSigmas`/`SplitSigmasDenoise` gekürzt — was davon existiert,
  entscheidet der Quellcode, nicht die Erinnerung.
- Video-Rezepte: kein Hi-Res-Fix in dieser Phase.

Node-Ids des Fragments: "40" (`LatentUpscaleBy`), "41" (zweiter Sampler; bei klein GGUF
"41"–"44" für Scheduler/Guider/Sampler), der Decode bleibt "8" und hängt um. LoRA-Kette und
IP-Adapter sitzen vor dem ersten Sampler und wirken automatisch auf den zweiten (gleicher
Model-Link).

## Abschnitt 3 — API/UI

`ImageRequest.hires: Option<HiresFix>` aus `params.hires` (`{ scale_by, denoise, steps }`),
`apply_to` schreibt es zurück; VRAM-Schätzung: der zweite Pass läuft auf `scale_by²`-facher
Latentgröße → `media_vram_mb` bekommt den Hi-Res-Faktor (Aktivierungs-Headroom × `scale_by²`
für den zweiten Pass, Gewichte unverändert). Image-Tab: Schalter „Hi-res fix" mit Skalierung
(1.5× / 2×) und Denoise-Regler, Voreinstellung 1.5× / 0.45 / Schritte = halbe Erst-Pass-
Schritte; die Ergebnis-Karte zeigt die finale Auflösung. Story Studio nutzt denselben
Request und bekommt den Schalter nicht (bewusst: dort zählt Konsistenz, nicht Auflösung).

## Abschnitt 4 — Tests

- Golden-Fixtures: vor dem Umbau werden die JSON-Graphen aller zwölf heutigen Rezepte mit
  festen Eingaben nach `core/tests/fixtures/graphs/*.json` geschrieben; nach dem Umbau muss
  jedes Rezept byte-identisch (nach `serde_json` Normalisierung) rendern.
- Fragment-Unit-Tests: jedes Fragment einzeln (Nodes, Inputs, gelieferte Links).
- Hi-Res-Fix: pro Familie ein Test (Upscale-Node, zweiter Pass mit `denoise`, Decode hängt am
  zweiten Pass, LoRA-Kette wirkt auf beide Pässe); ein Request-Parser-Test (Klammern).
- Integration: `core/tests/image_job.rs` bekommt einen Fall „mit Hi-Res-Fix", der über
  `aiwm-fake-comfy` beweist, dass der eingereichte Graph den zweiten Pass enthält (Fixture
  bekommt `GET /__test/last_graph_node_types`).
- Echte Hardware: ein SDXL-Render 1024² → 1.5× und einer → 2×, ein klein-4B-Render 1.5×,
  Zeit + VRAM-Spitze gemessen und in `docs/TODO.md` festgehalten; das ist der Beweis, ob
  der Aufwand sich lohnt.

## Nicht enthalten

Face-Restore, ControlNet, globale Qualitätsstufen, Import fremder Workflow-Dateien, Video-
Hi-Res, Sampler-/Scheduler-Auswahl pro Pass.
