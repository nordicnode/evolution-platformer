#!/usr/bin/env python3
"""Generate the earlier course-kit review concepts and boards.

The screen concepts and review boards stay beneath ``art-review/``. This is a
historical study: the shipped atlas is generated separately by
``tools/gen_tileset.py`` from ``tools/art/kit*.py``, which drops the object outline
these boards proposed. Nothing here writes into ``assets/``.
"""

from __future__ import annotations

import json
import math
import os
from pathlib import Path
from typing import Iterable

from PIL import Image, ImageDraw, ImageFont, ImageFilter


ROOT = Path(__file__).resolve().parent
SPRITE_DIR = ROOT / "sprites"
FONT_PATH = ROOT.parent / "crates" / "app" / "assets" / "press-start-2p.ttf"

# The proposed palette: warm toy-plastic surfaces, a cool ink outline, and a
# saturated course palette.  These names are shared by sprites and mockups.
INK = "#17324d"
INK_SOFT = "#31536b"
CREAM = "#fff3d3"
PAPER = "#fffaf0"
SKY = "#55bce9"
SKY_LIGHT = "#bfeef4"
HILL_FAR = "#9bd7c5"
HILL_NEAR = "#58b78c"
YELLOW = "#ffcc42"
YELLOW_LIGHT = "#ffe57d"
CORAL = "#f26b5e"
CORAL_DARK = "#b73d55"
GREEN = "#56bd64"
GREEN_LIGHT = "#a9e86b"
GREEN_DARK = "#25835b"
BLUE = "#40b7df"
BLUE_DARK = "#2279ae"
WATER = "#4fd7d1"
WATER_DARK = "#218aa9"
LAVA = "#f47743"
LAVA_DARK = "#c7464b"
PURPLE = "#9b6ae8"
PURPLE_DARK = "#63429f"
ORANGE = "#ff944d"
ORANGE_DARK = "#c45242"
SAND = "#f6da87"
SAND_DARK = "#c58d59"
BRICK = "#d8665f"
BRICK_DARK = "#994052"
WOOD = "#c88a50"
WOOD_DARK = "#784c3d"
STONE = "#9aa6bc"
STONE_DARK = "#59647e"
WHITE = "#ffffff"
MUTED = "#5d7585"


