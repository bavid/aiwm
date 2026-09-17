import type { RegistryDetails, RegistryFile } from "../../lib/ipc";

/** The files worth offering for download out of one resolved repo. A Hugging
 *  Face repo lists its README, configs and tokenizer next to the weights;
 *  only a file with a quant label or a VRAM estimate is a weight file. (A
 *  Civitai model lists weights only, so this is a no-op there.)
 *
 *  One definition, used by Discover's result/recommendation rows and the
 *  Models tab's Featured catalogue — they used to carry a copy each. */
export const weightFiles = (details: RegistryDetails): RegistryFile[] =>
  details.files.filter((f) => f.quant || f.vram_estimate_mb != null);
