"""Sidecar entry point.

Speaks JSON-RPC 2.0 over stdio, one JSON object per line. ``handshake`` and
``ping`` let the Rust ``SidecarClient`` verify the connection.
``synthesize_speech`` renders text to speech (WP-9 -- the Story Studio
narrator), on one of two engines picked by its ``engine`` param: the default
``"kokoro"`` (a local ONNX model, handled in this file) or ``"dia"`` (Nari
Labs' Dia-1.6B via `transformers`, a more expressive second engine with real
non-verbal tags and multi-speaker turns -- see ``aiwm_sidecar.dia``).
``inspect_model_file`` is declared in the contract but not served until
Phase 2.
"""

from __future__ import annotations

import base64
import io
import json
import re
import sys
from pathlib import Path
from typing import Any

import numpy as np

from aiwm_sidecar import PROTOCOL_VERSION, __version__

CAPABILITIES: list[str] = [
    "synthesize_speech",
    "clean_audio",
    "caption_frame",
    "caption_frame_pair",
]

_METHOD_NOT_FOUND = -32601
_INVALID_PARAMS = -32602
_NOT_IMPLEMENTED = -32001
_INTERNAL_ERROR = -32000

# Methods declared in the contract but not served yet.
_PLANNED = {"inspect_model_file"}

_DEFAULT_VOICE = "am_michael"
_DEFAULT_SPEED = 1.0
_DEFAULT_LANG = "en-us"

# Fed one long line, Kokoro reads it in a single flat, rushed breath -- it has
# no concept of a sentence boundary beyond whatever pause its own phonemizer
# infers from punctuation. Synthesizing sentence-by-sentence and stitching in
# a real silence gap gives each sentence its own natural start/end inflection
# instead of one monotone run-on -- the cheapest lever on "sounds AI-ish"
# available without swapping the model itself.
_SENTENCE_GAP_SECS = 0.35

# Neither Kokoro nor Dia understands a freeform stage direction like
# "(angry)" -- there is no model here that performs emotion from text alone
# (Dia's own real vocabulary is a fixed set of non-verbal sounds -- see
# `aiwm_sidecar.dia._DIA_NONVERBAL_TAGS` -- not an open-ended emotion/tone
# control). What *is* real: a precisely-timed silence, which this code
# controls directly regardless of engine. So a small, fixed vocabulary of
# pause markers gets a genuine gap; anything else in parentheses is stripped
# before it reaches the engine, since otherwise it just gets read aloud as
# literal words ("open paren angry close paren") -- except Dia's own real
# tags, which `_strip_unspoken_markup`'s `keep_tags` lets through verbatim.
_PAUSE_TAGS: dict[str, float] = {
    "pause": 0.35,
    "beat": 0.35,
    "breath": 0.55,
    "long pause": 0.9,
    "dramatic pause": 0.9,
}
# A local chat model (the narration prompt assistant) writes with "smart"
# typographic punctuation by default -- Kokoro's phonemizer doesn't
# recognize any of it, so it either mispronounces it or tries to read the
# character literally. Everything reaching an engine is plain ASCII.
_ASCII_PUNCTUATION: dict[str, str] = {
    "—": ", ",  # em dash -- read as a pause, not a word
    "–": "-",  # en dash
    "‘": "'",
    "’": "'",
    "“": '"',
    "”": '"',
    "…": "...",  # ellipsis character
    " ": " ",  # non-breaking space
}
_ASCII_PUNCTUATION_RE = re.compile("|".join(re.escape(k) for k in _ASCII_PUNCTUATION))


def _normalize_for_speech(text: str) -> str:
    return _ASCII_PUNCTUATION_RE.sub(lambda m: _ASCII_PUNCTUATION[m.group(0)], text)


_PAUSE_TAG_ALTERNATION = "|".join(re.escape(k) for k in sorted(_PAUSE_TAGS, key=len, reverse=True))
_PAUSE_TAG_RE = re.compile(rf"\(\s*({_PAUSE_TAG_ALTERNATION})\s*\)", re.IGNORECASE)
_UNSUPPORTED_TAG_RE = re.compile(r"\([^()]{1,60}\)")
_DELIM_RE = re.compile(
    rf"{_PAUSE_TAG_RE.pattern}|(?P<sentence_end>[.!?…]+)|(?P<newline>\n+)",
    re.IGNORECASE,
)


def _strip_unspoken_markup(text: str, keep_tags: frozenset[str] = frozenset()) -> str:
    """Removes any remaining parenthetical annotation (an unsupported
    emotion tag, a typo'd pause tag) that isn't one of the real, actionable
    pause markers -- those are consumed as delimiters before this ever runs.

    `keep_tags` lets a caller pass through its own real, recognized tags
    verbatim instead of stripping them -- Dia's actual non-verbal vocabulary
    ("(laughs)", "(sighs)", ...) is real input the model understands, not an
    unsupported stage direction. Kokoro's call sites pass none, so its
    behavior is exactly what it was before this parameter existed."""

    def _keep_or_drop(m: re.Match[str]) -> str:
        inner = m.group(0)[1:-1].strip().lower()
        return m.group(0) if inner in keep_tags else ""

    return _UNSUPPORTED_TAG_RE.sub(_keep_or_drop, text)


