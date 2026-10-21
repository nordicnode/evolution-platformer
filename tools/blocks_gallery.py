#!/usr/bin/env python3
"""Draw the README's blocks-and-cast sheet: `docs/images/blocks-gallery.png`.

The sheet is the *palette in pictures*: one cell for every part the editor can
place, plus the runner, taken straight from `assets/tileset.png` with the
manifest's own cell coordinates. Nothing here is drawn by hand, so a sprite that
changes in the atlas changes here the next time this runs.

The list below is not free to drift from the palette. `crates/app/src/editor.rs`
is the authority for which sprites the brushes draw, and `brushes()` parses it so
that a brush added to the editor and forgotten here fails the script instead of
quietly going missing from the README — which is exactly what happened to the
brittle materials, the spawn marker and the rope.

Usage:

    python3 tools/blocks_gallery.py            # write the sheet
    python3 tools/blocks_gallery.py --check    # fail if the sheet is stale

Needs Pillow, like `tools/gen_tileset.py`.
"""

from __future__ import annotations

import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
EDITOR = ROOT / "crates/app/src/editor.rs"
OUT = ROOT / "docs/images/blocks-gallery.png"

# The sheet's geometry, in pixels. A cell is one 16-pixel atlas tile at 6x, and
# the pitch leaves 54 pixels of paper between cells. The outer margin is wider
# than that by as much as the widest label ("CRACKED TIMBER") overhangs its cell.
SCALE = 6
PITCH = 150
MARGIN = 57
TITLE_TOP = 20
FIRST_ROW = 68
ROW_PITCH = 138
LABEL_GAP = 12

# The window's own paper and ink (`crates/app/src/theme.rs`).
PAPER = (255, 249, 235)
INK = (44, 24, 36)
TITLE = "THE BLOCKS AND THE CAST"

TITLE_SIZE = 20
TITLE_TRACKING = 4.4
LABEL_SIZE = 14
LABEL_TRACKING = 3.0


@dataclass(frozen=True)
class Item:
    """One cell of the sheet."""

    label: str
    #: What to draw, bottom layer first: a material and its crack overlay, or a
    #: surface rather than a body.
    art: tuple[str, ...]
    #: The `Brush::sprite` name this cell answers for, when the art differs from
    #: it (a cracked brush's own sprite is the transparent crack, not the tile).
    brush: str | None = None
    #: False for the runner, who is drawn but is not something a level places.
    placeable: bool = True

    @property
    def sprite(self) -> str:
        return self.brush or self.art[0]


# Palette order, in rows: the materials with their brittle twins beside them
# (`BrushGroup::Ground`), then the hazards and the ways through a level
# (`Hazard` and `World`), then the cast (`Enemy`) and the one piece of scenery.
# That is all twenty-three placeable brushes plus the runner, who is not a brush
# but is the one body every level is about — a 6x4 grid exactly.
ITEMS: tuple[Item, ...] = (
    Item("GRASS", ("ground_1",)),
    Item("CRACKED", ("ground_1", "cracked_ground"), brush="cracked_ground"),
    Item("SAND", ("sand_1",)),
    Item("CRACKED SAND", ("sand_1", "cracked_sand"), brush="cracked_sand"),
    Item("BRICK", ("brick_1",)),
    Item("CRACKED BRICK", ("brick_1", "cracked_brick"), brush="cracked_brick"),
    Item("TIMBER", ("wood_1",)),
    Item("CRACKED TIMBER", ("wood_1", "cracked_wood"), brush="cracked_wood"),
    # The ledge and the mover are single cells: their middle variants *are* the
    # whole shelf, drawn with a leg at each end, rather than a running segment.
    Item("ONE-WAY", ("oneway_m",)),
    Item("MOVER", ("mover_m",)),
    Item("SPIKES", ("spike",)),
    Item("SPRING", ("spring",)),
    # The animated surface is what a pool looks like as a whole; the plain body
    # cell is the same water without its foam, so the sheet shows the surface.
    Item("WATER", ("water_top_a",), brush="water"),
    Item("LAVA", ("lava_top_a",), brush="lava"),
    Item("VINE", ("vine",)),
    Item("ROPE", ("rope",)),
    Item("SPAWN", ("spawn_marker",)),
    Item("FLAG", ("goal_flag_2",), brush="goal_top"),
    Item("RUNNER", ("player_idle_a",), placeable=False),
    Item("WALKER", ("walker_a",)),
    Item("CHASER", ("chaser_a",)),
    Item("FLYER", ("flyer_a",)),
    Item("TURRET", ("turret_r",)),
    Item("SCENERY", ("bush_c",)),
)

COLUMNS = 6


def brushes() -> dict[str, str]:
    """`Brush::Name -> sprite`, read from the editor's own match arms.

    Parsing the Rust is deliberate: a hand-kept copy of the palette in Python is
    the drift this script exists to catch. The slice ends at `label`, the next
    method, so the labels' words are never mistaken for sprite names.
    """
    source = EDITOR.read_text(encoding="utf-8")
    body = source.split("pub fn sprite(self)", 1)[1].split("pub fn label(self)", 1)[0]
    return dict(re.findall(r'Brush::(\w+)\s*=>\s*"([a-z0-9_]+)"', body))


def coverage() -> list[str]:
    """What is wrong between the sheet and the palette, as sentences.

    Two directions, because a sheet can drift either way: a brush the editor has
    and the sheet does not show, and a cell claiming to be a brush the editor no
    longer draws. `Empty` and `Erase` are tools rather than parts — they act on a
    cell instead of putting an object in it — so a sheet of the level's parts has
    no cell for them.
    """
    palette = brushes()
    shown = {item.sprite for item in ITEMS if item.placeable}
    tools = {"Empty", "Erase"}
    missing = sorted(
        f"no cell for {name} ({sprite})"
        for name, sprite in palette.items()
        if name not in tools and sprite not in shown
    )
    unknown = sorted(
        f"{item.label} answers for {item.sprite}, which no brush draws"
        for item in ITEMS
        if item.placeable and item.sprite not in palette.values()
    )
    return missing + unknown


