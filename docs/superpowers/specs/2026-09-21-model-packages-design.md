# Model Packages — Design (Plan 14, draft for approval)

Status: drafted 2026-09-21 from the user's request: "I can download as many safetensors LoRAs as I
want — without the base model it is worth nothing. 'Set import type' does nothing except switch to
the Flux-Klein/SDXL packages. When I search a LoRA it should check what I already have, build a
package, download what is missing. Way smarter and easier to configure — improve this drastically."
**Approved 2026-09-21 — see "Decided by the user" below.**

## What exists (verified in code/data 2026-09-21)

- **"Set import type"** (`ui/src/features/models/Models.tsx` `useType`, ~:137) only sets the type
  dropdown of the *manual file import* form and jumps to the "Add models" section, whose top is the
  curated catalog (FLUX.2 klein / SDXL stacks). It downloads nothing. It dates from the
  "download in your browser, then import here" flow, although the app has had its own download
  manager (queue, resume, sha256, and since `afc006f` the Civitai API key) for a long time.
- **Stacks** (`core/src/model/catalog.rs` `MODEL_STACKS`): curated, sha256-pinned bundles —
  `sdxl` (1 file), `flux` (FLUX.1-dev + T5 + CLIP-L + VAE), `flux2-klein` (9B GGUF/safetensors +
  Qwen3 encoder + VAE), `wan22` (TI2V-5B + UMT5 + VAE), `ltx` (+ T5), plus voice and captioners.
  They only cover what AIWM curated; nothing links a downloaded LoRA to one.
- **Family is inconsistent.** Library (read-only query of the real DB): 17 of 77 models have **no
  family**; the others use `sdxl`, `flux`, `flux2`, `flux2-klein-4b`, `wan`, `ltx`, … The image job
  partly decides architecture by **file-name heuristics** (`name_is_flux2`,
  `core/src/capability/image.rs:983`). The LoRA picker filters by family string and shows any LoRA
  with an unknown family "since it can't be ruled out" (`ui/src/components/LoraPicker.tsx:52-57`).
- **Civitai tells us the base.** Every version carries `baseModel`. Sample of the 60 most
  downloaded LoRAs (2026-09-21): SD 1.5 ×22, Wan Video 2.2 I2V-A14B ×7, Pony ×6, SDXL 1.0 ×6,
  Krea 2 ×6, ZImageBase ×3, Anima ×2, Illustrious ×2, LTXV 2.3, MiniMax H3, Wan 2.2 T2V-A14B,
  NoobAI, Flux.1 D, Other. **Many target bases this app has no stack for, or cannot run** (Wan 14B on
  16 GB, unknown architectures) — today the user finds that out after the download.

## The idea

One vocabulary for "which base does this need", and a resolver that turns *any* pick — a Civitai
LoRA, a checkpoint, a Hugging Face repo, or a file already in the library — into a **package**:
the item plus everything it needs, each marked **installed ✓ / in the catalog ↓ / findable on
Civitai ? / not runnable here ✗**, with the total download size and one button for what is missing.

## Decisions

