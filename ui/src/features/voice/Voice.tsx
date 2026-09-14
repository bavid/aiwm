import { useEffect, useState } from "react";
import { PromptAssistant } from "../../components/PromptAssistant";
import { QueueList } from "../../components/QueueList";
import { useAbout, useJobs, useModels } from "../../lib/hooks";
import {
  cancelJob,
  deleteJob,
  jobDetail,
  jobOutputUrl,
  submitJob,
  type Job,
  type JobDetail,
  type JobState,
} from "../../lib/ipc";
import "./voice.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 500;
const MIN_SPEED = 0.5;
const MAX_SPEED = 2.0;

/** Curated narrator moods — real Kokoro voice ids, picked by ear (see the
 *  four samples generated while planning this feature), not invented names.
 *  "Preset" here means "voice + a speed that suits it", nothing more exotic —
 *  Kokoro has no separate emotion/tone control, and pretending otherwise
 *  would just be an unreliable, unlabelled speed change in disguise. */
const PRESETS: { id: string; label: string; voice: string; speed: number; blurb: string }[] = [
  { id: "ominous", label: "Ominous", voice: "am_fenrir", speed: 0.95, blurb: "Low, deliberate — something's wrong." },
  { id: "warm", label: "Warm storyteller", voice: "bf_emma", speed: 1.0, blurb: "Fireside, easy to listen to." },
  { id: "drywit", label: "Dry wit", voice: "bm_fable", speed: 1.05, blurb: "Amused, a little detached." },
  { id: "epic", label: "Grand / epic", voice: "bm_george", speed: 0.9, blurb: "Big, formal, scene-setting." },
];

/** Every English voice Kokoro ships (54 total across 11 languages) — scoped
 *  to English here since the sidecar always phonemizes as `en-us`; a
 *  non-English voice would just mispronounce the text, not a real feature. */
const ALL_VOICES = [
  "af_alloy", "af_aoede", "af_bella", "af_heart", "af_jessica", "af_kore", "af_nicole",
  "af_nova", "af_river", "af_sarah", "af_sky",
  "am_adam", "am_echo", "am_eric", "am_fenrir", "am_liam", "am_michael", "am_onyx",
  "am_puck", "am_santa",
  "bf_alice", "bf_emma", "bf_isabella", "bf_lily",
  "bm_daniel", "bm_fable", "bm_george", "bm_lewis",
];

const SAMPLE_LINE = "From the mist of the mountain pass, an old story stirs once more.";

function promptOf(job: Job): string {
  const p = job.params as { text?: unknown } | null;
  return p && typeof p.text === "string" ? p.text : "";
}

