# ComfyUI Workflow Engine — Plan 3 (fragment layer + Hi-Res-Fix)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `core/src/pipeline/mod.rs` (2 187 lines of hand-built ComfyUI graphs) into a small graph builder plus composable, individually tested fragments — byte-identical output for every existing recipe — and ship the first quality recipe on top of it: Hi-Res-Fix (latent upscale + second low-denoise pass) for SDXL/checkpoint, FLUX.1 and FLUX.2 [klein], with a per-generation toggle in the Image tab and real measurements on the 4080 Super.

**Architecture:** `pipeline::graph::Graph` (explicit node ids, typed links) → `pipeline::fragments::{loaders, conditioning, latent, sampling, output, loras, ipadapter, reference}` → `pipeline::recipes::{image, video, upscale}`; the public `core::pipeline` API stays (re-exports). Golden JSON fixtures written from the CURRENT code before any refactor prove no behaviour change. Hi-Res-Fix is one fragment inserted between the first sampler and the decode.

**Tech Stack:** Rust (`serde_json`), ComfyUI v0.34.0 core nodes only (`LatentUpscaleBy`, `KSampler`, `Flux2Scheduler`, `SplitSigmasDenoise`, `SamplerCustomAdvanced`), React/TS UI, `aiwm-fake-comfy` fixture for integration.

**Spec:** `docs/superpowers/specs/2026-09-17-comfyui-workflow-engine-design.md`.

**Verified facts (ComfyUI v0.34.0 source, 2026-09-17):** `LatentUpscaleBy` inputs `samples`, `upscale_method` ∈ {nearest-exact, bilinear, area, bicubic, bislerp}, `scale_by` (0.01–8.0, default 1.5), one `LATENT` output (`nodes.py:1371-1382`). `Flux2Scheduler` inputs `steps`, `width`, `height` — **no `denoise`** (`comfy_extras/nodes_flux.py:213-232`). `SplitSigmasDenoise` inputs `sigmas`, `denoise` (0–1); outputs slot 0 `high_sigmas`, slot 1 `low_sigmas` = the last `round(steps*denoise)` sigmas (`comfy_extras/nodes_custom_sampler.py:228-249`). So the klein-GGUF second pass = `Flux2Scheduler(steps, upscaled w/h)` → `SplitSigmasDenoise(denoise)` → `SamplerCustomAdvanced(sigmas = ["<split>", 1])`.

**Hard rules:** TDD with observed RED; `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm typecheck/lint/build` clean before every commit; never `--no-verify`; zero `unsafe`; no `unwrap`/`expect` in production; explicit-pathspec commits in the shared worktree; every measurement from a real run.

---

### Task 1: Golden fixtures for the twelve current recipes

**Files:** create `core/tests/fixtures/graphs/README.md`, `core/tests/fixtures/graphs/*.json` (12 files), `core/tests/pipeline_goldens.rs`.

- [ ] **Step 1:** Write `core/tests/pipeline_goldens.rs` with ONE fixed input set per recipe (deterministic seeds, fixed file names, two LoRAs on the checkpoint recipe, one LoRA on each other model recipe, an IP-Adapter spec, a reference image, a Wan i2v start frame, LTX t2v, RTX image scale-by 2 and video) and a test per recipe: `render_<recipe>()` → `serde_json::Value`; if `AIWM_WRITE_GOLDENS=1` write `core/tests/fixtures/graphs/<recipe>.json` (pretty, sorted keys via `serde_json::to_string_pretty` on a `BTreeMap`-normalised value), else read the fixture and `assert_eq!(normalise(actual), normalise(expected))` with a diff-friendly message. Recipes: `checkpoint_txt2img`, `flux_txt2img`, `flux2_klein_txt2img`, `flux2_klein_txt2img_safetensors`, `flux2_klein_edit`, `checkpoint_ipadapter_txt2img`, `flux2_klein_reference_txt2img`, `flux2_klein_reference_txt2img_safetensors`, `wan_ti2v` (t2v and i2v → two fixtures), `ltx_video` (t2v and i2v → two), `rtx_upscale_image`, `rtx_upscale_video` (that is 14 fixtures; name them exactly).
- [ ] **Step 2:** Run once with `AIWM_WRITE_GOLDENS=1` (from the UNCHANGED pipeline code), commit the fixtures, run again without the env → all pass. RED check: temporarily flip one number in a fixture and see the test fail with a readable diff; restore.
- [ ] **Step 3:** Commit `test(pipeline): golden graph fixtures for every recipe`.

---

### Task 2: Graph builder and fragments (no recipe changes yet)

**Files:** create `core/src/pipeline/graph.rs`, `core/src/pipeline/fragments/{mod.rs, loaders.rs, conditioning.rs, latent.rs, sampling.rs, output.rs}`; modify `core/src/pipeline/mod.rs` (`mod graph; pub mod fragments;` only).

