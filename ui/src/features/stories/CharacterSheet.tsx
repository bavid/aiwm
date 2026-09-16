import { useState } from "react";
import { useCharacterLog, useCharacterRelationships } from "../../lib/hooks";
import {
  addCharacterRelationship,
  deleteCharacter,
  removeCharacterRelationship,
  setCharacterInventory,
  setCharacterPortrait,
  updateCharacter,
  type Character,
  type CharacterBody,
  type Story,
} from "../../lib/ipc";
import { generateImage } from "./generateImage";
import { JobImage } from "./JobImage";
import { characterPortraitPrompt } from "./storyPrompts";

/** The persistent Character Sheet -- viewable from anywhere in the Timeline
 *  (an explicit, repeated requirement). Renders as a docked drawer; `null`
 *  character just means "closed". */
export function CharacterSheet({
  character,
  story,
  characters,
  onClose,
  onChanged,
}: {
  character: Character | null;
  story: Story | null;
  characters: Character[];
  onClose: () => void;
  onChanged: () => void;
}) {
  const { data: relationships, refetch: refetchRelationships } = useCharacterRelationships(
    character?.id ?? null,
  );
  const { data: log } = useCharacterLog(character?.id ?? null);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<CharacterBody | null>(null);
  const [newItem, setNewItem] = useState("");
  const [relTargetId, setRelTargetId] = useState("");
  const [relNote, setRelNote] = useState("");
  const [generating, setGenerating] = useState(false);

  if (!character) return null;

  const startEdit = () => {
    setDraft({
      name: character.name,
      traits: character.traits,
      backstory: character.backstory,
      alignment: character.alignment,
    });
    setEditing(true);
  };

  const saveEdit = async () => {
    if (!draft || !draft.name.trim()) return;
    await updateCharacter(character.id, draft);
    setEditing(false);
    onChanged();
  };

  const addItem = async () => {
    const item = newItem.trim();
    if (!item) return;
    await setCharacterInventory(character.id, [...character.inventory, item]);
    setNewItem("");
    onChanged();
  };

  const removeItem = async (item: string) => {
    await setCharacterInventory(
      character.id,
      character.inventory.filter((i) => i !== item),
    );
    onChanged();
  };

  const addRelationship = async () => {
    if (!relTargetId || !relNote.trim()) return;
    await addCharacterRelationship(character.id, relTargetId, relNote.trim());
    setRelTargetId("");
    setRelNote("");
    refetchRelationships();
  };

  const generatePortrait = async () => {
    if (!story) return;
    setGenerating(true);
    try {
      // Anchor a regeneration to the character's current portrait so it stays
      // recognizably the same character (Story Studio Phase 2). A first-ever
      // portrait has nothing to anchor to yet, so it generates independently,
      // same as Phase 1.
      const jobId = await generateImage(
        characterPortraitPrompt(story, character),
        character.portrait_job_id ?? undefined,
      );
      await setCharacterPortrait(character.id, jobId);
      onChanged();
    } finally {
      setGenerating(false);
    }
  };

  const remove = async () => {
    if (!window.confirm(`Delete character "${character.name}"? This cannot be undone.`)) return;
    await deleteCharacter(character.id);
    onClose();
    onChanged();
  };

  const otherCharacters = characters.filter((c) => c.id !== character.id);
  const nameOf = (id: string) => characters.find((c) => c.id === id)?.name ?? "(unknown)";

  return (
    <aside className="char-sheet" aria-label={`${character.name} — Character Sheet`}>
      <header className="char-sheet__head">
        <h3>{character.name}</h3>
        <button type="button" className="char-sheet__close" onClick={onClose} aria-label="Close character sheet">
          ×
        </button>
      </header>

      <JobImage
        jobId={character.portrait_job_id}
        alt={`${character.name}'s portrait`}
        className="char-sheet__portrait"
      />
      {character.portrait_job_id && (
        <span className="badge badge--soft" title="Regenerating this portrait, or a scene featuring only this character, anchors the render to it (IP-Adapter / reference-latent) instead of generating independently.">
          Consistency-anchored
        </span>
      )}
      <button type="button" className="char-sheet__generate" onClick={generatePortrait} disabled={generating || !story}>
        {generating ? "Generating…" : character.portrait_job_id ? "Regenerate portrait (anchored)" : "Generate portrait"}
      </button>

      {editing && draft ? (
        <div className="char-sheet__edit">
          <label>
            Name
            <input value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
          </label>
          <label>
            Alignment / role hint
            <input
              value={draft.alignment}
              onChange={(e) => setDraft({ ...draft, alignment: e.target.value })}
              placeholder="e.g. elf, friendly, skilled"
            />
          </label>
          <label>
            Traits
            <textarea value={draft.traits} onChange={(e) => setDraft({ ...draft, traits: e.target.value })} />
          </label>
          <label>
            Backstory
            <textarea
              value={draft.backstory}
              onChange={(e) => setDraft({ ...draft, backstory: e.target.value })}
            />
          </label>
          <div className="char-sheet__edit-actions">
            <button type="button" onClick={saveEdit} disabled={!draft.name.trim()}>
              Save
            </button>
            <button type="button" onClick={() => setEditing(false)}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <div className="char-sheet__body">
          {character.alignment && <span className="badge">{character.alignment}</span>}
          {character.traits && (
            <section>
              <h4>Traits</h4>
              <p>{character.traits}</p>
            </section>
          )}
          {character.backstory && (
            <section>
              <h4>Backstory</h4>
              <p>{character.backstory}</p>
            </section>
          )}
          <button type="button" className="char-sheet__edit-btn" onClick={startEdit}>
            Edit sheet
          </button>
        </div>
      )}

      <section className="char-sheet__section">
        <h4>Inventory</h4>
        <ul className="char-sheet__inventory">
          {character.inventory.map((item) => (
            <li key={item} className="chip">
              {item}
              <button type="button" onClick={() => removeItem(item)} aria-label={`Remove ${item}`}>
                ×
              </button>
            </li>
          ))}
          {character.inventory.length === 0 && <li className="muted">Empty-handed.</li>}
        </ul>
        <div className="char-sheet__add-row">
          <input
            value={newItem}
            onChange={(e) => setNewItem(e.target.value)}
            placeholder="Add item…"
            onKeyDown={(e) => e.key === "Enter" && addItem()}
          />
          <button type="button" onClick={addItem} disabled={!newItem.trim()}>
            Add
          </button>
        </div>
      </section>

      <section className="char-sheet__section">
        <h4>Relationships</h4>
        <ul className="char-sheet__relationships">
          {(relationships ?? []).map((r) => (
            <li key={r.id}>
              <span className="char-sheet__rel-target">{nameOf(r.related_character_id)}</span>
              <span className="muted">{r.note}</span>
              <button type="button" onClick={() => removeCharacterRelationship(r.id).then(refetchRelationships)}>
                ×
              </button>
            </li>
          ))}
          {(relationships ?? []).length === 0 && <li className="muted">No relationships recorded.</li>}
        </ul>
        {otherCharacters.length > 0 && (
          <div className="char-sheet__add-row">
            <select value={relTargetId} onChange={(e) => setRelTargetId(e.target.value)}>
              <option value="">Relates to…</option>
              {otherCharacters.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
            <input
              value={relNote}
              onChange={(e) => setRelNote(e.target.value)}
              placeholder="e.g. trusts her with her life"
            />
            <button type="button" onClick={addRelationship} disabled={!relTargetId || !relNote.trim()}>
              Add
            </button>
          </div>
        )}
      </section>

      <section className="char-sheet__section">
        <h4>Character Log</h4>
        <ul className="char-sheet__log">
          {(log ?? [])
            .slice()
            .reverse()
            .map((entry) => (
              <li key={entry.id}>
                <span className="muted">{new Date(entry.created_at).toLocaleString()}</span>
                <p>{entry.text}</p>
              </li>
            ))}
          {(log ?? []).length === 0 && <li className="muted">No scenes yet.</li>}
        </ul>
      </section>

      <button type="button" className="char-sheet__delete" onClick={remove}>
        Delete character
      </button>
    </aside>
  );
}
