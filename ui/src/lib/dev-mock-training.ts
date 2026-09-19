/** Dev-mock slice for the "Training & captioning" catalog: the three
 *  captioner stacks (`core::model::catalog`, `media: "training"`), download
 *  completion turning into library rows, and the captioner registry computed
 *  from those rows with the core's rule — installed only once **every**
 *  required file sits in one directory. Kept apart from `dev-mock.ts` (already
 *  far past a readable size). Sizes, file names, kinds and licences match the
 *  real catalog; the SHA-256s are fake but unique. */

type AnyRecord = Record<string, unknown>;

const FLORENCE_REV = "21a599d414c4d928c9032694c424fb94458e3594";
const QWEN_REV = "cc594898137f460bfe9f0759e9844b3ce807cfb5";

interface KindInfo {
  role: string;
  dir: string;
  repo: string;
  publisher: string;
  license: string;
  urlBase: string;
}

const KINDS: Record<string, KindInfo> = {
  wd_tagger: {
    role: "vision_wd_tagger",
    dir: "vision\\wd-tagger",
    repo: "SmilingWolf/wd-eva02-large-tagger-v3",
    publisher: "SmilingWolf",
    license: "Apache-2.0",
    urlBase: "https://huggingface.co/SmilingWolf/wd-eva02-large-tagger-v3/resolve/main",
  },
  florence2_engine: {
    role: "vision_florence2",
    dir: "vision\\florence2-large",
    repo: "microsoft/Florence-2-large",
    publisher: "Microsoft",
    license: "MIT",
    urlBase: `https://huggingface.co/microsoft/Florence-2-large/resolve/${FLORENCE_REV}`,
  },
  qwen_vl_engine: {
    role: "vision_qwen2_5_vl",
    dir: "vision\\qwen2.5-vl-7b",
    repo: "Qwen/Qwen2.5-VL-7B-Instruct",
    publisher: "Qwen",
    license: "Apache-2.0",
    urlBase: `https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct/resolve/${QWEN_REV}`,
  },
};

/** [id, kind, file, size_bytes, display name] — straight from the catalog. */
const MEMBERS: [string, string, string, number, string][] = [
  ["wd-eva02-large-tagger-v3-model", "wd_tagger", "model.onnx", 1_260_435_999, "WD EVA02-Large Tagger v3 — model"],
  ["wd-eva02-large-tagger-v3-tags", "wd_tagger", "selected_tags.csv", 308_468, "WD EVA02-Large Tagger v3 — tag list"],
  ["florence2-large-model", "florence2_engine", "model.safetensors", 1_553_563_458, "Florence-2 large — weights"],
  ["florence2-large-config", "florence2_engine", "config.json", 2_445, "Florence-2 large — model config"],
  ["florence2-large-configuration-florence2", "florence2_engine", "configuration_florence2.py", 15_119, "Florence-2 large — remote code (config)"],
  ["florence2-large-modeling-florence2", "florence2_engine", "modeling_florence2.py", 127_455, "Florence-2 large — remote code (model)"],
  ["florence2-large-processing-florence2", "florence2_engine", "processing_florence2.py", 48_674, "Florence-2 large — remote code (processor)"],
  ["florence2-large-preprocessor-config", "florence2_engine", "preprocessor_config.json", 806, "Florence-2 large — preprocessor config"],
  ["florence2-large-generation-config", "florence2_engine", "generation_config.json", 51, "Florence-2 large — generation defaults"],
  ["florence2-large-tokenizer", "florence2_engine", "tokenizer.json", 1_355_863, "Florence-2 large — tokenizer"],
  ["florence2-large-tokenizer-config", "florence2_engine", "tokenizer_config.json", 34, "Florence-2 large — tokenizer config"],
  ["florence2-large-vocab", "florence2_engine", "vocab.json", 1_099_884, "Florence-2 large — vocabulary"],
  ["qwen2.5-vl-7b-model-00001-of-00005", "qwen_vl_engine", "model-00001-of-00005.safetensors", 3_900_233_256, "Qwen2.5-VL 7B Instruct — weights (shard 1 of 5)"],
  ["qwen2.5-vl-7b-model-00002-of-00005", "qwen_vl_engine", "model-00002-of-00005.safetensors", 3_864_726_320, "Qwen2.5-VL 7B Instruct — weights (shard 2 of 5)"],
  ["qwen2.5-vl-7b-model-00003-of-00005", "qwen_vl_engine", "model-00003-of-00005.safetensors", 3_864_726_424, "Qwen2.5-VL 7B Instruct — weights (shard 3 of 5)"],
  ["qwen2.5-vl-7b-model-00004-of-00005", "qwen_vl_engine", "model-00004-of-00005.safetensors", 3_864_733_680, "Qwen2.5-VL 7B Instruct — weights (shard 4 of 5)"],
  ["qwen2.5-vl-7b-model-00005-of-00005", "qwen_vl_engine", "model-00005-of-00005.safetensors", 1_089_994_880, "Qwen2.5-VL 7B Instruct — weights (shard 5 of 5)"],
  ["qwen2.5-vl-7b-model-index", "qwen_vl_engine", "model.safetensors.index.json", 57_619, "Qwen2.5-VL 7B Instruct — weights shard index"],
  ["qwen2.5-vl-7b-config", "qwen_vl_engine", "config.json", 1_374, "Qwen2.5-VL 7B Instruct — model config"],
  ["qwen2.5-vl-7b-generation-config", "qwen_vl_engine", "generation_config.json", 216, "Qwen2.5-VL 7B Instruct — generation defaults"],
  ["qwen2.5-vl-7b-preprocessor-config", "qwen_vl_engine", "preprocessor_config.json", 350, "Qwen2.5-VL 7B Instruct — preprocessor config"],
  ["qwen2.5-vl-7b-chat-template", "qwen_vl_engine", "chat_template.json", 1_050, "Qwen2.5-VL 7B Instruct — chat template"],
  ["qwen2.5-vl-7b-tokenizer", "qwen_vl_engine", "tokenizer.json", 7_031_645, "Qwen2.5-VL 7B Instruct — tokenizer"],
  ["qwen2.5-vl-7b-tokenizer-config", "qwen_vl_engine", "tokenizer_config.json", 5_702, "Qwen2.5-VL 7B Instruct — tokenizer config"],
  ["qwen2.5-vl-7b-vocab", "qwen_vl_engine", "vocab.json", 2_776_833, "Qwen2.5-VL 7B Instruct — vocabulary"],
  ["qwen2.5-vl-7b-merges", "qwen_vl_engine", "merges.txt", 1_671_839, "Qwen2.5-VL 7B Instruct — BPE merges"],
];

