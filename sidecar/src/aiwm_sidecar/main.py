"""Sidecar entry point.

Speaks JSON-RPC 2.0 over stdio, one JSON object per line. ``handshake`` and
``ping`` let the Rust ``SidecarClient`` verify the connection.
``synthesize_speech`` renders text to speech with a local Kokoro ONNX model
(WP-9 -- the Story Studio narrator). ``inspect_model_file`` is declared in the
contract but not served until Phase 2.
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

CAPABILITIES: list[str] = ["synthesize_speech"]

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
_SENTENCE_SPLIT_RE = re.compile(r"(?<=[.!?…])\s+|\n+")


def _split_into_sentences(text: str) -> list[str]:
    parts = [p.strip() for p in _SENTENCE_SPLIT_RE.split(text)]
    return [p for p in parts if p]


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


def _synthesize_speech(params: dict[str, Any]) -> dict[str, Any]:
    text = str(params.get("text") or "").strip()
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
    sentences = _split_into_sentences(text) or [text]

    clips: list[np.ndarray] = []
    sample_rate = 0
    for sentence in sentences:
        samples, sample_rate = kokoro.create(sentence, voice=voice, speed=speed, lang=lang)
        clips.append(np.asarray(samples, dtype=np.float32))

    if len(clips) == 1:
        audio = clips[0]
    else:
        gap = np.zeros(int(_SENTENCE_GAP_SECS * sample_rate), dtype=np.float32)
        pieces: list[np.ndarray] = [clips[0]]
        for clip in clips[1:]:
            pieces.append(gap)
            pieces.append(clip)
        audio = np.concatenate(pieces)

    import soundfile as sf

    buf = io.BytesIO()
    sf.write(buf, audio, sample_rate, format="WAV")

    return {
        "audio_base64": base64.b64encode(buf.getvalue()).decode("ascii"),
        "sample_rate": sample_rate,
        "duration_secs": len(audio) / sample_rate,
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
            result = _synthesize_speech(params)
        except ValueError as e:
            return _error(req_id, _INVALID_PARAMS, str(e))
        except Exception as e:  # pragma: no cover - unexpected engine failure
            return _error(req_id, _INTERNAL_ERROR, f"synthesis failed: {e}")
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