def font(size: int, pixel: bool = False) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    """Load the game's pixel face when available, with a portable fallback."""
    candidates = [
        str(FONT_PATH) if pixel else "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    for candidate in candidates:
        if os.path.exists(candidate):
            return ImageFont.truetype(candidate, size)
    return ImageFont.load_default()


def text(draw: ImageDraw.ImageDraw, xy: tuple[int, int], value: str, size: int,
         fill=INK, pixel: bool = False, anchor: str | None = None, stroke: int = 0,
         stroke_fill=INK) -> None:
    draw.text(xy, value, font=font(size, pixel), fill=fill, anchor=anchor,
              stroke_width=stroke, stroke_fill=stroke_fill)


def rounded(draw: ImageDraw.ImageDraw, box: tuple[int, int, int, int], fill, radius: int = 12,
            outline=INK, width: int = 3, shadow: bool = False) -> None:
    """Draw a chunky outlined plate with an optional hard toy-like shadow."""
    x0, y0, x1, y1 = box
    if shadow:
        draw.rounded_rectangle((x0, y0 + 7, x1, y1 + 7), radius, fill="#10253b", outline="#10253b")
    draw.rounded_rectangle(box, radius, fill=fill, outline=outline, width=width)


def ribbon(draw: ImageDraw.ImageDraw, box: tuple[int, int, int, int], label: str,
           fill=BLUE, size: int = 15) -> None:
    x0, y0, x1, y1 = box
    draw.rounded_rectangle(box, 9, fill=fill, outline=INK, width=3)
    draw.polygon([(x0 + 11, y1), (x0 + 21, y1), (x0 + 15, y1 + 8), (x0 + 5, y1 + 8)], fill=INK)
    text(draw, ((x0 + x1) // 2, (y0 + y1) // 2 + 1), label.upper(), size, WHITE, pixel=True, anchor="mm")


def badge(draw: ImageDraw.ImageDraw, xy: tuple[int, int], label: str, fill=YELLOW,
          width: int | None = None, size: int = 12) -> None:
    x, y = xy
    w = width or max(92, len(label) * size // 2 + 28)
    rounded(draw, (x, y, x + w, y + 30), fill, 14, INK, 2, shadow=False)
    text(draw, (x + w // 2, y + 15), label.upper(), size, INK, pixel=True, anchor="mm")


def panel(draw: ImageDraw.ImageDraw, box: tuple[int, int, int, int], fill=PAPER,
          header: str | None = None, header_fill=BLUE) -> None:
    rounded(draw, box, fill, 17, INK, 4, shadow=True)
    if header:
        x0, y0, x1, _ = box
        ribbon(draw, (x0 + 10, y0 + 10, x1 - 10, y0 + 44), header, header_fill, 13)


def new(size: tuple[int, int], fill=CREAM) -> Image.Image:
    return Image.new("RGB", size, fill)


def layer(size: tuple[int, int]) -> Image.Image:
    return Image.new("RGBA", size, (0, 0, 0, 0))


def paste_center(dst: Image.Image, src: Image.Image, xy: tuple[int, int], scale: int = 1) -> None:
    if scale != 1:
        src = src.resize((src.width * scale, src.height * scale), Image.Resampling.NEAREST)
    dst.paste(src, (xy[0] - src.width // 2, xy[1] - src.height // 2), src)


def pixel_outline(img: Image.Image, color=INK, radius: int = 1) -> Image.Image:
    """Add a hard nearest-neighbour outline to an RGBA sprite."""
    alpha = img.getchannel("A")
    size = radius * 2 + 1
    expanded = alpha.filter(ImageFilter.MaxFilter(size))
    outline = Image.new("RGBA", img.size, color)
    outline.putalpha(expanded)
    outline.alpha_composite(img)
    return outline


def sprite_base() -> Image.Image:
    return Image.new("RGBA", (16, 16), (0, 0, 0, 0))


def rect(draw: ImageDraw.ImageDraw, box, fill, outline=None, width=1) -> None:
    draw.rectangle(box, fill=fill, outline=outline, width=width)


def tile_ground(fill=GREEN, body="#9b6a4c", body_dark="#6d4a45") -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    # This is a surface tile, not a sticker: every edge is allowed to meet its
    # neighbour.  The live renderer would pair it with body/deep masks for the
    # rows below; the review atlas keeps the surface texture itself seam-free.
    rect(d, (0, 0, 15, 2), GREEN_LIGHT)
    rect(d, (0, 3, 15, 5), fill)
    rect(d, (0, 6, 15, 15), body)
    for x, y, c in ((3, 8, body_dark), (11, 7, body_dark), (6, 12, "#c18359"),
                    (13, 13, body_dark), (2, 14, "#c18359"), (9, 10, body_dark)):
        rect(d, (x, y, x + 1, y + 1), c)
    rect(d, (0, 4, 15, 4), GREEN_DARK)
    return img


def tile_sand() -> Image.Image:
    return tile_ground(SAND, "#e5b968", SAND_DARK)


def tile_brick() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (0, 0, 15, 3), CORAL)
    rect(d, (0, 4, 15, 15), BRICK_DARK)
    for y in (6, 11):
        rect(d, (0, y, 15, y), BRICK)
    for x in (4, 12):
        rect(d, (x, 4, x, 6), BRICK)
        rect(d, (x - 4, 7, x - 4, 11), BRICK)
        rect(d, (x + 3, 12, x + 3, 15), BRICK)
    rect(d, (1, 1, 3, 1), YELLOW_LIGHT)
    return img


def tile_wood() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (0, 0, 15, 3), WOOD)
    rect(d, (0, 4, 15, 15), WOOD_DARK)
    for y in (6, 10, 14):
        rect(d, (0, y, 15, y), WOOD)
    rect(d, (3, 4, 4, 15), "#e2a65d")
    rect(d, (11, 4, 12, 15), "#9e633f")
    for x, y in ((2, 3), (13, 9), (7, 13)):
        rect(d, (x, y, x + 1, y + 1), YELLOW)
    return img


def spike_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (1, 12, 14, 15), STONE_DARK)
    rect(d, (2, 11, 13, 12), STONE)
    for x in (3, 7, 11):
        d.polygon([(x, 11), (x + 2, 2), (x + 4, 11)], fill=WHITE)
        d.polygon([(x + 1, 10), (x + 2, 4), (x + 3, 10)], fill=STONE)
    return pixel_outline(img)


def spring_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (1, 13, 14, 15), STONE_DARK)
    rect(d, (2, 12, 13, 13), WHITE)
    points = [(4, 11), (12, 11), (5, 9), (11, 9), (6, 7), (10, 7), (7, 5), (9, 5)]
    d.line(points, fill=YELLOW, width=2)
    rect(d, (4, 4, 11, 4), YELLOW_LIGHT)
    return img


def water_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    # The crest starts and ends at the same height, so a row of cells joins
    # without a step at the atlas boundary.
    d.polygon([(0, 3), (4, 1), (8, 3), (12, 1), (15, 3), (15, 15), (0, 15)], fill=WATER)
    d.line([(0, 3), (4, 1), (8, 3), (12, 1), (15, 3)], fill=WHITE, width=2)
    rect(d, (0, 6, 15, 15), BLUE_DARK)
    for x, y in ((3, 8), (11, 11), (7, 13), (14, 7)):
        rect(d, (x, y, x + 1, y + 1), WATER)
    return img


def lava_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    d.polygon([(0, 4), (4, 2), (8, 4), (12, 2), (15, 4), (15, 15), (0, 15)], fill=LAVA)
    d.line([(0, 4), (4, 2), (8, 4), (12, 2), (15, 4)], fill=YELLOW_LIGHT, width=2)
    for x, y in ((2, 9), (5, 13), (10, 8), (13, 12)):
        rect(d, (x, y, x + 1, y + 1), LAVA_DARK)
    rect(d, (7, 10, 8, 11), YELLOW)
    return img


def vine_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    # A continuous centre stem means stacked vine cells read as one climbable
    # plant instead of a column of disconnected icons.
    d.line([(8, 0), (8, 15)], fill=GREEN_DARK, width=3)
    for x, y, side in ((8, 3, -1), (8, 6, 1), (8, 9, -1), (8, 12, 1)):
        d.line([(x, y), (x + side * 3, y - 2)], fill=GREEN, width=2)
        rect(d, (x + side * 3 - 1, y - 3, x + side * 3 + 1, y - 1), GREEN_LIGHT)
    return img


def rope_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    d.line([(8, 0), (8, 9), (6, 11), (7, 14), (10, 14), (11, 12)], fill=YELLOW, width=2)
    d.line([(7, 0), (7, 9)], fill=WOOD_DARK, width=1)
    rect(d, (5, 14, 11, 15), WOOD_DARK)
    return pixel_outline(img)


def ball_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    cx, cy = 8, 9
    for angle in range(0, 360, 45):
        dx, dy = round(math.cos(math.radians(angle)) * 6), round(math.sin(math.radians(angle)) * 6)
        d.line([(cx + dx // 2, cy + dy // 2), (cx + dx, cy + dy)], fill=INK, width=2)
    d.ellipse((3, 4, 13, 14), fill=STONE, outline=INK, width=2)
    rect(d, (5, 6, 8, 8), WHITE)
    return pixel_outline(img)


def belt_sprite(left: bool = False) -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (0, 2, 15, 5), WOOD)
    rect(d, (0, 6, 15, 14), STONE_DARK)
    for x in (2, 7, 12):
        d.polygon([(x, 8), (x + 3, 10), (x, 12)], fill=YELLOW if not left else BLUE)
    if left:
        img = img.transpose(Image.Transpose.FLIP_LEFT_RIGHT)
    return img


def boulder_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    d.polygon([(3, 13), (2, 9), (5, 4), (10, 2), (14, 6), (14, 12), (11, 15), (5, 15)], fill=STONE)
    rect(d, (5, 5, 8, 7), WHITE)
    rect(d, (3, 12, 12, 14), STONE_DARK)
    return pixel_outline(img)


def crate_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (1, 1, 14, 14), WOOD, INK, 2)
    d.line([(2, 2), (13, 13)], fill=YELLOW, width=2)
    d.line([(13, 2), (2, 13)], fill=WOOD_DARK, width=2)
    rect(d, (4, 4, 5, 5), YELLOW_LIGHT)
    rect(d, (10, 10, 11, 11), WOOD_DARK)
    return pixel_outline(img)


def player_sprite(pose: str = "run", variant: int = 0) -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    # Expressive explorer: a connected, outlined silhouette with a large face,
    # bright cap, scarf, and two clear feet. Interior shapes are filled rather
    # than separately outlined so the dark rim remains an actual rim.
    offset = {"idle": 0, "run": 0, "jump": -2, "fall": 1, "hurt": 1}.get(pose, 0)
    leg_a, leg_b = ((4 + variant % 2, 14), (9 - variant % 2, 12)) if pose == "run" else ((5, 14), (8, 14))
    if pose == "jump":
        leg_a, leg_b = (4, 11), (10, 10)
    if pose == "fall":
        leg_a, leg_b = (4, 14), (10, 13)
    rect(d, (leg_a[0], leg_a[1] + offset, leg_a[0] + 2, leg_a[1] + 1 + offset), ORANGE_DARK)
    rect(d, (leg_b[0], leg_b[1] + offset, leg_b[0] + 2, leg_b[1] + 1 + offset), ORANGE)
    rect(d, (4, 7 + offset, 11, 12 + offset), ORANGE)
    rect(d, (4, 7 + offset, 11, 8 + offset), YELLOW)
    rect(d, (3, 8 + offset, 4, 10 + offset), CREAM)
    rect(d, (5, 2 + offset, 11, 6 + offset), CREAM)
    rect(d, (4, 1 + offset, 12, 3 + offset), CORAL)
    rect(d, (3, 2 + offset, 9, 3 + offset), CORAL)
    rect(d, (8, 4 + offset, 10, 5 + offset), INK if pose != "hurt" else CORAL_DARK)
    rect(d, (10, 9 + offset, 12, 11 + offset), ORANGE_DARK)
    rect(d, (2, 7 + offset + variant % 2, 3, 8 + offset + variant % 2), CORAL)
    rect(d, (5 + variant % 6, 9 + offset, 5 + variant % 6, 9 + offset),
         CORAL if variant % 2 else YELLOW)
    if pose == "hurt":
        d.line([(8, 4 + offset), (10, 5 + offset)], fill=INK, width=1)
        d.line([(10, 4 + offset), (8, 5 + offset)], fill=INK, width=1)
    return pixel_outline(img)


def tuft_sprite(variant: int = 0) -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    for x, lean in ((3, -2), (7 + variant % 2, 1), (11, -1)):
        d.line([(x, 15), (x + lean, 8 + (variant % 2))], fill=GREEN_DARK, width=1)
        rect(d, (x + lean, 8 + (variant % 2), x + lean + 1, 9 + (variant % 2)), GREEN_LIGHT)
    return img


def goal_top_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (4, 0, 5, 15), WHITE)
    rect(d, (3, 0, 6, 2), YELLOW)
    return img


def goal_bottom_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (4, 0, 5, 14), WOOD_DARK)
    rect(d, (2, 14, 7, 15), WOOD)
    return img


def goal_flag_sprite(variant: int = 0) -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    wave = variant
    points = [(6, 2), (13, 3 + wave), (15, 5 + wave), (13, 7 + wave), (6, 6)]
    d.polygon(points, fill=CORAL)
    rect(d, (6, 2, 6, 7), YELLOW_LIGHT)
    return img


def pine_near_sprite(variant: int = 0) -> Image.Image:
    img = tree_sprite()
    d = ImageDraw.Draw(img)
    # The near tree's visible trunk is deliberately muted against the hills.
    rect(d, (7, 12, 8, 15), (108, 88, 72, 255))
    if variant:
        rect(d, (4, 4, 5, 5), GREEN_LIGHT)
    return img


def walker_sprite(variant: int = 0) -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (3, 6, 12, 13), PURPLE, INK, 1)
    d.polygon([(4, 6), (6, 3), (8, 6), (10, 3), (12, 6)], fill=PURPLE)
    rect(d, (5, 8, 10, 10), YELLOW)
    rect(d, (7, 8, 8, 9), INK)
    rect(d, (4, 13, 6, 15), PURPLE_DARK)
    rect(d, (10, 13, 12, 15), PURPLE_DARK)
    rect(d, (6 + variant % 4, 11, 7 + variant % 4, 11), PURPLE_DARK)
    return pixel_outline(img)


def chaser_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    d.polygon([(2, 8), (5, 5), (11, 6), (14, 9), (12, 13), (4, 13)], fill=CORAL, outline=INK)
    d.polygon([(12, 7), (15, 8), (12, 10)], fill=YELLOW)
    rect(d, (8, 8, 11, 10), YELLOW)
    rect(d, (9, 8, 10, 9), INK)
    rect(d, (4, 13, 6, 15), CORAL_DARK)
    rect(d, (10, 13, 12, 15), CORAL_DARK)
    return pixel_outline(img)


def flyer_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    d.polygon([(7, 5), (2, 2), (4, 8), (7, 9)], fill=PURPLE)
    d.polygon([(9, 5), (14, 2), (12, 8), (9, 9)], fill=CORAL)
    rect(d, (6, 5, 10, 11), BLUE_DARK, INK, 1)
    rect(d, (7, 7, 8, 8), YELLOW)
    return pixel_outline(img)


def turret_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (2, 11, 13, 14), STONE_DARK)
    d.ellipse((4, 5, 11, 12), fill=STONE, outline=INK, width=1)
    rect(d, (9, 7, 15, 9), INK)
    rect(d, (13, 7, 15, 8), YELLOW)
    rect(d, (5, 12, 10, 13), WHITE)
    return pixel_outline(img)


def goal_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (7, 1, 8, 15), WHITE)
    d.polygon([(8, 2), (14, 4), (8, 7)], fill=CORAL)
    rect(d, (6, 14, 10, 15), WOOD_DARK)
    rect(d, (7, 13, 9, 14), WOOD)
    return pixel_outline(img)


def spawn_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    d.polygon([(2, 13), (8, 4), (14, 13)], fill=YELLOW, outline=INK)
    rect(d, (4, 12, 12, 14), YELLOW_LIGHT)
    rect(d, (7, 7, 9, 10), WHITE)
    return pixel_outline(img)


def bush_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    d.ellipse((1, 7, 8, 15), fill=GREEN_DARK, outline=INK)
    d.ellipse((6, 4, 14, 15), fill=GREEN, outline=INK)
    rect(d, (4, 10, 6, 12), GREEN_LIGHT)
    rect(d, (10, 7, 12, 9), GREEN_LIGHT)
    rect(d, (5, 14, 11, 15), GREEN_DARK)
    return pixel_outline(img)


def tree_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (7, 9, 9, 15), WOOD_DARK)
    d.ellipse((2, 2, 13, 12), fill=GREEN_DARK, outline=INK)
    d.ellipse((4, 1, 11, 8), fill=GREEN, outline=INK)
    rect(d, (5, 3, 7, 5), GREEN_LIGHT)
    rect(d, (9, 5, 11, 7), GREEN_LIGHT)
    return pixel_outline(img)


def mushroom_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (6, 9, 9, 15), CREAM, INK, 1)
    d.ellipse((2, 4, 13, 11), fill=CORAL, outline=INK, width=1)
    rect(d, (4, 6, 6, 7), YELLOW_LIGHT)
    rect(d, (9, 5, 11, 6), YELLOW_LIGHT)
    return pixel_outline(img)


def cloud_sprite(variant: int = 0) -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    d.ellipse((0, 6 + variant % 2, 8, 11 + variant % 2), fill=WHITE)
    d.ellipse((4, 2 + variant % 2, 11, 11 + variant % 2), fill=WHITE)
    d.ellipse((8, 5 + variant % 2, 15, 11 + variant % 2), fill=WHITE)
    rect(d, (2, 10 + variant % 2, 13, 11 + variant % 2), WHITE)
    return img


