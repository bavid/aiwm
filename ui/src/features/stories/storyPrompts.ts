import type { Character, SceneDetail, Story, StoryLocation } from "../../lib/ipc";

/** Assembles image-generation prompts for Story Studio (Phase 1: text +
 *  plain image). Every prompt leads with the Story's own art style so
 *  everything generated for one story shares a look, then layers on
 *  whatever the specific Character/Location/Scene knows about itself.
 *  Pure functions -- no IPC here, the caller submits the job. */

function join(parts: (string | undefined | null)[]): string {
  return parts
    .map((p) => p?.trim())
    .filter((p): p is string => Boolean(p))
    .join(", ");
}

export function characterPortraitPrompt(story: Story, character: Character): string {
  return join([
    story.art_style,
    `portrait of ${character.name}`,
    character.alignment,
    character.traits,
  ]);
}

export function locationReferencePrompt(story: Story, location: StoryLocation): string {
  return join([story.art_style, location.name, location.description]);
}

export function sceneImagePrompt(
  story: Story,
  scene: Pick<SceneDetail, "narrative" | "redline">,
  participantNames: string[],
  locationName: string | null,
): string {
  return join([
    story.art_style,
    locationName ?? undefined,
    participantNames.length > 0 ? `featuring ${participantNames.join(" and ")}` : undefined,
    scene.narrative || scene.redline,
  ]);
}
