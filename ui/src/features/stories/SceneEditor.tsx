import { useState } from "react";
import { createScene, updateScene, type Character, type DialogueLineBody, type SceneDetail, type StoryLocation } from "../../lib/ipc";

interface DraftLine extends DialogueLineBody {
  /** Local-only key for React's list reconciliation -- never sent to the API. */
  key: string;
}

let draftKeySeq = 0;
const nextKey = () => `draft-${draftKeySeq++}`;

function draftLinesOf(scene?: SceneDetail): DraftLine[] {
  return (scene?.dialogue ?? []).map((d) => ({ key: nextKey(), character_id: d.character_id, text: d.text }));
}

/** Create/edit form for a Scene -- narrative, redline, participants, location,
 *  and dialogue lines all in one place, matching how `createScene`/
 *  `updateScene` replace the whole thing together. */
export function SceneEditor({
  storyId,
  characters,
  locations,
  initial,
  onSaved,
  onCancel,
}: {
  storyId: string;
  characters: Character[];
  locations: StoryLocation[];
  initial?: SceneDetail;
  onSaved: () => void;
  onCancel: () => void;
}) {
  const [locationId, setLocationId] = useState<string | null>(initial?.location_id ?? null);
  const [narrative, setNarrative] = useState(initial?.narrative ?? "");
  const [redline, setRedline] = useState(initial?.redline ?? "");
  const [participantIds, setParticipantIds] = useState<string[]>(initial?.participant_ids ?? []);
  const [lines, setLines] = useState<DraftLine[]>(() => draftLinesOf(initial));
  const [saving, setSaving] = useState(false);

  const toggleParticipant = (id: string) => {
    const isRemoving = participantIds.includes(id);
    setParticipantIds((prev) => (isRemoving ? prev.filter((p) => p !== id) : [...prev, id]));
    // Dialogue attributed to a participant who was just removed would point
    // at someone no longer in the scene -- drop those lines with them.
    if (isRemoving) {
      setLines((prev) => prev.filter((l) => l.character_id !== id));
    }
  };

  const addLine = () => {
    setLines((prev) => [...prev, { key: nextKey(), character_id: participantIds[0] ?? "", text: "" }]);
  };

  const updateLine = (key: string, patch: Partial<DialogueLineBody>) => {
    setLines((prev) => prev.map((l) => (l.key === key ? { ...l, ...patch } : l)));
  };

  const removeLine = (key: string) => setLines((prev) => prev.filter((l) => l.key !== key));

  const save = async () => {
    setSaving(true);
    const body = {
      location_id: locationId,
      narrative,
      redline,
      participant_ids: participantIds,
      dialogue: lines
        .filter((l) => l.character_id && l.text.trim())
        .map(({ character_id, text }) => ({ character_id, text: text.trim() })),
    };
    try {
      if (initial) {
        await updateScene(initial.id, body);
      } else {
        await createScene(storyId, body);
      }
      onSaved();
    } finally {
      setSaving(false);
    }
  };

  const participants = characters.filter((c) => participantIds.includes(c.id));

  return (
    <div className="scene-editor">
      <label>
        Redline
        <input
          value={redline}
          onChange={(e) => setRedline(e.target.value)}
          placeholder="e.g. 5 travelers meet in a tavern, suddenly a shivering roar in the mountain"
        />
      </label>
      <label>
        Narrative
        <textarea value={narrative} onChange={(e) => setNarrative(e.target.value)} rows={4} />
      </label>
      <label>
        Location
        <select value={locationId ?? ""} onChange={(e) => setLocationId(e.target.value || null)}>
          <option value="">No fixed location</option>
          {locations.map((l) => (
            <option key={l.id} value={l.id}>
              {l.name}
            </option>
          ))}
        </select>
      </label>

      <fieldset className="scene-editor__participants">
        <legend>Participants</legend>
        {characters.length === 0 && <p className="muted">Add a character first.</p>}
        {characters.map((c) => (
          <label key={c.id} className="scene-editor__checkbox">
            <input
              type="checkbox"
              checked={participantIds.includes(c.id)}
              onChange={() => toggleParticipant(c.id)}
            />
            {c.name}
          </label>
        ))}
      </fieldset>

      <fieldset className="scene-editor__dialogue">
        <legend>Dialogue (always shown as overlay text, never in the image)</legend>
        {lines.map((line) => (
          <div key={line.key} className="scene-editor__line">
            <select
              value={line.character_id}
              onChange={(e) => updateLine(line.key, { character_id: e.target.value })}
            >
              <option value="">Speaker…</option>
              {participants.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
            <input
              value={line.text}
              onChange={(e) => updateLine(line.key, { text: e.target.value })}
              placeholder="Line"
            />
            <button type="button" onClick={() => removeLine(line.key)} aria-label="Remove line">
              ×
            </button>
          </div>
        ))}
        <button type="button" className="scene-editor__add-line" onClick={addLine} disabled={participants.length === 0}>
          + Add line
        </button>
      </fieldset>

      <div className="inline-form__actions">
        <button type="button" onClick={save} disabled={saving}>
          {initial ? "Save scene" : "Add scene"}
        </button>
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </div>
  );
}