def effect_sprite(kind: str) -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    if kind == "dust":
        for box in ((2, 11, 5, 14), (6, 8, 9, 12), (10, 11, 14, 15)):
            d.ellipse(box, fill=CREAM, outline=INK)
    elif kind == "puff":
        for box in ((2, 6, 7, 12), (6, 2, 12, 11), (10, 7, 15, 13)):
            d.ellipse(box, fill=CORAL, outline=INK)
    else:
        d.polygon([(8, 0), (10, 6), (15, 8), (10, 10), (8, 16), (6, 10), (1, 8), (6, 6)], fill=YELLOW_LIGHT, outline=INK)
    return pixel_outline(img)


def ui_icon(kind: str) -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    if kind == "undo":
        d.arc((2, 3, 14, 14), 65, 290, fill=INK, width=2)
        d.polygon([(2, 4), (6, 3), (4, 7)], fill=INK)
    elif kind == "redo":
        img = ui_icon("undo").transpose(Image.Transpose.FLIP_LEFT_RIGHT)
    elif kind == "fit":
        for x, y, sx, sy in ((2, 2, 1, 1), (13, 2, -1, 1), (2, 13, 1, -1), (13, 13, -1, -1)):
            d.line([(x, y), (x + sx * 4, y)], fill=INK, width=2)
            d.line([(x, y), (x, y + sy * 4)], fill=INK, width=2)
    elif kind == "search":
        d.ellipse((2, 2, 10, 10), outline=INK, width=2)
        d.line([(9, 9), (14, 14)], fill=INK, width=2)
    elif kind == "restart":
        d.arc((2, 2, 14, 14), 40, 320, fill=INK, width=2)
        d.polygon([(11, 2), (15, 3), (13, 6)], fill=INK)
    elif kind == "pause":
        rect(d, (4, 3, 6, 13), INK)
        rect(d, (10, 3, 12, 13), INK)
    elif kind == "play":
        d.polygon([(4, 2), (13, 8), (4, 14)], fill=INK)
    elif kind == "step":
        d.polygon([(2, 3), (9, 8), (2, 13)], fill=INK)
        rect(d, (11, 3, 13, 13), INK)
    elif kind == "star":
        pts = []
        for i in range(10):
            a = -math.pi / 2 + i * math.pi / 5
            r = 7 if i % 2 == 0 else 3
            pts.append((8 + math.cos(a) * r, 8 + math.sin(a) * r))
        d.polygon(pts, fill=YELLOW, outline=INK)
    return pixel_outline(img)


def deep_tile(fill=STONE, dark=STONE_DARK, light=WHITE) -> Image.Image:
    """A full opaque body tile for the non-surface autotile families."""
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (0, 0, 15, 15), fill)
    for y in (4, 9, 14):
        rect(d, (0, y, 15, y), dark)
    for x, y in ((3, 2), (11, 6), (6, 11), (13, 13)):
        rect(d, (x, y, x + 1, y + 1), light)
    return img


def material_tile(kind: str, mask: int) -> Image.Image:
    """Build one opaque, edge-aware autotile candidate.

    The live renderer asks for a mask rather than a single generic material
    picture.  Keeping the cell opaque while changing the crown, side shade,
    and underside makes adjacent cells meet cleanly without the sticker seams
    the first review pass exposed.
    """
    palettes = {
        "grass": (GREEN_LIGHT, GREEN, "#9b6a4c", "#6d4a45", GREEN_DARK),
        "sand": (SAND, "#e5b968", "#d5a463", SAND_DARK, "#a97852"),
        "brick": (CORAL, BRICK, BRICK_DARK, "#713847", BRICK_DARK),
        "wood": (WOOD, "#a96c47", WOOD_DARK, "#5b3b39", WOOD_DARK),
        "stone": (WHITE, STONE, STONE_DARK, "#414c68", STONE_DARK),
    }
    crown_light, crown, body, body_dark, edge = palettes[kind]
    img = sprite_base()
    d = ImageDraw.Draw(img)
    rect(d, (0, 0, 15, 15), body)
    if mask & 0b0001:
        crown_rows = 5
        rect(d, (0, 0, 15, 1), crown_light)
        rect(d, (0, 2, 15, crown_rows - 1), crown)
        rect(d, (0, crown_rows, 15, crown_rows), edge)
    else:
        for y in (3, 8, 13):
            rect(d, (0, y, 15, y), body_dark)
    if mask & 0b0010:
        rect(d, (0, 0, 0, 15), edge)
    if mask & 0b0100:
        rect(d, (15, 0, 15, 15), edge)
    if mask & 0b1000:
        rect(d, (0, 14, 15, 15), body_dark)
    for ox, oy in ((3, 8), (11, 7), (6, 12), (13, 13)):
        dot = crown_light if (mask & 0b0001 and (ox + oy + mask) % 3 == 0) else body_dark
        rect(d, (ox, oy, ox + 1, oy + 1), dot)
    return img


def surface_variant(base: Image.Image, kind: str, variant: int) -> Image.Image:
    """Add a small crown-only decoration without changing the tile body."""
    img = base.copy()
    d = ImageDraw.Draw(img)
    accents = {
        "grass": (GREEN_LIGHT, GREEN_DARK),
        "sand": (YELLOW_LIGHT, SAND_DARK),
        "brick": (YELLOW_LIGHT, BRICK_DARK),
        "wood": (YELLOW, WOOD_DARK),
    }
    light, dark = accents[kind]
    x = 3 + (variant * 5) % 9
    rect(d, (x, 1, x + 1, 2), light)
    rect(d, (x + 2, 2, x + 2, 3), dark)
    return img


def crack_overlay(kind: str, variant: int) -> Image.Image:
    """A transparent brittle-tile mark drawn over the material beneath it."""
    img = sprite_base()
    d = ImageDraw.Draw(img)
    colours = {
        "ground": GREEN_DARK,
        "sand": SAND_DARK,
        "brick": BRICK_DARK,
        "wood": WOOD_DARK,
    }
    colour = colours[kind]
    paths = (
        [(3, 3), (6, 6), (5, 9), (9, 12), (8, 14)],
        [(12, 2), (9, 5), (10, 8), (6, 10), (4, 13)],
        [(2, 8), (5, 6), (8, 8), (11, 6), (13, 8)],
    )
    path = paths[variant % len(paths)]
    d.line(path, fill=colour, width=1)
    for x, y in path[::2]:
        rect(d, (x, y, x, y), INK_SOFT)
    return img


def variant_mark(base: Image.Image, variant: int, colour: str) -> Image.Image:
    """Make a distinct animation frame without changing its anchor."""
    img = base.copy()
    d = ImageDraw.Draw(img)
    x = 2 + (variant * 3) % 11
    y = 4 + (variant * 2) % 8
    rect(d, (x, y, x, y), colour)
    return img


def platform_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    # The simulator treats the top three rows as the one-way deck. Keep those
    # rows solid and leave the space below open so the picture and collision
    # surface agree.
    rect(d, (0, 0, 15, 2), WOOD, INK)
    rect(d, (1, 1, 14, 1), YELLOW_LIGHT)
    rect(d, (3, 3, 4, 15), STONE_DARK)
    rect(d, (11, 3, 12, 15), STONE_DARK)
    return img


def shot_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    d.line([(1, 9), (10, 9)], fill=CORAL_DARK, width=2)
    d.polygon([(15, 8), (10, 5), (10, 11)], fill=YELLOW_LIGHT)
    rect(d, (11, 7, 13, 9), WHITE)
    return pixel_outline(img)


def butterfly_sprite() -> Image.Image:
    img = sprite_base()
    d = ImageDraw.Draw(img)
    d.ellipse((1, 5, 7, 11), fill=YELLOW_LIGHT, outline=INK)
    d.ellipse((9, 5, 15, 11), fill=PURPLE, outline=INK)
    rect(d, (7, 6, 9, 11), INK)
    return img


def seam_sprite(name: str) -> Image.Image:
    """Make a sparse material band from the pixels of its own plain tile."""
    rest = name.removeprefix("blend_")
    material, side = rest.rsplit("_", 1)
    aliases = {
        "grass": ("grass", 1), "dirt": ("grass", 0),
        "stone": ("stone", 0), "beach": ("sand", 1),
        "packed": ("sand", 0), "sandstone": ("sand", 0),
        "brick": ("brick", 1), "masonry": ("stone", 0),
        "wood": ("wood", 1), "timber": ("wood", 0),
    }
    kind, mask = aliases[material]
    allowed = list(material_tile(kind, mask).getdata())
    img = sprite_base()
    px = img.load()
    max_depth = {"grass": 3, "dirt": 3, "stone": 2, "beach": 5,
                 "packed": 5, "sandstone": 3, "brick": 2,
                 "masonry": 2, "wood": 3, "timber": 3}[material]
    seed = sum(ord(c) for c in name)
    for along in range(16):
        # Seven to fifteen edge pixels: enough to cross a seam, never a solid
        # stripe across the whole side.
        if (along * 7 + seed) % 11 >= 8:
            continue
        for depth in range(max_depth + 1):
            if depth > 0 and (along * 11 + depth * 7 + seed) % 5 >= 2:
                continue
            if side == "l":
                x, y = depth, along
            elif side == "r":
                x, y = 15 - depth, along
            elif side == "t":
                x, y = along, depth
            else:
                x, y = along, 15 - depth
            px[x, y] = allowed[(x * 5 + y * 3 + seed) % len(allowed)]
    return img


def crack_variant(base: Image.Image, variant: int) -> Image.Image:
    img = base.copy()
    d = ImageDraw.Draw(img)
    paths = (((3, 3), (7, 7), (5, 11), (10, 14)),
             ((12, 2), (8, 6), (10, 9), (4, 13)),
             ((2, 8), (6, 6), (9, 9), (13, 7)))
    d.line(paths[variant % len(paths)], fill=INK, width=1)
    d.line([(x + 1, y) for x, y in paths[variant % len(paths)]], fill=INK_SOFT, width=1)
    return img


