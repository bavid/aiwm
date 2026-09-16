"""`tag_frame` -- WD Danbooru tagger for the dataset-prep pipeline. The real
ONNX model (~1.2 GB) is never loaded here: `vision._load_wd_tagger` /
`vision._construct_wd_tagger` / `vision._wd_session` are monkeypatched with
fake doubles, the same way `test_vision_captioning.py` fakes Florence-2."""

from __future__ import annotations

from pathlib import Path
from typing import Any

import numpy as np
import pytest
from PIL import Image

from aiwm_sidecar import main, vision


class FakeWdTagger:
    """Stands in for `vision._WdTaggerEngine`: 6 tags, fixed probabilities."""

    def __init__(self) -> None:
        self.tag_names = ["general", "sensitive", "explicit", "kenji", "1boy", "outdoors"]
        self.categories = [9, 9, 9, 4, 0, 0]
        self.calls: list[dict[str, object]] = []

    def predict(self, image_path: str) -> np.ndarray:
        self.calls.append({"image_path": image_path})
        return np.array([0.10, 0.20, 0.85, 0.70, 0.90, 0.30], dtype=np.float32)


class _FakeOnnxInput:
    def __init__(self, name: str, shape: list[Any]) -> None:
        self.name = name
        self.shape = shape


class _FakeOnnxSession:
    """Stands in for an onnxruntime `InferenceSession` for `_construct_wd_tagger`
    tests -- exposes just the surface `_construct_wd_tagger` reads."""

    def __init__(self) -> None:
        self._input = _FakeOnnxInput("input", ["batch_size", 448, 448, 3])

    def get_inputs(self) -> list[_FakeOnnxInput]:
        return [self._input]

    def run(self, output_names: Any, feeds: dict[str, Any]) -> list[np.ndarray]:
        return [np.zeros((1, 4), dtype=np.float32)]


@pytest.fixture(autouse=True)
def clear_cache():
    vision._wd_tagger_cache.clear()
    yield
    vision._wd_tagger_cache.clear()


@pytest.fixture
def image_file(tmp_path: Path) -> str:
    p = tmp_path / "frame.png"
    Image.new("RGB", (8, 8), (200, 30, 30)).save(p)
    return str(p)


@pytest.fixture
def model_dir(tmp_path: Path) -> str:
    d = tmp_path / "wd-tagger"
    d.mkdir()
    (d / "model.onnx").write_bytes(b"not a real model")
    (d / "selected_tags.csv").write_text("tag_id,name,category,count\n", encoding="utf-8")
    return str(d)


# --- tag_frame: threshold, ordering, param validation ------------------------


def test_tag_frame_returns_general_and_character_tags_above_threshold(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
) -> None:
    fake = FakeWdTagger()
    monkeypatch.setattr(vision, "_load_wd_tagger", lambda _dir: fake)

    result = vision.tag_frame({"image_path": image_file, "model_dir": model_dir, "threshold": 0.5})

    assert result["engine"] == "wd-eva02-tagger-v3"
    # 1boy=0.90 outranks kenji=0.70 -- most confident first.
    assert result["caption"] == "1boy, kenji"
    assert result["rating"] == "explicit"
    assert fake.calls[0]["image_path"] == image_file


def test_tag_frame_explicit_zero_threshold_means_every_tag(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
) -> None:
    fake = FakeWdTagger()
    monkeypatch.setattr(vision, "_load_wd_tagger", lambda _dir: fake)

    result = vision.tag_frame({"image_path": image_file, "model_dir": model_dir, "threshold": 0.0})

    # All three general/character tags qualify at threshold 0, ordered by
    # confidence: 1boy=0.90, kenji=0.70, outdoors=0.30.
    assert result["caption"] == "1boy, kenji, outdoors"
    assert result["threshold"] == 0.0


def test_tag_frame_orders_tags_by_confidence_descending(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
) -> None:
    class _OrderingFakeTagger:
        tag_names = ["alpha", "beta", "gamma"]
        categories = [0, 0, 0]

        def predict(self, image_path: str) -> np.ndarray:
            return np.array([0.4, 0.9, 0.6], dtype=np.float32)

    monkeypatch.setattr(vision, "_load_wd_tagger", lambda _dir: _OrderingFakeTagger())

    result = vision.tag_frame({"image_path": image_file, "model_dir": model_dir, "threshold": 0.3})

    assert result["caption"] == "beta, gamma, alpha"


