"""`synthesize_speech` -- text-to-speech via a local Kokoro ONNX model.

The real Kokoro engine (and its ~140 MB of model files) is never required for
these tests: `_load_kokoro` is monkeypatched with a small fake that returns
known samples, so the tests exercise the JSON-RPC contract (param validation,
base64 encoding, error shapes) rather than the ONNX runtime itself.
"""

from __future__ import annotations

import base64
import io
import wave
from pathlib import Path
from typing import Any

import pytest

from aiwm_sidecar import main


class FakeKokoro:
    """Stands in for `kokoro_onnx.Kokoro` -- records the call it received and
    returns a short, deterministic sine-free block of silence."""

    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []

    def create(self, text: str, voice: str, speed: float, lang: str):
        self.calls.append({"text": text, "voice": voice, "speed": speed, "lang": lang})
        sample_rate = 24000
        samples = [0.0] * (sample_rate // 2)  # 0.5s of silence
        return samples, sample_rate


@pytest.fixture
def model_files(tmp_path: Path) -> tuple[str, str]:
    """Placeholder files -- `_load_kokoro` is faked, so their content never
    matters, only that a real path exists for the existence check."""
    model = tmp_path / "kokoro-v1.0.int8.onnx"
    voices = tmp_path / "voices-v1.0.bin"
    model.write_bytes(b"")
    voices.write_bytes(b"")
    return str(model), str(voices)


@pytest.fixture(autouse=True)
def clear_kokoro_cache():
    main._kokoro_cache.clear()
    yield
    main._kokoro_cache.clear()


def request_synth(params: dict[str, Any]) -> dict[str, Any]:
    req = {"jsonrpc": "2.0", "id": 1, "method": "synthesize_speech", "params": params}
    return handle_or_raise(req)


def handle_or_raise(req: dict[str, Any]) -> dict[str, Any]:
    resp = main.handle(req)
    assert resp is not None
    return resp


def test_capability_is_advertised():
    assert "synthesize_speech" in main.CAPABILITIES


def test_rejects_empty_text(model_files: tuple[str, str]):
    model_path, voices_path = model_files
    resp = request_synth({"text": "   ", "model_path": model_path, "voices_path": voices_path})
    assert resp["error"]["code"] == -32602
    assert "text" in resp["error"]["message"]


def test_requires_model_and_voices_paths():
    resp = request_synth({"text": "hello"})
    assert resp["error"]["code"] == -32602


def test_rejects_a_missing_model_file(tmp_path: Path):
    voices = tmp_path / "voices-v1.0.bin"
    voices.write_bytes(b"")
    resp = request_synth(
        {"text": "hello", "model_path": str(tmp_path / "nope.onnx"), "voices_path": str(voices)}
    )
    assert resp["error"]["code"] == -32602
    assert "not found" in resp["error"]["message"]


def test_synthesizes_and_returns_base64_wav(monkeypatch: pytest.MonkeyPatch, model_files):
    model_path, voices_path = model_files
    fake = FakeKokoro()
    monkeypatch.setattr(main, "_load_kokoro", lambda *_: fake)

    resp = request_synth(
        {
            "text": "From the mist, a story stirs.",
            "model_path": model_path,
            "voices_path": voices_path,
            "voice": "am_fenrir",
            "speed": 0.9,
        }
    )

    result = resp["result"]
    assert result["sample_rate"] == 24000
    assert result["duration_secs"] == pytest.approx(0.5)

    audio = base64.b64decode(result["audio_base64"])
    with wave.open(io.BytesIO(audio), "rb") as w:
        assert w.getnchannels() == 1
        assert w.getframerate() == 24000
        assert w.getnframes() == 12000

    assert fake.calls == [
        {
            "text": "From the mist, a story stirs.",
            "voice": "am_fenrir",
            "speed": 0.9,
            "lang": "en-us",
        }
    ]


def test_defaults_voice_speed_and_lang_when_omitted(monkeypatch: pytest.MonkeyPatch, model_files):
    model_path, voices_path = model_files
    fake = FakeKokoro()
    monkeypatch.setattr(main, "_load_kokoro", lambda *_: fake)

    request_synth({"text": "hello", "model_path": model_path, "voices_path": voices_path})

    assert fake.calls[0]["voice"] == "am_michael"
    assert fake.calls[0]["speed"] == 1.0
    assert fake.calls[0]["lang"] == "en-us"


def test_caches_the_loaded_engine_across_calls(monkeypatch: pytest.MonkeyPatch, model_files):
    """`_load_kokoro` itself does the caching; only the expensive constructor
    (`_construct_kokoro`) is faked here, so this actually exercises the cache
    instead of assuming it away."""
    model_path, voices_path = model_files
    construct_calls: list[tuple[str, str]] = []

    def fake_construct(m: str, v: str) -> FakeKokoro:
        construct_calls.append((m, v))
        return FakeKokoro()

    monkeypatch.setattr(main, "_construct_kokoro", fake_construct)

    request_synth({"text": "one", "model_path": model_path, "voices_path": voices_path})
    request_synth({"text": "two", "model_path": model_path, "voices_path": voices_path})

    assert construct_calls == [(model_path, voices_path)]