const fakeSha = (i: number) => (0xc0de0000 + i).toString(16).repeat(8);

export const TRAINING_KNOWN_MOCK: AnyRecord[] = MEMBERS.map(([id, kind, file, size, name], i) => {
  const k = KINDS[kind];
  return {
    id, name, kind, family: kind === "florence2_engine" ? "florence2" : null,
    publisher: k.publisher, repo: k.repo, file, url: `${k.urlBase}/${file}`,
    sha256: fakeSha(i), size_bytes: size, license: k.license,
    note: `${name} (dev mock).`, is_default: false, media: "training",
    fit: { level: "green" },
  };
});

const member = (id: string) => TRAINING_KNOWN_MOCK.find((m) => m.id === id);
const membersOf = (prefix: string) =>
  TRAINING_KNOWN_MOCK.filter((m) => String(m.id).startsWith(prefix));

export const TRAINING_STACKS_MOCK: AnyRecord[] = [
  {
    id: "wd-tagger", label: "WD EVA02-Large Tagger v3 (Danbooru tags)", media: "training",
    is_default: true, fit: { level: "green" },
    note: "The recommended dataset captioner: Danbooru-style tags for LoRA training captions. Runs on the CPU (onnxruntime), so it never competes with training for VRAM. Two files (~1.3 GB): the model and its tag list.",
    members: [member("wd-eva02-large-tagger-v3-model"), member("wd-eva02-large-tagger-v3-tags")],
  },
  {
    id: "florence2-large", label: "Florence-2 large (prose captions)", media: "training",
    is_default: false, fit: { level: "green" },
    note: "Sentence-style captions instead of tags. Ten files (~1.56 GB), pinned to one revision because three of them are Python the model runs on load. Needs ~2 GB VRAM.",
    members: membersOf("florence2-large-"),
  },
  {
    id: "qwen2.5-vl-7b", label: "Qwen2.5-VL 7B Instruct (second opinion)", media: "training",
    // `stack_fit` judges a captioner stack by its run-time VRAM (4-bit ~6 GiB),
    // not by its 16.6 GB of files.
    is_default: false, fit: { level: "green" },
    note: "Optional and large: re-captions a frame together with a later one when a Florence-2 caption looks unsure, describing what changes between them. Fourteen files (~16.6 GB download); loaded 4-bit it needs ~6 GB VRAM.",
    members: membersOf("qwen2.5-vl-7b-"),
  },
];