def blend_sprite(name: str) -> Image.Image:
    """A transparent material spill for the full candidate manifest."""
    img = sprite_base()
    d = ImageDraw.Draw(img)
    if "grass" in name:
        color = GREEN
    elif "sand" in name or "beach" in name:
        color = SAND
    elif "brick" in name or "masonry" in name:
        color = BRICK
    elif "wood" in name or "timber" in name:
        color = WOOD
    else:
        color = STONE
    if name.endswith("_l"):
        d.polygon([(0, 2), (5, 0), (3, 15), (0, 15)], fill=color)
    elif name.endswith("_r"):
        d.polygon([(15, 2), (10, 0), (12, 15), (15, 15)], fill=color)
    elif name.endswith("_t"):
        d.polygon([(0, 0), (15, 0), (15, 5), (7, 3), (0, 5)], fill=color)
    else:
        d.polygon([(0, 12), (15, 12), (15, 15), (0, 15)], fill=color)
    return img


def full_candidate_sprite(name: str, sprites: dict[str, Image.Image], deep: dict[str, Image.Image]) -> Image.Image:
    """Map every live manifest name to a review candidate sprite."""
    if name == "empty":
        return sprite_base()
    if name.startswith("blend_"):
        return seam_sprite(name)
    if name in {"ground_1_b", "ground_1_c", "deep_0_b", "deep_0_c", "sand_1_b", "sand_1_c",
                "brick_1_b", "brick_1_c", "wood_1_b", "wood_1_c"}:
        variant = 0 if name.endswith("_b") else 1
        if name.startswith("deep"):
            base = material_tile("stone", 0)
            return crack_variant(base, variant)
        material = "grass" if name.startswith("ground") else name.split("_", 1)[0]
        return surface_variant(material_tile(material, 1), material, variant)
    if name.startswith("ground_") and name.rsplit("_", 1)[1].isdigit():
        return material_tile("grass", int(name.rsplit("_", 1)[1]))
    if name.startswith("deep_") and name.rsplit("_", 1)[1].isdigit():
        return material_tile("stone", int(name.rsplit("_", 1)[1]))
    if name.startswith("sandstone_") and name.rsplit("_", 1)[1].isdigit():
        return material_tile("sand", int(name.rsplit("_", 1)[1]))
    if name.startswith("sand_") and name.rsplit("_", 1)[1].isdigit():
        return material_tile("sand", int(name.rsplit("_", 1)[1]))
    if name.startswith("masonry_") and name.rsplit("_", 1)[1].isdigit():
        return material_tile("stone", int(name.rsplit("_", 1)[1]))
    if name.startswith("brick_") and name.rsplit("_", 1)[1].isdigit():
        return material_tile("brick", int(name.rsplit("_", 1)[1]))
    if name.startswith("timber_") and name.rsplit("_", 1)[1].isdigit():
        return material_tile("wood", int(name.rsplit("_", 1)[1]))
    if name.startswith("wood_") and name.rsplit("_", 1)[1].isdigit():
        return material_tile("wood", int(name.rsplit("_", 1)[1]))
    if name.startswith("cracked_"):
        material = name.removeprefix("cracked_").split("_")[0]
        variant = 0 if name.endswith("_b") else 1 if name.endswith("_c") else 2
        return crack_overlay(material, variant)
    if name.startswith("water_b"):
        return deep["water"]
    if name in {"water_top_a", "water_top_b"}:
        return sprites["water"]
    if name in {"water", "water_bed", "water_wall_l", "water_wall_r"}:
        return deep["water"]
    if name.startswith("lava_b"):
        return deep["lava"]
    if name in {"lava_top_a", "lava_top_b"}:
        return sprites["lava"]
    if name in {"lava", "lava_bed", "lava_wall_l", "lava_wall_r"}:
        return deep["lava"]
    if name in {"spike", "spring", "vine", "rope", "spike_ball", "boulder", "block"}:
        return {"spike": sprites["spike"], "spring": sprites["spring"], "vine": sprites["vine"],
                "rope": sprites["rope"], "spike_ball": sprites["spiked_ball"],
                "boulder": sprites["boulder"], "block": sprites["crate"]}[name]
    if name in {"belt_r", "belt_l"}:
        return sprites["belt"]
    if name in {"oneway_l", "oneway_m", "oneway_r", "mover_l", "mover_m", "mover_r"}:
        return deep["platform"]
    if name == "lava_ball":
        return sprites["spark"]
    if name == "shot":
        return deep["shot"]
    if name in {"player_idle_a", "player_idle_b"}:
        return player_sprite("idle", 0 if name.endswith("_a") else 1)
    if name.startswith("player_run_"):
        return player_sprite("run", int(name.rsplit("_", 1)[1]) - 1)
    if name == "player_jump":
        return player_sprite("jump")
    if name == "player_fall":
        return player_sprite("fall")
    if name == "player_land":
        return player_sprite("idle", 2)
    if name == "player_hurt":
        return player_sprite("hurt")
    if name.startswith("walker"):
        if name == "walker_dead":
            return sprites["walker"]
        return walker_sprite(ord(name[-1]) - ord("a"))
    if name.startswith("chaser"):
        return sprites["chaser"]
    if name.startswith("flyer"):
        return sprites["flyer"]
    if name.startswith("turret"):
        return sprites["turret"]
    if name == "goal_top":
        return goal_top_sprite()
    if name == "goal_bottom":
        return goal_bottom_sprite()
    if name.startswith("goal_flag_"):
        return goal_flag_sprite(int(name.rsplit("_", 1)[1]) - 1)
    if name == "spawn_marker":
        return sprites["spawn"]
    if name.startswith("bush"):
        return variant_mark(sprites["bush"], ord(name[-1]) - ord("a"), GREEN_LIGHT)
    if name.startswith("tree"):
        return sprites["tree"]
    if name.startswith("tuft"):
        return tuft_sprite(ord(name[-1]) - ord("a"))
    if name.startswith("reeds"):
        return variant_mark(sprites["vine"], ord(name[-1]) - ord("a"), GREEN_LIGHT)
    if name.startswith("vine_"):
        return variant_mark(sprites["vine"], ord(name[-1]) - ord("a"), GREEN_LIGHT)
    if name.startswith("flowers"):
        return sprites["mushroom"]
    if name in {"rock", "pebble"}:
        return sprites["boulder"]
    if name.startswith("mushroom"):
        return sprites["mushroom"]
    if name in {"lantern", "crystal", "signpost", "crate", "banner_a", "banner_b", "log", "stump"}:
        return sprites["crate"]
    if name.startswith("cloud"):
        return cloud_sprite(ord(name[-1]) - ord("a"))
    if name.startswith("butterfly"):
        return deep["butterfly"]
    if name.startswith("pine_near"):
        return pine_near_sprite(ord(name[-1]) - ord("a"))
    if name.startswith("pine"):
        return sprites["tree"]
    if name in {"dust_a", "dust_b"}:
        return variant_mark(sprites["dust"], ord(name[-1]) - ord("a"), CREAM)
    if name == "puff":
        return sprites["puff"]
    if name.startswith("spark"):
        return variant_mark(sprites["spark"], ord(name[-1]) - ord("a"), YELLOW_LIGHT)
    if name.startswith("ui_"):
        ui_map = {"ui_undo": "ui_undo", "ui_redo": "ui_redo", "ui_fit": "ui_fit",
                  "ui_info": "ui_search", "ui_restart": "ui_restart", "ui_pause": "ui_pause",
                  "ui_play": "ui_play", "ui_step": "ui_step", "ui_medal": "ui_star",
                  "ui_mirror": "ui_fit"}
        return sprites.get(ui_map.get(name, "ui_fit"), sprites["ui_fit"])
    return sprite_base()


def build_complete_candidate(sprites: dict[str, Image.Image]) -> tuple[Image.Image, list[dict]]:
    """Build a full candidate atlas at the current manifest's coordinates."""
    source = ROOT.parent / "assets" / "tileset.json"
    manifest = json.loads(source.read_text(encoding="utf-8"))
    deep = {
        "stone": deep_tile(STONE, STONE_DARK, WHITE),
        "sand": deep_tile("#e5b968", SAND_DARK, YELLOW_LIGHT),
        "wood": deep_tile(WOOD_DARK, "#5b3b39", WOOD),
        "water": deep_tile(BLUE_DARK, "#175c8d", WATER),
        "lava": deep_tile(LAVA, LAVA_DARK, YELLOW_LIGHT),
        "platform": platform_sprite(),
        "shot": shot_sprite(),
        "butterfly": butterfly_sprite(),
    }
    atlas = Image.new("RGBA", (manifest["cols"] * 16, manifest["rows"] * 16), (0, 0, 0, 0))
    full_manifest = []
    for entry in manifest["sprites"]:
        name = entry["name"]
        sprite = full_candidate_sprite(name, sprites, deep)
        atlas.alpha_composite(sprite, (entry["col"] * 16, entry["row"] * 16))
        full_manifest.append({"name": name, "col": entry["col"], "row": entry["row"]})
    return atlas, full_manifest


def save_complete_candidate(sprites: dict[str, Image.Image]) -> None:
    """Write a full review atlas at the current manifest's coordinates."""
    atlas, full_manifest = build_complete_candidate(sprites)
    source = ROOT.parent / "assets" / "tileset.json"
    manifest = json.loads(source.read_text(encoding="utf-8"))
    atlas.save(ROOT / "complete_candidate_tileset.png")
    atlas.resize((atlas.width * 4, atlas.height * 4), Image.Resampling.NEAREST).save(
        ROOT / "09-complete-atlas.png"
    )
    (ROOT / "complete_candidate_tileset.json").write_text(json.dumps({
        "format": "one-two-hundred-review-complete-atlas", "cell": 16,
        "cols": manifest["cols"], "rows": manifest["rows"],
        "purpose": "promoted course-kit atlas; every current live manifest name",
        "sprites": full_manifest,
    }, indent=2) + "\n", encoding="utf-8")