| Question | Decision |
|---|---|
| Base-family registry | New `core/src/model/family.rs`: canonical families AIWM knows (`sdxl`, `flux1`, `flux2-klein-4b`, `flux2-klein-9b`, `wan22-5b`, `ltxv`, `sd15` if runnable — see open question 1). Each entry: display name, the Civitai `baseModel` labels and HF `base_model` ids that map to it, its **architecture group** (Pony/Illustrious/NoobAI → SDXL architecture), the catalog stack that provides base + companions, and **runnable here?** (yes / no + reason, e.g. "Wan 2.2 14B does not fit 16 GB"). Labels not in the table → `unknown`, never guessed. Unit-tested against the real labels above. |
| Family of what is already installed | Inferred **on read** by the resolver (recorded download metadata first, then the safetensors header — architecture-specific tensor names for checkpoints, LoRA key prefixes for LoRAs — then the file name), so nothing is written to the user's database just by looking. Persisted only (a) at download/import time, (b) when the user picks "What base is this?", or (c) when the user presses "Save detected families" after a preview in the Packages view. Stored in `models.family` plus a new `family_source` column (`civitai` / `header` / `name` / `user`; migration 0022) so a weak guess shows as such. A `user` value is never overwritten. |
| Downloads remember what they are | A Civitai/HF download records the family (from `baseModel`/`base_model`), the source (`civitai:<model>/<version>`), and for a LoRA the base family it targets. |
| Resolver | `resolve_package(item) → Package { item, needs: [Need { role: base \| vae \| text_encoder \| …, status }], missing_bytes }` where status is `Installed(model)`, `Catalog(stack member, size, sha256)`, `Findable(top Civitai checkpoints with that baseModel)`, or `NotRunnable(reason)`. Pure function over the library + catalog + registry; the Civitai lookup for `Findable` is the only network call and sits behind the offline gate. |
| "Made for" vs "works with" | An SDXL-architecture LoRA made for Pony works on any SDXL checkpoint but looks best on Pony. The package says so: "✓ works with your SDXL Base 1.0 — made for Pony (6.9 GB, get it?)". No false "missing". |
| Discover (result card) | "Set import type" is removed from Discover. In its place: **Get** → one dialog showing the package: each need with ✓/↓/?/✗, size, fit badge, licence; buttons **"Download LoRA only"** and **"Download LoRA + missing (N GB)"**. For `Findable`, the dialog lists the top 3 checkpoints for that base (by downloads) to pick from. A `NotRunnable` base shows on the card itself **before** any download: "Made for Wan 2.2 14B — this app runs the 5B; this LoRA will not load." |
| Library "Packages" view (reverse direction) | New Models section: the library grouped by base family — base ✓/✗, companions ✓/✗, and the LoRAs that target it. Incomplete groups get "Download missing (N GB)"; e.g. "4 LoRAs for Pony, no Pony checkpoint". LoRAs with an unknown or weakly-guessed family get a "What base is this for?" picker. |
| Manual import | The manual file import form stays for files from elsewhere, with its own type picker — it is no longer the target of any Discover button. |
| Downloads | Everything goes through the existing download manager (queue, resume, sha256 verify, API key per host). A package download is several queued items; the Downloads page groups them under the package. |

## Proof

Unit: label → family mapping for every label in the 2026-09-21 sample; header-based family detection
from synthetic safetensors headers per architecture; resolver cases (all installed; base in catalog;
base only findable; not runnable; "made for" vs "works with"; unknown family). UI live-verified in
the dev preview. **Real run** (look first, download only with the user's OK): resolve one LoRA per
family the user has (SDXL, FLUX.2 klein), one Pony LoRA, one Wan 14B LoRA; show the packages and the
library grouping on the real library; record timings.

## Decided by the user (2026-09-21)

1. **SD 1.5 — supported.** A curated SD 1.5 checkpoint in the catalog (sha256 from a file actually
   downloaded and hashed, never guessed) and a check that the image pipeline really runs it (512 px
   defaults); if it does not, that is reported honestly instead of shipping a stack that cannot work.
2. **Pony / Illustrious / NoobAI — suggest their own checkpoint.** "Works with your SDXL — made for
   Pony (6.9 GB), get it?", the user decides per LoRA. These are community checkpoints, so they come
   through the `Findable` path (Civitai's reported sha256, re-verified by the download manager), not
   as pinned catalog entries.
3. **Unknown bases — "not runnable here"** until a stack is added for them.

Approved with "go" on 2026-09-21. Real-run rule: resolve and display only; nothing is downloaded
into the user's library without an explicit OK at that point.

## Not in this plan

Automatic "best checkpoint" choice without asking; training bases (the Training tab's own
profiles stay as they are); converting LoRAs between architectures.
