import type { HelpSetting, HelpTopic } from "./types.ts";

/** Story Studio: stories, characters and sheets, world (locations, NPCs),
 *  the scene timeline, images and the Phase 2 consistency anchor. Facts
 *  from `features/stories/*` (`storyPrompts.ts`, `generateImage.ts`,
 *  `SceneCard.tsx`, `CharacterSheet.tsx`, `WorldSection.tsx`) and the Story
 *  Studio sections in `docs/TODO.md` (2026-09-16). */

const SETTINGS: readonly HelpSetting[] = [
  {
    key: "story",
    label: "Story",
    what: "The top-level container: a name, a setting/era, an art style and a premise. Everything else — characters, locations, NPCs, scenes — belongs to exactly one story.",
    why: "The art style is the first thing in every image prompt this story generates, so all its pictures share a look.",
    effect: "+ creates one, ✎ edits the four fields, × deletes the story with every character, location and scene in it (not undoable). The active story is remembered between visits.",
    benefit: "One place to set the look and the world; the rest inherits it.",
  },
  {
    key: "character",
    label: "Character",
    what: "A member of the cast: name, alignment/role hint, traits, backstory, an inventory, relationships to other characters, a portrait, and an append-only log of the scenes they appeared in.",
    why: "The sheet is what the images and the scenes draw on — the portrait prompt is the art style + \"portrait of <name>\" + alignment + traits.",
    effect: "Click a row to open the sheet as a drawer; ✎ opens it straight in edit mode. Generate portrait submits an image job with the Image tab's defaults (1024×1024, 25 steps, CFG 7, model Auto) and stores the job on the character. Adding a character to a scene appends a line to their log.",
    benefit: "Every image of the character starts from the same description.",
    pitfalls: "NPCs are deliberately lighter (name, role, location, one line) and live under World, not here.",
  },
  {
    key: "consistency-anchor",
    label: "Consistency-anchored",
    what: "Once a character has a portrait (or a location a reference image), regenerating it — or generating a scene in which that character is the only participant — is anchored to that image instead of generated independently.",
    why: "Phase 1 generated every picture from scratch, so the same character looked different in every image. The anchor keeps the look.",
    effect: "The image job carries the reference job's id. For the SDXL family the core builds an IP-Adapter graph (CLIP vision + IP-Adapter, both from the Models catalogue); for FLUX.2 [klein] it uses the model's own reference-latent conditioning — both server-side, nothing to set in the UI. The first-ever portrait has nothing to anchor to and generates independently.",
    benefit: "The same face across portraits and scenes.",
    pitfalls: "A scene with two or more participants is not anchored (one reference per graph); it generates independently, as in Phase 1. SDXL anchoring needs the IP-Adapter and CLIP-vision files imported. Hi-res fix is never applied to anchored renders.",
  },
  {
    key: "location-reference",
    label: "Generate reference (location)",
    what: "An image of a location, prompted with the art style, the location's name and its description; stored as the location's reference.",
    why: "A location with a reference image stays recognisable in every scene set there — and a regeneration is anchored to it.",
    effect: "Submits an image job; \"Regenerate (anchored)\" afterwards keeps the look.",
    benefit: "Places that look the same from scene to scene.",
  },
  {
    key: "scene",
    label: "Scene",
    what: "One beat of the story: a redline (one-line summary), the narrative, a location, participants from the cast, and dialogue lines attributed to participants.",
    why: "The scene is what gets illustrated: the image prompt is the art style + location + \"featuring <participants>\" + the narrative (or the redline when there is none).",
    effect: "Saving a scene replaces all of it at once. Removing a participant drops their dialogue lines with them. New scenes go to the end of the timeline.",
    benefit: "Text first; images follow from it.",
  },
  {
    key: "dialogue",
    label: "Dialogue",
    what: "Speaker + line pairs on a scene, shown as an overlay on the scene's image.",
    why: "Text rendered into an image is unreadable and uneditable; keeping it as UI text is a hard requirement of the studio.",
    effect: "Lines are never part of the image prompt; they are drawn over the canonical image on the card. Only participants can be speakers.",
    benefit: "Edit a line without regenerating the picture.",
  },
  {
    key: "scene-images",
    label: "Scene images",
    what: "Several generated images per scene, one of them canonical (shown large); the others are alternates you can promote or delete.",
    why: "The first render is rarely the one; keeping alternates lets you pick.",
    effect: "Generate image / Generate alternate submits an image job and adds the result to the scene; clicking an alternate makes it canonical (the database enforces exactly one). Deleting an alternate removes it from the scene and deletes the job.",
    benefit: "Roll a few, keep the best.",
  },
  {
    key: "assembly",
    label: "Assembly",
    what: "A placeholder: bundling scenes and images into a shareable export (HTML scroll, PDF, CBZ) is Phase 3 and not built.",
    why: "It is listed so the plan is visible; the Timeline is the source of truth meanwhile.",
    effect: "Nothing yet.",
    benefit: "Nothing yet — plan for the export, but do not wait for it.",
  },
];