def build_sprites() -> dict[str, Image.Image]:
    return {
        "grass": tile_ground(), "sand": tile_sand(), "brick": tile_brick(), "wood": tile_wood(),
        "spike": spike_sprite(), "spring": spring_sprite(), "water": water_sprite(), "lava": lava_sprite(),
        "vine": vine_sprite(), "rope": rope_sprite(), "spiked_ball": ball_sprite(), "belt": belt_sprite(),
        "boulder": boulder_sprite(), "crate": crate_sprite(), "runner": player_sprite(),
        "runner_jump": player_sprite("jump"), "walker": walker_sprite(), "chaser": chaser_sprite(),
        "flyer": flyer_sprite(), "turret": turret_sprite(), "goal": goal_sprite(), "spawn": spawn_sprite(),
        "bush": bush_sprite(), "tree": tree_sprite(), "mushroom": mushroom_sprite(), "cloud": cloud_sprite(),
        "dust": effect_sprite("dust"), "puff": effect_sprite("puff"), "spark": effect_sprite("spark"),
        "ui_undo": ui_icon("undo"), "ui_redo": ui_icon("redo"), "ui_fit": ui_icon("fit"),
        "ui_search": ui_icon("search"), "ui_restart": ui_icon("restart"), "ui_pause": ui_icon("pause"),
        "ui_play": ui_icon("play"), "ui_step": ui_icon("step"), "ui_star": ui_icon("star"),
    }


def save_sprites(sprites: dict[str, Image.Image]) -> None:
    SPRITE_DIR.mkdir(parents=True, exist_ok=True)
    for name, image in sprites.items():
        image.save(SPRITE_DIR / f"{name}.png")

    names = list(sprites)
    cols = 10
    cell = 16
    rows = math.ceil(len(names) / cols)
    atlas = Image.new("RGBA", (cols * cell, rows * cell), (0, 0, 0, 0))
    manifest = []
    for index, name in enumerate(names):
        col, row = index % cols, index // cols
        atlas.alpha_composite(sprites[name], (col * cell, row * cell))
        manifest.append({"name": name, "col": col, "row": row})
    atlas.save(ROOT / "alternate_tileset.png")
    (ROOT / "alternate_tileset.json").write_text(json.dumps({
        "format": "one-two-hundred-review-atlas", "cell": 16, "cols": cols, "rows": rows,
        "purpose": "visual review only; not the live manifest", "sprites": manifest,
    }, indent=2) + "\n", encoding="utf-8")


def draw_sky(draw: ImageDraw.ImageDraw, size: tuple[int, int], dark: bool = False) -> None:
    w, h = size
    top = (42, 116, 176) if dark else (76, 181, 229)
    bottom = (35, 82, 118) if dark else (174, 230, 225)
    for y in range(h):
        t = y / max(1, h - 1)
        color = tuple(round(top[i] * (1 - t) + bottom[i] * t) for i in range(3))
        draw.line((0, y, w, y), fill=color)

    # Oversized simple parallax shapes: a toy world, not a photographic backdrop.
    draw.ellipse((w - 270, 80, w - 70, 280), fill="#fff3a7", outline="#e9bb55", width=4)
    for x in range(-160, w + 220, 190):
        draw.ellipse((x, 295, x + 310, 575), fill=HILL_FAR)
    for x in range(-130, w + 220, 250):
        draw.ellipse((x, 410, x + 370, 720), fill=HILL_NEAR)
    for x, y, scale in ((72, 132, 1), (310, 214, 1), (660, 136, 1.2), (1025, 225, .9), (1240, 124, .8)):
        draw_cloud(draw, x, y, scale)


