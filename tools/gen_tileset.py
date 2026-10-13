#!/usr/bin/env python3
"""Generate the game's pixel-art tileset.

Writes `assets/tileset.png` (a 16x16-cell atlas) and `assets/tileset.json` (the
atlas manifest: cell size, grid, and where each named sprite lives), plus
`assets/tileset_preview.png`, an 8x nearest-neighbour upscale on a sky
background for eyeballing the art without launching the game.

    python3 tools/gen_tileset.py

Deterministic: the speckle texture uses a hash-based value noise, not `random`,
so regenerating never produces a diff. Replace the art by editing this file, or
by replacing the PNG and manifest with any tileset of the same shape — the app
reads the manifest, not hard-coded coordinates.
"""

from __future__ import annotations

import json
import os
from PIL import Image

CELL = 16
COLS = 8
ROWS = 3

# --- palette ---------------------------------------------------------------
TRANSPARENT = (0, 0, 0, 0)
SKY = (94, 168, 231)

GRASS_LIGHT = (140, 214, 88)
GRASS = (92, 176, 62)
GRASS_DARK = (58, 122, 44)
DIRT = (150, 104, 64)
DIRT_LIGHT = (182, 134, 88)
DIRT_DARK = (108, 70, 44)
STONE = (152, 156, 166)
STONE_DARK = (106, 110, 122)
STONE_DEEP = (74, 78, 90)

PLAYER_BODY = (92, 216, 234)
PLAYER_DARK = (32, 128, 156)
PLAYER_EYE = (250, 252, 255)
PLAYER_TRIM = (250, 214, 102)

WALKER_BODY = (188, 104, 220)
WALKER_DARK = (124, 58, 154)
WALKER_EYE = (255, 246, 126)

FLAG = (232, 80, 88)
FLAG_DARK = (168, 44, 52)
POLE = (236, 230, 214)
POLE_DARK = (178, 170, 152)

MARKER = (255, 214, 102)
MARKER_DIM = (214, 158, 52)

PALETTE = {
    ".": TRANSPARENT,
    # player
    "b": PLAYER_BODY,
    "d": PLAYER_DARK,
    "e": PLAYER_EYE,
    "t": PLAYER_TRIM,
    # walker
    "w": WALKER_BODY,
    "v": WALKER_DARK,
    "y": WALKER_EYE,
    # goal
    "f": FLAG,
    "g": FLAG_DARK,
    "p": POLE,
    "q": POLE_DARK,
    # editor marker
    "m": MARKER,
    "n": MARKER_DIM,
}


def h(x: int, y: int, seed: int) -> float:
    """Deterministic value noise in [0, 1) from integer coordinates."""
    n = (x * 73856093) ^ (y * 19349663) ^ (seed * 83492791)
    n &= 0xFFFFFFFF
    n ^= n >> 13
    n = (n * 1274126177) & 0xFFFFFFFF
    n ^= n >> 16
    return (n & 0xFFFF) / 65536.0


def cell() -> Image.Image:
    return Image.new("RGBA", (CELL, CELL), TRANSPARENT)


def art(rows: list[str]) -> Image.Image:
    """Draw a sprite from 16 rows of 16 palette keys."""
    assert len(rows) == CELL, f"expected {CELL} rows, got {len(rows)}"
    img = cell()
    px = img.load()
    for y, row in enumerate(rows):
        assert len(row) == CELL, f"row {y} is {len(row)} wide, expected {CELL}: {row!r}"
        for x, key in enumerate(row):
            px[x, y] = PALETTE[key]
    return img


# --- terrain ---------------------------------------------------------------

GROUND_ROWS = 4  # how much of a grass tile is grass


def terrain(grass_top: bool, edge: int = 0) -> Image.Image:
    """A solid tile. `edge` is -1 for a left-exposed tile, +1 for right-exposed."""
    img = cell()
    px = img.load()
    for y in range(CELL):
        for x in range(CELL):
            if grass_top and y < GROUND_ROWS:
                n = h(x, y, 7)
                colour = GRASS_LIGHT if y == 0 or n < 0.30 else GRASS
                if n > 0.86:
                    colour = GRASS_DARK
                px[x, y] = colour
            else:
                n = h(x, y, 13)
                colour = DIRT
                if n < 0.24:
                    colour = DIRT_LIGHT
                elif n > 0.78:
                    colour = DIRT_DARK
                px[x, y] = colour
        # A little shading where the tile meets open air.
        if edge < 0:
            px[0, y] = GRASS_DARK if (grass_top and y < GROUND_ROWS) else DIRT_DARK
        elif edge > 0:
            px[CELL - 1, y] = GRASS_DARK if (grass_top and y < GROUND_ROWS) else DIRT_DARK
    return img


