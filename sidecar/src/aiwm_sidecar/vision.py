"""Vision-language captioning for the dataset-prep pipeline (WP: "Lokale
KI-Trainings-Engine" -- see docs/TODO.md). Two engines, mirroring `dia.py`'s
lazy-import-heavy-deps and engine-cache patterns exactly:

* **Florence-2** (Microsoft, MIT license, verified against the current
  `microsoft/Florence-2-large` model card while building this) -- a small
  (~0.77B param) single-image captioning model driven by task-prompt tokens
  (`<DETAILED_CAPTION>`). The default bulk per-frame captioner: fast enough
  for thousands of frames, no quantization needed.
* **Qwen2.5-VL-7B-Instruct** (Alibaba, Apache-2.0 license, verified against
  the current `Qwen/Qwen2.5-VL-7B-Instruct` model card) -- a real 7B chat
  VLM that genuinely accepts *multiple* images in one prompt, unlike
  Florence-2. Used only for the escalation path: when a Florence-2 caption
  looks low-confidence (`capability::dataset::caption::is_low_confidence_
  caption`, decided Rust-side -- this module only ever executes what it's
  told), it is re-captioned together with a nearby frame so the model can
  describe the motion/action between the two. Loaded 4-bit via
  `bitsandbytes` by default -- fp16 alone is ~14 GB of weights, which does
  not comfortably share a 16 GB card with anything else AIWM already runs;
  `quantization="none"` is available for a bigger card.

Neither engine's real, multi-GB weights are ever touched by the fast unit
suite -- `tests/test_vision_captioning.py` monkeypatches `_load_florence2`/
`_load_qwen_vl` with fake doubles, exactly like `test_dia_synthesis.py` does
for Dia.
"""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING, Any, Protocol

if TYPE_CHECKING:
    import numpy as np


def _vision_value_error(message: str) -> ValueError:
    return ValueError(message)


class _CaptionEngine(Protocol):
    """Common shape both engines' loaded model+processor pair present to the
    dispatch functions below -- lets a test double stand in for either one
    without knowing which real `transformers` classes back it."""

    def caption(self, image_path: str, task_prompt: str) -> str: ...  # pragma: no cover

    def caption_pair(
        self, image_path: str, context_image_path: str, question: str
    ) -> str: ...  # pragma: no cover


class _WdTaggerLike(Protocol):
    """Shape `_load_wd_tagger`'s cache and `tag_frame` rely on -- lets a test
    double stand in for `_WdTaggerEngine` without touching onnxruntime."""

    tag_names: list[str]
    categories: list[int]

    def predict(self, image_path: str) -> np.ndarray: ...  # pragma: no cover


_DEFAULT_FLORENCE2_TASK = "<DETAILED_CAPTION>"


class _Florence2Engine:
    """Wraps a loaded Florence-2 model + processor. Florence-2 needs
    `trust_remote_code=True` -- its architecture ships as custom modeling
    code on the Hugging Face repo rather than a class built into
    `transformers` itself (true as of the model card checked while building
    this; worth re-checking if a future `transformers` release folds it in
    natively)."""

    def __init__(self, model: Any, processor: Any, device: str) -> None:
        self._model = model
        self._processor = processor
        self._device = device

    def caption(self, image_path: str, task_prompt: str) -> str:
        from PIL import Image

        image = Image.open(image_path).convert("RGB")
        inputs = self._processor(text=task_prompt, images=image, return_tensors="pt").to(
            self._device
        )
        generated_ids = self._model.generate(
            input_ids=inputs["input_ids"],
            pixel_values=inputs["pixel_values"],
            max_new_tokens=1024,
            num_beams=3,
            do_sample=False,
        )
        generated_text = self._processor.batch_decode(generated_ids, skip_special_tokens=False)[0]
        parsed = self._processor.post_process_generation(
            generated_text, task=task_prompt, image_size=(image.width, image.height)
        )
        return str(parsed.get(task_prompt, "")).strip()

    def caption_pair(self, image_path: str, context_image_path: str, question: str) -> str:
        raise _vision_value_error("Florence-2 cannot caption an image pair -- use Qwen2.5-VL")


