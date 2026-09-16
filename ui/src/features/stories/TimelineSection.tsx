import { useState } from "react";
import type { Character, SceneDetail, Story, StoryLocation } from "../../lib/ipc";
import { SceneCard } from "./SceneCard";
import { SceneEditor } from "./SceneEditor";

/** Timeline: the main view -- a vertical scroll of Scene cards in order.
 *  Also hosts the create/edit Scene form (swapped in for the whole section
 *  rather than a modal, since nothing else needs to stay visible while
 *  editing one). */
export function TimelineSection({
  story,
  characters,
  locations,
  scenes,
  onSelectCharacter,
  onChanged,
}: {
  story: Story;
  characters: Character[];
  locations: StoryLocation[];
  scenes: SceneDetail[];
  onSelectCharacter: (id: string) => void;
  onChanged: () => void;
}) {
  const [editingId, setEditingId] = useState<string | "new" | null>(null);

  const stopEditing = () => setEditingId(null);
  const saved = () => {
    stopEditing();
    onChanged();
  };

  if (editingId === "new") {
    return (
      <div className="card">
        <header className="card__head">
          <h2>New scene</h2>
        </header>
        <SceneEditor storyId={story.id} characters={characters} locations={locations} onSaved={saved} onCancel={stopEditing} />
      </div>
    );
  }

  const editingScene = scenes.find((s) => s.id === editingId);
  if (editingScene) {
    return (
      <div className="card">
        <header className="card__head">
          <h2>Edit scene</h2>
        </header>
        <SceneEditor
          storyId={story.id}
          characters={characters}
          locations={locations}
          initial={editingScene}
          onSaved={saved}
          onCancel={stopEditing}
        />
      </div>
    );
  }

  return (
    <div className="card">
      <header className="card__head">
        <h2>Timeline</h2>
        <span className="card__sub numeric">{scenes.length}</span>
      </header>

      {scenes.length === 0 && <p className="muted">No scenes yet -- start the story below.</p>}

      <div className="timeline">
        {scenes.map((scene, index) => (
          <SceneCard
            key={scene.id}
            story={story}
            scene={scene}
            index={index}
            characters={characters}
            locations={locations}
            onSelectCharacter={onSelectCharacter}
            onEdit={() => setEditingId(scene.id)}
            onChanged={onChanged}
          />
        ))}
      </div>

      <button type="button" className="section-card__add" onClick={() => setEditingId("new")}>
        + New scene
      </button>
    </div>
  );
}