export function Voice() {
  const about = useAbout();
  const { data: models } = useModels();
  const { data: jobs } = useJobs();

  const hasVoiceModel = (models ?? []).some((m) => m.roles.includes("voice_model"));
  const hasVoiceData = (models ?? []).some((m) => m.roles.includes("voice_data"));
  const ready = hasVoiceModel && hasVoiceData;

  const [text, setText] = useState(SAMPLE_LINE);
  const [voice, setVoice] = useState(PRESETS[0].voice);
  const [speed, setSpeed] = useState(PRESETS[0].speed);
  const [activePreset, setActivePreset] = useState<string | null>(PRESETS[0].id);
  const [sendError, setSendError] = useState<string | null>(null);

  const [pendingId, setPendingId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => {
    if (!pendingId) return;
    let alive = true;
    const tick = async () => {
      let d: JobDetail | null;
      try {
        d = await jobDetail(pendingId);
      } catch {
        return;
      }
      if (!alive || !d) return;
      setDetail(d);
      if (DONE.includes(d.job.state)) setPendingId(null);
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [pendingId]);

  const history = (jobs ?? [])
    .filter((j) => j.job_type === "tts" && j.state === "completed" && j.output_path)
    .sort((a, b) => b.created_at.localeCompare(a.created_at));

  const selected =
    (detail?.job.id === selectedId ? detail.job : null) ??
    (jobs ?? []).find((j) => j.id === selectedId) ??
    null;

  const stuck = detail?.job.state === "blocked";
  const canGenerate = !!text.trim() && (!pendingId || stuck) && ready;

  const applyPreset = (p: (typeof PRESETS)[number]) => {
    setActivePreset(p.id);
    setVoice(p.voice);
    setSpeed(p.speed);
  };

  const preview = async () => {
    const trimmed = text.trim();
    if (!trimmed || (pendingId && !stuck)) return;
    setSendError(null);
    try {
      const job = await submitJob({
        job_type: "tts",
        params: { text: trimmed, voice, speed },
      });
      setPendingId(job.id);
      setSelectedId(job.id);
      setDetail({ job, events: [] });
    } catch (err) {
      setSendError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteJob(id);
      if (selectedId === id) {
        setSelectedId(null);
        setDetail(null);
      }
    } catch (err) {
      setSendError(err instanceof Error ? err.message : String(err));
    }
  };

  if (!ready) {
    return (
      <div className="voice">
        <section className="card voice__empty">
          <h2>The narrator needs a voice model</h2>
          <p className="muted">
            Import Kokoro (the local text-to-speech engine) to give Story Studio scenes an
            off-screen narrator. It's a one-time, fully local download — no cloud calls.
          </p>
          <ul className="voice__empty-list">
            <li className={hasVoiceModel ? "done" : ""}>
              {hasVoiceModel ? "✓" : "○"} Kokoro voice model
            </li>
            <li className={hasVoiceData ? "done" : ""}>
              {hasVoiceData ? "✓" : "○"} Kokoro voices (54 presets)
            </li>
          </ul>
          <p className="voice__empty-hint">
            Models tab → Add models → Voice → “Download entire stack” gets both files in one go.
          </p>
        </section>
      </div>
    );
  }

  return (
    <div className="voice">
      <section className="card voice__form">
        <header className="card__head">
          <h2>Narrator</h2>
          <span className="card__sub">an off-screen voice, used a beat at a time</span>
        </header>

        <div className="voice__presets">
          {PRESETS.map((p) => (
            <button
              key={p.id}
              type="button"
              className={`voice__preset ${activePreset === p.id ? "voice__preset--on" : ""}`}
              onClick={() => applyPreset(p)}
              title={p.blurb}
            >
              <span className="voice__preset-label">{p.label}</span>
              <span className="voice__preset-blurb">{p.blurb}</span>
            </button>
          ))}
        </div>

        <PromptAssistant kind="narrate" sessionId={null} onApplyPrompt={setText} />

        <form
          className="voiceform"
          onSubmit={(e) => {
            e.preventDefault();
            preview();
          }}
        >
          <label className="voiceform__field">
            <span>Line to narrate</span>
            <textarea
              value={text}
              onChange={(e) => setText(e.target.value)}
              rows={3}
              spellCheck
              placeholder="Suddenly, a shivering roar echoes down from the mountain."
            />
          </label>

          <div className="voiceform__grid">
            <label className="voiceform__field">
              <span>Voice (advanced)</span>
              <select
                value={voice}
                onChange={(e) => {
                  setVoice(e.target.value);
                  setActivePreset(null);
                }}
              >
                {ALL_VOICES.map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </select>
            </label>
            <label className="voiceform__field">
              <span>Speed — {speed.toFixed(2)}×</span>
              <input
                type="range"
                min={MIN_SPEED}
                max={MAX_SPEED}
                step={0.05}
                value={speed}
                onChange={(e) => {
                  setSpeed(Number(e.target.value));
                  setActivePreset(null);
                }}
              />
            </label>
          </div>

          <button type="submit" className="voiceform__go" disabled={!canGenerate}>
            {pendingId && !stuck ? "Narrating…" : "Preview"}
          </button>
        </form>
        {sendError && <p className="voice__err">{sendError}</p>}
      </section>

      <div className="voice__result">
        <QueueList
          jobType="tts"
          jobs={jobs ?? []}
          modelNames={new Map()}
          selectedId={selectedId}
          onSelect={(id) => {
            setSelectedId(id);
            setDetail(null);
          }}
          onCancel={cancelJob}
          promptOf={promptOf}
        />
        <section className="card">
          <header className="card__head">
            <h2>Result</h2>
            {selected && <span className="card__sub">{selected.state}</span>}
          </header>
          <Result
            job={selected}
            port={about?.core_api_port ?? null}
            onCancel={selected ? () => cancelJob(selected.id) : undefined}
            onDelete={selected ? () => handleDelete(selected.id) : undefined}
          />
        </section>
      </div>

      <section className="card card--wide">
        <header className="card__head">
          <h2>History</h2>
          <span className="card__sub numeric">{history.length}</span>
        </header>
        {history.length === 0 ? (
          <p className="muted">Narrated lines show up here.</p>
        ) : (
          <ul className="voice__history">
            {history.map((j) => (
              <li key={j.id} className={j.id === selectedId ? "voice__hrow voice__hrow--selected" : "voice__hrow"}>
                <button type="button" className="voice__hrow-select" onClick={() => setSelectedId(j.id)}>
                  {promptOf(j) || "untitled"}
                </button>
                {about && (
                  <audio controls preload="none" src={jobOutputUrl(about.core_api_port, j.id)} />
                )}
                <button
                  type="button"
                  className="voice__hrow-delete"
                  title="Delete"
                  aria-label="Delete this narration"
                  onClick={() => handleDelete(j.id)}
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

function Result({
  job,
  port,
  onCancel,
  onDelete,
}: {
  job: Job | null;
  port: number | null;
  onCancel?: () => void;
  onDelete?: () => void;
}) {
  if (!job) return <p className="muted">Write a line and hit Preview.</p>;

  const running = !DONE.includes(job.state);

  return (
    <div className="voiceresult">
      <div className="voiceresult__canvas" data-state={job.state}>
        {job.state === "completed" && port != null ? (
          <audio controls autoPlay src={jobOutputUrl(port, job.id)} />
        ) : job.state === "failed" ? (
          <span className="voiceresult__err">{job.error_text ?? "narration failed"}</span>
        ) : job.state === "blocked" ? (
          <span className="voiceresult__err">{job.error_text ?? "the narrator is busy"}</span>
        ) : job.state === "cancelled" ? (
          <span className="muted">cancelled</span>
        ) : (
          <span className="voiceresult__spin">{job.state}…</span>
        )}
      </div>
      {running && onCancel && (
        <button type="button" className="voiceresult__cancel" onClick={onCancel}>
          Stop
        </button>
      )}
      {!running && onDelete && (
        <button type="button" className="voiceresult__cancel" onClick={onDelete}>
          Delete
        </button>
      )}
    </div>
  );
}