def spikes() -> Image.Image:
    """Three upward spikes on a stone base."""
    img = cell()
    px = img.load()
    base = CELL - 3
    for y in range(base, CELL):
        for x in range(CELL):
            n = h(x, y, 29)
            px[x, y] = STONE_DEEP if n < 0.5 else STONE_DARK
    for i in range(3):
        centre = 2.5 + i * 5.5
        apex = 5
        for y in range(apex, base):
            t = (y - apex) / (base - apex)
            half = 0.7 + t * 2.4
            for x in range(CELL):
                if abs((x + 0.5) - centre) <= half:
                    px[x, y] = STONE if abs((x + 0.5) - centre) < half * 0.45 else STONE_DARK
    return img


# --- characters ------------------------------------------------------------

PLAYER_STAND = [
    "................",
    "................",
    "......tttt......",
    ".....tbbbbt.....",
    ".....bdbbdb.....",
    ".....beeeeb.....",
    ".....bbbbbb.....",
    "....bbbbbbbb....",
    "...bbdbbbbdbb...",
    "...bbbbbbbbbb...",
    "....dddddddd....",
    "....bb....bb....",
    "....bb....bb....",
    "...ddd....ddd...",
    "...ddd....ddd...",
    "................",
]

PLAYER_RUN_A = [
    "................",
    "................",
    "......tttt......",
    ".....tbbbbt.....",
    ".....bdbbdb.....",
    ".....beeeeb.....",
    ".....bbbbbb.....",
    "...bbbbbbbb.....",
    "..bbdbbbbbd.....",
    "..bbbbbbbbb.....",
    "...dddddddd.....",
    "....bb..bb......",
    "...bb....bb.....",
    "..ddd....bb.....",
    ".ddd......d.....",
    "................",
]

PLAYER_RUN_B = [
    "................",
    "................",
    "......tttt......",
    ".....tbbbbt.....",
    ".....bdbbdb.....",
    ".....beeeeb.....",
    ".....bbbbbb.....",
    "....bbbbbbbb....",
    "....bbbbbbdbb...",
    "....bbbbbbbbb...",
    "....dddddddd....",
    "......bb..bb....",
    ".....bb....bb...",
    "....bb....ddd...",
    "....d......ddd..",
    "................",
]

PLAYER_JUMP = [
    "................",
    "...bb......bb...",
    "...bb.tttt.bb...",
    "....tbbbbt......",
    "....bdbbdb......",
    "....beeeeb......",
    "....bbbbbb......",
    "...bbbbbbbb.....",
    "..bbbbbbbbbb....",
    "..ddbbbbbbdd....",
    "...dddddddd.....",
    "....bbbbbb......",
    "...ddd..ddd.....",
    "...ddd..ddd.....",
    "................",
    "................",
]

WALKER_A = [
    "................",
    "................",
    "................",
    "......wwww......",
    "....wwwwwwww....",
    "...wwwwwwwwww...",
    "..wwywwwwwwyww..",
    "..wwywwwwwwyww..",
    "..wwwwvvwwwwww..",
    "..wwwwvvwwwwww..",
    ".wwwwwwwwwwwwww.",
    ".wvvvvvvvvvvvvw.",
    "..vvvvvvvvvvvv..",
    "....vv....vv....",
    "................",
    "................",
]

WALKER_B = [
    "................",
    "................",
    "................",
    "................",
    ".....wwwwww.....",
    "...wwwwwwwwww...",
    "..wwywwwwwwyww..",
    "..wwywwwwwwyww..",
    ".wwwwwvvwwwwww..",
    ".wwwwwvvwwwwww..",
    "wwwwwwwwwwwwwwww",
    "wvvvvvvvvvvvvvvw",
    ".vvvvvvvvvvvvvv.",
    "...vv......vv...",
    "................",
    "................",
]