def _split_into_beats(
    text: str, keep_tags: frozenset[str] = frozenset()
) -> list[tuple[str, float]]:
    """Splits narration text into (spoken_text, gap_after_seconds) beats on
    sentence boundaries and recognized pause markup ("(pause)", "(breath)",
    "(dramatic pause)", ...). A recognized tag contributes a real silence at
    that exact point and is never spoken; anything else in parentheses is
    stripped rather than read aloud literally -- unless it's in `keep_tags`
    (see `_strip_unspoken_markup`), in which case it's kept verbatim in the
    spoken text instead."""
    beats: list[tuple[str, float]] = []
    pos = 0
    current = ""
    for m in _DELIM_RE.finditer(text):
        current += text[pos : m.start()]
        pos = m.end()
        if m.lastgroup == "sentence_end":
            current += m.group("sentence_end")
            gap = _SENTENCE_GAP_SECS
        elif m.lastgroup == "newline":
            gap = _SENTENCE_GAP_SECS
        else:
            gap = _PAUSE_TAGS[m.group(1).lower()]
        spoken = _strip_unspoken_markup(current, keep_tags).strip()
        current = ""
        if spoken:
            beats.append((spoken, gap))
        elif beats:
            prev_text, prev_gap = beats[-1]
            beats[-1] = (prev_text, max(prev_gap, gap))
    tail = _strip_unspoken_markup(current + text[pos:], keep_tags).strip()
    if tail:
        beats.append((tail, 0.0))
    return beats


# One loaded Kokoro engine per (model_path, voices_path) -- constructing it
# costs about a second (loading the ONNX session); the sidecar is long-lived
# for the app's whole run, so that cost is paid once per model pair instead
# of on every line of narration.
_kokoro_cache: dict[tuple[str, str], Any] = {}


def _construct_kokoro(model_path: str, voices_path: str) -> Any:
    from kokoro_onnx import Kokoro

    return Kokoro(model_path, voices_path)


def _load_kokoro(model_path: str, voices_path: str) -> Any:
    key = (model_path, voices_path)
    if key not in _kokoro_cache:
        _kokoro_cache[key] = _construct_kokoro(model_path, voices_path)
    return _kokoro_cache[key]


_DEFAULT_ENGINE = "kokoro"
_ENGINES = frozenset({"kokoro", "dia"})


def _dispatch_synthesize_speech(params: dict[str, Any]) -> dict[str, Any]:
    """Picks the narration engine (`"kokoro"`, the default and only engine
    before this, or `"dia"`) and hands off to its own synthesis function.
    Dia's module is imported lazily, here, rather than at the top of this
    file -- `dia.py` itself imports this module (to reuse the pause-markup
    splitter and text normalizer, `keep_tags` and all), so importing it
    eagerly at module load time would be a circular import. By the time
    this function actually runs, `main` has already finished loading, so
    the lazy import resolves without trouble."""
    engine = str(params.get("engine") or _DEFAULT_ENGINE).strip().lower()
    if engine not in _ENGINES:
        raise ValueError(f"unknown tts engine: {engine!r} (expected kokoro or dia)")
    if engine == "dia":
        from aiwm_sidecar.dia import synthesize_dia

        return synthesize_dia(params)
    return _synthesize_speech(params)


def _synthesize_speech(params: dict[str, Any]) -> dict[str, Any]:
    text = _normalize_for_speech(str(params.get("text") or "")).strip()
    if not text:
        raise ValueError("`text` must not be empty")

    model_path = params.get("model_path")
    voices_path = params.get("voices_path")
    if not model_path or not voices_path:
        raise ValueError("`model_path` and `voices_path` are required")
    if not Path(model_path).is_file():
        raise ValueError(f"model file not found: {model_path}")
    if not Path(voices_path).is_file():
        raise ValueError(f"voices file not found: {voices_path}")

    voice = str(params.get("voice") or _DEFAULT_VOICE)
    speed = float(params.get("speed") or _DEFAULT_SPEED)
    lang = str(params.get("lang") or _DEFAULT_LANG)

    kokoro = _load_kokoro(model_path, voices_path)
    beats = _split_into_beats(text) or [(_strip_unspoken_markup(text).strip() or text, 0.0)]

    clips: list[np.ndarray] = []
    gaps_after: list[float] = []
    sample_rate = 0
    for spoken, gap_after in beats:
        samples, sample_rate = kokoro.create(spoken, voice=voice, speed=speed, lang=lang)
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


