#!/usr/bin/env python3
"""Generate the game's pixel-art tileset.

Writes `assets/tileset.png` (a 16-pixel-cell atlas), `assets/tileset.json` (the atlas
manifest: cell size, grid, and where each named sprite lives) and
`assets/tileset_preview.png`, a nearest-neighbour upscale on a sky background for
eyeballing the art without launching the game.

    python3 tools/gen_tileset.py

The art itself lives in `tools/art/`:

| module | draws |
|---|---|
| `art/kit.py` | the palette, the seamless terrain autotiles, the material seams, the fluids and the mechanical tiles |
| `art/kit_cast.py` | the runner's animation set, the enemies, the flag, the spawn, the butterflies |
| `art/kit_props.py` | scenery, clouds, effects and the window's glyphs |
| `art/kit_atlas.py` | the manifest's order: every named cell, once |

The direction is one light, no outlines and terrain that owns its seams; `art/kit.py`
states it once at the top. Every swatch is deterministic — the speckles come from an
integer hash, not `random` — so regenerating never produces a spurious diff.

A replacement tileset does not have to be generated here at all: the game reads
`tileset.json`, so any atlas of the same shape with the same names works.
"""

from __future__ import annotations

import json
import os
import sys

from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from art import kit  # noqa: E402
from art import kit_atlas  # noqa: E402

COLS = 16


def preview(atlas: Image.Image, scale: int = 8) -> Image.Image:
    """Upscaled atlas on a sky background, with a faint grid so cells are countable."""
    big = atlas.resize((atlas.width * scale, atlas.height * scale), Image.NEAREST)
    out = Image.new("RGBA", big.size, (150, 186, 218, 255))
    out.alpha_composite(big)
    px = out.load()
    for col in range(COLS + 1):
        for y in range(out.height):
            if y % 2 == 0:
                px[min(col * kit.CELL * scale, out.width - 1), y] = (255, 255, 255, 70)
    rows = atlas.height // kit.CELL
    for row in range(rows + 1):
        for x in range(out.width):
            if x % 2 == 0:
                px[x, min(row * kit.CELL * scale, out.height - 1)] = (255, 255, 255, 70)
    return out


def main() -> None:
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    assets = os.path.join(root, "assets")
    os.makedirs(assets, exist_ok=True)

    atlas, manifest = kit_atlas.build(COLS)
    atlas_path = os.path.join(assets, "tileset.png")
    atlas.save(atlas_path)

    doc = {
        "format": "evo-platformer-tileset",
        "version": 3,
        "image": "tileset.png",
        "cell": kit.CELL,
        "cols": COLS,
        "rows": atlas.height // kit.CELL,
        "generator": "tools/gen_tileset.py",
        "sprites": manifest,
    }
    with open(os.path.join(assets, "tileset.json"), "w", encoding="utf-8") as fh:
        json.dump(doc, fh, indent=2)
        fh.write("\n")

    preview(atlas).save(os.path.join(assets, "tileset_preview.png"))
    print(f"{atlas_path}  {atlas.width}x{atlas.height}px  {len(manifest)} sprites")


if __name__ == "__main__":
    main()
