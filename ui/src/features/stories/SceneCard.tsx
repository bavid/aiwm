import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import {
  addSceneImage,
  deleteScene,
  deleteSceneImage,
  setCanonicalSceneImage,
  type Character,
  type SceneDetail,
  type Story,
  type StoryLocation,
} from "../../lib/ipc";
import { generateImage } from "./generateImage";
import { JobImage } from "./JobImage";
import { sceneImagePrompt } from "./storyPrompts";

/** One Scene card in the Timeline: the canonical image with dialogue
 *  rendered as UI overlay text (never baked into the image -- a hard
 *  requirement), alternates to pick from, and the scene's narrative. */
export function SceneCard({
  story,
  scene,
  index,
  characters,
  locations,
  onSelectCharacter,
  onEdit,
  onChanged,
}: {
  story: Story;
  scene: SceneDetail;
  index: number;
  characters: Character[];
  locations: StoryLocation[];
  onSelectCharacter: (id: string) => void;
  onEdit: () => void;
  onChanged: () => void;
}) {
  const [generating, setGenerating] = useState(false);
  const generateId = useId();

  const nameOf = (id: string) => characters.find((c) => c.id === id)?.name ?? "(unknown)";
  const locationName = locations.find((l) => l.id === scene.location_id)?.name ?? null;
  const canonical = scene.images.find((img) => img.is_canonical) ?? scene.images[0] ?? null;
  const alternates = scene.images.filter((img) => img.id !== canonical?.id);
  // Anchor to a participant's portrait for character consistency (Story
  // Studio Phase 2) -- only when there's exactly one participant with a
  // portrait set. A multi-character scene has no single reference image the
  // IP-Adapter / reference-latent graphs could anchor to (that would need one
  // conditioning pass per character, a Phase-3+ extension), so those scenes
  // keep Phase 1's independent-generation behavior rather than silently
  // picking one participant to anchor to and not the others.
  const soloParticipant =
    scene.participant_ids.length === 1
      ? characters.find((c) => c.id === scene.participant_ids[0])
      : undefined;
  const anchorJobId = soloParticipant?.portrait_job_id ?? undefined;

  const generate = async () => {
    setGenerating(true);
    try {
      const participantNames = scene.participant_ids.map(nameOf);
      const jobId = await generateImage(
        sceneImagePrompt(story, scene, participantNames, locationName),
        anchorJobId,
      );
      await addSceneImage(scene.id, jobId);
      onChanged();
    } finally {
      setGenerating(false);
    }
  };

  const remove = async () => {
    if (!window.confirm("Delete this scene? This cannot be undone.")) return;
    await deleteScene(scene.id);
    onChanged();
  };

  return (
    <article className="scene-card">
      <header className="scene-card__head">
        <span className="scene-card__index numeric">#{index + 1}</span>
        {scene.redline && <span className="scene-card__redline">{scene.redline}</span>}
        {locationName && <span className="badge badge--soft">{locationName}</span>}
        <div className="scene-card__head-actions">
          <button type="button" onClick={onEdit}>
            Edit
          </button>
          <button type="button" className="tile__delete" onClick={remove}>
            Delete
          </button>
        </div>
      </header>

      <div className="scene-card__stage">
        <JobImage jobId={canonical?.job_id ?? null} alt="Scene" className="scene-card__image" />
        {scene.dialogue.length > 0 && (
          <div className="scene-card__overlay">
            {scene.dialogue.map((line) => (
              <p key={line.id}>
                <strong>{nameOf(line.character_id)}:</strong> {line.text}
              </p>
            ))}
          </div>
        )}
      </div>

      <div className="scene-card__image-actions">
        <button id={generateId} type="button" onClick={generate} disabled={generating}>
          {generating ? "Generating…" : canonical ? "Generate alternate" : "Generate image"}
          {anchorJobId && !generating && " (anchored)"}
          {anchorJobId && !generating && (
            <span className="visually-hidden"> — to {soloParticipant?.name}'s portrait</span>
          )}
        </button>
        {anchorJobId ? (
          <HelpHint area="stories" setting="consistency-anchor" describes={generateId} />
        ) : (
          <HelpHint area="stories" setting="scene-images" describes={generateId} />
        )}
        {alternates.length > 0 && (
          <ul className="scene-card__alternates">
            {alternates.map((img) => (
              <li key={img.id}>
                <button type="button" onClick={() => setCanonicalSceneImage(img.id).then(onChanged)}>
                  <JobImage jobId={img.job_id} alt="Alternate" className="scene-card__thumb" />
                </button>
                <button
                  type="button"
                  className="scene-card__thumb-delete"
                  onClick={() => deleteSceneImage(img.id).then(onChanged)}
                  aria-label="Delete this alternate"
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      {scene.narrative && <p className="scene-card__narrative">{scene.narrative}</p>}

      {scene.participant_ids.length > 0 && (
        <div className="scene-card__participants">
          {scene.participant_ids.map((id) => (
            <button key={id} type="button" className="chip chip--link" onClick={() => onSelectCharacter(id)}>
              {nameOf(id)}
            </button>
          ))}
        </div>
      )}
    </article>
  );
}