- [ ] **Step 1 (RED):** unit tests in each fragment file asserting the exact node JSON and returned links, e.g. `checkpoint_loader_emits_the_node_and_three_links` (`Loaded { model: Link("4",0), clip: Link("4",1), vae: Link("4",2) }`), `ksampler_wires_model_cond_latent_and_params`, `custom_advanced_chain_wires_select_scheduler_noise_guider_sampler` (ids "28","29","30","31","3" as today), `empty_latent_variants`, `decode_and_save`. Also `graph::tests`: `node_ids_are_explicit_and_links_render_as_pairs`, `next_id_allocates_from_a_base`.
- [ ] **Step 2:** Implement:

```rust
// graph.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct Link<'a> { pub node: &'a str, pub slot: u32 }
impl Link<'_> { pub fn json(self) -> Value { json!([self.node, self.slot]) } }
#[derive(Debug, Default)] pub struct Graph { nodes: serde_json::Map<String, Value> }
impl Graph {
    pub fn node(&mut self, id: &str, class_type: &str, inputs: Value) { self.nodes.insert(id.into(), json!({"class_type": class_type, "inputs": inputs})); }
    pub fn set_input(&mut self, id: &str, key: &str, value: Value) -> Result<(), PipelineError>; // node must exist
    pub fn input(&self, id: &str, key: &str) -> Option<&Value>;
    pub fn into_value(self) -> Value;
    pub fn contains(&self, id: &str) -> bool;
}
pub struct NextId(u32); impl NextId { pub fn new(base: u32) -> Self; pub fn take(&mut self) -> String }
```
Fragments return small structs of `Link<'static>`-like owned ids (`String` node + slot) — pick `OwnedLink { node: String, slot: u32 }` to avoid lifetime gymnastics. `sampling::SamplerParams { seed, steps, cfg, sampler, scheduler, denoise }`; `sampling::ksampler(g, id, model, positive, negative, latent, &params) -> OwnedLink`; `sampling::custom_advanced(g, ids: &CustomAdvancedIds { select, scheduler, noise, guider, sampler }, model, positive, negative, latent, &CustomAdvancedParams { seed, steps, width, height, sampler_name, cfg, sigmas_override: Option<OwnedLink> }) -> OwnedLink`.
- [ ] **Step 3:** gates; commit `feat(pipeline): graph builder and base fragments`.

---

### Task 3: Port the five image recipes onto fragments