/** The library row a finished download of a training file imports as
 *  (kind's store folder, original file name, kind's default role); `null`
 *  for any other download, which the base mock leaves row-less as before. */
export function trainingModelRow(download: AnyRecord): AnyRecord | null {
  const k = KINDS[String(download.model_type ?? "")];
  if (!k) return null;
  const file = String(download.filename);
  return {
    name: file, format: file.split(".").pop() ?? "", source: "download",
    file_path: `E:\\AI\\models\\${k.dir}\\${file}`, sha256: download.sha256 ?? null,
    size_bytes: Number(download.size_bytes ?? 0), roles: [k.role], runtimes: [],
  };
}

// Same order and flags as the core's `CAPTIONERS`: the tagger first (the
// Dataset form preselects the first usable one), Florence-2 flagged.
const CAPTIONERS = [
  {
    id: "wd-eva02-tagger-v3", name: "WD EVA02 Tagger v3 (Danbooru tags)", style: "tags",
    role: "vision_wd_tagger", vram_mb: 0, license: "Apache-2.0", supports_escalation: false,
    kind: "wd_tagger", known_issue: null,
  },
  {
    id: "florence2", name: "Florence-2 (prose)", style: "prose", role: "vision_florence2",
    vram_mb: 2048, license: "MIT", supports_escalation: true, kind: "florence2_engine",
    known_issue: "Does not load with the bundled transformers 5.x yet \u2014 a fix is planned.",
  },
];

/** Kinds whose pinned folder the mock pretends fails the load-time integrity
 *  check (`dev_mock_set_tampered`, mock-only) — to see the "not usable" path. */
const TAMPERED = new Set<string>();

export function setTampered(kind: string, tampered: boolean): void {
  if (tampered) TAMPERED.add(kind);
  else TAMPERED.delete(kind);
}

const tamperReason = (kind: string) =>
  `captioner folder check failed: ${KINDS[kind]?.dir ?? kind}\\config.json does not match the pinned catalog file (dev mock)`;

/** Whether every required file of `kind` sits in one library directory. */
function filesPresent(models: readonly AnyRecord[], kind: string): boolean {
  const role = KINDS[kind].role;
  const required = MEMBERS.filter((m) => m[1] === kind).map((m) => m[2]);
  const rows = models.filter((m) => (m.roles as string[]).includes(role));
  const dirs = [...new Set(rows.map((m) => dirOf(String(m.file_path))))];
  return dirs.some((dir) =>
    required.every((file) =>
      rows.some((m) => dirOf(String(m.file_path)) === dir && baseOf(String(m.file_path)) === file),
    ),
  );
}

/** `escalation_status` for the Qwen2.5-VL stack. */
export function trainingEscalation(models: readonly AnyRecord[]): AnyRecord {
  const present = filesPresent(models, "qwen_vl_engine");
  const bad = present && TAMPERED.has("qwen_vl_engine");
  return { files_present: present, usable: present && !bad, reason: bad ? tamperReason("qwen_vl_engine") : null };
}

const dirOf = (path: string) => path.slice(0, path.lastIndexOf("\\"));
const baseOf = (path: string) => path.slice(path.lastIndexOf("\\") + 1);

/** `list_captioners` with the core's complete-directory rule applied to the
 *  mock library. */
export function trainingCaptioners(models: readonly AnyRecord[]): AnyRecord[] {
  return CAPTIONERS.map(({ kind, ...c }) => {
    const required = MEMBERS.filter((m) => m[1] === kind).map((m) => m[2]);
    const rows = models.filter((m) => (m.roles as string[]).includes(c.role));
    const dirs = [...new Set(rows.map((m) => dirOf(String(m.file_path))))];
    const installed = dirs.some((dir) =>
      required.every((file) =>
        rows.some((m) => dirOf(String(m.file_path)) === dir && baseOf(String(m.file_path)) === file),
      ),
    );
    const bad = installed && kind !== "wd_tagger" && TAMPERED.has(kind);
    return {
      ...c,
      required_files: required,
      installed: installed && !bad,
      unusable: bad ? tamperReason(kind) : null,
    };
  });
}

/** The core's offline-gate refusals, byte for byte as Tauri rejects with them. */
export const OFFLINE_DOWNLOAD_REFUSAL =
  "configuration error: download: offline mode is on — cannot download";
export const OFFLINE_RESUME_REFUSAL =
  "configuration error: download: offline mode is on — cannot resume";
