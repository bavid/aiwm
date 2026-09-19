# Florence-2 on native transformers — Design (Plan 8)

Status: written 2026-09-19 after Plan 7's real run showed the pinned `microsoft/Florence-2-large`
remote code fails under the sidecar's transformers 5.17.0:
`'Florence2LanguageConfig' object has no attribute 'forced_bos_token_id'`
(`configuration_florence2.py:265`). Florence-2 is currently flagged with `known_issue` and cannot be
selected.

## Finding

transformers 5.x ships Florence-2 **natively**: `transformers/models/florence2/`
(`Florence2ForConditionalGeneration`, `Florence2Processor`), with the converted reference checkpoint
`florence-community/Florence-2-large`. The native processor supports the same task prompts
(`<DETAILED_CAPTION>`, …) and `post_process_generation`, so the sidecar's captioning logic stays the same.

## Decisions

| Question | Decision |
|---|---|
| Which weights? | `florence-community/Florence-2-large`, pinned to its current `main` commit; every file downloaded and hashed (never guessed). License read from its model card at that commit; if it is not a permissive license compatible with the original MIT release, stop and report. |
| Remote code | **None.** Load with `Florence2ForConditionalGeneration.from_pretrained(dir, local_files_only=True)` and `AutoProcessor.from_pretrained(dir, local_files_only=True)`, no `trust_remote_code`. The catalog stack contains no `.py` files. |
| What stays | The directory-shaped kind, the store subfolder, the load-time integrity check, JSON pinning and the import refusal of uncatalogued files — they now guard the converted checkpoint. The remote-code cache cleanup becomes unnecessary for Florence-2; keep it harmless or remove it if nothing else uses it (decide from the code). |
| Old files | Existing installs of the `microsoft/Florence-2-large` files in `E:\AI\models\vision\florence2-large\` will fail the integrity check against the new catalog and show "not usable → Re-download", which fetches the new files. Old files that are no longer in the catalog must be cleaned up by the re-download/repair path (otherwise the "no extra files" rule keeps it unusable). Decide: a one-time cleanup of non-catalog files in that pinned folder during a repair, refusing anything outside the folder. |
| known_issue | Removed for Florence-2 once a real captioning run succeeds. |
| Qwen escalation | After Florence-2 works: a real run with escalation ON. If 4-bit via bitsandbytes fails on Windows, record the exact error and keep escalation available only if a working mode exists (e.g. 8-bit or fp16 fits 16 GB?) — measured, not assumed. |

## Proof

Unit tests (catalog pins, no `.py` in the Florence stack, loader called without `trust_remote_code`,
repair removes stale non-catalog files inside the pinned folder only), and a real run on
`D:\Data\Test` (read-only source) with Florence-2 captions quoted, VRAM measured, then one with Qwen
escalation.