class _QwenVlEngine:
    """Wraps a loaded Qwen2.5-VL model + processor. Multi-image chat
    prompting via `qwen_vl_utils.process_vision_info`, the vendor-published
    helper for this exact model family."""

    def __init__(self, model: Any, processor: Any) -> None:
        self._model = model
        self._processor = processor

    def caption(self, image_path: str, task_prompt: str) -> str:
        raise _vision_value_error(
            "Qwen2.5-VL's single-image path is unused here -- caption_frame_pair only"
        )

    def caption_pair(self, image_path: str, context_image_path: str, question: str) -> str:
        from qwen_vl_utils import process_vision_info

        messages = [
            {
                "role": "user",
                "content": [
                    {"type": "image", "image": image_path},
                    {"type": "image", "image": context_image_path},
                    {"type": "text", "text": question},
                ],
            }
        ]
        text = self._processor.apply_chat_template(
            messages, tokenize=False, add_generation_prompt=True
        )
        image_inputs, video_inputs = process_vision_info(messages)
        inputs = self._processor(
            text=[text],
            images=image_inputs,
            videos=video_inputs,
            padding=True,
            return_tensors="pt",
        ).to(self._model.device)

        generated_ids = self._model.generate(**inputs, max_new_tokens=160)
        trimmed = [
            out_ids[len(in_ids) :]
            for in_ids, out_ids in zip(inputs.input_ids, generated_ids, strict=True)
        ]
        decoded = self._processor.batch_decode(
            trimmed, skip_special_tokens=True, clean_up_tokenization_spaces=False
        )
        return decoded[0].strip()


_florence2_cache: dict[str, _Florence2Engine] = {}
_qwen_vl_cache: dict[tuple[str, str], _QwenVlEngine] = {}


def _sanitize_module_name(name: str) -> str:
    """Mirror of `transformers.dynamic_module_utils._sanitize_module_name`
    (a private helper, so not imported): the folder name transformers gives
    a local model's copied remote code."""
    sanitized = name.replace(".", "_dot_").replace("-", "_hyphen_")
    if sanitized and sanitized[0].isdigit():
        sanitized = f"_{sanitized}"
    return sanitized


def _remote_code_cache_dir(model_dir: str, modules_cache: str) -> Path:
    """Where transformers copies a *local* folder's `trust_remote_code`
    Python before importing it: `<HF_MODULES_CACHE>/transformers_modules/
    <sanitized folder name>/<source hash>/` (transformers 5.x)."""
    name = _sanitize_module_name(Path(model_dir).name)
    return Path(modules_cache, "transformers_modules", name)


def _clear_remote_code_cache(model_dir: str) -> None:
    """Delete every earlier copy of this folder's remote code from the
    transformers modules cache (in the user profile, outside anything core
    verifies) so the load below imports only a fresh copy of the pinned,
    just-verified files -- never a stale or planted module left there."""
    import shutil

    from transformers.dynamic_module_utils import HF_MODULES_CACHE

    # A drive/filesystem root has no folder name: the path below would then
    # be all of `transformers_modules` -- never delete that.
    if not _sanitize_module_name(Path(model_dir).name):
        return
    stale = _remote_code_cache_dir(model_dir, str(HF_MODULES_CACHE))
    if stale.exists():
        shutil.rmtree(stale)


def _construct_florence2(model_dir: str) -> _Florence2Engine:
    import torch
    from transformers import AutoModelForCausalLM, AutoProcessor

    _clear_remote_code_cache(model_dir)

    device = "cuda" if torch.cuda.is_available() else "cpu"
    dtype = torch.float16 if device == "cuda" else torch.float32
    # `local_files_only=True`: core verified this exact folder against the
    # pinned catalog (`model::integrity`) before sending the request -- an
    # `auto_map` naming a Hub repo must never make `trust_remote_code` fetch
    # unpinned Python (or anything else) from the network instead.
    model = (
        AutoModelForCausalLM.from_pretrained(
            model_dir, trust_remote_code=True, dtype=dtype, local_files_only=True
        )
        .to(device)
        .eval()
    )
    processor = AutoProcessor.from_pretrained(
        model_dir, trust_remote_code=True, local_files_only=True
    )
    return _Florence2Engine(model, processor, device)


def _load_florence2(model_dir: str) -> _Florence2Engine:
    if model_dir not in _florence2_cache:
        _florence2_cache[model_dir] = _construct_florence2(model_dir)
    return _florence2_cache[model_dir]


