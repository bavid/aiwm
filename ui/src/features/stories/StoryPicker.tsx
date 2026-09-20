import { useId, useState } from "react";
import { HelpHint } from "../../components/HelpHint";
import { createStory, deleteStory, updateStory, type Story, type StoryBody } from "../../lib/ipc";

const BLANK: StoryBody = { name: "", setting: "", art_style: "", premise: "" };

/** Picks which Story is active, and creates/edits/deletes them. Stories are
 *  Story Studio's top-level container -- everything else (characters, world,
 *  timeline) belongs to exactly one. `stories`/`refetch` come from the
 *  parent, which needs the same list for the active Story object anyway. */
export function StoryPicker({
  stories,
  refetch,
  activeId,
  onChange,
}: {
  stories: Story[];
  refetch: () => void;
  activeId: string | null;
  onChange: (id: string | null) => void;
}) {
  const [mode, setMode] = useState<"idle" | "creating" | "editing">("idle");
  const [draft, setDraft] = useState<StoryBody>(BLANK);
  const selectId = useId();

  const current = (stories ?? []).find((s) => s.id === activeId) ?? null;

  const startCreate = () => {
    setDraft(BLANK);
    setMode("creating");
  };

  const startEdit = () => {
    if (!current) return;
    setDraft({
      name: current.name,
      setting: current.setting,
      art_style: current.art_style,
      premise: current.premise,
    });
    setMode("editing");
  };

  const cancel = () => setMode("idle");

  const save = async () => {
    const name = draft.name.trim();
    if (!name) return;
    const body = { ...draft, name };
    if (mode === "creating") {
      const story = await createStory(body);
      setMode("idle");
      refetch();
      onChange(story.id);
    } else if (mode === "editing" && current) {
      await updateStory(current.id, body);
      setMode("idle");
      refetch();
    }
  };

  const remove = async () => {
    if (!current) return;
    if (
      !window.confirm(
        `Delete story "${current.name}"?\n\nEvery character, location, and scene in it goes too. This cannot be undone.`,
      )
    )
      return;
    await deleteStory(current.id);
    refetch();
    onChange(null);
  };

  if (mode !== "idle") {
    return (
      <div className="story-picker story-picker--editing">
        <input
          autoFocus
          aria-label="Story name"
          value={draft.name}
          onChange={(e) => setDraft({ ...draft, name: e.target.value })}
          placeholder="Story name"
          spellCheck={false}
        />
        <input
          aria-label="Setting / era"
          value={draft.setting}
          onChange={(e) => setDraft({ ...draft, setting: e.target.value })}
          placeholder="Setting / era"
          spellCheck={false}
        />
        <input
          aria-label="Art style"
          value={draft.art_style}
          onChange={(e) => setDraft({ ...draft, art_style: e.target.value })}
          placeholder="Art style"
          spellCheck={false}
        />
        <textarea
          aria-label="Premise"
          value={draft.premise}
          onChange={(e) => setDraft({ ...draft, premise: e.target.value })}
          placeholder="Premise -- a paragraph or two about what this story is about"
          rows={3}
        />
        <button type="button" className="story-picker__save" onClick={save} disabled={!draft.name.trim()}>
          Save
        </button>
        <button type="button" className="story-picker__icon" onClick={cancel} aria-label="Cancel">
          ×
        </button>
      </div>
    );
  }

  return (
    <div className="story-picker">
      <select
        id={selectId}
        aria-label="Story"
        value={activeId ?? ""}
        onChange={(e) => onChange(e.target.value || null)}
      >
        <option value="">Choose a story…</option>
        {(stories ?? []).map((s: Story) => (
          <option key={s.id} value={s.id}>
            {s.name}
          </option>
        ))}
      </select>
      <HelpHint area="stories" setting="story" describes={selectId} />
      <button type="button" className="story-picker__icon" onClick={startCreate} aria-label="New story">
        +
      </button>
      {current && (
        <>
          <button
            type="button"
            className="story-picker__icon"
            onClick={startEdit}
            aria-label={`Edit story ${current.name}`}
          >
            ✎
          </button>
          <button
            type="button"
            className="story-picker__icon story-picker__icon--danger"
            onClick={remove}
            aria-label={`Delete story ${current.name}`}
          >
            ×
          </button>
        </>
      )}
    </div>
  );
}
