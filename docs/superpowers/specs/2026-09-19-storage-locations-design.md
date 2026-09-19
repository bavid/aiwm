# Storage Locations — Design (Plan 10)

Status: written 2026-09-19 from two backlog wishes (`docs/TODO.md`):

- "If I select a folder and start a training, the training data path should be defined by me — I have
  1 TB on the SSD where the tool runs, my training data is 100 GB, I don't want to dump my drive. Store
  it on a different drive — optional, otherwise use your default."
- "Add in the Settings page a default path for all generated images/datasets/files etc. to store data /
  look into data."

## What exists (verified in code, 2026-09-19)

- `AppPaths` (`core/src/paths.rs`) with overrides for `outputs`, `runtimes`, `cache` from config
  `[paths]` (`PathsConfig`, `core/src/config.rs`). Applied at startup; Settings saves them and says
  "restart to apply".
- Dataset work folders: `<outputs_dir>/datasets/<prep_job_id>/{raw|previews}/…`
  (`core/src/orchestrator/engine.rs` `work_dir = outputs_dir.join("datasets")`). **No path is stored
  on the `datasets` row**; the housekeeping guard (`core/src/capability/dataset/housekeeping/guard.rs`)
  derives the work folder as `canonical(outputs/datasets)/<prep_job_id>`.
- Training runs: `<root>/training/<run_id>/…` (`core/src/app.rs` `paths.root().join("training")`), no
  override. `training_runs.work_dir` is stored, and HTTP handlers read it (`run_work_dir`), but the
  runner recomputes `data_dir/<run_id>` everywhere (runner.rs, runner/settle.rs). Disk preflight
  (`MIN_FREE_DISK_BYTES` = 20 GB) checks the runner's root, not the run's folder.
- Free space: `cleanup::volume_free(path)`. Reveal: `revealItemInDir` from the opener plugin in the
  UI. Folder picker: `@tauri-apps/plugin-dialog` `open({ directory: true })` (already used).
- Settings "Data locations" shows `outputs/runtimes/cache` as text fields; sizes only for flat
  output files (`dir_file_bytes`).

## Decisions

| Question | Decision |
|---|---|
| Central defaults | Two new `[paths]` keys: `datasets_path` (default `<outputs_dir>/datasets`, i.e. today's location) and `training_path` (default `<data root>/training`, today's location). `AppPaths::datasets_dir()` / `training_dir()` with overrides, like the existing three. Restart-to-apply, like the others. Old data stays where it is (paths are recorded per dataset/run, see below). |
| Settings page | "Data locations" lists every location the app writes to — generated media (outputs), dataset work folders, training runs, model store, runtimes, cache, downloads staging — each with its path, **size on disk (recursive)**, free space on its drive, an **Open folder** button, and (for the five configurable ones: outputs, datasets, training, runtimes, cache, plus the model store field as today) a **Choose…** folder picker next to the text field. |
| Usage route | `GET /storage/locations` → `[{ key, label, path, configurable, bytes, files, volume_free_bytes, volume_total_bytes, exists }]`. Recursive walk in `spawn_blocking`, symlinks/junctions not followed, errors counted not fatal. Tauri command + ipc + dev mock. Computed on demand when the page opens (and a "Refresh" button), never on a timer. |
| Per-dataset location | `DatasetPrepRequest.data_dir: Option<PathBuf>` — "Store frames in" on the prep form, default = the central `datasets_dir()`. Work folder = `<data_dir>/<prep_job_id>`. Validation (fail fast, clear message): absolute; not inside and not equal to the source `root`; the source `root` not inside the work folder; not inside the model store; creatable. |
| Persist the location | Migration **0019**: `ALTER TABLE datasets ADD COLUMN work_dir TEXT` (nullable). Every new dataset stores its absolute work folder (default location too). Old rows stay NULL and use today's derivation — unchanged behaviour. |
| Housekeeping guard | For a row with `work_dir` set: that folder (canonicalised) is the dataset's app-owned work folder, walkable when it exists; still refused if it overlaps any dataset's `source_root` or another dataset's work folder, or is a drive root. Rows without `work_dir` use the existing derivation. "Loose" mode (no prep job, no `work_dir`) stays limited to the default datasets root. All existing safety tests keep passing; new ones cover a custom location. |
| Per-run location | `StartRequest.data_dir: Option<PathBuf>` (+ `StartRunDto`, `StartRunBody`) — "Store run in" on the new-run form, default `training_dir()`. Run folder = `<data_dir>/<run_id>`, stored in the existing `training_runs.work_dir`. **The runner reads the stored `work_dir`** everywhere it recomputes today (single helper), so resume/settle/samples/purge all follow the chosen folder. Same validation as datasets (absolute, not inside the model store). |
| Disk preflight | Training: the 20 GB check runs on the run's chosen folder. Dataset prep: refuse to start when the chosen drive has less than **5 GB** free (named constant), message names the drive and free space. Both forms show the free space of the chosen drive before starting. |
| Generated media (outputs) | Stays one central folder (existing override). Known limit, documented: `job_output_path` refuses files outside the *current* outputs folder, so after moving it, older media are shown only if moved along. Not changed in this plan. |
| Not moved | No automatic migration of existing data to a new location. The help text says: "Existing data stays where it is; new datasets/runs go to the new folder." |

## Tests / proof

Unit/integration: config round-trip of the new keys; `AppPaths` overrides; locations route (recursive
bytes, missing folder, no junction following); prep validation (relative, inside source, source inside,
inside store); prep writes frames into the custom folder and records `work_dir`; guard deletes a custom
-location dataset's files and never a neighbour's/source's (extend the mutation-style safety tests);
runner uses the stored `work_dir` (resume + settle + purge) and preflights the chosen drive. UI: forms
send `data_dir` only when changed; Settings shows sizes/free space, pickers and reveal; live-verified in
the dev preview.

**Real run:** real `aiwm-cored`, DB backed up; dataset prep on `D:\Data\Test` (read-only source, SHA-256
checked before/after) with "Store frames in" set to a folder in the session scratchpad; verify frames
land there, usage/cleanup/delete of that dataset work on the custom folder, nothing written under
`outputs/datasets`; then delete the test dataset. `GET /storage/locations` sizes recorded.
