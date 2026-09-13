/** Hugging Face tags carry a lot of infra/library noise (`pytorch`,
 *  `text-generation-inference`, namespaced ones like `license:apache-2.0` or
 *  `base_model:...`) alongside the genuinely useful content descriptors
 *  (`nsfw`, `not-for-all-audiences`, `uncensored`, `roleplay`, ...). Drop the
 *  noise so what's left is worth showing the user while they browse. */
const NOISE = new Set([
  "pytorch",
  "tensorflow",
  "jax",
  "safetensors",
  "gguf",
  "transformers",
  "transformers.js",
  "text-generation-inference",
  "endpoints_compatible",
  "autotrain_compatible",
  "diffusers",
  "peft",
  "conversational",
  "custom_code",
  "arxiv",
  "region:us",
]);

export function filterDisplayTags(tags: string[]): string[] {
  const seen = new Set<string>();
  const kept: string[] = [];
  for (const t of tags) {
    const tag = t.trim();
    if (!tag || tag.includes(":") || NOISE.has(tag.toLowerCase())) continue;
    if (seen.has(tag)) continue;
    seen.add(tag);
    kept.push(tag);
  }
  return kept.sort((a, b) => a.localeCompare(b));
}