# A gentle high-pass cutoff -- narration has no meaningful content below
# this, so it's a safe place to remove sub-audible rumble/DC drift without
# touching speech.
_HIGH_PASS_HZ = 80.0
_PEAK_CEILING = 0.98


def _clean_audio(params: dict[str, Any]) -> dict[str, Any]:
    """Post-processes an already-rendered clip: DC-offset removal, a gentle
    high-pass filter, and spectral-gate noise reduction (via `noisereduce`,
    which estimates the noise profile from the signal itself -- no separate
    noise sample needed). Runs on whatever's on disk already; this is not a
    new render, so it has no `voice`/`speed`/model concerns at all."""
    audio_b64 = params.get("audio_base64")
    if not audio_b64 or not isinstance(audio_b64, str):
        raise ValueError("`audio_base64` must not be empty")

    import soundfile as sf

    try:
        raw = base64.b64decode(audio_b64, validate=True)
        samples, sample_rate = sf.read(io.BytesIO(raw), dtype="float32")
    except Exception as e:
        raise ValueError(f"`audio_base64` is not a decodable WAV: {e}") from e

    import noisereduce as nr
    from scipy.signal import butter, sosfiltfilt

    samples = samples - np.mean(samples)
    sos = butter(2, _HIGH_PASS_HZ, btype="highpass", fs=sample_rate, output="sos")
    samples = sosfiltfilt(sos, samples).astype(np.float32)
    # Pinned explicitly (matches noisereduce's own defaults today) rather
    # than left implicit -- measured against real Kokoro int8-vs-fp32 output,
    # this combination cut error against the fp32 reference by ~25%; a
    # future library version changing its defaults shouldn't silently change
    # this pipeline's behavior.
    samples = nr.reduce_noise(
        y=samples, sr=sample_rate, stationary=False, prop_decrease=1.0
    ).astype(np.float32)

    peak = float(np.max(np.abs(samples))) if len(samples) else 0.0
    if peak > _PEAK_CEILING:
        samples = samples * (_PEAK_CEILING / peak)

    buf = io.BytesIO()
    sf.write(buf, samples, sample_rate, format="WAV")
    return {
        "audio_base64": base64.b64encode(buf.getvalue()).decode("ascii"),
        "sample_rate": sample_rate,
        "duration_secs": len(samples) / sample_rate,
    }


def _error(req_id: Any, code: int, message: str) -> dict[str, Any]:
    return {"jsonrpc": "2.0", "id": req_id, "error": {"code": code, "message": message}}


def handle(req: dict[str, Any]) -> dict[str, Any] | None:
    """Turn one JSON-RPC request into one response (or None for notifications)."""
    method = req.get("method")
    req_id = req.get("id")
    params = req.get("params") or {}

    if method == "handshake":
        result: Any = {
            "sidecar_version": __version__,
            "protocol_version": PROTOCOL_VERSION,
            "capabilities": CAPABILITIES,
        }
    elif method == "ping":
        result = "pong"
    elif method == "synthesize_speech":
        try:
            result = _dispatch_synthesize_speech(params)
        except ValueError as e:
            return _error(req_id, _INVALID_PARAMS, str(e))
        except Exception as e:  # pragma: no cover - unexpected engine failure
            return _error(req_id, _INTERNAL_ERROR, f"synthesis failed: {e}")
    elif method == "clean_audio":
        try:
            result = _clean_audio(params)
        except ValueError as e:
            return _error(req_id, _INVALID_PARAMS, str(e))
        except Exception as e:  # pragma: no cover - unexpected engine failure
            return _error(req_id, _INTERNAL_ERROR, f"cleanup failed: {e}")
    elif method == "caption_frame":
        from aiwm_sidecar.vision import caption_frame

        try:
            result = caption_frame(params)
        except ValueError as e:
            return _error(req_id, _INVALID_PARAMS, str(e))
        except Exception as e:  # pragma: no cover - unexpected engine failure
            return _error(req_id, _INTERNAL_ERROR, f"captioning failed: {e}")
    elif method == "caption_frame_pair":
        from aiwm_sidecar.vision import caption_frame_pair

        try:
            result = caption_frame_pair(params)
        except ValueError as e:
            return _error(req_id, _INVALID_PARAMS, str(e))
        except Exception as e:  # pragma: no cover - unexpected engine failure
            return _error(req_id, _INTERNAL_ERROR, f"captioning failed: {e}")
    elif method in _PLANNED:
        return _error(req_id, _NOT_IMPLEMENTED, f"{method} is not implemented yet")
    else:
        return _error(req_id, _METHOD_NOT_FOUND, f"method not found: {method}")

    if req_id is None:
        return None
    return {"jsonrpc": "2.0", "id": req_id, "result": result}


def main() -> None:
    for raw in sys.stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            req = json.loads(raw)
        except json.JSONDecodeError:
            continue
        resp = handle(req)
        if resp is not None:
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
