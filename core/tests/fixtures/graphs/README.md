# Pipeline golden graph fixtures

These JSON files are the recorded output of every recipe function in
`core/src/pipeline/recipes/` (`image.rs`, `story.rs`, `video.rs`,
`upscale.rs` — the recipes moved out of `core/src/pipeline/mod.rs` during the
refactor these fixtures guard), rendered from the fixed inputs in
`core/tests/pipeline_goldens.rs` and normalised (object keys recursively
sorted; array order — link pairs like `["4", 0]` — preserved).

Most recipes have one fixture, rendered with a LoRA chain. Two also have a
`*_no_loras` variant (`checkpoint_txt2img`, `flux2_klein_edit`): an absent
chain is a genuinely different graph from a present one, and the with-LoRAs
fixtures alone never pinned that shape. Those two were added in `e1b7767`,
i.e. **generated from the new code, not recorded from a pre-refactor
baseline** — the refactor had already landed by then, so there was no earlier
output to record. What backs them is the with-LoRAs fixture of the same
recipe, which *is* a pre-refactor recording: the no-LoRA graph is that graph
minus the chain, so a wrong shape here would have to be wrong in the recorded
one too.

Two more — `checkpoint_txt2img_hires` and `flux2_klein_txt2img_hires` — pin
the Hi-Res-Fix second pass, one per sampler family (`KSampler` and
`SamplerCustomAdvanced`). Unlike the rest of this directory, these two were
**generated from the code that introduced the feature**, not recorded from a
pre-refactor baseline: they are new behaviour, so there is no earlier output
to compare against. What proves the feature changed nothing else is the other
fixtures, which are byte-identical before and after it (every recipe defaults
to `hires: None`).

They are the safety net for the ComfyUI workflow engine refactor (Plan 3:
`docs/superpowers/plans/2026-09-17-comfyui-workflow-engine-plan-3.md`), which
tears the current hand-built graphs apart into a small graph builder plus
composable, individually tested fragments. The refactor is only allowed to
land once every fixture here still matches byte-for-byte.

## Regenerating

```
AIWM_WRITE_GOLDENS=1 cargo test -p aiwm-core --test pipeline_goldens
```

This overwrites every fixture with whatever the current pipeline code
produces for the fixed inputs. Without the env var, the same test binary
instead reads each fixture and compares.

## If a diff shows up here

A diff in one of these files after a plain refactor (no fixed-input change in
`pipeline_goldens.rs`) means the refactor changed the actual ComfyUI graph a
recipe emits — not necessarily wrong, but never incidental. Treat every
mismatch as a deliberate behaviour change:

1. Confirm the new graph shape is intentional (re-read the recipe's doc
   comment and the relevant ComfyUI node source if the change touches node
   wiring, not just a cosmetic key).
2. Regenerate with `AIWM_WRITE_GOLDENS=1` as above.
3. Commit the updated fixture(s) in the **same commit** as the code change
   that caused them to move, with a message explaining why.

Never regenerate a fixture just to make a failing test pass without first
understanding why the graph changed.
