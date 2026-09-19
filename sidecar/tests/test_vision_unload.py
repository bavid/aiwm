"""`unload_vision_models` -- releasing the cached captioner engines (and
their VRAM) once a dataset prep is done.

No real model is ever loaded: the module-level caches are filled with plain
sentinel objects, and `torch` is replaced in `sys.modules` by a tiny fake
whose `cuda` namespace records `empty_cache()` calls -- the same
fake-module approach as `test_vision_captioning._fake_ml_modules`.
"""

from __future__ import annotations

import sys
import types
from typing import Any

import pytest

from aiwm_sidecar import main, vision


@pytest.fixture(autouse=True)
def clear_caches():
    for cache in (vision._florence2_cache, vision._qwen_vl_cache, vision._wd_tagger_cache):
        cache.clear()
    yield
    for cache in (vision._florence2_cache, vision._qwen_vl_cache, vision._wd_tagger_cache):
        cache.clear()


def _fill_caches() -> None:
    vision._florence2_cache["C:/m/florence2-large"] = object()  # type: ignore[assignment]
    vision._qwen_vl_cache[("C:/m/qwen2.5-vl-7b", "4bit")] = object()  # type: ignore[assignment]
    vision._wd_tagger_cache["C:/m/wd-tagger"] = object()  # type: ignore[assignment]


def _fake_torch(monkeypatch: pytest.MonkeyPatch, *, cuda: bool) -> list[str]:
    calls: list[str] = []
    torch = types.ModuleType("torch")
    torch.cuda = types.SimpleNamespace(  # type: ignore[attr-defined]
        is_available=lambda: cuda,
        empty_cache=lambda: calls.append("empty_cache"),
    )
    monkeypatch.setitem(sys.modules, "torch", torch)
    return calls


def _rpc(params: dict[str, Any]) -> dict[str, Any]:
    resp = main.handle(
        {"jsonrpc": "2.0", "id": 7, "method": "unload_vision_models", "params": params}
    )
    assert resp is not None
    return resp


def test_unload_empties_every_cache_and_reports_what_was_released(
    monkeypatch: pytest.MonkeyPatch,
):
    _fake_torch(monkeypatch, cuda=False)
    _fill_caches()

    result = vision.unload_vision_models({})

    assert vision._florence2_cache == {}
    assert vision._qwen_vl_cache == {}
    assert vision._wd_tagger_cache == {}
    assert sorted((r["kind"], r["model_dir"]) for r in result["released"]) == [
        ("florence2", "C:/m/florence2-large"),
        ("qwen_vl", "C:/m/qwen2.5-vl-7b"),
        ("wd_tagger", "C:/m/wd-tagger"),
    ]


def test_unload_clears_the_cuda_cache_when_cuda_is_available(monkeypatch: pytest.MonkeyPatch):
    calls = _fake_torch(monkeypatch, cuda=True)
    _fill_caches()

    result = vision.unload_vision_models({})

    assert calls == ["empty_cache"]
    assert result["cuda_cache_cleared"] is True


def test_a_cuda_error_after_the_release_is_reported_not_raised(
    monkeypatch: pytest.MonkeyPatch,
):
    def broken_empty_cache() -> None:
        raise RuntimeError("CUDA error: an illegal memory access was encountered")

    torch = types.ModuleType("torch")
    torch.cuda = types.SimpleNamespace(  # type: ignore[attr-defined]
        is_available=lambda: True, empty_cache=broken_empty_cache
    )
    monkeypatch.setitem(sys.modules, "torch", torch)
    _fill_caches()

    resp = _rpc({})

    result = resp["result"]
    assert len(result["released"]) == 3, "the engines are dropped regardless"
    assert vision._florence2_cache == {}
    assert result["cuda_cache_cleared"] is False
    assert "illegal memory access" in result["cuda_error"]


def test_unload_does_not_touch_cuda_when_it_is_unavailable(monkeypatch: pytest.MonkeyPatch):
    calls = _fake_torch(monkeypatch, cuda=False)
    _fill_caches()

    result = vision.unload_vision_models({})

    assert calls == []
    assert result["cuda_cache_cleared"] is False


def test_unload_without_torch_imported_neither_imports_it_nor_fails(
    monkeypatch: pytest.MonkeyPatch,
):
    monkeypatch.delitem(sys.modules, "torch", raising=False)
    _fill_caches()

    result = vision.unload_vision_models({})

    assert "torch" not in sys.modules, "unloading must never import torch"
    assert result["cuda_cache_cleared"] is False
    assert len(result["released"]) == 3


def test_unload_with_nothing_loaded_is_a_harmless_no_op(monkeypatch: pytest.MonkeyPatch):
    _fake_torch(monkeypatch, cuda=True)

    result = vision.unload_vision_models({})

    assert result["released"] == []


def test_unload_can_be_narrowed_to_one_kind(monkeypatch: pytest.MonkeyPatch):
    _fake_torch(monkeypatch, cuda=False)
    _fill_caches()

    result = vision.unload_vision_models({"kind": "qwen_vl"})

    assert [r["kind"] for r in result["released"]] == ["qwen_vl"]
    assert vision._qwen_vl_cache == {}
    assert list(vision._florence2_cache) == ["C:/m/florence2-large"]
    assert list(vision._wd_tagger_cache) == ["C:/m/wd-tagger"]


def test_unload_can_be_narrowed_to_one_model_dir(monkeypatch: pytest.MonkeyPatch):
    _fake_torch(monkeypatch, cuda=False)
    _fill_caches()

    result = vision.unload_vision_models({"model_dir": "C:/m/florence2-large"})

    assert [r["kind"] for r in result["released"]] == ["florence2"]
    assert vision._florence2_cache == {}
    assert len(vision._qwen_vl_cache) == 1


def test_unload_rejects_an_unknown_kind():
    with pytest.raises(ValueError, match="kind"):
        vision.unload_vision_models({"kind": "potato"})


def test_unload_is_dispatched_over_json_rpc(monkeypatch: pytest.MonkeyPatch):
    _fake_torch(monkeypatch, cuda=False)
    _fill_caches()

    resp = _rpc({})

    assert resp["id"] == 7
    assert len(resp["result"]["released"]) == 3
    assert vision._florence2_cache == {}


def test_unload_reports_bad_params_as_invalid_params():
    resp = _rpc({"kind": "potato"})
    assert resp["error"]["code"] == -32602


def test_capabilities_advertise_unload():
    assert "unload_vision_models" in main.CAPABILITIES
