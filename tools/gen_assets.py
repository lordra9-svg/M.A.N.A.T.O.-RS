#!/usr/bin/env python3
"""Generate original MIT-licensed assets for M.A.N.A.T.O.-RS.

Outputs:
  content/fightfx.sff, content/fightfx.air
  content/fight.snd
  content/stages/training/stage1.sff, stage1.def

The sprites are simple procedural pixel art; no third-party art is used.
"""

from __future__ import annotations

import math
import random
import struct
import zlib
from pathlib import Path

from gen_sfx import build_bank, guard, impact

ROOT = Path(__file__).resolve().parent.parent
CONTENT = ROOT / "content"


def png_rgba(width: int, height: int, pixels: bytes) -> bytes:
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw.extend(pixels[y * width * 4 : (y + 1) * width * 4])

    def chunk(kind: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + kind
            + data
            + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
        )

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def build_sff_v2(sprites: list[tuple[int, int, int, int, int, int, bytes]]) -> bytes:
    """Build a minimal SFF v2 with PNG32 sprites and no palette node."""
    header = bytearray(512)
    header[:12] = b"ElecbyteSpr\0"
    header[13] = 1
    header[15] = 2

    sprite_ofs = 512
    ldata_ofs = sprite_ofs + len(sprites) * 28
    struct.pack_into("<I", header, 36, sprite_ofs)
    struct.pack_into("<I", header, 40, len(sprites))
    struct.pack_into("<I", header, 44, 0)
    struct.pack_into("<I", header, 48, 0)
    struct.pack_into("<I", header, 52, ldata_ofs)

    nodes = bytearray()
    data = bytearray()
    for group, index, width, height, axis_x, axis_y, png in sprites:
        payload = struct.pack("<I", width * height * 4) + png
        node = bytearray(28)
        struct.pack_into("<HHHHhh", node, 0, group, index, width, height, axis_x, axis_y)
        struct.pack_into("<H", node, 12, 0)
        node[14] = 12  # PNG32
        node[15] = 32
        struct.pack_into("<I", node, 16, len(data))
        struct.pack_into("<I", node, 20, len(payload))
        struct.pack_into("<H", node, 24, 0)
        struct.pack_into("<H", node, 26, 0)
        nodes += node
        data += payload

    total = ldata_ofs + len(data)
    struct.pack_into("<I", header, 56, len(data))
    struct.pack_into("<I", header, 60, total)
    out = bytes(header) + bytes(nodes) + bytes(data)
    assert len(out) == total
    return out


def put(px: bytearray, width: int, x: int, y: int, color: tuple[int, int, int, int]) -> None:
    at = (y * width + x) * 4
    px[at : at + 4] = bytes(color)


def make_spark(
    size: int,
    color: tuple[int, int, int],
    frame: int,
    frames: int,
    rays: int,
    seed: int,
) -> bytes:
    rng = random.Random(seed + frame)
    cx = cy = size * 0.5
    t = frame / max(1, frames - 1)
    radius = size * (0.30 + 0.38 * t)
    fade = (1.0 - t) ** 1.25
    px = bytearray(size * size * 4)

    for y in range(size):
        for x in range(size):
            dx = x + 0.5 - cx
            dy = y + 0.5 - cy
            r = math.hypot(dx, dy)
            if r > radius:
                continue
            angle = math.atan2(dy, dx)
            ray = 0.0
            for k in range(rays):
                a = 2.0 * math.pi * k / rays + frame * 0.13
                d = abs(((angle - a + math.pi) % (2.0 * math.pi)) - math.pi)
                ray = max(ray, math.exp(-((d / 0.16) ** 2)))
            core = math.exp(-((r / (radius * 0.32)) ** 2))
            ring = math.exp(-(((r - radius * 0.72) / (radius * 0.15)) ** 2))
            sparkle = 0.0
            if rng.random() < 0.02:
                sparkle = rng.uniform(0.3, 1.0)
            intensity = (ray * 0.9 + core * 1.15 + ring * 0.45 + sparkle) * fade
            intensity *= max(0.0, 1.0 - (r / radius) ** 1.6)
            if intensity <= 0.01:
                continue
            alpha = max(0, min(255, int(intensity * 255)))
            rr = max(0, min(255, int(color[0] * 0.55 + 255 * intensity * 0.75)))
            gg = max(0, min(255, int(color[1] * 0.55 + 255 * intensity * 0.75)))
            bb = max(0, min(255, int(color[2] * 0.55 + 255 * intensity * 0.75)))
            put(px, size, x, y, (rr, gg, bb, alpha))
    return png_rgba(size, size, px)


