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

/** Submits a plain `job_type=image` job for `prompt` -- the same capability
 *  the Image tab uses, model selection left at Auto. Returns the job id the
 *  caller should store (`setCharacterPortrait`, `setLocationReference`,
 *  `addSceneImage`) so the UI can show it once it finishes. */
export async function generateImage(prompt: string): Promise<string> {
  const job = await submitJob({
    job_type: "image",
    params: { prompt, ...DEFAULT_IMAGE_PARAMS },
  });
  return job.id;
}
