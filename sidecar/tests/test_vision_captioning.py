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


# --- offline loading: every from_pretrained stays on the local folder --------


class _Recorder:
    """A fake `transformers` class whose `from_pretrained` records its call."""

    def __init__(self, calls: list[tuple[str, str, dict[str, Any]]], name: str) -> None:
        self._calls = calls
        self._name = name

    def from_pretrained(self, path: str, **kwargs: Any) -> Any:
        self._calls.append((self._name, path, kwargs))
        return self

    # The model object's chained calls in `_construct_florence2`.
    def to(self, _device: str) -> Any:
        return self

    def eval(self) -> Any:
        return self


def _fake_ml_modules(
    monkeypatch: pytest.MonkeyPatch,
    modules_cache: Path | None = None,
) -> list[tuple[str, str, dict[str, Any]]]:
    import sys
    import types

    calls: list[tuple[str, str, dict[str, Any]]] = []
    dmu = types.ModuleType("transformers.dynamic_module_utils")
    dmu.HF_MODULES_CACHE = str(modules_cache or Path("does-not-exist"))  # type: ignore[attr-defined]
    dmu.TRANSFORMERS_DYNAMIC_MODULE_NAME = "transformers_modules"  # type: ignore[attr-defined]
    monkeypatch.setitem(sys.modules, "transformers.dynamic_module_utils", dmu)
    torch = types.ModuleType("torch")
    torch.cuda = types.SimpleNamespace(is_available=lambda: False)  # type: ignore[attr-defined]
    torch.float16 = "float16"  # type: ignore[attr-defined]
    torch.float32 = "float32"  # type: ignore[attr-defined]
    transformers = types.ModuleType("transformers")
    for name in ("AutoModelForCausalLM", "AutoProcessor", "Qwen2_5_VLForConditionalGeneration"):
        setattr(transformers, name, _Recorder(calls, name))
    transformers.BitsAndBytesConfig = lambda **kw: ("bnb", kw)  # type: ignore[attr-defined]
    monkeypatch.setitem(sys.modules, "torch", torch)
    monkeypatch.setitem(sys.modules, "transformers", transformers)
    return calls


def test_florence2_loads_model_and_processor_from_local_files_only(
    monkeypatch: pytest.MonkeyPatch, model_dir: str
):
    # `trust_remote_code=True` + an `auto_map` naming a Hub repo would
    # otherwise fetch unpinned Python -- the folder core verified is the
    # only source allowed.
    calls = _fake_ml_modules(monkeypatch)
    vision._construct_florence2(model_dir)
    assert [c[0] for c in calls] == ["AutoModelForCausalLM", "AutoProcessor"]
    for name, path, kwargs in calls:
        assert path == model_dir, name
        assert kwargs.get("local_files_only") is True, name
        assert kwargs.get("trust_remote_code") is True, name


def test_florence2_clears_its_stale_remote_code_copies_before_the_first_load(
    monkeypatch: pytest.MonkeyPatch, model_dir: str, tmp_path: Path
):
    # transformers copies a local folder's remote code to
    # <HF_MODULES_CACHE>/transformers_modules/<sanitized folder name>/<hash>/
    # and imports it from there -- a stale or planted copy must be gone
    # before the verified folder is loaded.
    cache = tmp_path / "hf-modules"
    ours = cache / "transformers_modules" / "florence_hyphen_2_hyphen_large"
    (ours / "oldhash").mkdir(parents=True)
    (ours / "oldhash" / "modeling_florence2.py").write_text("import os\n")
    other = cache / "transformers_modules" / "some_other_model"
    other.mkdir(parents=True)
    (other / "x.py").write_text("\n")
    calls = _fake_ml_modules(monkeypatch, cache)

    seen_stale: list[bool] = []
    real_from_pretrained = _Recorder.from_pretrained

    def spy(self: _Recorder, path: str, **kwargs: Any) -> Any:
        seen_stale.append(ours.exists())
        return real_from_pretrained(self, path, **kwargs)

    monkeypatch.setattr(_Recorder, "from_pretrained", spy)
    vision._construct_florence2(model_dir)

    assert len(calls) == 2
    assert seen_stale == [False, False], "cleared before every from_pretrained"
    assert (other / "x.py").exists(), "only Florence-2's own copies are removed"


def test_remote_code_cache_dir_follows_transformers_naming():
    assert vision._remote_code_cache_dir("C:/store/vision/florence2-large", "/c") == Path(
        "/c", "transformers_modules", "florence2_hyphen_large"
    )
    assert vision._remote_code_cache_dir("/m/2x.v1", "/c") == Path(
        "/c", "transformers_modules", "_2x_dot_v1"
    )


@pytest.mark.parametrize("quantization", ["4bit", "8bit", "none"])
def test_qwen_vl_loads_model_and_processor_from_local_files_only(
    monkeypatch: pytest.MonkeyPatch, model_dir: str, quantization: str
):
    calls = _fake_ml_modules(monkeypatch)
    vision._construct_qwen_vl(model_dir, quantization)
    assert [c[0] for c in calls] == ["Qwen2_5_VLForConditionalGeneration", "AutoProcessor"]
    for name, path, kwargs in calls:
        assert path == model_dir, name
        assert kwargs.get("local_files_only") is True, name
        assert "trust_remote_code" not in kwargs, name


# --- pure-function units, no engine/monkeypatching needed -------------------


def test_construct_qwen_vl_rejects_an_unknown_quantization(model_dir: str):
    with pytest.raises(ValueError, match="quantization"):
        vision._construct_qwen_vl(model_dir, "potato")


def test_capabilities_advertise_the_new_methods():
    assert "caption_frame" in main.CAPABILITIES
    assert "caption_frame_pair" in main.CAPABILITIES
