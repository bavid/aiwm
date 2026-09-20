import { useId, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useVoiceIdentities } from "../lib/hooks";
import { createVoiceIdentity, deleteVoiceIdentity } from "../lib/ipc";
import { HelpHint } from "./HelpHint";
import "./voice-identity-picker.css";

interface VoiceIdentityPickerProps {
  /** A saved identity's id, or `null` for "no saved voice" (the caller falls
   *  back to its own free-text, seed-only narrator identity). */
  value: string | null;
  onChange: (id: string | null) => void;
  /** The select's id, so the caller's label and hint can point at it. */
  selectId?: string;
}

/** Picks a saved Dia voice-cloning identity -- a reference clip + its own
 *  transcript, set up once under a name (e.g. "Old Man Gareth") and reused
 *  across many narration calls -- or falls back to `null` for the caller's
 *  own free-text, seed-only identity. Self-contained fetch/create/delete,
 *  self-contained the way the session sidebar is.
 *
 *  The file picker below is real only inside a Tauri window
 *  (`@tauri-apps/plugin-dialog`); in the browser dev preview it silently
 *  no-ops, same as Video's "Browse…" for a start frame. */
export function VoiceIdentityPicker({ value, onChange, selectId }: VoiceIdentityPickerProps) {
  const { data: identities, refetch } = useVoiceIdentities();
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [audioPath, setAudioPath] = useState("");
  const [transcript, setTranscript] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ids = useId();
  const nameId = `${ids}-name`;
  const clipId = `${ids}-clip`;
  const transcriptId = `${ids}-transcript`;

  const current = (identities ?? []).find((v) => v.id === value) ?? null;

  const startCreate = () => {
    setCreating(true);
    setName("");
    setAudioPath("");
    setTranscript("");
    setError(null);
  };

  const cancelCreate = () => setCreating(false);

  const browseForAudio = async () => {
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "Audio", extensions: ["wav", "mp3", "flac", "ogg", "m4a"] }],
      });
      if (typeof picked === "string") setAudioPath(picked);
    } catch {
      // Not running inside Tauri (e.g. the browser dev preview) — no-op.
    }
  };

  const canSave = !!name.trim() && !!audioPath.trim() && !!transcript.trim() && !saving;

  const save = async () => {
    if (!canSave) return;
    setSaving(true);
    setError(null);
    try {
      const identity = await createVoiceIdentity({
        name: name.trim(),
        source_audio_path: audioPath.trim(),
        reference_transcript: transcript.trim(),
      });
      setCreating(false);
      refetch();
      onChange(identity.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  const removeCurrent = async () => {
    if (!current) return;
    if (
      !window.confirm(`Delete the saved voice "${current.name}"? Its reference clip is removed too.`)
    ) {
      return;
    }
    await deleteVoiceIdentity(current.id);
    refetch();
    onChange(null);
  };

  if (creating) {
    return (
      <div className="voice-identity-picker voice-identity-picker--editing">
        <div className="voice-identity-picker__field">
          <span>
            <label htmlFor={nameId}>Name</label>
          </span>
          <input
            id={nameId}
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Old Man Gareth"
            spellCheck={false}
          />
        </div>
        <div className="voice-identity-picker__field">
          <span>
            <label htmlFor={clipId}>Reference clip</label>
            <HelpHint area="voice" setting="reference-clip" describes={clipId} />
          </span>
          <div className="voice-identity-picker__browse">
            <input
              id={clipId}
              value={audioPath}
              onChange={(e) => setAudioPath(e.target.value)}
              placeholder="A short (5–15s), clean, single-speaker clip"
              spellCheck={false}
            />
            <button type="button" className="chip" onClick={browseForAudio}>
              Browse…
            </button>
          </div>
        </div>
        <div className="voice-identity-picker__field">
          <span>
            <label htmlFor={transcriptId}>Transcript (exactly what that clip says)</label>
            <HelpHint area="voice" setting="transcript" describes={transcriptId} />
          </span>
          <textarea
            id={transcriptId}
            value={transcript}
            onChange={(e) => setTranscript(e.target.value)}
            rows={2}
            spellCheck
            placeholder="Transcribe the reference clip's words, verbatim"
          />
        </div>
        {error && <p className="voice-identity-picker__err">{error}</p>}
        <div className="voice-identity-picker__actions">
          <button type="button" className="voiceform__go" onClick={save} disabled={!canSave}>
            {saving ? "Saving…" : "Save voice"}
          </button>
          <button type="button" className="voice-identity-picker__cancel" onClick={cancelCreate}>
            Cancel
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="voice-identity-picker">
      <select id={selectId} value={value ?? ""} onChange={(e) => onChange(e.target.value || null)}>
        <option value="">Free-text identity (seed only)</option>
        {(identities ?? []).map((v) => (
          <option key={v.id} value={v.id}>
            {v.name}
          </option>
        ))}
      </select>
      <button
        type="button"
        className="voice-identity-picker__icon"
        onClick={startCreate}
        aria-label="Save a new voice from a reference clip"
      >
        +
      </button>
      {current && (
        <button
          type="button"
          className="voice-identity-picker__icon voice-identity-picker__icon--danger"
          onClick={removeCurrent}
          aria-label={`Delete the saved voice ${current.name}`}
        >
          ×
        </button>
      )}
    </div>
  );
}