export const STORIES_TOPICS: readonly HelpTopic[] = [
  {
    id: "purpose",
    area: "stories",
    title: "What Story Studio is for",
    summary: "A character- and scene-based story builder: cast, world, a scene timeline, and images that keep the same character recognisable from picture to picture.",
    body: [
      {
        kind: "p",
        text: "Story → characters (with a locked portrait), locations (with a locked reference image), NPCs → scenes (narrative, dialogue, participants, location, redline) → scene images (several, one canonical). Phase 1 (2026-09-16) built the text-and-image studio; Phase 2 added the consistency anchor. Assembly and export are Phase 3 and not built.",
      },
    ],
  },
  {
    id: "flow",
    area: "stories",
    title: "Step by step",
    summary: "Create a story with an art style, add the cast and their portraits, add locations, write scenes, generate and pick images.",
    body: [
      {
        kind: "steps",
        items: [
          "Create a story: name, setting/era, art style, premise. The art style leads every prompt.",
          "Characters: add each one with an alignment hint and traits; open the sheet and Generate portrait. Regenerate until you like it — from the second time on it is anchored to the first.",
          "World: add locations and generate a reference image for each; add NPCs.",
          "Timeline: New scene — redline, narrative, location, participants, dialogue. Save.",
          "On the scene card: Generate image. With exactly one participant who has a portrait, it is anchored to that portrait. Generate alternates; click the best one to make it canonical.",
          "Voice tab: narrate a line for the scene if you like.",
        ],
      },
    ],
    settings: SETTINGS,
  },
  {
    id: "disk-gpu",
    area: "stories",
    title: "What happens on disk and on the GPU",
    summary: "Rows in the database, images through the ordinary image pipeline, extra model files for SDXL anchoring.",
    body: [
      {
        kind: "list",
        items: [
          "Every story object is a database row; deleting a story cascades to everything in it.",
          "Every image is an ordinary image job (model Auto, 1024×1024, 25 steps, CFG 7) and a PNG in the generated-media folder; the story keeps the job id.",
          "Anchored SDXL renders load CLIP vision and the IP-Adapter alongside the checkpoint (both are in the Models catalogue, about 848 MB for the SDXL IP-Adapter); FLUX.2 [klein] anchoring needs nothing extra.",
          "GPU: the same as the Image tab — one render at a time, blocked when VRAM is short.",
        ],
      },
    ],
  },
  {
    id: "limits",
    area: "stories",
    title: "Limits and pitfalls",
    summary: "One anchor per image, no export yet, no LoRA per character.",
    body: [
      {
        kind: "list",
        items: [
          "Multi-character scenes are not anchored; they generate independently.",
          "Anchoring applies to the SDXL family (IP-Adapter) and FLUX.2 [klein] (reference latent); plain FLUX.1 has no anchor path.",
          "No Hi-res fix and no LoRA stack from here — use the Image tab for those.",
          "Assembly / export does not exist yet.",
          "Deleting a story, character or scene is not undoable.",
        ],
      },
    ],
  },
];
