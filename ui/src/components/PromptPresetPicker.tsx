import { useState } from "react";
import { usePromptPresets, type PresetKind } from "../lib/prompt-presets";
import "./prompt-preset-picker.css";

/** A preset dropdown for one prompt field (positive or negative) plus a
 *  "Manage" panel to add/edit/delete the user's own library. Picking a
 *  preset from the dropdown is a one-shot action -- it appends the preset's
 *  text into the field via `onApply` (same append-if-not-empty pattern as
 *  Image.tsx's smartphone preset) and resets, rather than holding a
 *  persistent "selected preset" the user would then have to un-select. */
export function PromptPresetPicker({
  kind,
  onApply,
}: {
  kind: PresetKind;
  onApply: (text: string) => void;
}) {
  const { presets, add, update, remove } = usePromptPresets(kind);
  const [managing, setManaging] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftText, setDraftText] = useState("");
  const [newName, setNewName] = useState("");
  const [newText, setNewText] = useState("");

  const label = kind === "positive" ? "Positive preset" : "Negative preset";

  const apply = (id: string) => {
    const preset = presets.find((p) => p.id === id);
    if (preset) onApply(preset.text);
  };

  const startEdit = (id: string, name: string, text: string) => {
    setEditingId(id);
    setDraftName(name);
    setDraftText(text);
  };

  const commitEdit = () => {
    if (editingId && draftName.trim() && draftText.trim()) {
      update(editingId, draftName.trim(), draftText.trim());
    }
    setEditingId(null);
  };

  const addPreset = () => {
    if (!newName.trim() || !newText.trim()) return;
    add(newName.trim(), newText.trim());
    setNewName("");
    setNewText("");
  };

  return (
    <div className="preset-picker">
      <div className="preset-picker__row">
        <select
          className="preset-picker__select"
          value=""
          onChange={(e) => {
            if (e.target.value) apply(e.target.value);
            e.target.value = "";
          }}
          aria-label={label}
        >
          <option value="" disabled>
            {label}…
          </option>
          {presets.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="preset-picker__manage"
          aria-pressed={managing}
          onClick={() => setManaging((v) => !v)}
        >
          {managing ? "Done" : "Edit presets"}
        </button>
      </div>

      {managing && (
        <div className="preset-picker__panel">
          {presets.map((p) =>
            editingId === p.id ? (
              <div key={p.id} className="preset-picker__edit">
                <input
                  autoFocus
                  value={draftName}
                  onChange={(e) => setDraftName(e.target.value)}
                  placeholder="Name"
                  spellCheck={false}
                />
                <textarea
                  value={draftText}
                  onChange={(e) => setDraftText(e.target.value)}
                  rows={2}
                  placeholder="Prompt text"
                  spellCheck
                />
                <div className="preset-picker__edit-actions">
                  <button type="button" className="chip" onClick={commitEdit}>
                    Save
                  </button>
                  <button type="button" className="chip" onClick={() => setEditingId(null)}>
                    Cancel
                  </button>
                </div>
              </div>
            ) : (
              <div key={p.id} className="preset-picker__item">
                <div className="preset-picker__item-text">
                  <strong>{p.name}</strong>
                  <span>{p.text}</span>
                </div>
                <div className="preset-picker__item-actions">
                  <button
                    type="button"
                    className="preset-picker__icon"
                    aria-label={`Edit preset ${p.name}`}
                    onClick={() => startEdit(p.id, p.name, p.text)}
                  >
                    ✎
                  </button>
                  <button
                    type="button"
                    className="preset-picker__icon preset-picker__icon--danger"
                    aria-label={`Delete preset ${p.name}`}
                    onClick={() => remove(p.id)}
                  >
                    ×
                  </button>
                </div>
              </div>
            ),
          )}
          <div className="preset-picker__add">
            <input
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="New preset name"
              spellCheck={false}
            />
            <textarea
              value={newText}
              onChange={(e) => setNewText(e.target.value)}
              rows={2}
              placeholder="Prompt text"
              spellCheck
            />
            <button
              type="button"
              className="chip"
              onClick={addPreset}
              disabled={!newName.trim() || !newText.trim()}
            >
              + Add preset
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
