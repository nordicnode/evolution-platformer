#!/usr/bin/env python3
"""Generate the game's pixel-art tileset.

Writes `assets/tileset.png` (a 16x16-cell atlas), `assets/tileset.json` (the atlas
manifest: cell size, grid, and where each named sprite lives), and
`assets/tileset_preview.png`, a nearest-neighbour upscale on a sky background for
eyeballing the art without launching the game.

    python3 tools/gen_tileset.py

The art itself lives in `tools/art/`, one module per family:

| module | draws |
|---|---|
| `art/terrain.py` | the autotiled ground, sand and the rock under both, the spikes, and the material seams |
| `art/characters.py` | the player's animation set, the walker, the goal flag, the spawn |
| `art/props.py` | decor (bushes, trees, flowers, rocks, mushrooms), butterflies, clouds and effect puffs |
| `art/ui.py` | the window's own tool glyphs — undo, redo, frame, details, restart — which the game never draws |

Each module returns `(name, image)` pairs; this file owns the *grid* and the
manifest, so a new sprite is one entry in one module and nothing else. Every module
draws from `art/common.py`'s colours, so three hands cannot produce three palettes.

Deterministic: the speckle texture uses a hash-based value noise, not `random`, so
regenerating never produces a diff. Replace the art by editing the modules, or by
replacing the PNG and manifest with any tileset of the same shape — the game reads
the manifest, not hard-coded coordinates.
"""

from __future__ import annotations

import json
import os
import sys

from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from art import characters, props, terrain, ui  # noqa: E402
from art.common import CELL, CHECKER, TRANSPARENT, cell, rgba  # noqa: E402

# The atlas is deliberately wider than tall: sprite names are grouped by family, and a
# family that grows (one more run frame, one more prop) should not need a new grid. It
# went from six rows to nine when the sand material arrived — twenty-six tiles, a second
# complete autotile family — and to ten with the material seams, which are per material
# rather than per pair of materials: a fifth material wants four more cells and nothing
# else. Widen it before the sprites stop fitting rather than after: the generator refuses
# to write an atlas a sprite does not fit in, so the failure is a clear error and not a
# silently missing tile.
#
# Fifteen rows held two hundred and thirty-three sprites with seven cells to spare, and `ROWS` went
# to sixteen for the trap tiles rather than to fifteen-and-a-half: the two tiles and their four
# crack overlays fit either way, and a growable atlas is the point of it being wider than it needs
# to be. Fourteen rows held two hundred and twenty-three with one cell spare — a full atlas rather
# than a growable one — and the window's own tool glyphs (`art/ui.py`) are the family that made it
# fifteen. They are the first sprites here that the *game* never draws, and they arrived last for
# the same reason the seams did: a family whose whole purpose is the application's chrome can wait
# for one that goes in the level.
COLS = 16
ROWS = 16

# Read order, so the manifest is stable and the preview is readable: nothing may
# reorder this except a change to `assets/README.md`, which lists the same names.
FAMILIES = (
    ("empty", lambda: [("empty", cell())]),
    ("terrain", terrain.sprites),
    ("characters", characters.sprites),
    ("props", props.sprites),
    # The window's tool icons, in front of the seams for the seam family's own reason (see
    # `art/ui.py`): they are drawn on the app's plates and over nothing at all.
    ("ui", ui.sprites),
    # The material seams go last, after every sprite that was here before them: they are
    # the only family drawn *over* other families' art, and keeping them at the end means
    # a new one is an additive change to a generated file rather than a reshuffle.
    ("seams", terrain.blends),
)


def build() -> tuple[Image.Image, list[dict]]:
    sprites: list[tuple[str, Image.Image]] = []
    for _family, produce in FAMILIES:
        sprites.extend(produce())

    names = [name for name, _ in sprites]
    duplicates = sorted({n for n in names if names.count(n) > 1})
    if duplicates:
        raise SystemExit(f"duplicate sprite name(s): {', '.join(duplicates)}")
    if len(sprites) > COLS * ROWS:
        raise SystemExit(
            f"{len(sprites)} sprites do not fit in a {COLS}x{ROWS} grid "
            f"({COLS * ROWS} cells); widen the grid and update the manifest docs"
        )

    atlas = Image.new("RGBA", (COLS * CELL, ROWS * CELL), TRANSPARENT)
    manifest = []
    for index, (name, sprite) in enumerate(sprites):
        if sprite.size != (CELL, CELL):
            raise SystemExit(f"{name} is {sprite.size}, expected a {CELL}x{CELL} cell")
        col, row = index % COLS, index // COLS
        atlas.paste(sprite, (col * CELL, row * CELL))
        manifest.append({"name": name, "col": col, "row": row})

    return atlas, manifest


def preview(atlas: Image.Image, scale: int = 8) -> Image.Image:
    """Upscaled atlas on a sky background, with a grid so cells are countable."""
    big = atlas.resize((atlas.width * scale, atlas.height * scale), Image.NEAREST)
    out = Image.new("RGBA", big.size, rgba(CHECKER))
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
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    assets = os.path.join(root, "assets")
    os.makedirs(assets, exist_ok=True)

    atlas, manifest = build()
    atlas_path = os.path.join(assets, "tileset.png")
    atlas.save(atlas_path)

    doc = {
        "format": "evo-platformer-tileset",
        "version": 2,
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
    col = -1
    for entry in manifest:
        if entry["col"] == 0:
            col = -1
            print(f"  row {entry['row']}:")
        print(f"    {entry['name']:<18} col {entry['col']} row {entry['row']}")


if __name__ == "__main__":
    main()
