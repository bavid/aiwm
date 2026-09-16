import { submitJob } from "../../lib/ipc";

/** Same defaults `ImageStudio` starts a fresh generation with (1024², 25
 *  steps, cfg 7) -- Story Studio reuses the Image capability exactly as-is,
 *  it does not get its own tuned defaults. */
const DEFAULT_IMAGE_PARAMS = {
  negative: "",
  width: 1024,
  height: 1024,
  steps: 25,
  cfg: 7,
};

/** Submits a `job_type=image` job for `prompt` -- the same capability the
 *  Image tab uses, model selection left at Auto. When `referenceJobId` is
 *  given (a prior portrait/reference job for the same character or
 *  location), the render is anchored to it instead of generated
 *  independently (Story Studio Phase 2 character consistency: SDXL gets
 *  IP-Adapter conditioning, FLUX.2 [klein] gets a reference-latent-anchored
 *  generation -- both server-side, nothing else changes here). Returns the
 *  job id the caller should store (`setCharacterPortrait`,
 *  `setLocationReference`, `addSceneImage`) so the UI can show it once it
 *  finishes. */
export async function generateImage(prompt: string, referenceJobId?: string): Promise<string> {
  const job = await submitJob({
    job_type: "image",
    params: {
      prompt,
      ...DEFAULT_IMAGE_PARAMS,
      ...(referenceJobId ? { reference_image: referenceJobId } : {}),
    },
  });
  return job.id;
}
