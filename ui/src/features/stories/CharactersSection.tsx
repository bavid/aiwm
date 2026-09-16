import { useState } from "react";
import { createCharacter, type Character, type CharacterBody } from "../../lib/ipc";

const BLANK: CharacterBody = { name: "", traits: "", backstory: "", alignment: "" };

/** The Characters section: the story's full cast, click a row to open the
 *  Character Sheet drawer. NPCs live in World instead -- deliberately
 *  lighter-weight than a full Character (Phase 1 scope cut). */
export function CharactersSection({
  storyId,
  characters,
  selectedId,
  onSelect,
  onChanged,
}: {
  storyId: string;
  characters: Character[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onChanged: () => void;
}) {
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState<CharacterBody>(BLANK);

  const create = async () => {
    const name = draft.name.trim();
    if (!name) return;
    const character = await createCharacter(storyId, { ...draft, name });
    setDraft(BLANK);
    setCreating(false);
    onChanged();
    onSelect(character.id);
  };

  return (
    <div className="card">
      <header className="card__head">
        <h2>Characters</h2>
        <span className="card__sub numeric">{characters.length}</span>
      </header>

      {characters.length === 0 && !creating && (
        <p className="muted">No characters yet. Add the first one below.</p>
      )}

      <ul className="roster">
        {characters.map((c) => (
          <li key={c.id}>
            <button
              type="button"
              className={c.id === selectedId ? "roster__row roster__row--active" : "roster__row"}
              onClick={() => onSelect(c.id)}
            >
              <span className="roster__name">{c.name}</span>
              {c.alignment && <span className="badge badge--soft">{c.alignment}</span>}
              {!c.portrait_job_id && <span className="muted roster__hint">no portrait yet</span>}
            </button>
          </li>
        ))}
      </ul>

      {creating ? (
        <div className="inline-form">
          <input
            autoFocus
            value={draft.name}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            placeholder="Name"
          />
          <input
            value={draft.alignment}
            onChange={(e) => setDraft({ ...draft, alignment: e.target.value })}
            placeholder="Alignment / role hint (e.g. elf, friendly, skilled)"
          />
          <textarea
            value={draft.traits}
            onChange={(e) => setDraft({ ...draft, traits: e.target.value })}
            placeholder="Traits"
          />
          <textarea
            value={draft.backstory}
            onChange={(e) => setDraft({ ...draft, backstory: e.target.value })}
            placeholder="Backstory"
          />
          <div className="inline-form__actions">
            <button type="button" onClick={create} disabled={!draft.name.trim()}>
              Add character
            </button>
            <button type="button" onClick={() => setCreating(false)}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button type="button" className="section-card__add" onClick={() => setCreating(true)}>
          + New character
        </button>
      )}
    </div>
  );
}
