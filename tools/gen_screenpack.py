#!/usr/bin/env python3
"""Generate the original MIT-licensed "Manato Grid" screenpack.

Outputs (under ``content/screenpacks/manato-grid``):
  system.def, fight.def, select.def, font.def, manato.sff

All art is procedural; no third-party assets are used. Run with
``python3 tools/gen_screenpack.py`` from the repository root.
"""

from __future__ import annotations

from pathlib import Path

from gen_assets import build_sff_v2, png_rgba, put

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "content" / "screenpacks" / "manato-grid"

TRANSPARENT = (0, 0, 0, 0)
INK = (232, 238, 248, 255)


def fill(width: int, height: int, color: tuple[int, int, int, int]) -> bytearray:
    return bytearray(bytes(color) * (width * height))


def border(pixels: bytearray, width: int, height: int, color, inset: int = 0) -> None:
    for x in range(inset, width - inset):
        put(pixels, width, x, inset, color)
        put(pixels, width, x, height - 1 - inset, color)
    for y in range(inset, height - inset):
        put(pixels, width, inset, y, color)
        put(pixels, width, width - 1 - inset, y, color)


def vertical_gradient(width: int, height: int, top, bottom) -> bytearray:
    px = bytearray()
    for y in range(height):
        t = y / max(1, height - 1)
        color = tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(4))
        px += bytes(color) * width
    return px


def make_panel(width: int, height: int, top, bottom, frame) -> bytes:
    px = vertical_gradient(width, height, top, bottom)
    border(px, width, height, frame)
    return png_rgba(width, height, bytes(px))


# ---------------------------------------------------------------------------
# 5x7 bitmap font, generated as one SFF sprite per character (group 1).
# ---------------------------------------------------------------------------