def fonts() -> tuple[ImageFont.FreeTypeFont, ImageFont.FreeTypeFont]:
    """The bold monospace the sheet is set in, wherever this machine keeps it."""
    candidates = (
        "/usr/share/fonts/TTF/DejaVuSansMono-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
        "/usr/share/fonts/liberation/LiberationMono-Bold.ttf",
        "/usr/share/fonts/noto/NotoSansMono-Bold.ttf",
    )
    for path in candidates:
        if Path(path).exists():
            return (
                ImageFont.truetype(path, TITLE_SIZE),
                ImageFont.truetype(path, LABEL_SIZE),
            )
    try:
        return (
            ImageFont.truetype("DejaVuSansMono-Bold.ttf", TITLE_SIZE),
            ImageFont.truetype("DejaVuSansMono-Bold.ttf", LABEL_SIZE),
        )
    except OSError as err:  # pragma: no cover - a machine with no mono font
        raise SystemExit(f"no bold monospace font found: {err}")


def tracked(
    canvas: Image.Image,
    centre: float,
    top: int,
    text: str,
    font: ImageFont.FreeTypeFont,
    tracking: float,
) -> None:
    """Draw `text` centred on `centre`, its ink starting at `top`, letter-spaced.

    Drawn character by character because Pillow has no tracking, and onto its own
    layer so the ink's own bounding box can be centred: a font's advance widths
    include side bearings, and centring those leaves the word visibly off-centre.
    """
    widths = [font.getlength(ch) for ch in text]
    total = sum(widths) + tracking * (len(text) - 1)
    ascent, descent = font.getmetrics()
    layer = Image.new("RGBA", (int(total) + 8, ascent + descent + 8), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    x = 4.0
    for ch, width in zip(text, widths):
        draw.text((x, 4), ch, font=font, fill=INK + (255,))
        x += width + tracking

    box = layer.getbbox()
    if box is None:
        return
    ink = layer.crop(box)
    canvas.alpha_composite(ink, (round(centre - ink.width / 2), top))


def load_sprites() -> dict[str, Image.Image]:
    """Every named cell of `assets/tileset.png`, keyed by name."""
    manifest = json.loads((ASSETS / "tileset.json").read_text(encoding="utf-8"))
    cell = int(manifest["cell"])
    sheet = Image.open(ASSETS / manifest.get("image", "tileset.png")).convert("RGBA")
    sprites: dict[str, Image.Image] = {}
    for entry in manifest["sprites"]:
        col, row = int(entry["col"]), int(entry["row"])
        sprites[entry["name"]] = sheet.crop(
            (col * cell, row * cell, (col + 1) * cell, (row + 1) * cell)
        )
    return sprites


def draw() -> tuple[Image.Image, list[str]]:
    """The finished sheet, and anything that stops it from being drawn."""
    broken = coverage()
    sprites = load_sprites()
    absent = sorted(
        f"the atlas has no sprite {name}"
        for name in {name for item in ITEMS for name in item.art}
        if name not in sprites
    )
    if broken or absent:
        return Image.new("RGBA", (1, 1)), broken + absent

    rows = (len(ITEMS) + COLUMNS - 1) // COLUMNS
    width = MARGIN * 2 + COLUMNS * 96 + (COLUMNS - 1) * (PITCH - 96)
    height = FIRST_ROW + (rows - 1) * ROW_PITCH + 96 + LABEL_GAP + LABEL_SIZE + MARGIN
    canvas = Image.new("RGBA", (width, height), PAPER + (255,))
    title_font, label_font = fonts()

    tracked(canvas, width / 2, TITLE_TOP, TITLE, title_font, TITLE_TRACKING)

    for index, item in enumerate(ITEMS):
        col, row = index % COLUMNS, index // COLUMNS
        left = MARGIN + col * PITCH
        top = FIRST_ROW + row * ROW_PITCH

        art = sprites[item.art[0]].resize((16 * SCALE, 16 * SCALE), Image.NEAREST)
        for name in item.art[1:]:
            overlay = sprites[name].resize((16 * SCALE, 16 * SCALE), Image.NEAREST)
            art = Image.alpha_composite(art, overlay)

        canvas.alpha_composite(art, (left, top))
        tracked(
            canvas,
            left + 48,
            top + 96 + LABEL_GAP,
            item.label,
            label_font,
            LABEL_TRACKING,
        )

    return canvas, []


def main(argv: list[str]) -> int:
    canvas, problems = draw()
    if problems:
        print(
            "the sheet disagrees with the palette: " + "; ".join(problems),
            file=sys.stderr,
        )
        return 1

    if "--check" in argv:
        if not OUT.exists():
            print(f"{OUT.relative_to(ROOT)} does not exist", file=sys.stderr)
            return 1
        current = Image.open(OUT).convert("RGBA")
        if current.size != canvas.size or current.tobytes() != canvas.tobytes():
            print(
                f"{OUT.relative_to(ROOT)} is out of date: run "
                "python3 tools/blocks_gallery.py",
                file=sys.stderr,
            )
            return 1
        print(f"{OUT.relative_to(ROOT)} is up to date")
        return 0

    OUT.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(OUT)
    print(f"wrote {OUT.relative_to(ROOT)} ({canvas.width}x{canvas.height})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
