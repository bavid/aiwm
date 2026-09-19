# Vision captioners release their VRAM — Design (Plan 9)

Status: written 2026-09-19 after the Plan 8 real runs measured that Florence-2 (~2.2 GB) and
Qwen2.5-VL 4-bit (~7.9 GB) stay resident in the Python sidecar after a dataset prep until the
daemon stops (VRAM ~4.0 GB after a Florence-only prep, ~11.8 GB after escalation, baseline
~1.8 GB). That VRAM is then missing for training and image generation.

## What exists

- **Sidecar** (`sidecar/src/aiwm_sidecar/vision.py`): three module-level engine caches —
  `_florence2_cache` (by model dir), `_qwen_vl_cache` (by model dir + quantization) and
  `_wd_tagger_cache` (by model dir; ONNX on the CPU). Nothing ever empties them.
- **Core** (`core/src/runtime/vision.rs`): `VisionAdapter::unload_model` only drops the Rust-side
  `loaded_models` entry. The sidecar keeps the weights (known gap in `docs/TODO.md`).
- **Engine** (`core/src/orchestrator/engine.rs`): a `dataset_prep` job reserves the synthetic
  model `dataset-vision-pipeline` on runtime `vision`. Like every model, it stays resident after
  the job (by design for llama.cpp/ComfyUI, where reuse is the point); it only goes away when the
  scheduler evicts it (`JobEngine::evict` → `runtime_with_model` → `unload_model`).
- **Concurrency**: the daemon's job loop (`api::run_job_loop`) drives exactly one job at a time
  (`run_next` → `drive`), so two dataset preps — or a prep and any other job — never run
  concurrently. No reference counting is needed; releasing at job end cannot pull the captioner
  out from under another running job.

## Decisions

| Question | Decision |
|---|---|
| Sidecar RPC | New method `unload_vision_models`, snake_case like `caption_frame` / `tag_frame`, advertised in `CAPABILITIES`. Optional params: `kind` (`florence2`, `qwen_vl`, `wd_tagger`) and `model_dir` narrow what is dropped; none means everything. |
| What it does | Removes matching engines from the caches, drops the references, runs `gc.collect()`, then `torch.cuda.empty_cache()` **only if torch is already imported** (`sys.modules`) and reports CUDA available — never imports torch just to unload. Returns `{"released": [{"kind", "model_dir"}...], "cuda_cache_cleared": bool}`. Safe when nothing is loaded. |
| `VisionAdapter::unload_model` | Clears the bookkeeping **first**, then sends the RPC if the sidecar is already running (never spawns one just to unload). A sidecar error is logged as a warning and not returned: bookkeeping must never claim a model the core asked to drop, and an eviction must not fail the job that needs the room. |
| When a prep releases | `JobEngine::drive` releases after every `dataset_prep` job that reached a resting state other than `Blocked` — completed, failed or cancelled — through the registry's `vision` runtime (`unload_model("dataset-vision-pipeline")`). Called unconditionally, not only when bookkeeping says resident, so a failed load still clears anything the sidecar may hold. A `Blocked` job never loaded anything, and re-checks every tick — no RPC for it. |
| Eviction | Unchanged path; it now reaches the sidecar because `unload_model` itself sends the RPC. |
| Trade-off | The next prep reloads Florence-2 (~7 s slower measured: 38 s vs 31 s captioning for 40 frames). Accepted: VRAM for training/generation matters more than a warm captioner. |

## Tests

- **Sidecar** (fake doubles, no real weights): caches emptied; `kind`/`model_dir` filters; safe
  on empty caches; `empty_cache` called when a fake CUDA reports available, not when it doesn't;
  no error when torch is absent from `sys.modules`; dispatched over JSON-RPC and advertised.
- **Core adapter**: a fake sidecar (a tiny Python JSON-RPC script) proves `unload_model` sends
  `unload_vision_models`, and that a sidecar error still clears the bookkeeping.
- **Core engine**: a fake `vision` runtime in the registry proves the release is requested when a
  prep completes, fails or is cancelled, and not while it is merely blocked.
- **Real run** on `D:\Data\Test` (read-only source) with Florence-2 + escalation every 5th frame,
  `nvidia-smi` sampled every second: target is back to baseline ±200 MiB 10 s after the job.