def test_tag_frame_threshold_default_is_the_model_cards_and_underscores_become_spaces(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
) -> None:
    fake = FakeWdTagger()
    fake.tag_names[5] = "blue_sky"
    fake.predict = lambda _p: np.array([0.0, 0.0, 0.0, 0.0, 0.0, 0.9], dtype=np.float32)  # type: ignore[method-assign]
    monkeypatch.setattr(vision, "_load_wd_tagger", lambda _dir: fake)

    result = vision.tag_frame({"image_path": image_file, "model_dir": model_dir})

    assert result["caption"] == "blue sky"
    assert result["threshold"] == vision._WD_DEFAULT_THRESHOLD


def test_tag_frame_validates_its_params(image_file: str, model_dir: str) -> None:
    with pytest.raises(ValueError, match="image_path"):
        vision.tag_frame({"model_dir": model_dir})
    with pytest.raises(ValueError, match="model_dir"):
        vision.tag_frame({"image_path": image_file})
    with pytest.raises(ValueError, match="not found"):
        vision.tag_frame({"image_path": "C:/nope.png", "model_dir": model_dir})


def test_tag_frame_is_dispatched_over_json_rpc(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
) -> None:
    monkeypatch.setattr(vision, "_load_wd_tagger", lambda _dir: FakeWdTagger())
    response = main.handle(
        {
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tag_frame",
            "params": {"image_path": image_file, "model_dir": model_dir},
        }
    )
    assert response["result"]["caption"] == "1boy, kenji"
    assert "tag_frame" in main.CAPABILITIES


def test_tag_frame_caches_the_constructed_engine_across_calls(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
) -> None:
    construct_calls: list[str] = []

    def fake_construct(m: str) -> FakeWdTagger:
        construct_calls.append(m)
        return FakeWdTagger()

    monkeypatch.setattr(vision, "_construct_wd_tagger", fake_construct)

    vision.tag_frame({"image_path": image_file, "model_dir": model_dir})
    vision.tag_frame({"image_path": image_file, "model_dir": model_dir})

    assert construct_calls == [model_dir]


# --- _wd_preprocess: pure-function unit ---------------------------------------


def test_preprocess_produces_448_bgr_float_batch(image_file: str) -> None:
    batch = vision._wd_preprocess(image_file, 448)
    assert batch.shape == (1, 448, 448, 3)
    assert batch.dtype == np.float32
    assert batch.flags["C_CONTIGUOUS"]
    # Source pixel was (R=200, G=30, B=30); BGR order means channel 0 is blue.
    assert batch[0, 224, 224, 0] == pytest.approx(30.0)
    assert batch[0, 224, 224, 2] == pytest.approx(200.0)


# --- _construct_wd_tagger: real CSV parsing + missing-file/malformed-row -----


def test_construct_wd_tagger_parses_the_tag_csv(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    d = tmp_path / "wd-tagger-real"
    d.mkdir()
    (d / "model.onnx").write_bytes(b"not a real model")
    (d / "selected_tags.csv").write_text(
        "tag_id,name,category,count\n"
        "0,rating_general,9,100\n"
        "1,kenji,4,50\n"
        "2,1boy,0,80\n"
        "3,outdoors,0,60\n",
        encoding="utf-8",
    )
    monkeypatch.setattr(vision, "_wd_session", lambda _model_path: _FakeOnnxSession())

    engine = vision._construct_wd_tagger(str(d))

    assert engine.tag_names == ["rating_general", "kenji", "1boy", "outdoors"]
    assert engine.categories == [9, 4, 0, 0]


def test_construct_wd_tagger_raises_when_model_onnx_is_missing(tmp_path: Path) -> None:
    d = tmp_path / "wd-tagger-missing-model"
    d.mkdir()
    (d / "selected_tags.csv").write_text("tag_id,name,category,count\n", encoding="utf-8")

    with pytest.raises(ValueError, match="model.onnx"):
        vision._construct_wd_tagger(str(d))


def test_construct_wd_tagger_raises_when_tag_csv_is_missing(tmp_path: Path) -> None:
    d = tmp_path / "wd-tagger-missing-csv"
    d.mkdir()
    (d / "model.onnx").write_bytes(b"not a real model")

    with pytest.raises(ValueError, match="selected_tags.csv"):
        vision._construct_wd_tagger(str(d))


def test_construct_wd_tagger_raises_on_malformed_csv_row(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    d = tmp_path / "wd-tagger-malformed"
    d.mkdir()
    (d / "model.onnx").write_bytes(b"not a real model")
    (d / "selected_tags.csv").write_text(
        "tag_id,name,category,count\n"
        "0,rating_general,9,100\n"
        "1,kenji\n",  # missing category (and count)
        encoding="utf-8",
    )
    monkeypatch.setattr(vision, "_wd_session", lambda _model_path: _FakeOnnxSession())

    with pytest.raises(ValueError, match="malformed"):
        vision._construct_wd_tagger(str(d))