def _construct_qwen_vl(model_dir: str, quantization: str) -> _QwenVlEngine:
    import torch
    from transformers import AutoProcessor, Qwen2_5_VLForConditionalGeneration

    quant_config = None
    if quantization == "4bit":
        from transformers import BitsAndBytesConfig

        quant_config = BitsAndBytesConfig(
            load_in_4bit=True,
            bnb_4bit_compute_dtype=torch.float16,
            bnb_4bit_quant_type="nf4",
        )
    elif quantization == "8bit":
        from transformers import BitsAndBytesConfig

        quant_config = BitsAndBytesConfig(load_in_8bit=True)
    elif quantization != "none":
        raise _vision_value_error(
            f"unknown quantization: {quantization!r} (expected 4bit, 8bit or none)"
        )

    model = Qwen2_5_VLForConditionalGeneration.from_pretrained(
        model_dir,
        dtype="auto" if quant_config is None else torch.float16,
        device_map="auto",
        quantization_config=quant_config,
        local_files_only=True,
    )
    # Same offline rule as Florence-2: only the verified local folder.
    processor = AutoProcessor.from_pretrained(model_dir, local_files_only=True)
    return _QwenVlEngine(model, processor)


_DEFAULT_QUANTIZATION = "4bit"


def _load_qwen_vl(model_dir: str, quantization: str) -> _QwenVlEngine:
    key = (model_dir, quantization)
    if key not in _qwen_vl_cache:
        _qwen_vl_cache[key] = _construct_qwen_vl(model_dir, quantization)
    return _qwen_vl_cache[key]


def _require_existing_file(path: str, label: str) -> None:
    if not Path(path).is_file():
        raise _vision_value_error(f"{label} not found: {path}")


def _require_dir(path: str, label: str) -> None:
    if not Path(path).is_dir():
        raise _vision_value_error(f"{label} directory not found: {path}")


def caption_frame(params: dict[str, Any]) -> dict[str, Any]:
    """Florence-2, single image. JSON-RPC `caption_frame`."""
    image_path = str(params.get("image_path") or "")
    model_dir = str(params.get("model_dir") or "")
    task_prompt = str(params.get("task_prompt") or _DEFAULT_FLORENCE2_TASK)
    if not image_path:
        raise _vision_value_error("`image_path` is required")
    if not model_dir:
        raise _vision_value_error("`model_dir` is required")
    _require_existing_file(image_path, "image")
    _require_dir(model_dir, "Florence-2 model")

    engine = _load_florence2(model_dir)
    caption = engine.caption(image_path, task_prompt)
    if not caption:
        raise _vision_value_error("Florence-2 returned an empty caption")
    return {"caption": caption, "engine": "florence2"}


def caption_frame_pair(params: dict[str, Any]) -> dict[str, Any]:
    """Qwen2.5-VL, two images + a question about what changed between them.
    JSON-RPC `caption_frame_pair`."""
    image_path = str(params.get("image_path") or "")
    context_image_path = str(params.get("context_image_path") or "")
    model_dir = str(params.get("model_dir") or "")
    question = str(params.get("question") or "").strip()
    quantization = str(params.get("quantization") or _DEFAULT_QUANTIZATION)
    if not image_path or not context_image_path:
        raise _vision_value_error("`image_path` and `context_image_path` are required")
    if not model_dir:
        raise _vision_value_error("`model_dir` is required")
    if not question:
        raise _vision_value_error("`question` is required")
    _require_existing_file(image_path, "image")
    _require_existing_file(context_image_path, "context image")
    _require_dir(model_dir, "Qwen2.5-VL model")

    engine = _load_qwen_vl(model_dir, quantization)
    caption = engine.caption_pair(image_path, context_image_path, question)
    if not caption:
        raise _vision_value_error("Qwen2.5-VL returned an empty caption")
    return {"caption": caption, "engine": "qwen2.5-vl"}


# --- WD Danbooru tagger (SmilingWolf, Apache-2.0) ---------------------------
#
# A 0.3B ONNX classifier over the Danbooru tag vocabulary. Runs on the CPU
# via onnxruntime; no torch involved. Input contract (from the reference
# tagger implementations for the v3 models): 448x448, BGR, float32 0..255,
# NHWC; outputs are sigmoid probabilities in `selected_tags.csv` row order.
# Confirmed against the real wd-eva02-tagger-v3 `model.onnx` (1,260,435,999
# bytes): input name "input", shape ['batch_size', 448, 448, 3], dtype
# tensor(float) -- matches the assumed NHWC/float32 contract exactly.

_WD_DEFAULT_THRESHOLD = 0.35
_WD_RATING_CATEGORY = 9
_WD_CHARACTER_CATEGORY = 4
_WD_GENERAL_CATEGORY = 0


