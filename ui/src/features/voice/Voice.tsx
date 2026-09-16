import { useEffect, useState } from "react";
import { PromptAssistant } from "../../components/PromptAssistant";
import { QueueList } from "../../components/QueueList";
import { VoiceIdentityPicker } from "../../components/VoiceIdentityPicker";
import { useAbout, useJobs, useModels } from "../../lib/hooks";
import {
  cancelJob,
  cleanAudio,
  deleteJob,
  jobDetail,
  jobOutputUrl,
  submitJob,
  type Job,
  type JobDetail,
  type JobState,
} from "../../lib/ipc";
import type { NarrateEngine } from "../../lib/prompt-assistant";
import "./voice.css";

const DONE: JobState[] = ["completed", "failed", "cancelled"];
const POLL_MS = 500;
const MIN_SPEED = 0.5;
const MAX_SPEED = 2.0;
/** Dia has no named voice presets -- `voice` becomes a free-text "narrator
 *  identity" label the sidecar hashes into a stable seed (see
 *  `aiwm_sidecar.dia._seed_for_narrator`), so a repeat render actually
 *  sounds like the same speaker instead of a new random one every time. */
const DEFAULT_DIA_NARRATOR = "narrator";

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
  { id: "clinical", label: "Cold and clinical", voice: "am_onyx", speed: 0.92, blurb: "Detached, precise — reciting facts, not comfort." },
  { id: "wonder", label: "Wide-eyed wonder", voice: "af_sky", speed: 1.05, blurb: "Breathless, delighted — something amazing just happened." },
  { id: "veteran", label: "Weary veteran", voice: "bm_lewis", speed: 0.9, blurb: "Tired, seen-it-all — one more war story." },
  { id: "trickster", label: "Playful trickster", voice: "am_puck", speed: 1.1, blurb: "Mischievous, quick — enjoying this more than you are." },
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

/** Kokoro's own voice-id convention: `<accent><gender>_<name>`. This is the
 *  only thing about a voice we can honestly label sight-unseen — matching it
 *  to a personality would mean actually listening to each one (measured pitch
 *  alone is a bad proxy: `am_fenrir`, the "Ominous/low" preset above, isn't
 *  even below the median pitch of the male voices here — timbre and delivery
 *  carry more of "ominous" than raw pitch does). */
const VOICE_ACCENT_GENDER: Record<string, string> = {
  af: "American, female",
  am: "American, male",
  bf: "British, female",
  bm: "British, male",
};

