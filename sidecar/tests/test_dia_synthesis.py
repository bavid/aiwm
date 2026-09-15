"""`synthesize_speech` with `engine="dia"` -- Nari Labs' Dia-1.6B narrator.

The real Dia model (~6.4 GB, GPU-resident) is never required for these
tests: `dia._load_dia` is monkeypatched with `FakeDia`, a small test double
mirroring `FakeKokoro` in `test_synthesize_speech.py` -- so these tests
exercise the JSON-RPC contract, the pause/non-verbal-tag handling, the
`[S1]` auto-prefixing, and the narrator-identity seeding, not the real
`transformers`/`torch` model.
"""

from __future__ import annotations

import base64
import io
import wave
from pathlib import Path
from typing import Any

import pytest

from aiwm_sidecar import dia, main


class FakeDia:
    """Stands in for `dia._DiaEngine` -- records the call it received and
    returns a short, deterministic block of silence."""

    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []

    def render(self, text: str, seed: int):
        self.calls.append({"text": text, "seed": seed})
        sample_rate = 44_100
        samples = [0.0] * (sample_rate // 2)  # 0.5s of silence
        return samples, sample_rate


@pytest.fixture
def dia_dirs(tmp_path: Path) -> tuple[str, str]:
    """Placeholder directories -- `dia._load_dia` is faked, so their
    contents never matter, only that real directories exist for the
    existence check `synthesize_dia` does up front."""
    model_dir = tmp_path / "dia-engine"
    dac_dir = tmp_path / "dia-codec"
    model_dir.mkdir()
    dac_dir.mkdir()
    return str(model_dir), str(dac_dir)


@pytest.fixture(autouse=True)
def clear_dia_cache():
    dia._dia_cache.clear()
    yield
    dia._dia_cache.clear()


def request_dia(params: dict[str, Any]) -> dict[str, Any]:
    body = {"engine": "dia", **params}
    req = {"jsonrpc": "2.0", "id": 1, "method": "synthesize_speech", "params": body}
    resp = main.handle(req)
    assert resp is not None
    return resp


def test_rejects_empty_text(dia_dirs: tuple[str, str]):
    model_dir, dac_dir = dia_dirs
    resp = request_dia({"text": "   ", "model_dir": model_dir, "dac_dir": dac_dir})
    assert resp["error"]["code"] == -32602
    assert "text" in resp["error"]["message"]


def test_requires_model_and_dac_dirs():
    resp = request_dia({"text": "hello"})
    assert resp["error"]["code"] == -32602


def test_rejects_a_missing_model_dir(tmp_path: Path):
    dac_dir = tmp_path / "dia-codec"
    dac_dir.mkdir()
    resp = request_dia(
        {"text": "hello", "model_dir": str(tmp_path / "nope"), "dac_dir": str(dac_dir)}
    )
    assert resp["error"]["code"] == -32602
    assert "not found" in resp["error"]["message"]


def test_rejects_a_missing_dac_dir(tmp_path: Path):
    model_dir = tmp_path / "dia-engine"
    model_dir.mkdir()
    resp = request_dia(
        {"text": "hello", "model_dir": str(model_dir), "dac_dir": str(tmp_path / "nope")}
    )
    assert resp["error"]["code"] == -32602
    assert "not found" in resp["error"]["message"]


def test_synthesizes_and_returns_base64_wav(monkeypatch: pytest.MonkeyPatch, dia_dirs):
    model_dir, dac_dir = dia_dirs
    fake = FakeDia()
    monkeypatch.setattr(dia, "_load_dia", lambda *_: fake)

    resp = request_dia(
        {
            "text": "From the mist, a story stirs.",
            "model_dir": model_dir,
            "dac_dir": dac_dir,
        }
    )

    result = resp["result"]
    assert result["sample_rate"] == 44_100
    assert result["duration_secs"] == pytest.approx(0.5)

    audio = base64.b64decode(result["audio_base64"])
    with wave.open(io.BytesIO(audio), "rb") as w:
        assert w.getnchannels() == 1
        assert w.getframerate() == 44_100


def test_a_plain_line_with_no_speaker_tag_is_auto_prefixed_with_s1(
    monkeypatch: pytest.MonkeyPatch, dia_dirs
):
    model_dir, dac_dir = dia_dirs
    fake = FakeDia()
    monkeypatch.setattr(dia, "_load_dia", lambda *_: fake)

    request_dia({"text": "just one line", "model_dir": model_dir, "dac_dir": dac_dir})

    assert fake.calls == [{"text": "[S1] just one line", "seed": fake.calls[0]["seed"]}]


def test_an_explicit_speaker_tag_is_left_untouched(monkeypatch: pytest.MonkeyPatch, dia_dirs):
    model_dir, dac_dir = dia_dirs
    fake = FakeDia()
    monkeypatch.setattr(dia, "_load_dia", lambda *_: fake)

    request_dia({"text": "[S2] your turn now.", "model_dir": model_dir, "dac_dir": dac_dir})

    assert fake.calls[0]["text"] == "[S2] your turn now."


def test_dia_nonverbal_tags_pass_through_to_the_engine_untouched(
    monkeypatch: pytest.MonkeyPatch, dia_dirs
):
    model_dir, dac_dir = dia_dirs
    fake = FakeDia()
    monkeypatch.setattr(dia, "_load_dia", lambda *_: fake)

    request_dia(
        {
            "text": "That's absurd. (laughs) Truly absurd.",
            "model_dir": model_dir,
            "dac_dir": dac_dir,
        }
    )

    spoken = [c["text"] for c in fake.calls]
    assert any("(laughs)" in s for s in spoken), spoken


def test_unsupported_emotion_tags_are_still_stripped_for_dia(
    monkeypatch: pytest.MonkeyPatch, dia_dirs
):
    """Dia's real vocabulary is a fixed set of non-verbal sounds, not a
    freeform emotion/tone control -- "(angry)" is exactly as unsupported
    for Dia as it is for Kokoro."""
    model_dir, dac_dir = dia_dirs
    fake = FakeDia()
    monkeypatch.setattr(dia, "_load_dia", lambda *_: fake)

    request_dia(
        {
            "text": "Text text (angry) more text (mysterious tone) final text",
            "model_dir": model_dir,
            "dac_dir": dac_dir,
        }
    )

    spoken = " ".join(c["text"] for c in fake.calls)
    assert "angry" not in spoken.lower()
    assert "mysterious" not in spoken.lower()


def test_pause_markup_still_becomes_a_real_gap_for_dia(monkeypatch: pytest.MonkeyPatch, dia_dirs):
    model_dir, dac_dir = dia_dirs
    fake = FakeDia()
    monkeypatch.setattr(dia, "_load_dia", lambda *_: fake)

    resp = request_dia(
        {
            "text": "Text text (pause) more text",
            "model_dir": model_dir,
            "dac_dir": dac_dir,
        }
    )

    spoken = [c["text"] for c in fake.calls]
    assert not any("pause" in s.lower() for s in spoken)
    expected_secs = 0.5 + main._PAUSE_TAGS["pause"] + 0.5
    # `int(gap_secs * sample_rate)` (same truncation main.py's Kokoro path
    # uses) can land one sample short at 44.1kHz purely from 0.35's binary
    # floating-point representation -- an absolute tolerance comfortably
    # wider than one sample (1/44100 ~= 0.0000227s) avoids a false failure
    # on that, while still catching a real wrong-gap bug (off by far more).
    assert resp["result"]["duration_secs"] == pytest.approx(expected_secs, abs=1e-3)


def test_same_narrator_identity_uses_the_same_seed_across_separate_calls(
    monkeypatch: pytest.MonkeyPatch, dia_dirs
):
    model_dir, dac_dir = dia_dirs
    fake = FakeDia()
    monkeypatch.setattr(dia, "_load_dia", lambda *_: fake)

    request_dia(
        {"text": "one", "voice": "Gravelly Old Man", "model_dir": model_dir, "dac_dir": dac_dir}
    )
    request_dia(
        {"text": "two", "voice": "gravelly old man", "model_dir": model_dir, "dac_dir": dac_dir}
    )

    assert fake.calls[0]["seed"] == fake.calls[1]["seed"], "same identity, case/space-insensitive"


def test_different_narrator_identities_use_different_seeds(
    monkeypatch: pytest.MonkeyPatch, dia_dirs
):
    model_dir, dac_dir = dia_dirs
    fake = FakeDia()
    monkeypatch.setattr(dia, "_load_dia", lambda *_: fake)

    request_dia({"text": "one", "voice": "narrator a", "model_dir": model_dir, "dac_dir": dac_dir})
    request_dia({"text": "two", "voice": "narrator b", "model_dir": model_dir, "dac_dir": dac_dir})

    assert fake.calls[0]["seed"] != fake.calls[1]["seed"]


def test_omitting_voice_still_gets_a_stable_default_identity(
    monkeypatch: pytest.MonkeyPatch, dia_dirs
):
    model_dir, dac_dir = dia_dirs
    fake = FakeDia()
    monkeypatch.setattr(dia, "_load_dia", lambda *_: fake)

    request_dia({"text": "one", "model_dir": model_dir, "dac_dir": dac_dir})
    request_dia({"text": "two", "model_dir": model_dir, "dac_dir": dac_dir})

    assert fake.calls[0]["seed"] == fake.calls[1]["seed"]


def test_caches_the_loaded_dia_engine_across_calls(monkeypatch: pytest.MonkeyPatch, dia_dirs):
    """`_load_dia` itself does the caching; only the expensive constructor
    (`_construct_dia`) is faked here, so this actually exercises the cache
    instead of assuming it away -- same shape as Kokoro's own cache test."""
    model_dir, dac_dir = dia_dirs
    construct_calls: list[tuple[str, str]] = []

    def fake_construct(m: str, d: str) -> FakeDia:
        construct_calls.append((m, d))
        return FakeDia()

    monkeypatch.setattr(dia, "_construct_dia", fake_construct)

    request_dia({"text": "one", "model_dir": model_dir, "dac_dir": dac_dir})
    request_dia({"text": "two", "model_dir": model_dir, "dac_dir": dac_dir})

    assert construct_calls == [(model_dir, dac_dir)]


def test_kokoro_is_still_the_default_engine_when_engine_is_omitted(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    from test_synthesize_speech import FakeKokoro

    fake = FakeKokoro()
    monkeypatch.setattr(main, "_load_kokoro", lambda *_: fake)
    model = tmp_path / "kokoro.onnx"
    voices = tmp_path / "voices.bin"
    model.write_bytes(b"")
    voices.write_bytes(b"")

    resp = main.handle(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "synthesize_speech",
            "params": {"text": "hello", "model_path": str(model), "voices_path": str(voices)},
        }
    )

    assert resp is not None
    assert "error" not in resp
    assert fake.calls, "Kokoro's engine must still run when engine is omitted"


def test_an_unknown_engine_is_a_clear_invalid_params_error():
    resp = main.handle(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "synthesize_speech",
            "params": {"text": "hello", "engine": "nonexistent-engine"},
        }
    )
    assert resp is not None
    assert resp["error"]["code"] == -32602
    assert "nonexistent-engine" in resp["error"]["message"]


