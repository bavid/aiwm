# Pipeline golden graph fixtures

These 14 JSON files are the recorded output of every recipe function in
`core/src/pipeline/mod.rs`, rendered from the fixed inputs in
`core/tests/pipeline_goldens.rs` and normalised (object keys recursively
sorted; array order — link pairs like `["4", 0]` — preserved).

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
