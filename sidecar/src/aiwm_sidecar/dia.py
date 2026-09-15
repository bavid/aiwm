"""The Dia-1.6B narrator engine (Nari Labs' `nari-labs/Dia-1.6B-0626`, via
mainline `transformers`) -- a second, more expressive narration engine
alongside Kokoro (`aiwm_sidecar.main`). Real capability, stated honestly:

* Recognized non-verbal tags -- `(laughs)`, `(sighs)`, `(clears throat)`, ...
  -- are genuine input the model understands, unlike a freeform emotional
  stage direction such as "(angry)", which no engine here can perform from
  text alone (see `_DIA_NONVERBAL_TAGS` below).
* `[S1]` / `[S2]` speaker tags produce actual multi-speaker dialogue turns
  (useful for the planned Story Studio feature); a plain narration line with
  no tag at all still works -- see `_with_speaker_tag`.
* Dia has no named voice presets the way Kokoro does. Without an audio
  prompt, it samples a speaker identity stochastically per generation -- so
  repeat narration would otherwise sound like a brand-new, unrelated speaker
  every single call. `_seed_for_narrator` fixes *that* case (a stable but
  still generic identity) by deriving a seed from the `voice` field
  (repurposed as a free-text "narrator identity" label).
* Real voice **character** -- e.g. "a specific old man with a scratchy,
  smokey voice" -- needs more than a seed: Dia's actual mechanism for it is
  audio-prompt voice cloning, given a short reference clip plus its own
  transcript (`reference_audio_path` / `reference_transcript` in
  `synthesize_dia`'s params; see `_resolve_reference`/`_DiaEngine.render`).
  The Rust core resolves a *named, saved* voice identity (a reference clip +
  transcript picked once and reused, `core::db::voice_identities`) into
  these two params before ever calling here -- this module only ever sees
  the raw path + transcript, never a "name".

Mirrors `aiwm_sidecar.main`'s `_load_kokoro`/`_construct_kokoro` cache
pattern via `_load_dia`/`_construct_dia`, in its own module for cohesion --
the real `transformers`/`torch` import only happens inside `_construct_dia`,
so nothing here requires the (multi-GB) model or even a working CUDA torch
install just to import this module.
"""

from __future__ import annotations

import base64
import hashlib
import io
import os
import re
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np

# `main` imports `dia` lazily (only once a request actually asks for the Dia
# engine -- see `_dispatch_synthesize_speech`), so by the time *this* import
# runs, `aiwm_sidecar.main` has always already finished loading. Importing
# it here (rather than duplicating the pause-markup splitter and text
# normalizer) keeps that logic in exactly one place for both engines.
from aiwm_sidecar import main as _main

# Dia's real, recognized non-verbal vocabulary (the model card / processor
# tokenizer's special tokens) -- distinct from `main._PAUSE_TAGS`, which
# convert to a timed silence and are never spoken. These are spoken *as
# tags*, verbatim, because Dia itself performs the sound from them.
_DIA_NONVERBAL_TAGS: frozenset[str] = frozenset(
    {
        "laughs",
        "clears throat",
        "sighs",
        "gasps",
        "coughs",
        "singing",
        "sings",
        "mumbles",
        "beep",
        "groans",
        "sniffs",
    }
)

_DEFAULT_NARRATOR = "narrator"
# DAC ("descript-audio-codec") 44kHz -- Dia's audio tokenizer/detokenizer,
# named directly in its own Hugging Face repo id.
_DAC_REPO_ID = "descript/dac_44khz"
_DIA_SAMPLE_RATE = 44_100

_SPEAKER_TAG_RE = re.compile(r"^\s*\[S[12]\]")