def make_wall_tile(width: int, height: int) -> bytes:
    px = bytearray(width * height * 4)
    for y in range(height):
        for x in range(width):
            shade = 28 + int(18 * (y / height))
            r, g, b = shade, shade + 6, shade + 12
            if x % 24 == 0 or y % 24 == 0:
                r, g, b = 62, 76, 92
            if x % 24 == 1 or y % 24 == 1:
                r, g, b = 44, 54, 68
            put(px, width, x, y, (r, g, b, 255))
    return png_rgba(width, height, px)


def make_wall_panel(width: int, height: int) -> bytes:
    px = bytearray(width * height * 4)
    for y in range(height):
        for x in range(width):
            p = y / height
            r = int(34 + 18 * p)
            g = int(40 + 20 * p)
            b = int(48 + 24 * p)
            if y < 5:
                r, g, b = 86, 102, 120
            elif y < 8:
                r, g, b = 52, 62, 76
            elif y % 48 == 0:
                r, g, b = 54, 66, 82
            if x % 32 == 0:
                r, g, b = 48, 58, 72
            put(px, width, x, y, (r, g, b, 255))
    return png_rgba(width, height, px)


def make_floor_tile(width: int, height: int) -> bytes:
    px = bytearray(width * height * 4)
    for y in range(height):
        for x in range(width):
            p = y / height
            r = int(92 - 30 * p)
            g = int(66 - 24 * p)
            b = int(48 - 18 * p)
            # Perspective grid: lines get farther apart toward the bottom.
            row = int((1.0 - 1.0 / (1.0 + y / 12.0)) * height)
            if y % 18 == 0 or x % 21 == 0:
                r, g, b = 138, 102, 72
            if y % 18 == 1 or x % 21 == 1:
                r, g, b = 66, 46, 34
            put(px, width, x, y, (r, g, b, 255))
            _ = row
    return png_rgba(width, height, px)


def make_preview(width: int, height: int) -> bytes:
    px = bytearray(width * height * 4)
    for y in range(height):
        for x in range(width):
            if y < 170:
                shade = 24 + int(18 * (y / 170))
                r, g, b = shade, shade + 6, shade + 12
                if x % 48 == 0 or y % 48 == 0:
                    r, g, b = 54, 66, 82
            else:
                p = (y - 170) / (height - 170)
                r = int(96 - 34 * p)
                g = int(70 - 26 * p)
                b = int(50 - 20 * p)
                if x % 42 == 0 or (y - 170) % 24 == 0:
                    r, g, b = 138, 102, 72
            put(px, width, x, y, (r, g, b, 255))
    return png_rgba(width, height, px)


