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
  prompt (voice cloning, not implemented here yet), it samples a speaker
  identity stochastically per generation -- so repeat narration would
  otherwise sound like a brand-new, unrelated speaker every single call.
  `_seed_for_narrator` fixes that by deriving a stable seed from the
  `voice` field (repurposed here as a free-text "narrator identity" label,
  since Dia has no voice-id list to pick from).

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

    def render(self, text: str, seed: int) -> tuple[np.ndarray, int]:
        import torch

        torch.manual_seed(seed)
        if torch.cuda.is_available():
            torch.cuda.manual_seed_all(seed)

        inputs = self._processor(text=[text], padding=True, return_tensors="pt").to(
            self._model.device
        )
        outputs = self._model.generate(**inputs, max_new_tokens=_max_new_tokens_for(text))
        decoded = self._processor.batch_decode(outputs)
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

    narrator = str(params.get("voice") or _DEFAULT_NARRATOR)
    seed = _seed_for_narrator(narrator)
    engine = _load_dia(model_dir, dac_dir)

    beats = _main._split_into_beats(text, keep_tags=_DIA_NONVERBAL_TAGS) or [
        (_main._strip_unspoken_markup(text, keep_tags=_DIA_NONVERBAL_TAGS).strip() or text, 0.0)
    ]

    clips: list[np.ndarray] = []
    gaps_after: list[float] = []
    sample_rate = _DIA_SAMPLE_RATE
    for spoken, gap_after in beats:
        samples, sample_rate = engine.render(_with_speaker_tag(spoken), seed)
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
