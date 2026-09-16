"""`tag_frame` -- WD Danbooru tagger for the dataset-prep pipeline. The real
ONNX model (~1.2 GB) is never loaded here: `vision._load_wd_tagger` is
monkeypatched with a fake session/tag table, the same way
`test_vision_captioning.py` fakes Florence-2."""

from __future__ import annotations

from pathlib import Path

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


def test_tag_frame_returns_general_and_character_tags_above_threshold(
    monkeypatch: pytest.MonkeyPatch, image_file: str, model_dir: str
) -> None:
    fake = FakeWdTagger()
    monkeypatch.setattr(vision, "_load_wd_tagger", lambda _dir: fake)

    result = vision.tag_frame({"image_path": image_file, "model_dir": model_dir, "threshold": 0.5})

    assert result["engine"] == "wd-eva02-tagger-v3"
    assert result["caption"] == "kenji, 1boy"
    assert result["rating"] == "explicit"
    assert fake.calls[0]["image_path"] == image_file


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
    assert response["result"]["caption"] == "kenji, 1boy"
    assert "tag_frame" in main.CAPABILITIES


def test_preprocess_produces_448_bgr_float_batch(image_file: str) -> None:
    batch = vision._wd_preprocess(image_file, 448)
    assert batch.shape == (1, 448, 448, 3)
    assert batch.dtype == np.float32
    # Source pixel was (R=200, G=30, B=30); BGR order means channel 0 is blue.
    assert batch[0, 224, 224, 0] == pytest.approx(30.0)
    assert batch[0, 224, 224, 2] == pytest.approx(200.0)