def _with_speaker_tag(text: str) -> str:
    """Dia needs an explicit `[S1]`/`[S2]` speaker tag to know whose turn it
    is. A plain single-narrator line the user typed with no tag at all must
    still work without asking them to learn Dia's syntax, so an untagged
    beat is auto-prefixed with `[S1]`. A beat that already starts with an
    explicit tag -- real `[S1]`/`[S2]` dialogue, the planned Story Studio
    multi-speaker case -- is left exactly as written."""
    return text if _SPEAKER_TAG_RE.match(text) else f"[S1] {text}"


def _seed_for_narrator(identity: str) -> int:
    """A stable seed derived from a free-text "narrator identity" label, so
    the same label reproduces (approximately) the same speaker across
    separate render calls -- see the module docstring for why this is
    necessary at all. Case/whitespace-insensitive so "Old Man" and "old
    man" -- almost certainly meant as the same character -- land on the
    same seed."""
    digest = hashlib.sha256(identity.strip().lower().encode("utf-8")).digest()
    return int.from_bytes(digest[:4], "big")


# Per the confirmed-working reference call, ~256 tokens produced ~2s of
# audio -- a documented ratio, not a guess.
_DIA_TOKENS_PER_SEC = 128.0
# A natural narration pace (~150-165 wpm); the estimate only sets an upper
# *bound* Dia's own end-of-sequence token can stop well short of, so this
# doesn't need to be exact.
_DIA_WORDS_PER_SEC = 2.6
_DIA_MAX_NEW_TOKENS_FLOOR = 256
_DIA_MAX_NEW_TOKENS_CEILING = 3072
# Generous headroom over the raw estimate -- an upper bound that's too low
# would truncate the clip mid-sentence, which is a worse failure than
# spending a bit more compute on a bound the model stops short of anyway.
_DIA_TOKEN_BUDGET_HEADROOM = 2.0


def _max_new_tokens_for(text: str) -> int:
    """An upper bound on generation length scaled to roughly how long `text`
    should take to say -- not a target duration Dia is forced to fill."""
    words = max(len(text.split()), 1)
    est_secs = words / _DIA_WORDS_PER_SEC
    tokens = int(est_secs * _DIA_TOKENS_PER_SEC * _DIA_TOKEN_BUDGET_HEADROOM)
    return max(_DIA_MAX_NEW_TOKENS_FLOOR, min(tokens, _DIA_MAX_NEW_TOKENS_CEILING))


def _stage_local_hub_cache(repo_id: str, source_dir: Path) -> Path:
    """Stages `source_dir`'s already-verified files into a local Hugging
    Face Hub cache layout for `repo_id`, so any `from_pretrained(repo_id)`
    call inside `transformers` -- not just a call this module makes
    directly -- resolves those exact files with zero network access.

    This exists for one reason: Dia's own weights load from a local
    directory we pass explicitly (`DiaForConditionalGeneration
    .from_pretrained(model_dir)`), but its audio codec (`descript/dac_44khz`,
    a *separate* Hugging Face repo) is looked up by repo id somewhere inside
    `AutoProcessor`'s own loading code, not a local path this module
    controls -- so the only way to keep that lookup from calling home is to
    make it resolve locally through the Hub's own cache mechanism, which is
    keyed by repo id, not by a path we can just hand it. `HF_HUB_OFFLINE`
    (set by `_construct_dia`) then makes a network fetch impossible even if
    this staging were somehow incomplete, instead of it silently succeeding
    online.

    Idempotent: re-staging the same source directory is a cheap no-op after
    the first call (hardlinked files, not copied, so this costs no extra
    disk for the ~300 MB codec).
    """
    cache_root = source_dir.parent / ".hf-cache"
    repo_cache_name = "models--" + repo_id.replace("/", "--")
    snapshot_dir = cache_root / repo_cache_name / "snapshots" / "local"
    refs_dir = cache_root / repo_cache_name / "refs"

    marker = snapshot_dir / ".aiwm-staged-from"
    if marker.is_file() and marker.read_text(encoding="utf-8") == str(source_dir):
        return cache_root

    snapshot_dir.mkdir(parents=True, exist_ok=True)
    refs_dir.mkdir(parents=True, exist_ok=True)
    for f in source_dir.iterdir():
        if not f.is_file():
            continue
        dest = snapshot_dir / f.name
        if dest.exists():
            continue
        try:
            os.link(f, dest)
        except OSError:
            shutil.copyfile(f, dest)
    (refs_dir / "main").write_text("local", encoding="utf-8")
    marker.write_text(str(source_dir), encoding="utf-8")
    return cache_root


