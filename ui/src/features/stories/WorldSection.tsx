import { useState, type ReactNode } from "react";
import {
  createLocation,
  createNpc,
  deleteLocation,
  deleteNpc,
  setLocationReference,
  updateLocation,
  updateNpc,
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
  // Same "edit icon on the row opens an inline form" pattern as the
  // character roster (CharactersSection). One location edits at a time.
  const [editingId, setEditingId] = useState<string | null>(null);

  const create = async () => {
    const name = draft.name.trim();
    if (!name) return;
    await createLocation(story.id, { ...draft, name });
    setDraft(BLANK_LOCATION);
    setCreating(false);
    onChanged();
  };

  const save = async (id: string, body: LocationBody) => {
    await updateLocation(id, body);
    setEditingId(null);
    onChanged();
  };

  const generateReference = async (location: StoryLocation) => {
    setGeneratingId(location.id);
    try {
      // Same anchoring treatment as a Character's portrait: a regeneration
      // stays recognizably the same location instead of drifting (Story
      // Studio Phase 2). A first-ever reference has nothing to anchor to.
      const jobId = await generateImage(
        locationReferencePrompt(story, location),
        location.reference_job_id ?? undefined,
      );
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
            {editingId === loc.id ? (
              <LocationEditForm
                location={loc}
                onSave={(body) => save(loc.id, body)}
                onCancel={() => setEditingId(null)}
              />
            ) : (
              <div className="tile__body">
                <div className="tile__head">
                  <span className="tile__name">{loc.name}</span>
                  <button
                    type="button"
                    className="roster__edit"
                    onClick={() => setEditingId(loc.id)}
                    aria-label={`Edit ${loc.name}`}
                    title="Edit location"
                  >
                    ✎
                  </button>
                </div>
                {loc.description && <p className="muted">{loc.description}</p>}
                <div className="tile__actions">
                  <button
                    type="button"
                    onClick={() => generateReference(loc)}
                    disabled={generatingId === loc.id}
                    title={
                      loc.reference_job_id
                        ? "Anchored to the current reference image for a consistent look"
                        : undefined
                    }
                  >
                    {generatingId === loc.id
                      ? "Generating…"
                      : loc.reference_job_id
                        ? "Regenerate (anchored)"
                        : "Generate reference"}
                  </button>
                  <button
                    type="button"
                    className="tile__delete"
                    onClick={() =>
                      window.confirm(`Delete "${loc.name}"?`) && deleteLocation(loc.id).then(onChanged)
                    }
                  >
                    Delete
                  </button>
                </div>
              </div>
            )}
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

/** Inline edit form for one location, rendered in place of its tile body.
 *  The draft is seeded from the location at mount; only one location is in
 *  edit mode at a time, so switching locations always mounts a fresh form. */
function LocationEditForm({
  location,
  onSave,
  onCancel,
}: {
  location: StoryLocation;
  onSave: (body: LocationBody) => Promise<void>;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState<LocationBody>({
    name: location.name,
    description: location.description,
  });
  const [saving, setSaving] = useState(false);

  const save = async () => {
    const name = draft.name.trim();
    if (!name) return;
    setSaving(true);
    try {
      await onSave({ ...draft, name });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="inline-form inline-form--tile" aria-label={`Edit ${location.name}`}>
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
        <button type="button" onClick={save} disabled={saving || !draft.name.trim()}>
          {saving ? "Saving…" : "Save"}
        </button>
        <button type="button" onClick={onCancel} disabled={saving}>
          Cancel
        </button>
      </div>
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
  const [editingId, setEditingId] = useState<string | null>(null);

  const create = async () => {
    const name = draft.name.trim();
    if (!name) return;
    await createNpc(storyId, { ...draft, name });
    setDraft(BLANK_NPC);
    setCreating(false);
    onChanged();
  };

  const save = async (id: string, body: NpcBody) => {
    await updateNpc(id, body);
    setEditingId(null);
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
            {editingId === npc.id ? (
              <NpcEditForm
                npc={npc}
                locations={locations}
                onSave={(body) => save(npc.id, body)}
                onCancel={() => setEditingId(null)}
              />
            ) : (
              <>
                <div className="roster__item">
                  <div className="roster__row roster__row--static">
                    <span className="roster__name">{npc.name}</span>
                    {npc.role && <span className="badge badge--soft">{npc.role}</span>}
                    {locationName(npc.location_id) && (
                      <span className="muted">@ {locationName(npc.location_id)}</span>
                    )}
                    <button
                      type="button"
                      className="tile__delete"
                      onClick={() =>
                        window.confirm(`Delete "${npc.name}"?`) && deleteNpc(npc.id).then(onChanged)
                      }
                    >
                      ×
                    </button>
                  </div>
                  <button
                    type="button"
                    className="roster__edit"
                    onClick={() => setEditingId(npc.id)}
                    aria-label={`Edit ${npc.name}`}
                    title="Edit NPC"
                  >
                    ✎
                  </button>
                </div>
                {npc.description && <p className="muted roster__desc">{npc.description}</p>}
              </>
            )}
          </li>
        ))}
        {npcs.length === 0 && !creating && <li className="muted">No NPCs yet.</li>}
      </ul>

      {creating ? (
        <NpcFields draft={draft} locations={locations} onChange={setDraft}>
          <button type="button" onClick={create} disabled={!draft.name.trim()}>
            Add NPC
          </button>
          <button type="button" onClick={() => setCreating(false)}>
            Cancel
          </button>
        </NpcFields>
      ) : (
        <button type="button" className="section-card__add" onClick={() => setCreating(true)}>
          + New NPC
        </button>
      )}
    </div>
  );
}

/** Inline edit form for one NPC, rendered in place of its roster row. Only
 *  one NPC is in edit mode at a time, so the draft is always seeded from
 *  the NPC that was just opened. */
function NpcEditForm({
  npc,
  locations,
  onSave,
  onCancel,
}: {
  npc: Npc;
  locations: StoryLocation[];
  onSave: (body: NpcBody) => Promise<void>;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState<NpcBody>({
    name: npc.name,
    role: npc.role,
    location_id: npc.location_id,
    description: npc.description,
  });
  const [saving, setSaving] = useState(false);

  const save = async () => {
    const name = draft.name.trim();
    if (!name) return;
    setSaving(true);
    try {
      await onSave({ ...draft, name });
    } finally {
      setSaving(false);
    }
  };

  return (
    <NpcFields draft={draft} locations={locations} onChange={setDraft} label={`Edit ${npc.name}`}>
      <button type="button" onClick={save} disabled={saving || !draft.name.trim()}>
        {saving ? "Saving…" : "Save"}
      </button>
      <button type="button" onClick={onCancel} disabled={saving}>
        Cancel
      </button>
    </NpcFields>
  );
}

/** The NPC field set shared by the create and edit forms; `children` are
 *  the action buttons. */
function NpcFields({
  draft,
  locations,
  onChange,
  label,
  children,
}: {
  draft: NpcBody;
  locations: StoryLocation[];
  onChange: (next: NpcBody) => void;
  label?: string;
  children: ReactNode;
}) {
  return (
    <div className="inline-form" aria-label={label}>
      <input
        autoFocus
        value={draft.name}
        onChange={(e) => onChange({ ...draft, name: e.target.value })}
        placeholder="Name"
      />
      <input
        value={draft.role}
        onChange={(e) => onChange({ ...draft, role: e.target.value })}
        placeholder="Role (e.g. harbourmaster)"
      />
      <select
        value={draft.location_id ?? ""}
        onChange={(e) => onChange({ ...draft, location_id: e.target.value || null })}
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
        onChange={(e) => onChange({ ...draft, description: e.target.value })}
        placeholder="One-line description"
      />
      <div className="inline-form__actions">{children}</div>
    </div>
  );
}