def write_fightfx() -> None:
    sprites = []
    actions = []
    index = 0
    for action, size, color, frames, rays, seed in [
        (0, 48, (255, 232, 150), 3, 8, 100),
        (1, 56, (255, 180, 80), 4, 10, 200),
        (2, 64, (255, 245, 190), 4, 12, 300),
        (40, 48, (140, 210, 255), 3, 8, 400),
    ]:
        actions.append((action, index, frames))
        for frame in range(frames):
            png = make_spark(size, color, frame, frames, rays, seed)
            sprites.append((0, index, size, size, size // 2, size // 2, png))
            index += 1

    (CONTENT / "fightfx.sff").write_bytes(build_sff_v2(sprites))

    lines = [
        "; M.A.N.A.T.O.-RS original fight effects",
        "; MIT License - Copyright (c) 2026 M.A.N.A.T.O.-RS contributors",
        "",
    ]
    durations = {0: [2, 3, 4], 1: [2, 3, 4, 5], 2: [2, 3, 4, 5], 40: [2, 3, 4]}
    for action, start, frames in actions:
        lines.append(f"[Begin Action {action}]")
        for i in range(frames):
            lines.append(f"0,{start + i},0,0,{durations[action][i]}")
        lines.append("")
    (CONTENT / "fightfx.air").write_text("\n".join(lines), encoding="utf-8", newline="\n")
    print("wrote content/fightfx.sff and content/fightfx.air")


def write_fight_snd() -> None:
    entries = [
        (0, 0, impact(0.16, 150.0, 2_600.0, 0.5, 501)),
        (0, 1, impact(0.22, 105.0, 1_900.0, 0.4, 502)),
        (0, 2, impact(0.32, 68.0, 1_300.0, 0.3, 503)),
        (1, 0, guard(0.12, 504)),
        (2, 0, impact(0.36, 60.0, 900.0, 0.2, 505)),
        (21, 0, impact(0.55, 48.0, 700.0, 0.25, 506)),
    ]
    (CONTENT / "fight.snd").write_bytes(build_bank(entries))
    print("wrote content/fight.snd")


def write_stage() -> None:
    sprites = [
        (1, 1, 96, 120, 0, 0, make_wall_tile(96, 120)),
        (1, 0, 96, 318, 0, 0, make_wall_panel(96, 318)),
        (0, 0, 105, 82, 0, 0, make_floor_tile(105, 82)),
        (9000, 1, 576, 240, 0, 0, make_preview(576, 240)),
    ]
    stage_dir = CONTENT / "stages/training"
    stage_dir.mkdir(parents=True, exist_ok=True)
    (stage_dir / "stage1.sff").write_bytes(build_sff_v2(sprites))

    def_text = """; M.A.N.A.T.O.-RS Training Grid
; MIT License - Copyright (c) 2026 M.A.N.A.T.O.-RS contributors
;
; Original procedural training stage. No third-party art is used.

[Info]
name = "Training Grid"
displayName = "Training Grid"
versionDate = 1.0
mugenVersion = 1.0
author = "M.A.N.A.T.O.-RS contributors"
localCoord = 320, 240

[Camera]
startX = 0
startY = 0
boundLeft = -125
boundRight = 125
boundHigh = -76
boundLow = 0
verticalFollow = .2
floorTension = 0
tension = 50

[PlayerInfo]
p1StartX = -70
p1StartY = 0
p1Facing = 1
p2StartX = 70
p2StartY = 0
p2Facing = -1
leftBound = -1000
rightBound = 1000

[Bound]
screenLeft = 15
screenRight = 15

[StageInfo]
zoffset = 204
autoturn = 1
resetBG = 1
localCoord = 320, 240

[Shadow]
intensity = 128
color = 0, 0, 0
yscale = .3
fade.range = 0, 0

[BGdef]
spr = stage1.sff

[Begin Action 9000]
9000,1, 0,0, -1

[BG 0]
type = normal
spriteno = 1, 1
start = 0, 0
delta = 1, 1
tile = 1, 1

[BG 0]
type = normal
spriteno = 1, 0
start = 0, -46
delta = 1, 1
tile = 1, 0

[BG 0]
type = normal
spriteno = 0, 0
start = 12, 190
delta = 1, 1
tile = 1, 0
"""
    (stage_dir / "stage1.def").write_text(def_text, encoding="utf-8", newline="\n")
    print("wrote content/stages/training/stage1.sff and stage1.def")


def main() -> None:
    write_fightfx()
    write_fight_snd()
    write_stage()


if __name__ == "__main__":
    main()