**Files:** create `core/src/pipeline/recipes/{mod.rs, image.rs}`; modify `core/src/pipeline/mod.rs` (move the five functions' bodies into `recipes::image`, keep `pub use`), `core/src/pipeline/fragments/loras.rs` (move `apply_loras`, make it work on `Graph`).

- [ ] Port `checkpoint_txt2img`, `flux_txt2img`, `flux2_klein_txt2img`, `flux2_klein_txt2img_safetensors`, `flux2_klein_edit` one at a time; after each, `cargo test -p aiwm-core --test pipeline_goldens` must stay green (byte-identical). Keep the existing unit tests in `mod.rs` passing unchanged (they are the second safety net). Add fragments as needed (`conditioning::flux_guidance`, `latent::empty_sd3`, `latent::empty_flux2`, `loaders::flux_gguf`, `loaders::flux2_klein_gguf`, `loaders::flux2_klein_safetensors`, `output::load_image`/`vae_encode` for edit).
- [ ] Commit per recipe or as one: `refactor(pipeline): image recipes composed from fragments (golden-identical)`.

---

### Task 4: Port Story Studio, video and upscale recipes

**Files:** create `core/src/pipeline/fragments/{ipadapter.rs, reference.rs}`, `core/src/pipeline/recipes/{video.rs, upscale.rs}`; modify `core/src/pipeline/mod.rs` (becomes: types, `Recipe`/`VideoRecipe`, re-exports, ≤ 400 lines).

- [ ] Same procedure; goldens green after each. `mod.rs` tests may move next to their recipes. At the end `core/src/pipeline/mod.rs` ≤ 400 lines, every file ≤ 800.
- [ ] Commit `refactor(pipeline): story-studio, video and upscale recipes composed from fragments`.

---

### Task 5: Hi-Res-Fix fragment and the four txt2img recipes

**Files:** create `core/src/pipeline/fragments/hires.rs`; modify `core/src/pipeline/mod.rs` (`HiresFix` type on `Txt2ImgInputs`), `core/src/pipeline/recipes/image.rs`.

- [ ] **Step 1 (RED):** tests per family: `checkpoint_hires_inserts_latent_upscale_and_a_second_pass_then_decodes_from_it` (nodes "40" `LatentUpscaleBy { samples: ["3",0], upscale_method: "nearest-exact", scale_by: 1.5 }`, "41" `KSampler { denoise: 0.45, steps: 12, seed same, model/positive/negative same links as "3", latent_image: ["40",0] }`, "8".samples == ["41",0]); same for `flux_txt2img` and `flux2_klein_txt2img_safetensors`; `flux2_klein_gguf_hires_uses_flux2_scheduler_at_the_upscaled_size_and_split_sigmas_denoise` ("40" upscale, "42" `Flux2Scheduler { steps, width: w*scale, height: h*scale }` rounded to multiples of 16, "43" `SplitSigmasDenoise { sigmas: ["42",0], denoise }`, "44" `SamplerCustomAdvanced { noise: ["30",0], guider: ["31",0], sampler: ["28",0], sigmas: ["43",1], latent_image: ["40",0] }`, "8".samples == ["44",0]); `hires_none_leaves_the_graph_byte_identical` (goldens still pass); `loras_apply_to_both_passes` (model link of "41" equals the LoRA chain's model output).
- [ ] **Step 2:** `pub struct HiresFix { pub scale_by: f64, pub denoise: f64, pub steps: u32, pub upscale_method: &'static str /* "nearest-exact" */ }`, `Txt2ImgInputs.hires: Option<HiresFix>` (update every constructor site incl. tests with `hires: None`); `fragments::hires::apply_ksampler_family(g, first_pass: &OwnedLink, model, positive, negative, params_of_first_pass, &HiresFix) -> OwnedLink` and `apply_flux2_klein_gguf(...)`; recipes route the decode to the returned link when `hires` is `Some`.
- [ ] **Step 3:** gates; commit `feat(pipeline): Hi-Res-Fix fragment for checkpoint, FLUX.1 and FLUX.2 klein recipes`.

---

### Task 6: Request, VRAM estimate, API, ipc, dev-mock, integration test

**Files:** modify `core/src/capability/image.rs` (`ImageRequest.hires: Option<HiresFix>` parsed from `params.hires { scale_by, denoise, steps }` with clamps scale_by [1.25, 2.0], denoise [0.2, 0.7], steps [4, 60] and defaults 1.5 / 0.45 / max(4, steps/2); `apply_to`), `core/src/orchestrator/engine.rs` (`media_vram_mb`: headroom × `scale_by²` for the second pass — read the existing decomposition and add the factor with a test), `ui/src/lib/ipc.ts` (`ImageParams.hires?: { scale_by, denoise, steps }`), `ui/src/lib/dev-mock.ts` (echo it back), `core/src/bin/aiwm-fake-comfy.rs` (`GET /__test/last_graph_node_types` → the class_type list of the last submitted graph), `core/tests/image_job.rs` (a job with `hires` → the submitted graph contains `LatentUpscaleBy` and two samplers; without → one).
- [ ] Commit `feat(image): Hi-Res-Fix request option, VRAM factor, API and fixture proof`.

---

### Task 7: Image tab toggle

**Files:** modify `ui/src/features/image/Image.tsx` (+ css): a **Hi-res fix** checkbox with Scale (1.5× / 2×) and Denoise slider (0.2–0.7, default 0.45), steps auto (half of the first pass, editable in an "advanced" row), the result card shows the final resolution; Story Studio's submit path unchanged (no toggle there).
- [ ] Live verify against the dev-mock (start a job with hires, the submitted params carry `hires`, the result card shows e.g. 1536×1536); `pnpm typecheck && pnpm lint && pnpm build`; commit `feat(ui): Hi-res fix toggle in the Image tab`.

---

### Task 8: Real measurements and docs

**Files:** `docs/TODO.md` (replace the "ComfyUI Workflow-Engine (geplant)" block with "✅ Phase A + Hi-Res-Fix umgesetzt (2026-09-17)"), `docs/superpowers/plans/…` (this file: measured numbers appended).
- [x] Start the real app (`aiwm-cored` from this worktree with `AIWM_DATA_DIR=E:\AI\data`), run: SDXL 1024² without hires, with 1.5×, with 2× (same seed/prompt); FLUX.2 klein 9B fp8 (or 4B if installed) 1024² without and with 1.5×. Record wall time, peak VRAM (telemetry), output resolution, and whether the 2× run fits on 16 GB (if OOM: record it; do not tune away the truth). Save the four PNGs under `E:\AI\.smoke-hires\` (gitignored) and note their paths.
- [x] docs/TODO.md block in the existing German/✅ style: what shipped (graph builder, fragments, golden fixtures, Hi-Res-Fix recipes, toggle), the measured table, leftovers (Face-Restore, ControlNet, global quality tiers, video hires, sampler per pass). Full gates. Commit `feat(pipeline): docs + measured Hi-Res-Fix numbers`.
- [ ] Hand back: final whole-branch review → controller re-verifies gates → merge `--no-ff` → push.
---

## Measured 2026-09-17 (RTX 4080 SUPER, 16 GB)

Real `aiwm-cored` from this worktree (`AIWM_DATA_DIR=E:\AI\data`) against the
app-managed ComfyUI v0.34.0 — no fixtures. Prompt "a lighthouse on a rocky coast
at golden hour, dramatic clouds, highly detailed", negative "blurry, lowres",
1024x1024, 25 first-pass steps; Hi-Res at denoise 0.45, 12 executed steps,
`nearest-exact`. Each row is the median of >= 3 runs, **each with its own seed** —
ComfyUI caches node outputs, so a repeated seed returns the finished image in
~1.5 s without rendering anything. Peak VRAM = max of `nvidia-smi` sampled 5x/s
across the whole job, i.e. whole-card usage including ~1.4 GB desktop baseline
and the resident model.

| Modell | Auflösung | Hi-Res | Wandzeit warm | VRAM-Spitze | Ausgabe | Ergebnis/Anmerkung |
|---|---|---|---|---|---|---|
| SDXL base 1.0 (safetensors, cfg 7, euler/normal) | 1024×1024 | aus | **5,7 s** (3×5,7) | 10 573 MB | 1024×1024 | Referenzlauf |
| SDXL base 1.0 | 1024×1024 | 1,5× | **12,6 s** (12,0–13,2) | 14 005 MB | 1536×1536 | +6,9 s, Faktor 2,2. Mehr Fels-/Gischtdetail; das Web-/Gittermuster des Basisbilds im Wasser verschwindet. Komposition bleibt. |
| SDXL base 1.0 | 1024×1024 | 2,0× | **19,8 s** (18,0–21,1) | 14 701 MB | 2048×2048 | **Passt in 16 GB** — kein OOM, keine Planer-Absage, ~1,7 GB Luft. Aber die Komposition verschiebt sich sichtbar; 2,0× bei denoise 0,45 interpretiert um. |
| FLUX.2 [klein] 9B fp8mixed (safetensors → KSampler-Familie, guidance 4) | 1024×1024 | aus | **15,6 s** (3×15,6) | 13 075 MB | 1024×1024 | Referenzlauf |
| FLUX.2 [klein] 9B fp8mixed | 1024×1024 | 1,5× | **34,4 s** (33,0–35,8) | 14 095 MB | 1536×1536 | +18,8 s, Faktor 2,2. Deutlich mehr Textur bei praktisch identischer Komposition — bestes Qualität/Zeit-Verhältnis der Reihe. |
| FLUX.2 [klein] **GGUF** (`SamplerCustomAdvanced`-Zweig) | — | — | — | — | — | **Nicht messbar** — kein GGUF-klein-Modell installiert; nur fixture-bewiesen (`flux2_klein_txt2img_hires.json`). |

**Cold start (one-off, excluded from the table):** first image job of a session
(ComfyUI boot + model load + render), SDXL 1024² without hires: **45.6 s**. First
klein job after that (server up, SDXL evicted, klein + Qwen text encoder loaded):
**23.3 s**. Switching back and forth between SDXL and klein mid-session costs
24–41 s instead of the warm 15.6 s — the reload dominates, not the sampling.

**Honest gaps.** (1) The klein-GGUF branch is unmeasured — no such model is
installed. (2) klein + 1.5× sits on the VRAM limit: the planner charges
`13 092 MB` (import estimate) as `10 532 MB` weights + `2 560 MB` headroom and
scales the headroom by 2.25 (1.5² pixels) → **16 292 MB** against a 16 376 MB
budget, i.e. 99.5 %. With klein already resident (or an otherwise empty card) the
job runs; with SDXL still resident it never reaches the renderer and is `blocked`,
verbatim: "not enough VRAM for flux-2-klein-9b-fp8mixed: 16292 MB needed, but
this GPU only has 6176 MB usable in total — this model doesn't fit this card no
matter what else is running. Try a smaller quant/model." The message is also
misleading in that situation (the card *did* just carry the model, only not
alongside SDXL) — a calibration/wording item for later, not a Hi-Res regression.
(3) The 2.0× row proves "fits", not "is good".

**Proof images** (gitignored, `E:\AI\.smoke-hires\`): `sdxl-seed202-base-1024.png`
/ `sdxl-seed202-hires1.5x-1536.png`, `sdxl-seed523-base-1024.png` /
`sdxl-seed523-hires2.0x-2048.png`, `klein9b-seed302-base-1024.png` /
`klein9b-seed302-hires1.5x-1536.png` — each pair shares a seed, so the base image
*is* the hires run's first pass — plus the side-by-side crops `cmp-*.png`.
Resolutions above were read from the PNG headers, not from the job params.