class _DiaEngine:
    """Pairs a loaded Dia model with its processor behind one `.render()`
    call -- mirrors `kokoro_onnx.Kokoro`'s own single-method shape, so
    `synthesize_dia` stays engine-agnostic-looking and a test double
    (`FakeDia`) only needs to implement one method, exactly like
    `FakeKokoro` does for the Kokoro engine today."""

    def __init__(self, model: Any, processor: Any) -> None:
        self._model = model
        self._processor = processor

    @property
    def processor(self) -> Any:
        """Exposed so callers can read `.feature_extractor.sampling_rate`
        off it (see `_reference_sample_rate`) without reaching into a
        private attribute."""
        return self._processor

    def render(
        self, text: str, seed: int, reference_audio: np.ndarray | None = None
    ) -> tuple[np.ndarray, int]:
        """Renders `text` (already fully tagged/prompted by the caller).
        With `reference_audio` (a mono array at the feature extractor's own
        sample rate -- see `_resolve_reference`), this is Dia's real
        voice-cloning path: the processor is handed the reference clip
        alongside `text`, and `get_audio_prompt_len`/`batch_decode`'s
        `audio_prompt_len` strip the reference back out of the generated
        output, per Dia's own confirmed calling convention. Without it, this
        is the original seed-only path, unchanged."""
        import torch

        torch.manual_seed(seed)
        if torch.cuda.is_available():
            torch.cuda.manual_seed_all(seed)

        if reference_audio is None:
            inputs = self._processor(text=[text], padding=True, return_tensors="pt").to(
                self._model.device
            )
            outputs = self._model.generate(**inputs, max_new_tokens=_max_new_tokens_for(text))
            decoded = self._processor.batch_decode(outputs)
            samples = np.asarray(decoded[0], dtype=np.float32)
            return samples, _DIA_SAMPLE_RATE

        inputs = self._processor(
            text=text, audio=reference_audio, padding=True, return_tensors="pt"
        ).to(self._model.device)
        prompt_len = self._processor.get_audio_prompt_len(inputs["decoder_attention_mask"])
        outputs = self._model.generate(**inputs, max_new_tokens=_max_new_tokens_for(text))
        decoded = self._processor.batch_decode(outputs, audio_prompt_len=prompt_len)
        samples = np.asarray(decoded[0], dtype=np.float32)
        return samples, _DIA_SAMPLE_RATE


# One loaded Dia engine per (model_dir, dac_dir) -- loading the ~1.6B-param
# model onto the GPU is expensive; the sidecar is long-lived for the app's
# whole run, so that cost is paid once, exactly like Kokoro's own cache.
_dia_cache: dict[tuple[str, str], _DiaEngine] = {}


def _construct_dia(model_dir: str, dac_dir: str) -> _DiaEngine:
    import torch
    from transformers import AutoProcessor, DiaForConditionalGeneration

    cache_root = _stage_local_hub_cache(_DAC_REPO_ID, Path(dac_dir))
    # Belt and suspenders: the staged local cache above should already
    # satisfy any lookup, but forcing offline mode turns a staging bug into
    # a loud failure instead of a silent network call.
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
    os.environ["HUGGINGFACE_HUB_CACHE"] = str(cache_root)

    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = DiaForConditionalGeneration.from_pretrained(model_dir, dtype=torch.bfloat16).to(device)
    processor = AutoProcessor.from_pretrained(model_dir)
    return _DiaEngine(model, processor)


