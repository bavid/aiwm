import { useState } from "react";
import {
  createLocation,
  createNpc,
  deleteLocation,
  deleteNpc,
  setLocationReference,
  type LocationBody,
  type Npc,
  type NpcBody,
  type Story,
  type StoryLocation,
} from "../../lib/ipc";
import { generateImage } from "./generateImage";
import { JobImage } from "./JobImage";
import { locationReferencePrompt } from "./storyPrompts";

const BLANK_LOCATION: LocationBody = { name: "", description: "" };
const BLANK_NPC: NpcBody = { name: "", role: "", location_id: null, description: "" };

/** World: Locations (with a reference image, same job_id treatment as a
 *  Character's portrait) and NPCs (deliberately lightweight -- NOT the full
 *  Character shape). */
export function WorldSection({
  story,
  locations,
  npcs,
  onChanged,
}: {
  story: Story;
  locations: StoryLocation[];
  npcs: Npc[];
  onChanged: () => void;
}) {
  return (
    <div className="world">
      <LocationsPanel story={story} locations={locations} onChanged={onChanged} />
      <NpcsPanel storyId={story.id} locations={locations} npcs={npcs} onChanged={onChanged} />
    </div>
  );
}

function LocationsPanel({
  story,
  locations,
  onChanged,
}: {
  story: Story;
  locations: StoryLocation[];
  onChanged: () => void;
}) {
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState<LocationBody>(BLANK_LOCATION);
  const [generatingId, setGeneratingId] = useState<string | null>(null);

  const create = async () => {
    const name = draft.name.trim();
    if (!name) return;
    await createLocation(story.id, { ...draft, name });
    setDraft(BLANK_LOCATION);
    setCreating(false);
    onChanged();
  };

  const generateReference = async (location: StoryLocation) => {
    setGeneratingId(location.id);
    try {
      const jobId = await generateImage(locationReferencePrompt(story, location));
      await setLocationReference(location.id, jobId);
      onChanged();
    } finally {
      setGeneratingId(null);
    }
  };

  return (
    <div className="card">
      <header className="card__head">
        <h2>Locations</h2>
        <span className="card__sub numeric">{locations.length}</span>
      </header>

      <ul className="tile-list">
        {locations.map((loc) => (
          <li key={loc.id} className="tile">
            <JobImage jobId={loc.reference_job_id} alt={loc.name} className="tile__image" />
            <div className="tile__body">
              <span className="tile__name">{loc.name}</span>
              {loc.description && <p className="muted">{loc.description}</p>}
              <div className="tile__actions">
                <button
                  type="button"
                  onClick={() => generateReference(loc)}
                  disabled={generatingId === loc.id}
                >
                  {generatingId === loc.id
                    ? "Generating…"
                    : loc.reference_job_id
                      ? "Regenerate"
                      : "Generate reference"}
                </button>
                <button
                  type="button"
                  className="tile__delete"
                  onClick={() => window.confirm(`Delete "${loc.name}"?`) && deleteLocation(loc.id).then(onChanged)}
                >
                  Delete
                </button>
              </div>
            </div>
          </li>
        ))}
        {locations.length === 0 && !creating && <li className="muted">No locations yet.</li>}
      </ul>

      {creating ? (
        <div className="inline-form">
          <input
            autoFocus
            value={draft.name}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            placeholder="Name"
          />
          <textarea
            value={draft.description}
            onChange={(e) => setDraft({ ...draft, description: e.target.value })}
            placeholder="Description"
          />
          <div className="inline-form__actions">
            <button type="button" onClick={create} disabled={!draft.name.trim()}>
              Add location
            </button>
            <button type="button" onClick={() => setCreating(false)}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button type="button" className="section-card__add" onClick={() => setCreating(true)}>
          + New location
        </button>
      )}
    </div>
  );
}

function NpcsPanel({
  storyId,
  locations,
  npcs,
  onChanged,
}: {
  storyId: string;
  locations: StoryLocation[];
  npcs: Npc[];
  onChanged: () => void;
}) {
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState<NpcBody>(BLANK_NPC);

  const create = async () => {
    const name = draft.name.trim();
    if (!name) return;
    await createNpc(storyId, { ...draft, name });
    setDraft(BLANK_NPC);
    setCreating(false);
    onChanged();
  };

  const locationName = (id: string | null) => locations.find((l) => l.id === id)?.name ?? null;

  return (
    <div className="card">
      <header className="card__head">
        <h2>NPCs</h2>
        <span className="card__sub numeric">{npcs.length}</span>
      </header>

      <ul className="roster">
        {npcs.map((npc) => (
          <li key={npc.id}>
            <div className="roster__row roster__row--static">
              <span className="roster__name">{npc.name}</span>
              {npc.role && <span className="badge badge--soft">{npc.role}</span>}
              {locationName(npc.location_id) && (
                <span className="muted">@ {locationName(npc.location_id)}</span>
              )}
              <button
                type="button"
                className="tile__delete"
                onClick={() => window.confirm(`Delete "${npc.name}"?`) && deleteNpc(npc.id).then(onChanged)}
              >
                ×
              </button>
            </div>
            {npc.description && <p className="muted roster__desc">{npc.description}</p>}
          </li>
        ))}
        {npcs.length === 0 && !creating && <li className="muted">No NPCs yet.</li>}
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
            value={draft.role}
            onChange={(e) => setDraft({ ...draft, role: e.target.value })}
            placeholder="Role (e.g. harbourmaster)"
          />
          <select
            value={draft.location_id ?? ""}
            onChange={(e) => setDraft({ ...draft, location_id: e.target.value || null })}
          >
            <option value="">No fixed location</option>
            {locations.map((l) => (
              <option key={l.id} value={l.id}>
                {l.name}
              </option>
            ))}
          </select>
          <input
            value={draft.description}
            onChange={(e) => setDraft({ ...draft, description: e.target.value })}
            placeholder="One-line description"
          />
          <div className="inline-form__actions">
            <button type="button" onClick={create} disabled={!draft.name.trim()}>
              Add NPC
            </button>
            <button type="button" onClick={() => setCreating(false)}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button type="button" className="section-card__add" onClick={() => setCreating(true)}>
          + New NPC
        </button>
      )}
    </div>
  );
}
