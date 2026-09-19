import { useCallback, useId, useMemo, useRef, useState } from "react";
import { useCaptioners } from "../../lib/hooks";
import type { DatasetMode, DatasetPrepParams } from "../../lib/ipc";
import { browseForDirectory } from "./browse";
import { CaptionerHint, CaptionerPicker } from "./CaptionerSetup";
import { useCaptionerInstall } from "./useCaptionerInstall";

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
  /** Opens the Models tab's "Training & captioning" section. */
  onMoreCaptioners: () => void;
};

/** The left-hand prep form: root folder, mode, sampling knobs, the captioning
 *  fieldset and the Start button. It owns the draft parameters and hands the
 *  finished {@link DatasetPrepParams} to the container, which submits them. */
export function PrepForm({ isRunning, error, onStart, onMoreCaptioners }: Props) {
  const { data: captioners, refetch: refetchCaptioners } = useCaptioners();
  // Selectable = installed and without a known issue (Florence-2 on
  // transformers 5.x would only fail after the whole extraction).
  const installed = useMemo(
    () => (captioners ?? []).filter((c) => c.installed && !c.known_issue),
    [captioners],
  );
  const noneInstalled = captioners !== null && installed.length === 0;

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

  // One-click captioner installs: see useCaptionerInstall for when a finished
  // install may change this form.
  const captioningRef = useRef<HTMLFieldSetElement>(null);
  const choose = useCallback((captionerId: string) => {
    setPickedCaptioner(captionerId);
    setCaptionWanted(true);
  }, []);
  const install = useCaptionerInstall({
    captioners,
    refetchCaptioners,
    onChosen: choose,
    containerRef: captioningRef,
  });

  const modeId = useId();
  const escalateNoteId = useId();

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

      <fieldset className="datasetform__captioning" ref={captioningRef}>
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

        {noneInstalled && (
          <CaptionerHint
            stacks={install.stacks}
            installer={install.installer}
            settling={install.settling}
            onInstall={install.startInstall}
            onMoreCaptioners={onMoreCaptioners}
          />
        )}

        {/* The one announcement of a finished install; focus goes to the
            newly selected radio, not to this text. */}
        <p className="datasetform__installed" role="status">
          {install.notice}
        </p>

        {captionOn && captioners && (
          <CaptionerPicker
            captioners={captioners}
            selectedId={captionerId}
            onSelect={setPickedCaptioner}
            stacks={install.stacks}
            installer={install.installer}
            settling={install.settling}
            onInstall={install.startInstall}
            onMoreCaptioners={onMoreCaptioners}
          />
        )}

        {captionOn && chosenCaptioner && (
          <>
            <label className="datasetform__check">
              <input
                type="checkbox"
                checked={escalate && chosenCaptioner.supports_escalation}
                disabled={!chosenCaptioner.supports_escalation}
                aria-describedby={chosenCaptioner.supports_escalation ? undefined : escalateNoteId}
                onChange={(e) => setEscalate(e.target.checked)}
              />
              <span>
                Escalate uncertain captions to Qwen2.5-VL with temporal context (frame vs. a later
                frame)
                {!chosenCaptioner.supports_escalation && (
                  <em id={escalateNoteId}>
                    Only a prose captioner can escalate — {chosenCaptioner.name} writes tags, so
                    there is no uncertain sentence to re-check.
                  </em>
                )}
              </span>
            </label>
            {escalate && chosenCaptioner.supports_escalation && (
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
      {error && (
        <p className="dataset__err" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
