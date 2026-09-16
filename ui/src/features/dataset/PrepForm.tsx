import { useId, useMemo, useState } from "react";
import { useCaptioners } from "../../lib/hooks";
import type { DatasetMode, DatasetPrepParams } from "../../lib/ipc";
import { browseForDirectory } from "./browse";

const DEFAULT_SAMPLE_FPS = 1.5;
const DEFAULT_BLUR_THRESHOLD = 100;
const DEFAULT_PHASH_MAX_DISTANCE = 6;
const DEFAULT_ESCALATE_EVERY_NTH = 20;
const DEFAULT_CONTEXT_OFFSET = 5;
const DEFAULT_MAX_FRAMES_PER_CLIP = 40;
const DEFAULT_MIN_CLIP_SECS = 2;

type Props = {
  /** A prep run is in flight — the Start button stays disabled until it ends. */
  isRunning: boolean;
  /** Why the last submit was rejected, shown under the button. */
  error: string | null;
  onStart: (params: DatasetPrepParams) => void;
};

/** The left-hand prep form: root folder, mode, sampling knobs, the captioning
 *  fieldset and the Start button. It owns the draft parameters and hands the
 *  finished {@link DatasetPrepParams} to the container, which submits them. */
export function PrepForm({ isRunning, error, onStart }: Props) {
  const { data: captioners } = useCaptioners();
  const installed = useMemo(() => (captioners ?? []).filter((c) => c.installed), [captioners]);

  const [root, setRoot] = useState("");
  const [mode, setMode] = useState<DatasetMode>("frames");
  const [sampleFps, setSampleFps] = useState(DEFAULT_SAMPLE_FPS);
  const [blurThreshold, setBlurThreshold] = useState(DEFAULT_BLUR_THRESHOLD);
  const [phashMaxDistance, setPhashMaxDistance] = useState(DEFAULT_PHASH_MAX_DISTANCE);
  const [maxFramesPerClip, setMaxFramesPerClip] = useState(DEFAULT_MAX_FRAMES_PER_CLIP);
  const [minClipSecs, setMinClipSecs] = useState(DEFAULT_MIN_CLIP_SECS);
  const [escalate, setEscalate] = useState(true);
  const [escalateEveryNth, setEscalateEveryNth] = useState(DEFAULT_ESCALATE_EVERY_NTH);
  const [contextOffset, setContextOffset] = useState(DEFAULT_CONTEXT_OFFSET);

  // Captioning is on by default, but only ever resolves to a captioner that is
  // actually installed -- with an empty library it stays off (and the checkbox
  // is disabled). Derived during render rather than synced by an effect, so
  // unchecking cannot be undone by the next captioner poll.
  const [captionWanted, setCaptionWanted] = useState(true);
  const [pickedCaptioner, setPickedCaptioner] = useState<string | null>(null);
  const captionerId = captionWanted
    ? (installed.find((c) => c.id === pickedCaptioner)?.id ?? installed[0]?.id ?? null)
    : null;
  const captionOn = captionerId !== null;
  const chosenCaptioner = installed.find((c) => c.id === captionerId) ?? null;

  const modeId = useId();
  const captionerSelectId = useId();

  const start = () => {
    const path = root.trim();
    if (!path) return;
    onStart({
      root: path,
      mode,
      captioner: captionerId,
      max_frames_per_clip: maxFramesPerClip,
      min_clip_secs: minClipSecs,
      sample_fps: sampleFps,
      blur_threshold: blurThreshold,
      phash_max_distance: phashMaxDistance,
      // Temporal escalation rides on top of a captioner that supports it.
      escalate: escalate && !!chosenCaptioner?.supports_escalation,
      escalate_every_nth: escalateEveryNth,
      context_offset: contextOffset,
    });
  };

  return (
    <div className="datasetform">
      <label className="datasetform__field">
        <span>Root folder</span>
        <div className="datasetform__row">
          <input
            type="text"
            value={root}
            onChange={(e) => setRoot(e.target.value)}
            placeholder="E:\Data\MyArtStyle"
          />
          <button type="button" className="chip" onClick={() => browseForDirectory(setRoot)}>
            Browse…
          </button>
        </div>
      </label>

      <label className="datasetform__field" htmlFor={modeId}>
        <span>Mode</span>
        <select id={modeId} value={mode} onChange={(e) => setMode(e.target.value as DatasetMode)}>
          <option value="frames">Frames (stills from video + images)</option>
          <option value="clips">Clips (whole videos, for video models)</option>
        </select>
      </label>

      <div className="datasetform__grid">
        <label className="datasetform__field">
          <span>Sample rate (fps)</span>
          <input
            type="number"
            min={0.1}
            max={10}
            step={0.1}
            value={sampleFps}
            onChange={(e) => setSampleFps(Number(e.target.value))}
          />
        </label>
        <label className="datasetform__field">
          <span>Blur threshold</span>
          <input
            type="number"
            min={0}
            max={10000}
            step={5}
            value={blurThreshold}
            onChange={(e) => setBlurThreshold(Number(e.target.value))}
          />
        </label>
        <label className="datasetform__field">
          <span>Duplicate distance</span>
          <input
            type="number"
            min={0}
            max={64}
            step={1}
            value={phashMaxDistance}
            onChange={(e) => setPhashMaxDistance(Number(e.target.value))}
          />
        </label>
        <label className="datasetform__field">
          <span>Max frames per clip (0 = all)</span>
          <input
            type="number"
            min={0}
            max={500}
            step={1}
            value={maxFramesPerClip}
            onChange={(e) => setMaxFramesPerClip(Number(e.target.value))}
          />
        </label>
        {mode === "clips" && (
          <label className="datasetform__field">
            <span>Min clip length (s)</span>
            <input
              type="number"
              min={0}
              max={600}
              step={0.5}
              value={minClipSecs}
              onChange={(e) => setMinClipSecs(Number(e.target.value))}
            />
          </label>
        )}
      </div>

      <fieldset className="datasetform__captioning">
        <legend>Auto-caption</legend>
        <label className="datasetform__check">
          <input
            type="checkbox"
            checked={captionOn}
            disabled={installed.length === 0}
            onChange={(e) => setCaptionWanted(e.target.checked)}
          />
          <span>
            Describe every kept frame automatically.{" "}
            <em>
              Recommended for style LoRAs: what is described stays controllable, what is not becomes
              part of the style.
            </em>
          </span>
        </label>

        {installed.length === 0 && (
          <p className="datasetform__hint">
            No captioner installed — import Florence-2 or the WD tagger on the Models tab. Without
            one, everything recurring in your frames flows into the trigger word.
          </p>
        )}

        {captionOn && (
          <label
            className="datasetform__field datasetform__field--inline"
            htmlFor={captionerSelectId}
          >
            <span>Describe with</span>
            <select
              id={captionerSelectId}
              value={captionerId ?? ""}
              onChange={(e) => setPickedCaptioner(e.target.value)}
            >
              {installed.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name} {c.style === "tags" ? "· tags" : "· prose"}
                </option>
              ))}
            </select>
          </label>
        )}

        {captionOn && chosenCaptioner?.supports_escalation && (
          <>
            <label className="datasetform__check">
              <input
                type="checkbox"
                checked={escalate}
                onChange={(e) => setEscalate(e.target.checked)}
              />
              <span>
                Escalate uncertain captions to Qwen2.5-VL with temporal context (frame vs. a later
                frame)
              </span>
            </label>
            {escalate && (
              <>
                <label className="datasetform__field datasetform__field--inline">
                  <span>Escalate every Nth frame too</span>
                  <input
                    type="number"
                    min={0}
                    max={500}
                    step={1}
                    value={escalateEveryNth}
                    onChange={(e) => setEscalateEveryNth(Number(e.target.value))}
                  />
                </label>
                <label className="datasetform__field datasetform__field--inline">
                  <span>Context offset (frames)</span>
                  <input
                    type="number"
                    min={1}
                    max={50}
                    step={1}
                    value={contextOffset}
                    onChange={(e) => setContextOffset(Number(e.target.value))}
                  />
                </label>
              </>
            )}
          </>
        )}
      </fieldset>

      <button
        type="button"
        className="datasetform__go"
        onClick={start}
        disabled={!root.trim() || isRunning}
      >
        {isRunning ? "Pipeline running…" : "Run pipeline"}
      </button>
      {error && <p className="dataset__err">{error}</p>}
    </div>
  );
}