def draw_cloud(draw: ImageDraw.ImageDraw, x: int, y: int, scale: float = 1) -> None:
    fill = WHITE
    shadow = "#d3f1f4"
    r = int(28 * scale)
    draw.ellipse((x, y + r // 2, x + r * 2, y + r * 2), fill=shadow)
    draw.ellipse((x + r, y, x + r * 2.4, y + r * 1.7), fill=fill)
    draw.ellipse((x + r * 1.8, y + r // 2, x + r * 3.3, y + r * 2), fill=fill)
    draw.rectangle((x + r // 2, y + r, x + r * 3, y + r * 2), fill=fill)


def draw_block(draw: ImageDraw.ImageDraw, x: int, y: int, size: int, kind: str = "grass",
               surface: bool = False) -> None:
    palettes = {
        "grass": (GREEN, GREEN_LIGHT, "#986a4c"), "sand": (SAND, YELLOW_LIGHT, "#d5a463"),
        "brick": (BRICK, CORAL, BRICK_DARK), "wood": (WOOD, YELLOW, WOOD_DARK),
        "stone": (STONE, WHITE, STONE_DARK),
    }
    fill, top, body = palettes.get(kind, palettes["grass"])
    # Course pieces are deliberately edge-to-edge.  The navy outline belongs
    # around the whole course card and around movable objects, never between
    # two adjacent ground cells.
    draw.rectangle((x, y, x + size - 1, y + size - 1), fill=body)
    if surface:
        crown = max(5, size // 5)
        draw.rectangle((x, y, x + size - 1, y + crown), fill=top)
        draw.rectangle((x, y + crown + 1, x + size - 1, y + crown + 4), fill=fill)
    else:
        draw.line((x, y + 4, x + size - 1, y + 4), fill=fill, width=max(1, size // 12))
    for ox, oy in ((size // 3, size // 2), (size * 3 // 4, size * 3 // 5), (size // 2, size * 4 // 5)):
        dot = max(2, size // 10)
        draw.rectangle((x + ox, y + oy, x + ox + dot, y + oy + dot), fill=body)


def draw_platform(draw: ImageDraw.ImageDraw, x: int, y: int, w: int, kind: str = "wood") -> None:
    fill = WOOD if kind == "wood" else SAND
    top = YELLOW_LIGHT if kind == "wood" else WHITE
    draw.rounded_rectangle((x, y, x + w, y + 22), 7, fill=INK, outline=INK)
    draw.rounded_rectangle((x + 3, y + 2, x + w - 3, y + 17), 4, fill=fill, outline=top, width=3)
    for ox in range(x + 12, x + w - 8, 24):
        draw.line((ox, y + 6, ox + 8, y + 13), fill=kind == "wood" and WOOD_DARK or SAND_DARK, width=2)


def draw_course(draw: ImageDraw.ImageDraw, origin=(40, 480), scale: int = 30, compact: bool = False) -> None:
    """A shared toy-course composition used by each screen concept."""
    ox, oy = origin
    unit = scale
    if compact:
        coords = [(0, 2, 7, "grass"), (8, 1, 4, "sand"), (13, 2, 4, "brick"),
                  (18, 1, 4, "wood"), (23, 3, 5, "grass"), (29, 1, 2, "sand")]
    else:
        coords = [(0, 2, 7, "grass"), (8, 4, 4, "grass"), (13, 1, 4, "sand"),
                  (18, 2, 5, "brick"), (24, 4, 5, "wood"), (30, 2, 7, "grass"),
                  (38, 1, 4, "sand")]
    for start, height, length, kind in coords:
        for col in range(length):
            for row in range(height):
                draw_block(draw, ox + (start + col) * unit, oy - (row + 1) * unit,
                           unit, kind, surface=row == height - 1)
    # Water gap.
    wx = ox + 7 * unit
    draw.rectangle((wx, oy - unit + 1, wx + unit - 1, oy - 1), fill=BLUE_DARK)
    draw.line((wx + 4, oy - unit + 8, wx + unit - 5, oy - unit + 8), fill=WHITE, width=3)
    # Playable toys and landmarks.
    sprite_scale = max(2, unit // 14)
    goal_col = 28 if compact else 40
    bush_col = 5 if compact else 6
    tree_col = 10 if compact else 12
    mushroom_col = 15 if compact else 20
    walker_col = 21 if compact else 28
    spike_col = 25 if compact else 33
    platform_col = 12 if compact else 15
    sand_platform_col = 20 if compact else 26
    paste_center(draw._image, player_sprite(), (ox + 3 * unit, oy - 2 * unit - unit // 2), sprite_scale)
    paste_center(draw._image, goal_sprite(), (ox + goal_col * unit, oy - 2 * unit), sprite_scale)
    paste_center(draw._image, bush_sprite(), (ox + bush_col * unit, oy - 2 * unit), sprite_scale)
    paste_center(draw._image, tree_sprite(), (ox + tree_col * unit, oy - 4 * unit), sprite_scale)
    paste_center(draw._image, mushroom_sprite(), (ox + mushroom_col * unit, oy - 2 * unit), sprite_scale)
    paste_center(draw._image, walker_sprite(), (ox + walker_col * unit, oy - 4 * unit), sprite_scale)
    paste_center(draw._image, spike_sprite(), (ox + spike_col * unit, oy - 2 * unit), sprite_scale)
    draw_platform(draw, ox + platform_col * unit, oy - 6 * unit, 4 * unit)
    draw_platform(draw, ox + sand_platform_col * unit, oy - 5 * unit, 5 * unit, "sand")


def button(draw: ImageDraw.ImageDraw, box: tuple[int, int, int, int], label: str,
           fill=CREAM, icon: Image.Image | None = None, selected: bool = False,
           subtitle: str | None = None, pixel: bool = True) -> None:
    """A reusable chunky game button used in the screen concepts."""
    x0, y0, x1, y1 = box
    face = YELLOW if selected else fill
    rounded(draw, box, face, 13, INK, 3, shadow=True)
    if icon is not None:
        side = min(y1 - y0 - 14, 34)
        paste_center(draw._image, icon, (x0 + 27, (y0 + y1) // 2), max(1, side // 16))
        tx = x0 + 53
        anchor = "lm"
    else:
        tx = (x0 + x1) // 2
        anchor = "mm"
    if subtitle:
        text(draw, (tx, (y0 + y1) // 2 - 8), label.upper(), 13, INK, pixel=pixel, anchor=anchor)
        text(draw, (tx, (y0 + y1) // 2 + 11), subtitle, 10, MUTED, pixel=False, anchor=anchor)
    else:
        text(draw, (tx, (y0 + y1) // 2), label.upper(), 13, INK, pixel=pixel, anchor=anchor)


def mode_tab(draw: ImageDraw.ImageDraw, x: int, label: str, icon: Image.Image,
             selected: bool = False, width: int = 118) -> None:
    rounded(draw, (x, 18, x + width, 76), YELLOW if selected else CREAM, 13, INK, 3, shadow=selected)
    paste_center(draw._image, icon, (x + 27, 47), 2)
    text(draw, (x + 48, 47), label.upper(), 11, INK, pixel=True, anchor="lm")


def metric_card(draw: ImageDraw.ImageDraw, box: tuple[int, int, int, int], value: str,
                label: str, fill=CREAM, accent=GREEN) -> None:
    rounded(draw, box, fill, 14, INK, 3, shadow=False)
    x0, y0, x1, y1 = box
    draw.rounded_rectangle((x0, y0, x0 + 8, y1), 4, fill=accent)
    text(draw, (x0 + 22, y0 + 17), value, 21, INK, pixel=True)
    text(draw, (x0 + 22, y0 + 49), label.upper(), 9, MUTED, pixel=True)


def draw_top_bar(draw: ImageDraw.ImageDraw, sprites: dict[str, Image.Image], selected: str = "BUILD") -> None:
    draw.rectangle((0, 0, 1360, 94), fill=CREAM)
    draw.line((0, 91, 1360, 91), fill=INK, width=4)
    # Small brand plate.
    rounded(draw, (18, 16, 202, 75), YELLOW, 15, INK, 3, shadow=True)
    text(draw, (31, 31), "ONE TWO", 11, INK, pixel=True)
    text(draw, (31, 51), "HUNDRED", 14, INK, pixel=True)
    text(draw, (185, 47), "*", 20, CORAL, anchor="mm")
    x = 225
    for label, icon_key, width in (("BUILD", "grass", 110), ("PLAY", "runner", 104),
                                   ("TRAIN", "walker", 112), ("REPLAY", "goal", 124)):
        mode_tab(draw, x, label, sprites[icon_key], label == selected, width)
        x += width + 8
    rounded(draw, (1110, 18, 1218, 76), CREAM, 13, INK, 3, shadow=False)
    paste_center(draw._image, sprites["ui_search"], (1136, 47), 2)
    text(draw, (1158, 47), "HELP", 10, INK, pixel=True, anchor="lm")
    rounded(draw, (1234, 18, 1342, 76), CREAM, 13, INK, 3, shadow=False)
    text(draw, (1288, 47), "MUSIC  ON", 9, INK, pixel=True, anchor="mm")


def draw_title(sprites: dict[str, Image.Image]) -> Image.Image:
    img = new((1360, 800), SKY)
    d = ImageDraw.Draw(img)
    draw_sky(d, img.size)
    # A soft stage under the course makes the title and doors feel anchored.
    d.rectangle((0, 610, 1360, 800), fill="#173d59")
    d.rectangle((0, 610, 1360, 635), fill="#4b9e82")
    draw_course(d, (72, 584), 28)
    # Brand lockup on a sticker-like sign.
    rounded(d, (353, 40, 1007, 205), CREAM, 26, INK, 6, shadow=True)
    text(d, (680, 78), "ONE TWO", 26, CORAL, pixel=True, anchor="mm", stroke=1, stroke_fill=INK)
    text(d, (680, 130), "HUNDRED", 42, YELLOW, pixel=True, anchor="mm", stroke=2, stroke_fill=INK)
    text(d, (680, 182), "BUILD A COURSE - GROW A CHAMPION", 12, INK, pixel=True, anchor="mm")
    # Menu doors.
    rounded(d, (400, 640, 960, 777), "#163852", 20, INK, 5, shadow=True)
    doors = (("BUILD", "make", "grass", GREEN), ("PLAY", "try it", "runner", CORAL),
             ("TRAIN", "teach AI", "walker", PURPLE), ("REPLAY", "watch", "goal", BLUE))
    for index, (label, sub, icon, fill) in enumerate(doors):
        x0 = 418 + index * 133
        rounded(d, (x0, 658, x0 + 121, 752), fill if index == 0 else CREAM, 14, INK, 3, shadow=index == 0)
        paste_center(img, sprites[icon], (x0 + 60, 686), 3)
        text(d, (x0 + 60, 717), label, 11, INK, pixel=True, anchor="mm")
        text(d, (x0 + 60, 737), sub, 8, INK_SOFT, anchor="mm")
    text(d, (680, 795), "UP DOWN CHOOSE     ENTER START     P PLAY", 10, CREAM, pixel=True, anchor="mm")
    text(d, (22, 774), "A FRIENDLY PLATFORMER FOR CURIOUS MINDS", 9, CREAM, pixel=True)
    text(d, (1338, 774), "v0.18  REVIEW", 9, CREAM, pixel=True, anchor="ra")
    return img


def draw_editor(sprites: dict[str, Image.Image]) -> Image.Image:
    img = new((1360, 800), SKY_LIGHT)
    d = ImageDraw.Draw(img)
    draw_sky(d, img.size)
    draw_top_bar(d, sprites, "BUILD")
    # Playfield frame.
    rounded(d, (218, 118, 1068, 708), "#d7f4e9", 18, INK, 4, shadow=True)
    d.rectangle((238, 138, 1048, 688), fill="#72c8dd")
    # A little scenic horizon behind the authored course.
    for x in range(260, 1040, 126):
        paste_center(img, tree_sprite(), (x, 386), 3)
    draw_course(d, (255, 632), 24, compact=True)
    # Editor selection guides and active brush cursor.
    d.line((515, 360, 515, 632), fill=YELLOW, width=3)
    d.line((488, 495, 542, 495), fill=YELLOW, width=3)
    rounded(d, (475, 465, 555, 530), (255, 204, 66, 80), 9, YELLOW, 3, shadow=False)
    text(d, (585, 170), "CLICK TO BUILD", 12, INK, pixel=True)
    badge(d, (865, 154), "BEATABLE!", GREEN_LIGHT, 145, 10)
    # Left palette as a friendly toy tray.
    panel(d, (18, 118, 198, 708), PAPER, "PARTS", BLUE)
    text(d, (36, 173), "SEARCH PARTS", 9, MUTED, pixel=True)
    rounded(d, (31, 188, 185, 222), WHITE, 8, INK, 2)
    paste_center(img, sprites["ui_search"], (50, 205), 1)
    text(d, (67, 205), "find a toy...", 11, MUTED, anchor="lm")
    categories = (("BUILD", GREEN, ("grass", "sand", "wood", "brick")),
                  ("DANGER", CORAL, ("spike", "lava", "spiked_ball")),
                  ("HELPERS", BLUE, ("spring", "water", "vine", "rope")),
                  ("FRIENDS", PURPLE, ("walker", "chaser", "flyer", "turret")),
                  ("FINISH", YELLOW, ("spawn", "goal", "crate")))
    cy = 255
    for heading, color, keys in categories:
        text(d, (35, cy), heading, 9, color, pixel=True)
        cy += 17
        for index, key in enumerate(keys):
            x = 34 + (index % 3) * 47
            y = cy + (index // 3) * 45
            rounded(d, (x, y, x + 39, y + 38), YELLOW if key == "grass" else CREAM, 9, INK, 2, shadow=False)
            paste_center(img, sprites[key], (x + 19, y + 19), 2)
        cy += ((len(keys) + 2) // 3) * 45 + 16
    # Bottom brush card and controls.
    rounded(d, (235, 724, 1052, 777), CREAM, 13, INK, 3, shadow=False)
    paste_center(img, sprites["grass"], (267, 750), 2)
    text(d, (292, 741), "GRASS BLOCK", 11, INK, pixel=True)
    text(d, (292, 760), "solid ground - drag to paint - shift for a line", 10, MUTED)
    button(d, (730, 737, 808, 768), "UNDO", CREAM, sprites["ui_undo"], False, None, True)
    button(d, (817, 737, 895, 768), "REDO", CREAM, sprites["ui_redo"], False, None, True)
    button(d, (904, 737, 1032, 768), "PLAYTEST", YELLOW, sprites["runner"], True, None, True)
    # Right-side level card.
    panel(d, (1090, 118, 1342, 708), PAPER, "COURSE CARD", PURPLE)
    text(d, (1110, 178), "SUNNY SWITCHBACK", 12, INK, pixel=True)
    text(d, (1110, 201), "your course", 11, MUTED)
    metric_card(d, (1108, 244, 1324, 312), "60 x 15", "course size", CREAM, BLUE)
    metric_card(d, (1108, 325, 1324, 393), "3", "tricks placed", CREAM, CORAL)
    metric_card(d, (1108, 406, 1324, 474), "OK", "ready to play", CREAM, GREEN)
    text(d, (1110, 516), "QUICK KEYS", 9, MUTED, pixel=True)
    for i, row in enumerate(("P     playtest", "G     show guides", "F     fit course", "?     key card")):
        text(d, (1110, 544 + i * 27), row, 10, INK, pixel=True)
    rounded(d, (1108, 658, 1324, 688), YELLOW, 10, INK, 2)
    text(d, (1216, 673), "SAVE COURSE", 9, INK, pixel=True, anchor="mm")
    text(d, (18, 785), "BUILD MODE", 9, INK, pixel=True)
    text(d, (1342, 785), "brush ready - grass block", 9, INK, pixel=True, anchor="ra")
    return img


def draw_training(sprites: dict[str, Image.Image]) -> Image.Image:
    img = new((1360, 800), SKY_LIGHT)
    d = ImageDraw.Draw(img)
    draw_sky(d, img.size, dark=True)
    draw_top_bar(d, sprites, "TRAIN")
    # Stage.
    rounded(d, (22, 118, 913, 588), "#204760", 18, INK, 4, shadow=True)
    d.rectangle((42, 138, 893, 568), fill="#3d99ba")
    draw_course(d, (62, 520), 21, compact=True)
    # Training trails: colorful dots and tiny ghost bodies.
    for points, color in (((130, 474, 215, 445, 295, 460, 380, 390), CORAL),
                          ((130, 474, 240, 458, 334, 430, 410, 390), YELLOW),
                          ((130, 474, 275, 470, 366, 470, 446, 414), PURPLE),
                          ((130, 474, 305, 458, 390, 405, 500, 404), GREEN_LIGHT)):
        d.line([(points[i], points[i + 1]) for i in range(0, len(points), 2)], fill=color, width=5)
        for i in range(0, len(points), 2):
            d.ellipse((points[i] - 5, points[i + 1] - 5, points[i] + 5, points[i + 1] + 5), fill=color)
    paste_center(img, sprites["runner"], (550, 425), 3)
    paste_center(img, sprites["runner"], (694, 367), 3)
    paste_center(img, sprites["runner_jump"], (778, 386), 3)
    badge(d, (57, 155), "AI PLAYGROUND", YELLOW, 186, 11)
    text(d, (57, 203), "THE POPULATION IS LEARNING", 16, CREAM, pixel=True)
    text(d, (57, 230), "watch the little runners try, miss, and improve", 12, "#cde9ef")
    # Right control board.
    panel(d, (945, 118, 1342, 588), PAPER, "TRAINING RUN", PURPLE)
    text(d, (970, 177), "SUNNY SWITCHBACK", 12, INK, pixel=True)
    text(d, (970, 201), "generation 48 of 200", 11, MUTED)
    metric_card(d, (967, 236, 1320, 300), "18 / 200", "runners reached the flag", CREAM, GREEN)
    metric_card(d, (967, 314, 1136, 386), "4.2s", "best time", CREAM, YELLOW)
    metric_card(d, (1150, 314, 1320, 386), "72%", "course learned", CREAM, BLUE)
    rounded(d, (967, 412, 1320, 468), YELLOW, 14, INK, 3, shadow=True)
    paste_center(img, sprites["ui_pause"], (996, 440), 2)
    text(d, (1020, 430), "PAUSE TRAINING", 11, INK, pixel=True)
    text(d, (1020, 452), "space", 10, INK_SOFT)
    button(d, (967, 486, 1137, 538), "NEXT TRY", CREAM, sprites["ui_step"], False, None, True)
    button(d, (1150, 486, 1320, 538), "SAVE STAR", CREAM, sprites["ui_star"], False, None, True)
    text(d, (970, 555), "UP faster     DOWN slower", 10, MUTED, pixel=True)
    # Chart board.
    rounded(d, (22, 612, 1342, 762), "#17324d", 18, INK, 4, shadow=True)
    text(d, (46, 638), "LEARNING METER", 11, YELLOW, pixel=True)
    text(d, (46, 666), "the best run", 10, CREAM, pixel=True)
    text(d, (46, 704), "the whole group", 10, "#8fd9ee", pixel=True)
    chart_left, chart_top, chart_right, chart_bottom = 210, 638, 1308, 738
    d.line((chart_left, chart_bottom, chart_right, chart_bottom), fill="#587a8e", width=2)
    d.line((chart_left, chart_top + 15, chart_right, chart_top + 15), fill="#31566d", width=2)
    best = [(chart_left, 728), (330, 724), (430, 720), (530, 695), (630, 702), (730, 676), (830, 681),
            (930, 660), (1030, 650), (1130, 646), (1230, 642), (chart_right, 638)]
    avg = [(chart_left, 735), (330, 733), (430, 731), (530, 727), (630, 729), (730, 722), (830, 724),
           (930, 716), (1030, 713), (1130, 709), (1230, 707), (chart_right, 704)]
    d.line(best, fill=GREEN_LIGHT, width=4)
    d.line(avg, fill="#8fd9ee", width=3)
    for x, y in best[::2]:
        d.ellipse((x - 4, y - 4, x + 4, y + 4), fill=YELLOW)
    text(d, (22, 788), "TRAIN MODE", 9, INK, pixel=True)
    text(d, (1342, 788), "the population is getting curious", 9, INK, pixel=True, anchor="ra")
    return img


def draw_play(sprites: dict[str, Image.Image]) -> Image.Image:
    img = new((1360, 800), SKY_LIGHT)
    d = ImageDraw.Draw(img)
    draw_sky(d, img.size)
    draw_top_bar(d, sprites, "PLAY")
    # Full-width course stage with HUD cards floating over it.
    rounded(d, (22, 118, 1338, 690), "#d7f4e9", 18, INK, 4, shadow=True)
    d.rectangle((42, 138, 1318, 670), fill="#70c6df")
    # Make the playtest course long enough to suggest movement.
    draw_course(d, (58, 600), 30)
    # More visible player and ghost on the stage.
    paste_center(img, sprites["runner"], (418, 472), 4)
    ghost = sprites["runner"].copy()
    ghost.putalpha(90)
    paste_center(img, ghost, (308, 503), 4)
    text(d, (300, 542), "GHOST", 9, PURPLE_DARK, pixel=True, anchor="mm")
    # HUD top-left and top-right.
    rounded(d, (63, 156, 332, 234), CREAM, 16, INK, 3, shadow=True)
    text(d, (87, 180), "SUNNY SWITCHBACK", 11, INK, pixel=True)
    text(d, (87, 211), "PLAYTEST", 10, CORAL, pixel=True)
    rounded(d, (1046, 156, 1292, 234), CREAM, 16, INK, 3, shadow=True)
    text(d, (1070, 181), "TIME", 10, MUTED, pixel=True)
    text(d, (1070, 211), "00:04.28", 20, INK, pixel=True)
    # Goal ribbon and mini status at the center top.
    badge(d, (560, 155), "REACH THE FLAG", YELLOW, 240, 11)
    # Bottom action strip.
    rounded(d, (363, 708, 997, 778), "#17324d", 18, INK, 4, shadow=True)
    keys = (("A / LEFT", "MOVE"), ("D / RIGHT", "MOVE"), ("SPACE", "JUMP"), ("SHIFT", "RUN"))
    for i, (key, label) in enumerate(keys):
        x = 382 + i * 147
        rounded(d, (x, 723, x + 130, 762), CREAM if i != 2 else YELLOW, 10, INK, 2)
        text(d, (x + 65, 738), key, 10, INK, pixel=True, anchor="mm")
        text(d, (x + 65, 754), label, 8, MUTED, pixel=True, anchor="mm")
    # Compact game controls / ghost card.
    rounded(d, (1080, 570, 1292, 646), PAPER, 14, INK, 3, shadow=False)
    paste_center(img, sprites["walker"], (1107, 608), 2)
    text(d, (1134, 595), "GHOST ON", 10, PURPLE_DARK, pixel=True)
    text(d, (1134, 620), "beat 00:03.92", 10, MUTED)
    rounded(d, (63, 604, 268, 652), CREAM, 13, INK, 3)
    text(d, (83, 628), "R  RESTART", 10, INK, pixel=True)
    text(d, (22, 788), "PLAY MODE", 9, INK, pixel=True)
    text(d, (1338, 788), "P pause menu - G ghost", 9, INK, pixel=True, anchor="ra")
    return img


def draw_replay(sprites: dict[str, Image.Image]) -> Image.Image:
    img = new((1360, 800), SKY_LIGHT)
    d = ImageDraw.Draw(img)
    draw_sky(d, img.size, dark=True)
    draw_top_bar(d, sprites, "REPLAY")
    rounded(d, (22, 118, 958, 690), "#204760", 18, INK, 4, shadow=True)
    d.rectangle((42, 138, 938, 670), fill="#3e9bb9")
    draw_course(d, (62, 600), 28, compact=True)
    # Ghost field in rank colors.
    trails = [((154, 533), (265, 488), (396, 502), PURPLE), ((154, 533), (283, 475), (440, 438), BLUE),
              ((154, 533), (312, 502), (457, 477), GREEN_LIGHT), ((154, 533), (341, 488), (484, 401), CORAL)]
    for (a, b, c, color) in trails:
        d.line([a, b, c], fill=color, width=5)
        for x, y in (a, b, c):
            d.ellipse((x - 5, y - 5, x + 5, y + 5), fill=color)
        ghost = sprites["runner"].copy()
        ghost.putalpha(110)
        paste_center(img, ghost, b, 3)
    paste_center(img, sprites["runner"], (645, 391), 4)
    paste_center(img, sprites["goal"], (859, 502), 4)
    badge(d, (62, 158), "HIGHLIGHT REEL", YELLOW, 190, 11)
    rounded(d, (62, 205, 538, 282), CREAM, 16, INK, 3, shadow=True)
    paste_center(img, sprites["ui_star"], (96, 243), 3)
    text(d, (133, 228), "THE STAR RUN", 13, INK, pixel=True)
    text(d, (133, 255), "generation 48 - quickest clear", 10, MUTED)
    # Right replay card.
    panel(d, (985, 118, 1342, 690), PAPER, "REPLAY CARD", BLUE)
    text(d, (1010, 178), "SUNNY SWITCHBACK", 12, INK, pixel=True)
    text(d, (1010, 201), "the best run, side by side", 10, MUTED)
    metric_card(d, (1007, 238, 1320, 305), "00:03.92", "fastest clear", CREAM, YELLOW)
    metric_card(d, (1007, 320, 1158, 387), "48", "generation", CREAM, PURPLE)
    metric_card(d, (1168, 320, 1320, 387), "4", "ghost paths", CREAM, BLUE)
    button(d, (1007, 416, 1320, 468), "PLAY HIGHLIGHT", YELLOW, sprites["ui_play"], True)
    button(d, (1007, 480, 1320, 532), "SHOW GHOSTS", CREAM, sprites["walker"], False)
    button(d, (1007, 544, 1320, 596), "TRY IT YOURSELF", CREAM, sprites["runner"], False)
    text(d, (1010, 635), "LEFT RIGHT scrub run", 10, MUTED, pixel=True)
    # Timeline.
    rounded(d, (62, 710, 1292, 770), "#17324d", 14, INK, 3, shadow=True)
    d.line((105, 741, 1250, 741), fill="#7aaec1", width=4)
    d.line((105, 741, 864, 741), fill=YELLOW, width=6)
    d.ellipse((856, 732, 876, 752), fill=YELLOW, outline=INK, width=2)
    for x in (105, 344, 582, 820, 1059, 1250):
        d.line((x, 733, x, 750), fill=CREAM, width=2)
    text(d, (62, 788), "REPLAY MODE", 9, INK, pixel=True)
    text(d, (1338, 788), "star solid - ghosts translucent", 9, INK, pixel=True, anchor="ra")
    return img


def draw_asset_board(sprites: dict[str, Image.Image]) -> Image.Image:
    img = new((1440, 1020), PAPER)
    d = ImageDraw.Draw(img)
    d.rectangle((0, 0, 1440, 96), fill=INK)
    text(d, (48, 36), "THE COURSE KIT", 24, YELLOW, pixel=True)
    text(d, (48, 72), "A brighter, friendlier pass across every graphic family", 14, CREAM)
    text(d, (1392, 52), "REVIEW ONLY", 10, YELLOW, pixel=True, anchor="ra")
    groups = [
        ("GROUND", GREEN, ["grass", "sand", "brick", "wood", "crate"]),
        ("TRAPS + ELEMENTS", CORAL, ["spike", "spring", "water", "lava", "vine", "rope", "spiked_ball", "belt"]),
        ("CAST", PURPLE, ["runner", "runner_jump", "walker", "chaser", "flyer", "turret", "goal", "spawn"]),
        ("DRESSING", BLUE, ["tree", "bush", "mushroom", "boulder", "cloud", "dust", "puff", "spark"]),
        ("UI GLYPHS", YELLOW, ["ui_undo", "ui_redo", "ui_fit", "ui_search", "ui_restart", "ui_pause", "ui_play", "ui_step", "ui_star"]),
    ]
    sprites_per_row = 9
    y = 124
    for heading, color, keys in groups:
        text(d, (48, y), heading, 13, color, pixel=True)
        d.line((220, y + 6, 1392, y + 6), fill="#d2e5e1", width=2)
        y += 28
        for i, key in enumerate(keys):
            col = i % sprites_per_row
            row = i // sprites_per_row
            x = 62 + col * 150
            cell_y = y + row * 112
            rounded(d, (x, cell_y, x + 126, cell_y + 84), CREAM, 14, INK, 3, shadow=False)
            paste_center(img, sprites[key], (x + 63, cell_y + 35), 4)
            label = key.replace("_", " ").upper()
            text(d, (x + 63, cell_y + 72), label, 8, INK, pixel=True, anchor="mm")
        y += ((len(keys) + sprites_per_row - 1) // sprites_per_row) * 112 + 18
    # A small callout at the bottom reinforces the asset rules.
    rounded(d, (48, 940, 1392, 995), "#e4f6ee", 14, INK, 3)
    text(d, (72, 958), "SHARED RULE", 10, GREEN_DARK, pixel=True)
    text(d, (214, 965), "chunky outline - bright face - simple texture - readable silhouette at 1x", 13, INK)
    return img


def draw_ui_kit(sprites: dict[str, Image.Image]) -> Image.Image:
    img = new((1440, 920), CREAM)
    d = ImageDraw.Draw(img)
    d.rectangle((0, 0, 1440, 98), fill=INK)
    text(d, (48, 38), "UI KIT", 26, YELLOW, pixel=True)
    text(d, (48, 74), "same toy-box language on every screen", 14, CREAM)
    text(d, (1392, 51), "PROPOSED SYSTEM", 10, YELLOW, pixel=True, anchor="ra")
    # Palette row.
    panel(d, (48, 126, 1392, 256), PAPER, "COLOR ROLES", BLUE)
    swatches = ((INK, "outline / type"), (YELLOW, "selection / reward"), (CORAL, "player / action"),
                (GREEN, "success / build"), (BLUE, "navigation / water"), (PURPLE, "AI / replay"),
                (CREAM, "card / surface"))
    for i, (color, label) in enumerate(swatches):
        x = 75 + i * 185
        d.rounded_rectangle((x, 187, x + 52, 232), 10, fill=color, outline=INK, width=3)
        text(d, (x + 64, 204), label.upper(), 9, INK, pixel=True, anchor="lm")
    # Button grammar.
    panel(d, (48, 282, 700, 540), PAPER, "BUTTONS + TABS", CORAL)
    button(d, (78, 340, 308, 400), "BUILD", YELLOW, sprites["grass"], True, "selected mode")
    button(d, (330, 340, 560, 400), "PLAY", CREAM, sprites["runner"], False, "resting mode")
    button(d, (78, 420, 308, 480), "SAVE COURSE", CREAM, sprites["crate"], False)
    button(d, (330, 420, 560, 480), "PLAY HIGHLIGHT", YELLOW, sprites["ui_play"], True)
    rounded(d, (78, 497, 560, 515), INK, 8, INK, 0)
    text(d, (78, 525), "Selected = bright face + hard shadow", 11, MUTED)
    # Cards and status language.
    panel(d, (730, 282, 1392, 540), PAPER, "CARDS + STATUS", GREEN)
    metric_card(d, (762, 340, 1020, 410), "18 / 200", "runners reached the flag", CREAM, GREEN)
    metric_card(d, (1042, 340, 1300, 410), "4.2s", "best clear", CREAM, YELLOW)
    badge(d, (762, 444), "BEATABLE!", GREEN_LIGHT, 140, 10)
    badge(d, (925, 444), "TRY AGAIN", YELLOW_LIGHT, 140, 10)
    badge(d, (1088, 444), "WATCHING", "#cbd5ff", 140, 10)
    text(d, (762, 502), "Ribbons name sections; cards hold facts; badges say how the course feels.", 11, MUTED)
    # Palette anatomy.
    panel(d, (48, 566, 1392, 858), PAPER, "PALETTE ANATOMY", PURPLE)
    rounded(d, (78, 626, 410, 818), "#d7f4e9", 15, INK, 3, shadow=False)
    text(d, (104, 652), "BUILD", 12, GREEN_DARK, pixel=True)
    parts = (("GROUND", GREEN, "grass / sand / wood"), ("DANGER", CORAL, "spikes / lava"),
             ("HELPERS", BLUE, "spring / water / rope"), ("FRIENDS", PURPLE, "walkers / flyers"),
             ("FINISH", YELLOW, "spawn / goal"))
    for i, (label, color, desc) in enumerate(parts):
        yy = 686 + i * 24
        d.rounded_rectangle((104, yy, 119, yy + 15), 4, fill=color, outline=INK, width=1)
        text(d, (132, yy + 8), label, 9, INK, pixel=True, anchor="lm")
        text(d, (226, yy + 8), desc, 10, MUTED, anchor="lm")
    # Icon sizing / states.
    text(d, (470, 650), "ICON STATES", 11, INK, pixel=True)
    states = (("RESTING", CREAM, False), ("HOVER", "#ffffff", False), ("SELECTED", YELLOW, True), ("DISABLED", "#d3d8d6", False))
    for i, (label, fill, selected) in enumerate(states):
        x = 470 + (i % 2) * 180
        yy = 681 + (i // 2) * 66
        rounded(d, (x, yy, x + 148, yy + 46), fill, 12, INK, 3, shadow=selected)
        paste_center(img, sprites["runner"], (x + 25, yy + 23), 2)
        text(d, (x + 48, yy + 23), label, 9, INK, pixel=True, anchor="lm")
    # Review callout.
    rounded(d, (860, 626, 1358, 818), "#17324d", 15, INK, 3, shadow=True)
    text(d, (890, 660), "THE FEEL", 12, YELLOW, pixel=True)
    text(d, (890, 700), "friendly", 20, CREAM, pixel=True)
    text(d, (890, 733), "clear", 20, GREEN_LIGHT, pixel=True)
    text(d, (890, 766), "exciting", 20, CORAL, pixel=True)
    text(d, (890, 800), "without losing the numbers", 11, "#cde9ef")
    return img


def draw_tiling_test(sprites: dict[str, Image.Image]) -> Image.Image:
    """Show the repeated surfaces at their intended pixel scale.

    This is intentionally a separate board from the asset gallery: a sprite can
    look attractive in isolation and still fail as a surface. The rows below
    make the edge-to-edge rule visible before any renderer integration.
    """
    img = new((1440, 760), "#d7f4e9")
    d = ImageDraw.Draw(img)
    d.rectangle((0, 0, 1440, 100), fill=INK)
    text(d, (48, 38), "TILE TEST", 26, YELLOW, pixel=True)
    text(d, (48, 75), "surface cells repeated with no per-cell outline", 14, CREAM)
    text(d, (1392, 52), "EDGE-TO-EDGE", 10, YELLOW, pixel=True, anchor="ra")
    rows = (("GRASS", "grass", GREEN), ("SAND", "sand", SAND), ("BRICK", "brick", BRICK),
            ("WOOD", "wood", WOOD), ("WATER", "water", WATER), ("LAVA", "lava", LAVA))
    scale = 7
    x_start = 270
    y = 122
    for label, key, color in rows:
        rounded(d, (48, y, 1392, y + 82), PAPER, 14, INK, 3, shadow=False)
        text(d, (76, y + 41), label, 11, color, pixel=True, anchor="lm")
        tile = sprites[key].resize((16 * scale, 16 * scale), Image.Resampling.NEAREST)
        for col in range(10):
            img.paste(tile, (x_start + col * 112, y + 8), tile)
        y += 99
    rounded(d, (48, 724, 1392, 748), "#17324d", 8, INK, 0)
    text(d, (720, 736), "terrain owns the seams; objects own the outline", 10, CREAM, pixel=True, anchor="mm")
    return img


def write_outputs() -> None:
    sprites = build_sprites()
    save_sprites(sprites)
    save_complete_candidate(sprites)
    outputs = {
        "01-title-concept.png": draw_title(sprites),
        "02-editor-concept.png": draw_editor(sprites),
        "03-training-concept.png": draw_training(sprites),
        "04-play-concept.png": draw_play(sprites),
        "05-replay-concept.png": draw_replay(sprites),
        "06-asset-board.png": draw_asset_board(sprites),
        "07-ui-kit.png": draw_ui_kit(sprites),
        "08-tiling-test.png": draw_tiling_test(sprites),
    }
    for name, image in outputs.items():
        image.save(ROOT / name)
    print(f"wrote {len(outputs)} screen concepts and {len(sprites)} review sprites to {ROOT}")


if __name__ == "__main__":
    write_outputs()