function voiceLabel(id: string): string {
  const prefix = id.split("_")[0];
  const tag = VOICE_ACCENT_GENDER[prefix];
  return tag ? `${id} — ${tag}` : id;
}

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
  const kokoroReady = hasVoiceModel && hasVoiceData;

  const hasDiaEngine = (models ?? []).some((m) => m.roles.includes("dia_engine"));
  const hasDiaCodec = (models ?? []).some((m) => m.roles.includes("dia_codec"));
  const diaReady = hasDiaEngine && hasDiaCodec;

  // The tab is usable once *either* engine is fully imported -- someone who
  // only ever imports Dia shouldn't be told "no voice model imported" when
  // they have a perfectly working narrator.
  const ready = kokoroReady || diaReady;

  const [text, setText] = useState(SAMPLE_LINE);
  const [engine, setEngineState] = useState<NarrateEngine>("kokoro");
  const [voice, setVoice] = useState(PRESETS[0].voice);
  const [speed, setSpeed] = useState(PRESETS[0].speed);
  const [activePreset, setActivePreset] = useState<string | null>(PRESETS[0].id);
  const [sendError, setSendError] = useState<string | null>(null);
  // A saved Dia voice-cloning identity (reference clip + transcript) --
  // `null` keeps today's free-text, seed-only path (`voice` above). Kokoro
  // has no equivalent, so this only ever matters while `engine === "dia"`.
  const [voiceIdentityId, setVoiceIdentityId] = useState<string | null>(null);
  const engineReady = engine === "dia" ? diaReady : kokoroReady;

  // Kokoro is imported far more often than Dia, so it's the sane default --
  // but if only Dia turns out to be ready (Kokoro was never imported), land
  // on the engine that actually works instead of a dead default.
  useEffect(() => {
    if (!kokoroReady && diaReady) setEngineState("dia");
  }, [kokoroReady, diaReady]);

  const selectEngine = (next: NarrateEngine) => {
    setEngineState(next);
    setSendError(null);
    setVoiceIdentityId(null);
    if (next === "kokoro") {
      applyPreset(PRESETS[0]);
    } else {
      setVoice(DEFAULT_DIA_NARRATOR);
      setActivePreset(null);
    }
  };

  const [pendingId, setPendingId] = useState<string | null>(null);
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // A completed clip is cleaned in place (same file, same URL) -- bumping a
  // per-job version and appending it as a query param forces the <audio>
  // element to actually re-fetch instead of quietly keeping the old bytes.
  const [cleanedVersions, setCleanedVersions] = useState<Record<string, number>>({});
  const [cleaningId, setCleaningId] = useState<string | null>(null);
  const [cleanError, setCleanError] = useState<string | null>(null);

  const audioUrlFor = (port: number, id: string) =>
    `${jobOutputUrl(port, id)}?v=${cleanedVersions[id] ?? 0}`;

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
  const canGenerate = !!text.trim() && (!pendingId || stuck) && engineReady;

  const applyPreset = (p: (typeof PRESETS)[number]) => {
    setActivePreset(p.id);
    setVoice(p.voice);
    setSpeed(p.speed);
  };

  const preview = async () => {
    const trimmed = text.trim();
    if (!trimmed || (pendingId && !stuck) || !engineReady) return;
    setSendError(null);
    try {
      const job = await submitJob({
        job_type: "tts",
        // `speed` has no effect on Dia (it has no rate control) -- sent
        // regardless since the sidecar simply ignores it for that engine,
        // same as the UI hides the slider rather than special-casing the
        // request body. `voice_identity_id` only makes sense for Dia (a
        // saved reference clip); omitted entirely rather than sent as
        // `null` when none is picked, or for Kokoro.
        params: {
          text: trimmed,
          engine,
          voice,
          speed,
          ...(engine === "dia" && voiceIdentityId ? { voice_identity_id: voiceIdentityId } : {}),
        },
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

  const handleClean = async (id: string) => {
    setCleaningId(id);
    setCleanError(null);
    try {
      await cleanAudio(id);
      setCleanedVersions((v) => ({ ...v, [id]: (v[id] ?? 0) + 1 }));
    } catch (err) {
      setCleanError(err instanceof Error ? err.message : String(err));
    } finally {
      setCleaningId(null);
    }
  };

  if (!ready) {
    return (
      <div className="voice">
        <section className="card voice__empty">
          <h2>The narrator needs a voice model</h2>
          <p className="muted">
            Import a narration engine to give Story Studio scenes an off-screen narrator. Both
            are one-time, fully local downloads — no cloud calls. Kokoro is fast and simple;
            Dia is slower but supports real non-verbal sounds and multi-speaker dialogue — pick
            either, or both.
          </p>
          <p className="voice__empty-subhead">Kokoro (fast, flat)</p>
          <ul className="voice__empty-list">
            <li className={hasVoiceModel ? "done" : ""}>
              {hasVoiceModel ? "✓" : "○"} Kokoro voice model
            </li>
            <li className={hasVoiceData ? "done" : ""}>
              {hasVoiceData ? "✓" : "○"} Kokoro voices (54 presets)
            </li>
          </ul>
          <p className="voice__empty-subhead">Dia (expressive, slower)</p>
          <ul className="voice__empty-list">
            <li className={hasDiaEngine ? "done" : ""}>
              {hasDiaEngine ? "✓" : "○"} Dia engine (9 files)
            </li>
            <li className={hasDiaCodec ? "done" : ""}>
              {hasDiaCodec ? "✓" : "○"} Dia audio codec — DAC (3 files)
            </li>
          </ul>
          <p className="voice__empty-hint">
            Models tab → Add models → Voice → “Download entire stack” gets every file for
            whichever engine you pick in one go.
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

        <div className="voice__engines" role="radiogroup" aria-label="Narration engine">
          <button
            type="button"
            role="radio"
            aria-checked={engine === "kokoro"}
            className={`voice__engine ${engine === "kokoro" ? "voice__engine--on" : ""}`}
            disabled={!kokoroReady}
            title={kokoroReady ? undefined : "Import Kokoro on the Models tab to unlock this"}
            onClick={() => selectEngine("kokoro")}
          >
            <span className="voice__engine-label">Kokoro</span>
            <span className="voice__engine-blurb">
              {kokoroReady ? "Fast, flat, 28 English voices" : "Not imported yet"}
            </span>
          </button>
          <button
            type="button"
            role="radio"
            aria-checked={engine === "dia"}
            className={`voice__engine ${engine === "dia" ? "voice__engine--on" : ""}`}
            disabled={!diaReady}
            title={diaReady ? undefined : "Import Dia on the Models tab to unlock this"}
            onClick={() => selectEngine("dia")}
          >
            <span className="voice__engine-label">Dia (expressive)</span>
            <span className="voice__engine-blurb">
              {diaReady
                ? "Real (laughs)/(sighs)/[S1] tags, slower"
                : "Models tab → Add models → Voice"}
            </span>
          </button>
        </div>

        {engine === "kokoro" && (
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
        )}

        <PromptAssistant
          kind="narrate"
          sessionId={null}
          onApplyPrompt={setText}
          narrateEngine={engine}
        />

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

          {engine === "kokoro" ? (
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
                      {voiceLabel(v)}
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
          ) : (
            <div className="voiceform__field">
              <span>Voice</span>
              <VoiceIdentityPicker value={voiceIdentityId} onChange={setVoiceIdentityId} />
              {voiceIdentityId ? (
                <span className="voice__hint">
                  Cloned from a saved reference clip — a real, chosen voice character, not just
                  a stable-but-generic seed. No speed control either way; real sounds like
                  (laughs)/(sighs) work via the assistant above or typed directly.
                </span>
              ) : (
                <>
                  <label className="voiceform__field">
                    <span>Narrator identity (free text)</span>
                    <input
                      type="text"
                      value={voice}
                      onChange={(e) => setVoice(e.target.value)}
                      placeholder={DEFAULT_DIA_NARRATOR}
                    />
                  </label>
                  <span className="voice__hint">
                    Dia has no named voices — reusing the same identity (e.g. "gravelly old
                    man") keeps repeat narration sounding like the same speaker, but it's still
                    a randomly-sampled voice, not a chosen one. Save a reference clip above (the
                    "+") for a real voice character. No speed control or freeform mood tags
                    either way; real sounds like (laughs)/(sighs) work via the assistant above
                    or typed directly.
                  </span>
                </>
              )}
            </div>
          )}

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
            audioUrlFor={audioUrlFor}
            onCancel={selected ? () => cancelJob(selected.id) : undefined}
            onDelete={selected ? () => handleDelete(selected.id) : undefined}
            onClean={selected ? () => handleClean(selected.id) : undefined}
            cleaning={!!selected && cleaningId === selected.id}
          />
          {cleanError && <p className="voice__err">{cleanError}</p>}
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
                  <audio controls preload="none" src={audioUrlFor(about.core_api_port, j.id)} />
                )}
                <button
                  type="button"
                  className="voice__hrow-clean"
                  title="Reduce background noise on this clip"
                  disabled={cleaningId === j.id}
                  onClick={() => handleClean(j.id)}
                >
                  {cleaningId === j.id ? "Cleaning…" : "Clean"}
                </button>
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
  audioUrlFor,
  onCancel,
  onDelete,
  onClean,
  cleaning,
}: {
  job: Job | null;
  port: number | null;
  audioUrlFor: (port: number, id: string) => string;
  onCancel?: () => void;
  onDelete?: () => void;
  onClean?: () => void;
  cleaning?: boolean;
}) {
  if (!job) return <p className="muted">Write a line and hit Preview.</p>;

  const running = !DONE.includes(job.state);
  const finished = job.state === "completed";

  return (
    <div className="voiceresult">
      <div className="voiceresult__canvas" data-state={job.state}>
        {finished && port != null ? (
          <audio controls autoPlay src={audioUrlFor(port, job.id)} />
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
      <div className="voiceresult__actions">
        {running && onCancel && (
          <button type="button" className="voiceresult__cancel" onClick={onCancel}>
            Stop
          </button>
        )}
        {finished && onClean && (
          <button
            type="button"
            className="voiceresult__cancel"
            onClick={onClean}
            disabled={cleaning}
            title="Reduce background noise on this clip"
          >
            {cleaning ? "Cleaning…" : "Clean audio"}
          </button>
        )}
        {!running && onDelete && (
          <button type="button" className="voiceresult__cancel" onClick={onDelete}>
            Delete
          </button>
        )}
      </div>
    </div>
  );
}
