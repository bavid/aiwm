"""`caption_frame` / `caption_frame_pair` -- Florence-2 and Qwen2.5-VL
captioning for the dataset-prep pipeline.

Neither real model (Florence-2 ~0.77B, Qwen2.5-VL ~7B, both GPU-resident) is
ever required for these tests: `vision._load_florence2` / `_load_qwen_vl`
are monkeypatched with small fake doubles, exactly the way
`test_dia_synthesis.py` fakes `dia._load_dia` -- these tests exercise the
JSON-RPC contract and parameter validation, not real `transformers` model
loading.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest

from aiwm_sidecar import main, vision


class FakeFlorence2:
    """Stands in for `vision._Florence2Engine`."""

    def __init__(self, caption: str = "a lush forest clearing at dawn") -> None:
        self.caption_value = caption
        self.calls: list[dict[str, Any]] = []

    def caption(self, image_path: str, task_prompt: str) -> str:
        self.calls.append({"image_path": image_path, "task_prompt": task_prompt})
        return self.caption_value


class FakeQwenVl:
    """Stands in for `vision._QwenVlEngine`."""

    def __init__(self, caption: str = "the character turns and walks toward the door") -> None:
        self.caption_value = caption
        self.calls: list[dict[str, Any]] = []

    def caption_pair(self, image_path: str, context_image_path: str, question: str) -> str:
        self.calls.append(
            {
                "image_path": image_path,
                "context_image_path": context_image_path,
                "question": question,
            }
        )
        return self.caption_value


@pytest.fixture(autouse=True)
def clear_caches():
    vision._florence2_cache.clear()
    vision._qwen_vl_cache.clear()
    yield
    vision._florence2_cache.clear()
    vision._qwen_vl_cache.clear()


@pytest.fixture
def image_file(tmp_path: Path) -> str:
    p = tmp_path / "frame.png"
    p.write_bytes(b"pretend png bytes")
    return str(p)


@pytest.fixture
def context_image_file(tmp_path: Path) -> str:
    p = tmp_path / "frame_context.png"
    p.write_bytes(b"pretend png bytes, later frame")
    return str(p)


@pytest.fixture
def model_dir(tmp_path: Path) -> str:
    d = tmp_path / "florence-2-large"
    d.mkdir()
    return str(d)


def rpc(method: str, params: dict[str, Any]) -> dict[str, Any]:
    req = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    resp = main.handle(req)
    assert resp is not None
    return resp


# --- caption_frame (Florence-2) ---------------------------------------------


def test_caption_frame_requires_image_path(model_dir: str):
    resp = rpc("caption_frame", {"model_dir": model_dir})
    assert resp["error"]["code"] == -32602
    assert "image_path" in resp["error"]["message"]


def test_caption_frame_requires_model_dir(image_file: str):
    resp = rpc("caption_frame", {"image_path": image_file})
    assert resp["error"]["code"] == -32602
    assert "model_dir" in resp["error"]["message"]


def test_caption_frame_rejects_a_missing_image(model_dir: str, tmp_path: Path):
    resp = rpc(
        "caption_frame",
        {"image_path": str(tmp_path / "gone.png"), "model_dir": model_dir},
    )
    assert resp["error"]["code"] == -32602
    assert "not found" in resp["error"]["message"]


def test_caption_frame_rejects_a_missing_model_dir(image_file: str, tmp_path: Path):
    resp = rpc(
        "caption_frame",
        {"image_path": image_file, "model_dir": str(tmp_path / "nope")},
    )
    assert resp["error"]["code"] == -32602
    assert "directory not found" in resp["error"]["message"]


def test_caption_frame_returns_the_engines_caption(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
):
    fake = FakeFlorence2("a red bicycle leaning against a brick wall")
    monkeypatch.setattr(vision, "_load_florence2", lambda *_: fake)

    resp = rpc("caption_frame", {"image_path": image_file, "model_dir": model_dir})

    assert "error" not in resp
    assert resp["result"] == {
        "caption": "a red bicycle leaning against a brick wall",
        "engine": "florence2",
    }
    assert fake.calls == [{"image_path": image_file, "task_prompt": "<DETAILED_CAPTION>"}]


def test_caption_frame_accepts_a_custom_task_prompt(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
):
    fake = FakeFlorence2()
    monkeypatch.setattr(vision, "_load_florence2", lambda *_: fake)

    rpc(
        "caption_frame",
        {"image_path": image_file, "model_dir": model_dir, "task_prompt": "<CAPTION>"},
    )

    assert fake.calls[0]["task_prompt"] == "<CAPTION>"


def test_caption_frame_rejects_an_empty_caption(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
):
    monkeypatch.setattr(vision, "_load_florence2", lambda *_: FakeFlorence2(""))

    resp = rpc("caption_frame", {"image_path": image_file, "model_dir": model_dir})

    # `caption_frame` raises `ValueError` for this (same as every other
    # validation failure in this module); `main.handle` maps every
    # `ValueError` from a capability call to INVALID_PARAMS uniformly (see
    # `dia.py`'s own empty-text case for the same convention), not just
    # ones caused by the caller's literal input.
    assert resp["error"]["code"] == -32602


def test_caption_frame_caches_the_loaded_engine_across_calls(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
):
    construct_calls: list[str] = []

    def fake_construct(m: str) -> FakeFlorence2:
        construct_calls.append(m)
        return FakeFlorence2()

    monkeypatch.setattr(vision, "_construct_florence2", fake_construct)

    rpc("caption_frame", {"image_path": image_file, "model_dir": model_dir})
    rpc("caption_frame", {"image_path": image_file, "model_dir": model_dir})

    assert construct_calls == [model_dir]


# --- caption_frame_pair (Qwen2.5-VL) -----------------------------------------


def test_caption_frame_pair_requires_both_images(model_dir: str):
    resp = rpc("caption_frame_pair", {"model_dir": model_dir, "question": "what happens?"})
    assert resp["error"]["code"] == -32602
    assert "image_path" in resp["error"]["message"]


def test_caption_frame_pair_requires_a_question(
    image_file: str, context_image_file: str, model_dir: str
):
    resp = rpc(
        "caption_frame_pair",
        {
            "image_path": image_file,
            "context_image_path": context_image_file,
            "model_dir": model_dir,
        },
    )
    assert resp["error"]["code"] == -32602
    assert "question" in resp["error"]["message"]


def test_caption_frame_pair_rejects_a_missing_context_image(
    image_file: str, model_dir: str, tmp_path: Path
):
    resp = rpc(
        "caption_frame_pair",
        {
            "image_path": image_file,
            "context_image_path": str(tmp_path / "gone.png"),
            "model_dir": model_dir,
            "question": "what happens?",
        },
    )
    assert resp["error"]["code"] == -32602
    assert "context image" in resp["error"]["message"]


def test_caption_frame_pair_returns_the_engines_caption(
    monkeypatch: pytest.MonkeyPatch, image_file: str, context_image_file: str, model_dir: str
):
    fake = FakeQwenVl("the figure raises an arm and begins to run")
    monkeypatch.setattr(vision, "_load_qwen_vl", lambda *_: fake)

    resp = rpc(
        "caption_frame_pair",
        {
            "image_path": image_file,
            "context_image_path": context_image_file,
            "model_dir": model_dir,
            "question": "what changed between these two frames?",
        },
    )

    assert "error" not in resp
    assert resp["result"] == {
        "caption": "the figure raises an arm and begins to run",
        "engine": "qwen2.5-vl",
    }
    assert fake.calls == [
        {
            "image_path": image_file,
            "context_image_path": context_image_file,
            "question": "what changed between these two frames?",
        }
    ]


def test_caption_frame_pair_defaults_to_4bit_quantization(
    monkeypatch: pytest.MonkeyPatch, image_file: str, context_image_file: str, model_dir: str
):
    load_calls: list[tuple[str, str]] = []

    def fake_load(m: str, q: str) -> FakeQwenVl:
        load_calls.append((m, q))
        return FakeQwenVl()

    monkeypatch.setattr(vision, "_load_qwen_vl", fake_load)

    rpc(
        "caption_frame_pair",
        {
            "image_path": image_file,
            "context_image_path": context_image_file,
            "model_dir": model_dir,
            "question": "what happens?",
        },
    )

    assert load_calls == [(model_dir, "4bit")]


def test_caption_frame_pair_accepts_an_explicit_quantization(
    monkeypatch: pytest.MonkeyPatch, image_file: str, context_image_file: str, model_dir: str
):
    load_calls: list[tuple[str, str]] = []
    monkeypatch.setattr(
        vision,
        "_load_qwen_vl",
        lambda m, q: (load_calls.append((m, q)), FakeQwenVl())[1],
    )

    rpc(
        "caption_frame_pair",
        {
            "image_path": image_file,
            "context_image_path": context_image_file,
            "model_dir": model_dir,
            "question": "what happens?",
            "quantization": "none",
        },
    )

    assert load_calls == [(model_dir, "none")]


def test_caption_frame_pair_caches_the_loaded_engine_per_quantization(
    monkeypatch: pytest.MonkeyPatch, image_file: str, context_image_file: str, model_dir: str
):
    construct_calls: list[tuple[str, str]] = []

    def fake_construct(m: str, q: str) -> FakeQwenVl:
        construct_calls.append((m, q))
        return FakeQwenVl()

    monkeypatch.setattr(vision, "_construct_qwen_vl", fake_construct)

    params = {
        "image_path": image_file,
        "context_image_path": context_image_file,
        "model_dir": model_dir,
        "question": "what happens?",
    }
    rpc("caption_frame_pair", params)
    rpc("caption_frame_pair", params)

    assert construct_calls == [(model_dir, "4bit")]


# --- pure-function units, no engine/monkeypatching needed -------------------


def test_construct_qwen_vl_rejects_an_unknown_quantization(model_dir: str):
    with pytest.raises(ValueError, match="quantization"):
        vision._construct_qwen_vl(model_dir, "potato")


def test_capabilities_advertise_the_new_methods():
    assert "caption_frame" in main.CAPABILITIES
    assert "caption_frame_pair" in main.CAPABILITIES