GLYPHS: dict[str, list[str]] = {
    " ": ["   "] * 7,
    "A": [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
    "B": ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."],
    "C": [".####", "#....", "#....", "#....", "#....", "#....", ".####"],
    "D": ["####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####."],
    "E": ["#####", "#....", "#....", "####.", "#....", "#....", "#####"],
    "F": ["#####", "#....", "#....", "####.", "#....", "#....", "#...."],
    "G": [".####", "#....", "#....", "#..##", "#...#", "#...#", ".####"],
    "H": ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
    "I": ["###", ".#.", ".#.", ".#.", ".#.", ".#.", "###"],
    "J": ["..###", "....#", "....#", "....#", "#...#", "#...#", ".###."],
    "K": ["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"],
    "L": ["#....", "#....", "#....", "#....", "#....", "#....", "#####"],
    "M": ["#...#", "##.##", "#.#.#", "#...#", "#...#", "#...#", "#...#"],
    "N": ["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"],
    "O": [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "P": ["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."],
    "Q": [".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#"],
    "R": ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"],
    "S": [".####", "#....", "#....", ".###.", "....#", "....#", "####."],
    "T": ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."],
    "U": ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "V": ["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."],
    "W": ["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#"],
    "X": ["#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#"],
    "Y": ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."],
    "Z": ["#####", "....#", "...#.", "..#..", ".#...", "#....", "#####"],
    "0": [".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###."],
    "1": ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."],
    "2": [".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####"],
    "3": ["####.", "....#", "....#", ".###.", "....#", "....#", "####."],
    "4": ["...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#."],
    "5": ["#####", "#....", "####.", "....#", "....#", "#...#", ".###."],
    "6": ["..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###."],
    "7": ["#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#..."],
    "8": [".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."],
    "9": [".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##.."],
    ".": ["..", "..", "..", "..", "..", "##", "##"],
    ",": ["..", "..", "..", "..", ".#", ".#", "#."],
    ":": ["..", "##", "##", "..", "##", "##", ".."],
    "!": ["#", "#", "#", "#", "#", ".", "#"],
    "-": ["....", "....", "....", "####", "....", "....", "...."],
    "/": ["....#", "....#", "...#.", "..#..", ".#...", "#....", "#...."],
    "%": ["##..#", "##.#.", "..#..", ".#...", "#..##", "..###", "....."],
    "+": [".....", "..#..", "..#..", "#####", "..#..", "..#..", "....."],
    "(": [".#", "#.", "#.", "#.", "#.", "#.", ".#"],
    ")": ["#.", ".#", ".#", ".#", ".#", ".#", "#."],
    "?": [".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#.."],
    "'": ["#", "#", ".", ".", ".", ".", "."],
    '"': ["#.#", "#.#", "...", "...", "...", "...", "..."],
}


# The grid uses a small-caps style: lowercase letters reuse uppercase cells.
for _upper in [key for key in GLYPHS if key.isalpha() and key.isupper()]:
    GLYPHS.setdefault(_upper.lower(), GLYPHS[_upper])


def glyph_sprite(ch: str) -> tuple[int, int, int, int, int, int, bytes]:
    rows = GLYPHS.get(ch.upper(), GLYPHS["?"])
    width = max(len(row) for row in rows)
    height = 7
    px = fill(width, height, TRANSPARENT)
    for y, row in enumerate(rows):
        for x, bit in enumerate(row):
            if bit == "#":
                put(px, width, x, y, INK)
    return (1, ord(ch), width, height, 0, 0, png_rgba(width, height, bytes(px)))


# ---------------------------------------------------------------------------
# Screenpack sprites (group 0).
# ---------------------------------------------------------------------------


def make_sprites() -> list[tuple[int, int, int, int, int, int, bytes]]:
    sprites: list[tuple[int, int, int, int, int, int, bytes]] = []

    # 0,0 life bar background / 0,1 front / 0,2 mid / 0,3 top frame.
    sprites.append(
        (0, 0, 132, 12, 0, 0, make_panel(132, 12, (26, 30, 44, 255), (12, 14, 22, 255), (70, 82, 108, 255)))
    )
    sprites.append(
        (0, 1, 132, 12, 0, 0, png_rgba(132, 12, bytes(vertical_gradient(132, 12, (120, 236, 255, 255), (34, 150, 210, 255)))))
    )
    sprites.append(
        (0, 2, 132, 12, 0, 0, png_rgba(132, 12, bytes(vertical_gradient(132, 12, (255, 120, 96, 255), (170, 42, 54, 255)))))
    )
    top = fill(132, 12, TRANSPARENT)
    border(top, 132, 12, (180, 204, 240, 200))
    sprites.append((0, 3, 132, 12, 0, 0, png_rgba(132, 12, bytes(top))))

    # 0,4 power background / 0,5 power front / 0,14 power top.
    sprites.append(
        (0, 4, 122, 8, 0, 0, make_panel(122, 8, (26, 30, 44, 255), (12, 14, 22, 255), (70, 82, 108, 255)))
    )
    sprites.append(
        (0, 5, 122, 8, 0, 0, png_rgba(122, 8, bytes(vertical_gradient(122, 8, (255, 232, 128, 255), (216, 150, 30, 255)))))
    )
    power_top = fill(122, 8, TRANSPARENT)
    border(power_top, 122, 8, (180, 204, 240, 160))
    sprites.append((0, 14, 122, 8, 0, 0, png_rgba(122, 8, bytes(power_top))))

    # 0,6 timer panel.
    sprites.append(
        (0, 6, 46, 22, 0, 0, make_panel(46, 22, (30, 36, 54, 255), (14, 18, 30, 255), (120, 140, 180, 255)))
    )

    # 0,7 win pip / 0,15 perfect / 0,16 clutch.
    pip = fill(10, 10, (24, 30, 44, 255))
    for y in range(10):
        for x in range(10):
            if 2 <= x <= 7 and 2 <= y <= 7:
                put(pip, 10, x, y, (255, 214, 92, 255))
    sprites.append((0, 7, 10, 10, 0, 0, png_rgba(10, 10, bytes(pip))))

    perfect = fill(10, 10, TRANSPARENT)
    for y in range(10):
        for x in range(10):
            if x in (0, 9) or y in (0, 9):
                put(perfect, 10, x, y, (255, 255, 255, 230))
    sprites.append((0, 15, 10, 10, 0, 0, png_rgba(10, 10, bytes(perfect))))

    clutch = fill(10, 10, TRANSPARENT)
    for y in range(10):
        for x in range(10):
            if x in (0, 9) or y in (0, 9):
                put(clutch, 10, x, y, (140, 255, 170, 200))
    sprites.append((0, 16, 10, 10, 0, 0, png_rgba(10, 10, bytes(clutch))))

    # 0,8 title banner.
    banner = vertical_gradient(320, 72, (18, 26, 46, 255), (6, 8, 16, 255))
    for x in range(320):
        for y in (0, 71):
            put(banner, 320, x, y, (120, 200, 255, 255))
    for y in range(72):
        for x in (0, 319):
            put(banner, 320, x, y, (120, 200, 255, 255))
    sprites.append((0, 8, 320, 72, 0, 0, png_rgba(320, 72, bytes(banner))))

    # 0,9 cursor.
    cursor = fill(14, 14, TRANSPARENT)
    for y in range(14):
        for x in range(14):
            if x <= y and x < 10 and y >= 2:
                put(cursor, 14, x + 2, y, (255, 214, 92, 255))
    sprites.append((0, 9, 14, 14, 0, 0, png_rgba(14, 14, bytes(cursor))))

    # 0,10 select cell background.
    cell = fill(40, 40, (22, 28, 44, 235))
    border(cell, 40, 40, (96, 120, 160, 255))
    for y in range(3, 6):
        for x in range(3, 37):
            put(cell, 40, x, y, (60, 74, 104, 255))
    sprites.append((0, 10, 40, 40, 0, 0, png_rgba(40, 40, bytes(cell))))

    # 0,12 title background, 0,13 select background.
    for group_index, top_c, bottom_c in (
        (12, (18, 28, 54, 255), (4, 6, 14, 255)),
        (13, (10, 22, 30, 255), (4, 8, 12, 255)),
    ):
        bg = vertical_gradient(320, 240, top_c, bottom_c)
        for y in range(0, 240, 24):
            for x in range(320):
                put(bg, 320, x, y, (40, 60, 90, 255))
        for x in range(0, 320, 24):
            for y in range(240):
                put(bg, 320, x, y, (30, 44, 70, 255))
        sprites.append((0, group_index, 320, 240, 0, 0, png_rgba(320, 240, bytes(bg))))

    # 9000,0 cell portrait / 9000,1 large portrait (generic silhouette used
    # when the character SFF has no portrait sprite).
    for index, size in ((0, 32), (1, 48)):
        portrait = fill(size, size, TRANSPARENT)
        cx, cy = size / 2.0, size * 0.42
        for y in range(size):
            for x in range(size):
                dx = (x - cx) / (size * 0.28)
                dy = (y - cy) / (size * 0.24)
                if dx * dx + dy * dy <= 1.0:
                    shade = 120 + int(80 * (1.0 - y / size))
                    put(portrait, size, x, y, (shade, shade + 30, shade + 60, 255))
                if y > size * 0.72 and abs(x - cx) < size * 0.34:
                    shade = 90 + int(50 * (1.0 - (y - size * 0.72) / (size * 0.28)))
                    put(portrait, size, x, y, (shade, shade + 24, shade + 48, 255))
        sprites.append((9000, index, size, size, size // 2, size // 2, png_rgba(size, size, bytes(portrait))))

    # 0,17 combo background.
    combo = fill(86, 18, (10, 14, 24, 190))
    border(combo, 86, 18, (120, 200, 255, 180))
    sprites.append((0, 17, 86, 18, 0, 0, png_rgba(86, 18, bytes(combo))))

    return sprites


def write_system_def() -> None:
    (OUT / "system.def").write_text(
        """; Manato Grid screenpack - MIT License
; Copyright (c) 2026 M.A.N.A.T.O.-RS contributors
; Original procedural art. No third-party assets.

[Info]
name = "Manato Grid"
author = "M.A.N.A.T.O.-RS contributors"
localcoord = 320, 240
mugenversion = 1.1

[Files]
spr = manato.sff
fight = fight.def
select = select.def
font0 = font.def

[Title Info]
title.spr = 0, 8
title.pos = 0, 24
title.text = "MANATO GRID"
title.font = 0
title.offset = 160, 30
title.scale = 3, 3
title.align = 1
title.color = 180, 228, 255
menu.pos = 160, 128
menu.item.offset = 0, 0
menu.item.spacing = 0, 13
menu.item.font = 0
menu.item.scale = 1, 1
menu.item.active.font = 0
menu.item.active.scale = 1.3, 1.3
menu.itemname.arcade = "ARCADE"
menu.itemname.versus = "VERSUS"
menu.itemname.training = "TRAINING"
menu.itemname.options = "OPTIONS"
menu.itemname.quit = "QUIT"

[TitleBGdef]
spr = manato.sff
bgclearcolor = 4, 6, 14

[TitleBGdef bg 0]
type = normal
spriteno = 0, 12
start = 0, 0
delta = 0, 0

[TitleBGdef BGCtrl 0]
type = SinX
value = 3, 180, 0

[Select Info]
rows = 1
columns = 4
pos = 40, 104
cell.size = 40, 40
cell.spacing = 8, 4
cell.bg.spr = 0, 10
cell.name.font = 0
cell.name.offset = 20, 42
cell.name.align = 1
cell.name.scale = 1, 1
cell.face.spr = 9000, 0
cell.face.offset = 4, 4
p1.cursor.startcell = 0, 0
p1.cursor.spr = 0, 9
p1.cursor.offset = 4, 4
p1.name.font = 0
p1.name.offset = 80, 200
p1.name.align = 1
p1.face.spr = 9000, 1
p1.face.offset = 4, 146
p2.cursor.startcell = 3, 0
p2.cursor.spr = 0, 9
p2.cursor.offset = 4, 4
p2.name.font = 0
p2.name.offset = 240, 200
p2.name.align = 1
p2.face.spr = 9000, 1
p2.face.offset = 268, 146
stage.spr = 0, 6
stage.pos = 137, 214
stage.align = 1
timer.count = 15
timer.pos = 286, 36
timer.counter.font = 0
timer.counter.text = "%d"
timer.counter.align = 1
timer.counter.scale = 1, 1

[SelectBGdef]
spr = manato.sff
bgclearcolor = 4, 8, 12

[SelectBGdef bg 0]
type = normal
spriteno = 0, 13
start = 0, 0
delta = 0, 0
""",
        encoding="utf-8",
        newline="\n",
    )


def write_fight_def() -> None:
    (OUT / "fight.def").write_text(
        """; Manato Grid fight screen - MIT License

[Info]
name = "Manato Grid"
author = "M.A.N.A.T.O.-RS contributors"
localcoord = 320, 240

[Files]
spr = manato.sff
font0 = font.def

[LifeBar]
pos = 6, 6
range.x = 132
range.y = 12
bg0.spr = 0, 0
front.spr = 0, 1
mid.spr = 0, 2
top.spr = 0, 3
mid.delay = 18
mid.steps = 2
mid.mult = 1
value.font = 0
value.text = ""
value.offset = 66, 3
value.align = 1

[LifeBarP2]
pos = 182, 6

[PowerBar]
pos = 6, 222
range.x = 122
range.y = 8
bg0.spr = 0, 4
front.spr = 0, 5
top.spr = 0, 14
counter.font = 0
counter.text = "%d"
counter.offset = 61, -9
counter.align = 1

[PowerBarP2]
pos = 192, 222

[Face]
pos = 6, 22
face.spr = 9000, 0
face.palshare = 1

[Name]
pos = 6, 22
name.font = 0
name.text =
name.align = 0
name.scale = 1, 1

[NameP2]
pos = 314, 22
name.align = 2

[Time]
pos = 137, 6
bg.spr = 0, 6
counter.font = 0
counter.text = "%d"
counter.offset = 23, 6
counter.align = 1
counter.scale = 1, 1
counter.displaytime = 99

[WinIcon]
pos = 250, 224
iconoffset = 12, 0
n.spr = 0, 7
t.spr = 0, 7
perfect.spr = 0, 15
clutch.spr = 0, 16
useiconupto = 0

[WinIconP2]
pos = 70, 224

[Combo]
pos = 22, 56
start.x = 48
bg.spr = 0, 17
counter.font = 0
counter.text = "%d"
counter.offset = 8, 5
text.font = 0
text.text = " HITS"
text.offset = 26, 5
displaytime = 60
showspeed = 4
hidespeed = 4

[ComboP2]
pos = 212, 56
start.x = -48

[Action]
pos = 20, 196
spacing = 0, 11
start.x = 40
displaytime = 90
showspeed = 4
hidespeed = 4
text.font = 0
text.align = 0
text.offset = 6, 5
text.color = 180, 228, 255
bg.spr = 0, 17
team2.pos = 214, 196
team2.start.x = -40

[Round]
pos = 0, 0
start.waittime = 20
round.time = 40
fight.time = 40
ctrl.time = 30
ko.time = 40
dko.time = 40
to.time = 40
win.time = 60
win2.time = 60
slow.time = 60
slow.fadetime = 45
slow.speed = 0.25

round.default.text = "ROUND %d"
round.default.font = 0
round.default.pos = 160, 96
round.default.scale = 3, 3
round.default.align = 1
round.default.color = 180, 228, 255

round.final.text = "FINAL ROUND"
round.final.font = 0
round.final.pos = 160, 96
round.final.scale = 2, 2
round.final.align = 1
round.final.color = 255, 214, 92

fight.text = "FIGHT!"
fight.font = 0
fight.pos = 160, 96
fight.scale = 3, 3
fight.align = 1
fight.color = 255, 232, 140

ko.text = "K.O."
ko.font = 0
ko.pos = 160, 96
ko.scale = 4, 4
ko.align = 1
ko.color = 255, 120, 96

dko.text = "DRAW GAME"
dko.font = 0
dko.pos = 160, 96
dko.scale = 2, 2
dko.align = 1

to.text = "TIME OVER"
to.font = 0
to.pos = 160, 96
to.scale = 2, 2
to.align = 1

win.text = "WINS"
win.font = 0
win.pos = 160, 96
win.scale = 3, 3
win.align = 1
win.color = 180, 255, 200

win2.text = "PERFECT"
win2.font = 0
win2.pos = 160, 96
win2.scale = 2, 2
win2.align = 1
win2.color = 255, 214, 92
""",
        encoding="utf-8",
        newline="\n",
    )


def write_select_def() -> None:
    (OUT / "select.def").write_text(
        """; Manato Grid roster - MIT License

[Characters]
content/chars/kaen/kaen.def
content/chars/nival/nival.def
content/chars/ordan/ordan.def

[ExtraStages]
content/stages/training/stage1.def
""",
        encoding="utf-8",
        newline="\n",
    )


def write_font_def() -> None:
    (OUT / "font.def").write_text(
        """; Manato Grid bitmap font - MIT License

[FNT v2]
version = 2, 1

[Def]
file = manato.sff
bank = 1
size = 0, 8
spacing = 1, 0
offset = 0, 0
type = 0
""",
        encoding="utf-8",
        newline="\n",
    )


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    sprites = make_sprites()
    for ch in GLYPHS:
        sprites.append(glyph_sprite(ch))
    (OUT / "manato.sff").write_bytes(build_sff_v2(sprites))
    write_system_def()
    write_fight_def()
    write_select_def()
    write_font_def()
    print(f"wrote {OUT.relative_to(ROOT)} (system.def, fight.def, select.def, font.def, manato.sff)")


if __name__ == "__main__":
    main()