def _wd_preprocess(image_path: str, size: int) -> np.ndarray:
    import numpy as np
    from PIL import Image

    image = Image.open(image_path).convert("RGBA")
    # White background (Danbooru images are padded on white), square pad.
    canvas = Image.new("RGBA", image.size, (255, 255, 255, 255))
    canvas.alpha_composite(image)
    rgb = canvas.convert("RGB")
    w, h = rgb.size
    side = max(w, h)
    square = Image.new("RGB", (side, side), (255, 255, 255))
    square.paste(rgb, ((side - w) // 2, (side - h) // 2))
    resized = square.resize((size, size), Image.Resampling.BICUBIC)
    bgr = np.asarray(resized, dtype=np.float32)[:, :, ::-1]  # RGB -> BGR
    return np.expand_dims(np.ascontiguousarray(bgr), 0)


class _WdTaggerEngine:
    """Wraps a loaded WD tagger ONNX session plus its parsed tag table."""

    def __init__(
        self, session: Any, tag_names: list[str], categories: list[int], size: int
    ) -> None:
        self._session = session
        self.tag_names = tag_names
        self.categories = categories
        self._size = size
        self._input_name = session.get_inputs()[0].name

    def predict(self, image_path: str) -> np.ndarray:
        batch = _wd_preprocess(image_path, self._size)
        outputs = self._session.run(None, {self._input_name: batch})
        return outputs[0][0]


_wd_tagger_cache: dict[str, _WdTaggerLike] = {}


def _wd_session(model_path: str) -> Any:
    import onnxruntime as ort

    return ort.InferenceSession(model_path, providers=["CPUExecutionProvider"])


def _construct_wd_tagger(model_dir: str) -> _WdTaggerEngine:
    import csv

    model_path = Path(model_dir) / "model.onnx"
    tags_path = Path(model_dir) / "selected_tags.csv"
    _require_existing_file(str(model_path), "WD tagger model")
    _require_existing_file(str(tags_path), "WD tagger tag list")

    session = _wd_session(str(model_path))
    size = int(session.get_inputs()[0].shape[1])  # NHWC: [1, H, W, 3]
    tag_names: list[str] = []
    categories: list[int] = []
    with tags_path.open(encoding="utf-8", newline="") as fh:
        for i, row in enumerate(csv.DictReader(fh), start=1):
            try:
                tag_names.append(row["name"])
                categories.append(int(row["category"]))
            except (KeyError, ValueError, TypeError) as e:
                raise _vision_value_error(
                    f"WD tagger tag list malformed at row {i} ({tags_path}): {e}"
                ) from e
    return _WdTaggerEngine(session, tag_names, categories, size)


def _load_wd_tagger(model_dir: str) -> _WdTaggerLike:
    if model_dir not in _wd_tagger_cache:
        _wd_tagger_cache[model_dir] = _construct_wd_tagger(model_dir)
    return _wd_tagger_cache[model_dir]


def tag_frame(params: dict[str, Any]) -> dict[str, Any]:
    """WD tagger, single image. JSON-RPC `tag_frame`. Returns the general +
    character tags at or above `threshold` as one comma-separated caption,
    most confident tag first (Danbooru underscores become spaces), plus the
    top rating tag separately. An explicit `threshold: 0` means "every tag
    qualifies" -- only a genuinely missing/`None` threshold falls back to the
    model card's default."""
    image_path = str(params.get("image_path") or "")
    model_dir = str(params.get("model_dir") or "")
    raw_threshold = params.get("threshold")
    threshold = _WD_DEFAULT_THRESHOLD if raw_threshold is None else float(raw_threshold)
    if not image_path:
        raise _vision_value_error("`image_path` is required")
    if not model_dir:
        raise _vision_value_error("`model_dir` is required")
    _require_existing_file(image_path, "image")
    _require_dir(model_dir, "WD tagger model")

    engine = _load_wd_tagger(model_dir)
    probs = engine.predict(image_path)

    rating = ""
    rating_prob = -1.0
    scored_tags: list[tuple[float, str]] = []
    for name, category, prob in zip(engine.tag_names, engine.categories, probs, strict=True):
        p = float(prob)
        if category == _WD_RATING_CATEGORY:
            if p > rating_prob:
                rating, rating_prob = name, p
        elif category in (_WD_CHARACTER_CATEGORY, _WD_GENERAL_CATEGORY) and p >= threshold:
            scored_tags.append((p, name.replace("_", " ")))
    ordered = sorted(scored_tags, key=lambda t: t[0], reverse=True)
    return {
        "caption": ", ".join(name for _, name in ordered),
        "rating": rating,
        "threshold": threshold,
        "engine": "wd-eva02-tagger-v3",
    }
