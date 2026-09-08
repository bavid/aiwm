"""Generate the placeholder app icon: scripts/gen_icon.py -> src-tauri/icons/icon.png

A 1024x1024 RGBA PNG, no third-party deps. Replace with real branding later
(tracked in docs/TODO.md). After running this, generate the platform icon set:

    pnpm --dir ui exec tauri icon ../src-tauri/icons/icon.png
"""

from __future__ import annotations

import struct
import zlib
from binascii import crc32
from pathlib import Path

SIZE = 1024
BG = (76, 110, 245, 255)      # --accent
BAR = (255, 255, 255, 235)
RADIUS = 180
BARS = [(0.30, 0.14), (0.52, 0.14), (0.74, 0.14)]  # (y fraction, height fraction)


def _rounded(x: int, y: int, w: int, h: int, r: int) -> bool:
    if x < 0 or y < 0 or x >= w or y >= h:
        return False
    cx = min(max(x, r), w - 1 - r)
    cy = min(max(y, r), h - 1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def _pixels() -> bytearray:
    buf = bytearray()
    bar_spans = [
        (int(fy * SIZE), int(fy * SIZE) + int(fh * SIZE)) for fy, fh in BARS
    ]
    inset = int(SIZE * 0.16)
    for y in range(SIZE):
        buf.append(0)  # filter type 0
        for x in range(SIZE):
            if not _rounded(x, y, SIZE, SIZE, RADIUS):
                buf.extend((0, 0, 0, 0))
                continue
            px = BG
            if inset <= x < SIZE - inset:
                for y0, y1 in bar_spans:
                    if y0 <= y < y1 and _rounded(
                        x - inset, y - y0, SIZE - 2 * inset, y1 - y0, 40
                    ):
                        px = BAR
                        break
            buf.extend(px)
    return buf


def _chunk(tag: bytes, data: bytes) -> bytes:
    return struct.pack(">I", len(data)) + tag + data + struct.pack(
        ">I", crc32(tag + data) & 0xFFFFFFFF
    )


def main() -> None:
    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    png = b"\x89PNG\r\n\x1a\n"
    png += _chunk(b"IHDR", ihdr)
    png += _chunk(b"IDAT", zlib.compress(bytes(_pixels()), 9))
    png += _chunk(b"IEND", b"")

    out = Path(__file__).resolve().parent.parent / "src-tauri" / "icons" / "icon.png"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(png)
    print(f"wrote {out} ({len(png)} bytes)")


if __name__ == "__main__":
    main()