WALKER_DEAD = [
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "..www......www..",
    ".wwvvwwwwwwvvww.",
    "wvvvvvvvvvvvvvvw",
    ".vvvvvvvvvvvvvv.",
    "................",
]

GOAL_TOP = [
    "................",
    "....qfffffff....",
    "....qffffffff...",
    "....qfffffff....",
    "....qffffff.....",
    "....qffff.......",
    "....qff.........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
]

GOAL_BOTTOM = [
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "....q...........",
    "...qqq..........",
    "..qqqqq.........",
    "..qqqqq.........",
]

SPAWN_MARKER = [
    "................",
    "................",
    "..mmmmmmmmmmmm..",
    "..m..........m..",
    "..m..........m..",
    "..m..........m..",
    "..m..........m..",
    "..m..........m..",
    "..m..........m..",
    "..m..........m..",
    "..m..........m..",
    "..m..........m..",
    "..m..........m..",
    "..mmmmmmmmmmmm..",
    "................",
    "................",
]


def build() -> tuple[Image.Image, list[dict]]:
    blank = cell()
    sprites = [
        ("empty", blank),
        ("grass", terrain(grass_top=True)),
        ("grass_left", terrain(grass_top=True, edge=-1)),
        ("grass_right", terrain(grass_top=True, edge=1)),
        ("dirt", terrain(grass_top=False)),
        ("dirt_left", terrain(grass_top=False, edge=-1)),
        ("dirt_right", terrain(grass_top=False, edge=1)),
        ("spike", spikes()),
        ("player_stand", art(PLAYER_STAND)),
        ("player_run_a", art(PLAYER_RUN_A)),
        ("player_run_b", art(PLAYER_RUN_B)),
        ("player_jump", art(PLAYER_JUMP)),
        ("walker_a", art(WALKER_A)),
        ("walker_b", art(WALKER_B)),
        ("walker_dead", art(WALKER_DEAD)),
        ("goal_top", art(GOAL_TOP)),
        ("goal_bottom", art(GOAL_BOTTOM)),
        ("spawn_marker", art(SPAWN_MARKER)),
    ]
    assert len(sprites) <= COLS * ROWS, "atlas is too small for the sprite list"

    atlas = Image.new("RGBA", (COLS * CELL, ROWS * CELL), TRANSPARENT)
    manifest = []
    for index, (name, sprite) in enumerate(sprites):
        col, row = index % COLS, index // COLS
        atlas.paste(sprite, (col * CELL, row * CELL))
        manifest.append({"name": name, "col": col, "row": row})

    return atlas, manifest


def preview(atlas: Image.Image, scale: int = 8) -> Image.Image:
    """Upscaled atlas on a sky background, with a grid so cells are countable."""
    big = atlas.resize(
        (atlas.width * scale, atlas.height * scale), Image.NEAREST
    )
    out = Image.new("RGBA", big.size, SKY + (255,))
    out.alpha_composite(big)
    px = out.load()
    for col in range(COLS + 1):
        for y in range(out.height):
            if y % 2 == 0:
                px[min(col * CELL * scale, out.width - 1), y] = (255, 255, 255, 90)
    for row in range(ROWS + 1):
        for x in range(out.width):
            if x % 2 == 0:
                px[x, min(row * CELL * scale, out.height - 1)] = (255, 255, 255, 90)
    return out


def main() -> None:
    assets = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "assets")
    os.makedirs(assets, exist_ok=True)

    atlas, manifest = build()
    atlas_path = os.path.join(assets, "tileset.png")
    atlas.save(atlas_path)

    doc = {
        "format": "evo-platformer-tileset",
        "version": 1,
        "image": "tileset.png",
        "cell": CELL,
        "cols": COLS,
        "rows": ROWS,
        "generator": "tools/gen_tileset.py",
        "sprites": manifest,
    }
    with open(os.path.join(assets, "tileset.json"), "w", encoding="utf-8") as fh:
        json.dump(doc, fh, indent=2)
        fh.write("\n")

    preview(atlas).save(os.path.join(assets, "tileset_preview.png"))

    print(f"{atlas_path}  {atlas.width}x{atlas.height}px  {len(manifest)} sprites")
    for entry in manifest:
        print(f"  {entry['name']:<14} col {entry['col']} row {entry['row']}")


if __name__ == "__main__":
    main()