def _load_dia(model_dir: str, dac_dir: str) -> _DiaEngine:
    key = (model_dir, dac_dir)
    if key not in _dia_cache:
        _dia_cache[key] = _construct_dia(model_dir, dac_dir)
    return _dia_cache[key]


# --- voice cloning: reference audio + transcript ----------------------------
#
# Dia's real mechanism for a *chosen* voice character (not just a stable-but-
# generic seeded one, see `_seed_for_narrator` above): a short reference clip
# plus its own transcript, fed back to the model alongside the new text so it
# continues in that voice. `core::db::voice_identities` (Rust) is where a
# *named* identity is saved and reused; this module only ever receives the
# resolved path + transcript for one render call.

# `DiaFeatureExtractor.__init__`'s own default `sampling_rate` (confirmed
# from `transformers`' source, `models/dia/feature_extraction_dia.py`) --
# used only as a fallback if a loaded processor doesn't expose
# `.feature_extractor.sampling_rate` for some reason. Dia's *output* audio is
# 44.1kHz (`_DIA_SAMPLE_RATE`, the DAC codec's rate); the feature extractor
# that reads a *reference-audio prompt back in* is a different, lower rate --
# the two must never be confused.
_DIA_FEATURE_EXTRACTOR_SR_FALLBACK = 16_000


def _reference_sample_rate(processor: Any) -> int:
    """The sample rate Dia's feature extractor expects a reference-audio
    prompt at. Read live off the loaded processor rather than hardcoded, so
    a future `transformers` release changing this default doesn't silently
    resample a reference clip to the wrong rate."""
    feature_extractor = getattr(processor, "feature_extractor", None)
    rate = getattr(feature_extractor, "sampling_rate", None)
    if isinstance(rate, int | float) and rate > 0:
        return int(rate)
    return _DIA_FEATURE_EXTRACTOR_SR_FALLBACK


def _resample_to(samples: np.ndarray, orig_sr: int, target_sr: int) -> np.ndarray:
    """Resamples a mono reference clip to `target_sr` -- a no-op when it's
    already there (the common case: many reference clips will already be
    16kHz, or whatever the loaded feature extractor expects)."""
    if orig_sr == target_sr or len(samples) == 0:
        return samples
    from scipy.signal import resample

    target_len = max(1, round(len(samples) * target_sr / orig_sr))
    return np.asarray(resample(samples, target_len), dtype=np.float32)


def _read_reference_audio(path: Path) -> tuple[np.ndarray, int]:
    """Loads a reference clip via `soundfile` (already a sidecar dependency
    -- handles arbitrary input sample rates/formats), collapsing to mono if
    the file has more than one channel (Dia's feature extractor expects a
    single channel)."""
    import soundfile as sf

    samples, sample_rate = sf.read(str(path), dtype="float32", always_2d=False)
    samples = np.asarray(samples, dtype=np.float32)
    if samples.ndim > 1:
        samples = samples.mean(axis=1).astype(np.float32)
    return samples, sample_rate


@dataclass(frozen=True)
class _ReferenceVoice:
    samples: np.ndarray
    transcript: str


def _reference_inputs(params: dict[str, Any]) -> tuple[str, str] | None:
    """Validates the `reference_audio_path` / `reference_transcript` pair up
    front, before the expensive model load -- `None` when neither is given
    (today's seed-only path); the pair when both are. Exactly one given is a
    clear error rather than a silent fall-back to the seed-only path."""
    audio_path = str(params.get("reference_audio_path") or "").strip()
    transcript = str(params.get("reference_transcript") or "").strip()
    if not audio_path and not transcript:
        return None
    if not audio_path or not transcript:
        raise ValueError(
            "`reference_audio_path` and `reference_transcript` must be given together"
        )
    if not Path(audio_path).is_file():
        raise ValueError(f"reference audio file not found: {audio_path}")
    return audio_path, transcript