# --- pure-function units, no engine/monkeypatching needed ------------------


def test_seed_for_narrator_is_stable_and_case_insensitive():
    a = dia._seed_for_narrator("Old Man")
    b = dia._seed_for_narrator("old man")
    c = dia._seed_for_narrator("  OLD MAN  ")
    assert a == b == c
    assert isinstance(a, int)


def test_seed_for_narrator_differs_across_distinct_identities():
    assert dia._seed_for_narrator("narrator") != dia._seed_for_narrator("villain")


def test_max_new_tokens_grows_with_text_length_within_bounds():
    short = dia._max_new_tokens_for("hi there")
    long = dia._max_new_tokens_for(" ".join(["word"] * 200))
    assert dia._DIA_MAX_NEW_TOKENS_FLOOR <= short <= long <= dia._DIA_MAX_NEW_TOKENS_CEILING


def test_with_speaker_tag_prefixes_only_when_no_tag_is_present():
    assert dia._with_speaker_tag("hello") == "[S1] hello"
    assert dia._with_speaker_tag("[S1] hello") == "[S1] hello"
    assert dia._with_speaker_tag("[S2] hello") == "[S2] hello"
    assert dia._with_speaker_tag("  [S1] hello") == "  [S1] hello"


def test_stage_local_hub_cache_creates_the_expected_hub_layout(tmp_path: Path):
    source = tmp_path / "dia-codec"
    source.mkdir()
    (source / "config.json").write_text("{}", encoding="utf-8")
    (source / "model.safetensors").write_bytes(b"weights")
    (source / "preprocessor_config.json").write_text("{}", encoding="utf-8")

    cache_root = dia._stage_local_hub_cache("descript/dac_44khz", source)

    snapshot = cache_root / "models--descript--dac_44khz" / "snapshots" / "local"
    assert (snapshot / "config.json").read_text(encoding="utf-8") == "{}"
    assert (snapshot / "model.safetensors").read_bytes() == b"weights"
    refs_main = cache_root / "models--descript--dac_44khz" / "refs" / "main"
    assert refs_main.read_text(encoding="utf-8") == "local"


def test_stage_local_hub_cache_is_idempotent(tmp_path: Path):
    source = tmp_path / "dia-codec"
    source.mkdir()
    (source / "config.json").write_text("{}", encoding="utf-8")

    first = dia._stage_local_hub_cache("descript/dac_44khz", source)
    second = dia._stage_local_hub_cache("descript/dac_44khz", source)

    assert first == second
    snapshot = first / "models--descript--dac_44khz" / "snapshots" / "local"
    assert (snapshot / "config.json").is_file()