def _resolve_reference(
    reference_inputs: tuple[str, str] | None, engine: _DiaEngine
) -> _ReferenceVoice | None:
    """Loads and resamples the validated reference clip (see
    `_reference_inputs`) to whatever sample rate `engine`'s feature
    extractor actually expects. `None` in, `None` out -- the seed-only path."""
    if reference_inputs is None:
        return None
    audio_path, transcript = reference_inputs
    samples, native_rate = _read_reference_audio(Path(audio_path))
    target_rate = _reference_sample_rate(engine.processor)
    resampled = _resample_to(samples, native_rate, target_rate)
    return _ReferenceVoice(samples=resampled, transcript=transcript)


def synthesize_dia(params: dict[str, Any]) -> dict[str, Any]:
    """The Dia counterpart to `main._synthesize_speech` -- same JSON-RPC
    result shape, same pause-markup beat-splitting, but a directory path
    per model component (not two file paths) and Dia's own non-verbal tags
    and speaker-tag handling on top."""
    text = _main._normalize_for_speech(str(params.get("text") or "")).strip()
    if not text:
        raise ValueError("`text` must not be empty")

    model_dir = params.get("model_dir")
    dac_dir = params.get("dac_dir")
    if not model_dir or not dac_dir:
        raise ValueError("`model_dir` and `dac_dir` are required")
    if not Path(model_dir).is_dir():
        raise ValueError(f"Dia model directory not found: {model_dir}")
    if not Path(dac_dir).is_dir():
        raise ValueError(f"DAC codec directory not found: {dac_dir}")
    # Validated up front, before the (potentially first-ever, multi-GB) model
    # load below -- a bad reference clip shouldn't cost that load just to fail.
    reference_inputs = _reference_inputs(params)

    narrator = str(params.get("voice") or _DEFAULT_NARRATOR)
    seed = _seed_for_narrator(narrator)
    engine = _load_dia(model_dir, dac_dir)
    reference = _resolve_reference(reference_inputs, engine)

    beats = _main._split_into_beats(text, keep_tags=_DIA_NONVERBAL_TAGS) or [
        (_main._strip_unspoken_markup(text, keep_tags=_DIA_NONVERBAL_TAGS).strip() or text, 0.0)
    ]

    clips: list[np.ndarray] = []
    gaps_after: list[float] = []
    sample_rate = _DIA_SAMPLE_RATE
    for spoken, gap_after in beats:
        tagged = _with_speaker_tag(spoken)
        if reference is not None:
            # Dia's own confirmed calling convention: the reference clip's
            # transcript and the new text share one prompt, both tagged as
            # the same speaker (a continuation, not a new turn) -- see
            # `_DiaEngine.render`'s docstring for what happens with this text.
            prompt_text = f"[S1] {reference.transcript} {tagged}"
            samples, sample_rate = engine.render(
                prompt_text, seed, reference_audio=reference.samples
            )
        else:
            samples, sample_rate = engine.render(tagged, seed)
        clips.append(np.asarray(samples, dtype=np.float32))
        gaps_after.append(gap_after)

    if len(clips) == 1:
        audio = clips[0]
    else:
        pieces: list[np.ndarray] = [clips[0]]
        for i in range(1, len(clips)):
            gap_secs = gaps_after[i - 1]
            if gap_secs > 0:
                pieces.append(np.zeros(int(gap_secs * sample_rate), dtype=np.float32))
            pieces.append(clips[i])
        audio = np.concatenate(pieces)

    import soundfile as sf

    buf = io.BytesIO()
    sf.write(buf, audio, sample_rate, format="WAV")

    return {
        "audio_base64": base64.b64encode(buf.getvalue()).decode("ascii"),
        "sample_rate": sample_rate,
        "duration_secs": len(audio) / sample_rate,
    }
