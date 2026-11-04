"""The production course-kit: terrain, fluids and mechanical tiles.

This module is the *source* of the shipped atlas. `tools/gen_tileset.py` imports it,
flattens the sprite families in a fixed order and writes `assets/tileset.png` plus the
manifest. Nothing here is review-only art: what this file draws is what the game loads.

The direction, stated once because every function follows it:

* **No outlines.** Form is carried by value — a lit rim on the top edge, a darker
  sel-out on the bottom, three to five tones per material. Black or navy rims around
  every object is what makes 16-pixel art read as a prototype.
* **Terrain owns its seams.** Every material is a *texture*, not a sticker: the body
  of a tile is a function of its own coordinates, so two stacked or side-by-side
  tiles continue each other, and the autotile adds only crowns and edge light.
* **One light.** Sunlight comes from the upper left, warm. Lit edges are warmer and
  lighter, shadows are cooler and darker, and the same ramp is used by the terrain,
  the cast and the props.

Deterministic: every speckle comes from an integer hash rather than `random`, so
regenerating never produces a spurious diff.
"""

from __future__ import annotations

import math

from PIL import Image, ImageDraw

CELL = 16
CLEAR = (0, 0, 0, 0)
OPAQUE = 255


# --------------------------------------------------------------------------------------
# Colour
# --------------------------------------------------------------------------------------


def rgb(value: str | tuple[int, int, int]) -> tuple[int, int, int]:
    if isinstance(value, tuple):
        return value
    value = value.lstrip("#")
    return tuple(int(value[i : i + 2], 16) for i in (0, 2, 4))


def rgba(value: str | tuple[int, int, int], alpha: int = OPAQUE) -> tuple[int, int, int, int]:
    return (*rgb(value), alpha)


def tint(colour: tuple[int, ...], alpha: int) -> tuple[int, int, int, int]:
    """The same colour at a different opacity, for overlays and glows."""
    return (colour[0], colour[1], colour[2], alpha)


class Ramp:
    """One material's five tones, from lit to buried.

    Five rather than three because 16-pixel art needs a rim, a face, a body, a shadow
    and a sel-out; three tones make a shape read as a silhouette, which is exactly the
    flat-sticker look this pass removes.
    """

    __slots__ = ("hi", "lit", "mid", "sh", "deep")

    def __init__(self, hi: str, lit: str, mid: str, sh: str, deep: str):
        self.hi = rgba(hi)
        self.lit = rgba(lit)
        self.mid = rgba(mid)
        self.sh = rgba(sh)
        self.deep = rgba(deep)

    def tone(self, index: int) -> tuple[int, int, int, int]:
        return (self.hi, self.lit, self.mid, self.sh, self.deep)[max(0, min(4, index))]


# The palette. Warm, slightly desaturated, one light from the upper left. Hues are
# spread far enough that a player can tell the five materials apart at one tile, and
# values are close enough that a wall of any of them does not shimmer.
SOIL = Ramp("#C4956D", "#A4774F", "#875B40", "#694432", "#4E3128")
GRASS = Ramp("#BCDE80", "#97C968", "#71AB55", "#548946", "#3D6C3B")
SAND = Ramp("#F3E2B4", "#E7CB93", "#D6B378", "#BC9260", "#9A724C")
BRICK = Ramp("#DE9A7C", "#C87B65", "#AE6252", "#8B4A43", "#6B3937")
WOOD = Ramp("#D9B183", "#BE9265", "#A0744F", "#7C583D", "#5C3F2F")
STONE = Ramp("#C6CDD6", "#A7B0BC", "#8A93A1", "#6B7484", "#4F5666")
SANDSTONE = Ramp("#EAD4A6", "#D9BD8A", "#C4A473", "#A6855B", "#846847")
MASONRY = Ramp("#CFD3CF", "#B2B6B2", "#949A98", "#737A7A", "#565C5E")
TIMBER = Ramp("#C69A6B", "#AA8056", "#8C6644", "#6C4E37", "#4E3929")

WATER = Ramp("#BDEFF2", "#7FD4DE", "#3FA6C4", "#2A7FA5", "#1E5F86")
LAVA = Ramp("#FFE08A", "#F7A93E", "#E4713A", "#BE4A2D", "#8E2C2A")

# The cast and props. Kept outside the material ramps so the palette has one entry
# per idea rather than a colour per sprite.
CORAL = Ramp("#F6A08C", "#EC7A62", "#D9574A", "#B03E3F", "#822E36")
GOLD = Ramp("#FFF0B0", "#F7D468", "#EDB640", "#C88A2C", "#94631F")
CREAM = Ramp("#FFFDF6", "#F8EDD6", "#EBD9B8", "#CDB894", "#A48F70")
SKIN = Ramp("#FFE0BE", "#F6C79C", "#E2A878", "#BE8159", "#94603F")
LEAF = Ramp("#BCDC8C", "#98C471", "#77A75A", "#578547", "#3E6539")
LEAF_DARK = Ramp("#94BC7E", "#77A167", "#5C8453", "#456843", "#334F36")
IRON = Ramp("#D9DFE6", "#B2BAC4", "#8C95A1", "#6B7480", "#4E5662")
PURPLE = Ramp("#C4B0F0", "#9E85DB", "#7C62BB", "#5D4793", "#43326E")
COOL = Ramp("#D6ECF4", "#A7D2E2", "#7CB0C8", "#5A8AA4", "#3F6880")
TEAL = Ramp("#B8EBDD", "#84D2BF", "#54AE9C", "#3B897B", "#28665B")
CYAN = Ramp("#E6FBFF", "#A9E9F5", "#6FCBDF", "#4A9FB6", "#32788C")
DARK = rgba("#3B2D33")
INK = rgba("#17324d")
WHITE = rgba("#FFFFFF")


# --------------------------------------------------------------------------------------
# Canvas and pixel helpers
# --------------------------------------------------------------------------------------


def canvas() -> Image.Image:
    return Image.new("RGBA", (CELL, CELL), CLEAR)


def rect(img: Image.Image, x0: int, y0: int, x1: int, y1: int, colour) -> None:
    """An inclusive rectangle, clipped to the cell."""
    x0, x1 = max(0, min(15, min(x0, x1))), max(0, min(15, max(x0, x1)))
    y0, y1 = max(0, min(15, min(y0, y1))), max(0, min(15, max(y0, y1)))
    if x0 > x1 or y0 > y1:
        return
    ImageDraw.Draw(img).rectangle((x0, y0, x1, y1), fill=colour)


def put(img: Image.Image, x: int, y: int, colour) -> None:
    if 0 <= x < 16 and 0 <= y < 16:
        img.putpixel((x, y), colour)


def h32(x: int, y: int, salt: int = 0) -> int:
    """A stable integer hash, wrapped to 32 bits.

    Python's own `hash` is salted per process, which would make a regenerated atlas
    differ from the committed one; this is the same arithmetic everywhere.
    """
    n = (x * 374761393 + y * 668265263 + salt * 1103515245 + 0x9E3779B9) & 0xFFFFFFFF
    n = (n ^ (n >> 13)) & 0xFFFFFFFF
    n = (n * 1274126177) & 0xFFFFFFFF
    return (n ^ (n >> 16)) & 0xFFFFFFFF


def noise(x: int, y: int, salt: int = 0, wrap: int = 16) -> float:
    return h32(x % wrap, y % wrap, salt) / 0xFFFFFFFF


def chance(x: int, y: int, salt: int, threshold: float) -> bool:
    return noise(x, y, salt) < threshold


def _smooth(t: float) -> float:
    return t * t * (3.0 - 2.0 * t)


def vnoise(x: float, y: float, salt: int, cells: int = 4, period: int = 16) -> float:
    """Smooth value noise, periodic over `period` pixels.

    The first version of this atlas used a hash per two-pixel block, which is fine at
    one tile and a **plaid** at ten: the blocks line up with the lattice and every tile
    repeats the weave. This interpolates a lattice of `cells` samples instead, so the
    tone fields are broad, organic and still seamless, which is what a dirt body needs.
    """
    step = period / cells
    fx, fy = x / step, y / step
    x0, y0 = int(fx) % cells, int(fy) % cells
    x1, y1 = (x0 + 1) % cells, (y0 + 1) % cells
    sx, sy = _smooth(fx - int(fx)), _smooth(fy - int(fy))
    a = h32(x0, y0, salt) / 0xFFFFFFFF
    b = h32(x1, y0, salt) / 0xFFFFFFFF
    c = h32(x0, y1, salt) / 0xFFFFFFFF
    d = h32(x1, y1, salt) / 0xFFFFFFFF
    top = a + (b - a) * sx
    bottom = c + (d - c) * sx
    return top + (bottom - top) * sy


def field(x: int, y: int, salt: int, period: int = 16) -> float:
    """Two octaves of [`vnoise`]: broad shade with a little fine grain in it."""
    return vnoise(x, y, salt, 3, period) * 0.68 + vnoise(x, y, salt + 1, 7, period) * 0.32


# --------------------------------------------------------------------------------------
# Terrain
# --------------------------------------------------------------------------------------

MATERIALS = {
    # (body ramp, crown ramp, crowned). Soil's body is dirt and its crown is grass;
    # every other material is one ramp at two depths.
    "grass": (SOIL, GRASS, True),
    "sand": (SAND, SAND, True),
    "brick": (BRICK, BRICK, True),
    "wood": (WOOD, WOOD, True),
    "stone": (STONE, STONE, False),
    "sandstone": (SANDSTONE, SAND, False),
    "masonry": (MASONRY, BRICK, False),
    "timber": (TIMBER, WOOD, False),
}

# Materials whose tiles are a crown over a body; the buried half of the family is a
# separate drawing (`deep_*` for soil, `sandstone_*` for sand, and so on).
CROWNED = ("grass", "sand", "brick", "wood")


def _soil_body(img: Image.Image, ramp: Ramp, salt: int) -> None:
    """Soil as clods and grain, not as television static.

    The first shipped pass keyed every pixel off its own hash, which is white noise:
    a wall of it shimmers, and across a long platform the eye reads the 16-pixel stamp
    rather than the earth. This keeps about three quarters of the tile on one tone and
    spends the rest on a handful of *clustered* marks — a two-by-two clod, a short
    grain — so the texture has structure at the scale soil actually has it.
    """
    rect(img, 0, 0, 15, 15, ramp.mid)
    # Fine grain only. A two-by-two clod is a *shape*, and a shape in a 16-pixel tile
    # is a stamp: four of them per tile is a polka-dot field once the tile repeats.
    # Single grains at low density read as earth at any distance and never line up
    # into a motif.
    for y in range(16):
        for x in range(16):
            v = h32(x, y, salt + 7) % 100
            if v < 3:
                put(img, x, y, ramp.sh)
            elif v < 7:
                put(img, x, y, ramp.lit)
    # No pebble, and no clod. Any recognisable *shape* in the body repeats with the
    # tile, and a floor is thirty tiles wide: one pebble in the stamp is one pebble
    # every sixteen pixels, in a grid. The crown carries the detail instead.


def _sand_body(img: Image.Image, ramp: Ramp, salt: int) -> None:
    for y in range(16):
        for x in range(16):
            v = h32(x, y, salt) % 100
            if v < 16:
                tone = ramp.lit
            elif v < 24:
                tone = ramp.sh
            else:
                tone = ramp.mid
            put(img, x, y, tone)
    # Wind ripples: two long, broken crests rather than a zigzag. `lit` on the crest
    # and one row of shadow under it gives the beach a direction without weaving it.
    for base, phase in ((6, 0.0), (13, 2.4)):
        for x in range(16):
            y = base + round(0.9 * math.sin(2 * math.pi * (x / 16.0) + phase))
            if h32(x, 0, salt + base) % 100 < 34:
                continue
            put(img, x, (y - 1) % 16, ramp.hi)
            put(img, x, y % 16, ramp.sh)


def _brick_body(img: Image.Image, ramp: Ramp, salt: int) -> None:
    for y in range(16):
        course = y // 4
        offset = 4 if course % 2 else 0
        for x in range(16):
            brick = (x + offset) // 8
            local = (x + offset) % 8
            row = y % 4
            if row == 3 or local == 7:
                put(img, x, y, ramp.deep)  # mortar
                continue
            tint = noise(brick, course, salt)
            if row == 0:
                tone = ramp.hi if tint > 0.55 else ramp.lit
            elif row == 2:
                tone = ramp.sh
            elif local in (0, 6):
                tone = ramp.sh
            else:
                tone = ramp.mid if tint > 0.28 else ramp.lit
            put(img, x, y, tone)
    # A little wear: single pixels of the pale tone at the top of a course.
    for y in range(16):
        for x in range(16):
            if y % 4 == 0 and chance(x, y, salt + 3, 0.08):
                put(img, x, y, ramp.hi)


def _wood_body(img: Image.Image, ramp: Ramp, salt: int) -> None:
    for y in range(16):
        plank = y // 4
        row = y % 4
        for x in range(16):
            if row == 3:
                put(img, x, y, ramp.deep)
                continue
            grain = field(x, y, salt + plank, period=16)
            if row == 0:
                tone = ramp.lit
            elif row == 1:
                tone = ramp.hi if grain > 0.74 else ramp.lit
            elif row == 2:
                tone = ramp.sh if grain < 0.42 else ramp.mid
            else:
                tone = ramp.mid
            put(img, x, y, tone)
    # Long grain, one pixel wide: a run of the shadow tone that follows the board.
    for plank in range(4):
        for run in range(2):
            x0 = int(noise(plank, run, salt + 2) * 10) + 1
            y = plank * 4 + 1 + run
            for i in range(6):
                put(img, (x0 + i) % 16, y, ramp.sh)
    # Nails, at the same two columns of every plank: the pack builds with boards.
    for plank in range(4):
        for x in (2, 13):
            put(img, x, plank * 4 + 1, ramp.sh)
            put(img, x, plank * 4 + 2, ramp.lit)


def _stone_body(img: Image.Image, ramp: Ramp, salt: int) -> None:
    """Deep rock: strata and rubble, not a course of bricks.

    The first version drew three rows of jointed blocks, which is masonry — a wall of it
    under every lawn read as a cellar the level was standing on. Rock is *irregular*:
    two wandering bedding lines that cross a tile edge to edge, a handful of broad patches,
    and a little grain. The bedding is a function of x with a whole number of periods, so
    it meets its own edge on the next tile instead of stepping there.
    """
    rect(img, 0, 0, 15, 15, ramp.sh)
    # Deep rock is the one surface a player almost never studies: it is the dark under
    # a lawn. Give it *grain* and nothing else. Every shape tried here — courses,
    # bedding waves, a handful of cracks — became a legible motif at the tile repeat,
    # which is worse than plain. The autotile's edge light already says "rock".
    for y in range(16):
        for x in range(16):
            v = h32(x, y, salt + 7) % 100
            if v < 7:
                put(img, x, y, ramp.mid)
            elif v < 9:
                put(img, x, y, ramp.deep)
            elif v < 10:
                put(img, x, y, ramp.lit)


def _sandstone_body(img: Image.Image, ramp: Ramp, salt: int) -> None:
    for y in range(16):
        for x in range(16):
            band = (y + int(noise(x // 4, 0, salt) * 2)) // 5
            v = noise(x // 2, y // 2, salt + band)
            if band % 2:
                tone = ramp.lit if v > 0.5 else ramp.mid
            else:
                tone = ramp.mid if v > 0.3 else ramp.sh
            put(img, x, y, tone)
        if y % 5 == 4:
            for x in range(16):
                put(img, x, y, ramp.sh)


def _masonry_body(img: Image.Image, ramp: Ramp, salt: int) -> None:
    rows = ((0, 6), (7, 11), (12, 15))
    for index, (y0, y1) in enumerate(rows):
        offset = 6 if index % 2 else 0
        for y in range(y0, y1 + 1):
            for x in range(16):
                joint = (x + offset) % 11 == 10
                if y == y1 or joint:
                    put(img, x, y, ramp.sh)
                    continue
                v = noise(x // 2, y // 2, salt + index)
                if y == y0:
                    tone = ramp.lit
                elif y == y1 - 1:
                    tone = ramp.sh
                else:
                    tone = ramp.mid if v > 0.4 else ramp.lit
                put(img, x, y, tone)


def _timber_body(img: Image.Image, ramp: Ramp, salt: int) -> None:
    for x in range(16):
        beam = x // 8
        local = x % 8
        for y in range(16):
            if local == 7:
                put(img, x, y, ramp.deep)
                continue
            v = noise(beam, y // 3, salt)
            if local == 0:
                tone = ramp.lit
            elif local == 6:
                tone = ramp.sh
            else:
                tone = ramp.mid if v > 0.35 else ramp.lit
            put(img, x, y, tone)
    # Iron bolts where a beam meets the one beside it.
    for x in (3, 12):
        for y in (4, 12):
            put(img, x, y, IRON.lit)
            put(img, x + 1, y, IRON.sh)


BODY = {
    "grass": _soil_body,
    "sand": _sand_body,
    "brick": _brick_body,
    "wood": _wood_body,
    "stone": _stone_body,
    "sandstone": _sandstone_body,
    "masonry": _masonry_body,
    "timber": _timber_body,
}


def _crown(img: Image.Image, kind: str, ramp: Ramp, salt: int) -> None:
    """Rows 0..4 of a surface tile: the part the sky touches.

    Row 0 is one flat colour across the tile, by contract: the surface line of a level
    is the sum of its tiles' top rows, so a crown that varied along x would put a step
    in a flat floor. The life is in rows 1..4 — vertical blades, a lit ridge, and a
    boundary into the body that no straight line could be.
    """
    if kind == "grass":
        for x in range(16):
            # Row 0 is the surface line and is flat by contract. It is the *lit* tone
            # rather than the highlight: a one-pixel line of `hi` across every floor
            # is the neon rim the first pass had, and grass is a mass, not a wire.
            put(img, x, 0, ramp.lit)
            # Clumps rather than independent columns: a blade is two or three pixels
            # of one tone, and a run of independent columns is a picket fence.
            clump = vnoise(x, 0, salt, 3)
            tone = ramp.lit if clump > 0.45 else ramp.mid
            put(img, x, 1, tone)
            put(img, x, 2, ramp.mid if clump < 0.32 else tone)
            put(img, x, 3, ramp.mid if clump > 0.5 else ramp.sh)
            # Blade tips: a bright pixel where a clump peaks, so the line has life in
            # it without breaking into a dotted border.
            if clump > 0.74:
                put(img, x, 1, ramp.hi)
            if chance(x, 4, salt + 3, 0.3):
                put(img, x, 4, ramp.mid if clump > 0.5 else ramp.sh)
    elif kind == "sand":
        for x in range(16):
            put(img, x, 0, ramp.hi)
            column = noise(x, 1, salt)
            put(img, x, 1, ramp.hi if column > 0.7 else ramp.lit)
            put(img, x, 2, ramp.lit if column > 0.4 else ramp.mid)
            put(img, x, 3, ramp.sh if column < 0.34 else ramp.mid)
            if chance(x, 4, salt + 1, 0.3):
                put(img, x, 4, ramp.mid if column > 0.5 else ramp.sh)
    elif kind == "brick":
        for x in range(16):
            # A coping stone: one flat course with a joint every eight pixels.
            put(img, x, 0, ramp.hi)
            put(img, x, 1, ramp.lit if x % 8 not in (7,) else ramp.deep)
            put(img, x, 2, ramp.mid if x % 8 not in (7,) else ramp.deep)
            put(img, x, 3, ramp.sh)
            put(img, x, 4, ramp.mid)
    else:  # wood
        for x in range(16):
            put(img, x, 0, ramp.hi)
            put(img, x, 1, ramp.lit)
            put(img, x, 2, ramp.sh if x % 9 == 8 else ramp.mid)
            put(img, x, 3, ramp.sh)
            put(img, x, 4, ramp.hi if chance(x, 4, salt, 0.08) else ramp.lit)


def material_tile(kind: str, mask: int, variant: int = 0) -> Image.Image:
    """One solid tile of `kind` for a 4-bit open-side mask.

    bit 0 above, bit 1 left, bit 2 right, bit 3 below — the renderer's own contract
    (`view::ground_mask`). The tile is always fully opaque: it is solid ground, and a
    transparent pixel in it would read as a hole in the world.
    """
    body_ramp, crown_ramp, crowned = MATERIALS[kind]
    img = canvas()
    BODY[kind](img, body_ramp, salt=sum(map(ord, kind)) * 7 + variant)
    open_above = bool(mask & 0b0001)
    open_left = bool(mask & 0b0010)
    open_right = bool(mask & 0b0100)
    open_below = bool(mask & 0b1000)

    if crowned and open_above:
        _crown(img, kind, crown_ramp, salt=sum(map(ord, kind)) * 3)
        crown_rows = 5
    else:
        crown_rows = 0

    # Directional light. An exposed left face is lit (the sun is upper-left) and an
    # exposed right face is in shade; neither is an outline, because both are the
    # material's own tones, three steps apart rather than ink against fill. The darker
    # tone is the ramp's deepest brown, never black: a terrain edge is shadow, not ink.
    if open_left:
        for y in range(crown_rows, 16):
            put(img, 0, y, body_ramp.hi)
            put(img, 1, y, body_ramp.lit)
    if open_right:
        for y in range(crown_rows, 16):
            put(img, 15, y, body_ramp.deep)
            put(img, 14, y, body_ramp.sh)
    if open_below:
        for x in range(16):
            # A two-row occlusion band, broken by the odd hanging grain so the
            # underside of a platform is not a ruled line.
            put(img, x, 15, body_ramp.deep if chance(x, 15, 11, 0.7) else body_ramp.sh)
            put(img, x, 14, body_ramp.sh if chance(x, 14, 12, 0.55) else body_ramp.mid)
    # A lit shoulder where the top and the left meet: one pixel, and only on a
    # genuinely exposed convex corner.
    if open_above and open_left:
        # One pixel of shoulder, in the edge column the crown's flat-band rule already
        # excludes. `ground_1`..`ground_15` share row 0 across columns 1..14; a second
        # highlight pixel inside that band would put a step in every flat floor's own
        # surface line, which is the one thing autotiling exists to prevent.
        put(img, 0, 0, crown_ramp.hi if crowned else body_ramp.hi)
        put(img, 0, 1, crown_ramp.lit if crowned else body_ramp.lit)
    if open_above and open_right:
        put(img, 15, 0, crown_ramp.lit if crowned else body_ramp.lit)
    return img


def surface_variant(kind: str, variant: int) -> Image.Image:
    """`ground_1_b` and friends: the plain surface tile plus crown detail.

    Only rows 0..3 may change. Row 4 is the ragged boundary the crown shares with the
    body and every row below it is the body's, which is what lets a level sprinkle
    variants without moving the surface line.
    """
    base = material_tile(kind, 1)
    img = base.copy()
    ramp = MATERIALS[kind][0]
    if kind == "grass":
        if variant == 0:  # flowers
            for x, petal in ((3, GOLD.hi), (8, CREAM.hi), (12, CORAL.hi)):
                put(img, x, 2, LEAF.sh)
                put(img, x, 1, petal)
                put(img, x - 1, 1, petal)
                put(img, x + 1, 1, petal)
                put(img, x, 0, GOLD.lit)
        else:  # a tuft and a pebble
            for x in (4, 5, 10):
                put(img, x, 1, LEAF.hi)
                put(img, x, 2, LEAF.mid)
            put(img, 11, 1, SAND.hi)
            put(img, 12, 1, SAND.lit)
            put(img, 12, 2, SAND.sh)
    elif kind == "sand":
        if variant == 0:  # wind ripples
            for x in range(2, 15):
                y = 2 + round(1.0 * math.sin(2 * math.pi * (x - 2) / 12.0))
                put(img, x, y, ramp.hi)
        else:  # a shell and grit
            put(img, 5, 2, CREAM.hi)
            put(img, 6, 1, CREAM.hi)
            put(img, 6, 2, CREAM.mid)
            put(img, 7, 2, CREAM.sh)
            for x, y in ((10, 2), (11, 1), (12, 2)):
                put(img, x, y, ramp.lit)
    elif kind == "brick":
        if variant == 0:  # moss in the joints
            for x, y in ((2, 2), (3, 2), (9, 2), (10, 2), (10, 1), (11, 2)):
                put(img, x, y, LEAF.mid)
        else:  # chipped mortar
            for x, y in ((4, 2), (12, 1), (13, 2), (7, 2)):
                put(img, x, y, ramp.hi)
                put(img, x + 1, y, ramp.sh)
    else:  # wood
        if variant == 0:  # knots
            for x in (4, 11):
                put(img, x, 2, ramp.sh)
                put(img, x, 1, ramp.sh)
                put(img, x + 1, 2, ramp.sh)
                put(img, x, 3, ramp.sh)
                put(img, x, 2, ramp.deep)
                put(img, x, 1, ramp.mid)
        else:  # countersunk rivets
            for x in (3, 12):
                put(img, x, 1, IRON.lit)
                put(img, x + 1, 2, IRON.sh)
                put(img, x, 2, IRON.mid)
    return img


def rock_variant(variant: int) -> Image.Image:
    """`deep_0_b`/`deep_0_c`: the plain rock plus surface detail.

    Row 0 and row 15 belong to the tile's neighbours, so a variant may not touch them.
    """
    base = material_tile("stone", 0)
    img = base.copy()
    if variant == 0:
        for x, y in ((4, 5), (5, 5), (5, 6), (10, 9), (11, 9), (11, 10)):
            put(img, x, y, STONE.hi if y % 2 else STONE.sh)
            put(img, x, y + 1, STONE.deep)
    else:
        path = ((3, 3), (6, 6), (5, 9), (9, 12), (8, 14), (11, 13))
        for index in range(len(path) - 1):
            x0, y0 = path[index]
            x1, y1 = path[index + 1]
            steps = max(abs(x1 - x0), abs(y1 - y0))
            for step in range(steps + 1):
                x = x0 + round((x1 - x0) * step / max(1, steps))
                y = y0 + round((y1 - y0) * step / max(1, steps))
                put(img, x, y, STONE.deep)
                put(img, x + 1, y, STONE.hi)
    return img


def crack_overlay(material: str, variant: int) -> Image.Image:
    """Damage on an otherwise transparent cell, drawn over the material it marks.

    Three tones — dark heart, chipped shoulder, lit lip — so a crack reads as a
    fracture in the material rather than as more of its texture.
    """
    # The *body* ramp, not the crown: a crack in a lawn is dark soil showing through,
    # and the first version drew it in the grass ramp — a green squiggle lying on the
    # dirt, which reads as a vine rather than as damage.
    ramps = {"ground": SOIL, "sand": SAND, "brick": BRICK, "wood": WOOD}
    ramp = ramps[material]
    img = canvas()
    paths = (
        ((3, 2), (6, 5), (5, 8), (9, 11), (8, 14)),
        ((12, 1), (9, 4), (10, 7), (6, 9), (4, 13)),
        ((2, 7), (5, 5), (8, 8), (11, 6), (13, 9)),
    )
    for index in range(len(paths[variant % 3]) - 1):
        x0, y0 = paths[variant % 3][index]
        x1, y1 = paths[variant % 3][index + 1]
        steps = max(abs(x1 - x0), abs(y1 - y0))
        for step in range(steps + 1):
            x = x0 + round((x1 - x0) * step / max(1, steps))
            y = y0 + round((y1 - y0) * step / max(1, steps))
            put(img, x, y, ramp.deep)
            put(img, x + 1, y, ramp.hi)
            if step % 2 == 0:
                put(img, x, y - 1, ramp.sh)
    # A branch, so the fracture has a direction rather than being a squiggle.
    bx, by = paths[variant % 3][1]
    for i in range(3):
        put(img, bx + 1 + i, by + i, ramp.deep)
        put(img, bx + 2 + i, by + i, ramp.hi)
    return img


SEAM_LOOKS = {
    # (tile kind, flat mask, spill depth). The depths are the renderer's own: sand
    # creeps furthest into its neighbour, stone least, and the ordering is pinned by
    # `atlas::tests::every_seam_is_a_band_along_its_own_side`.
    "grass": ("grass", 1, 3),
    "dirt": ("grass", 0, 3),
    "stone": ("stone", 0, 2),
    "beach": ("sand", 1, 5),
    "packed": ("sand", 0, 5),
    "sandstone": ("sandstone", 0, 3),
    "brick": ("brick", 1, 2),
    "masonry": ("masonry", 0, 2),
    "wood": ("wood", 1, 3),
    "timber": ("timber", 0, 3),
}


def seam_sprite(look: str, side: str) -> Image.Image:
    """One material's spill across one edge, cut from that material's own pixels.

    Mostly transparent by design: the largest band is about a fifth of the cell. It
    is a scatter of grains rather than a wedge, so two materials interlock along a
    wandering boundary instead of stacking a stripe of each inside the other.
    """
    kind, mask, max_depth = SEAM_LOOKS[look]
    tile = material_tile(kind, mask)
    img = canvas()
    salt = sum(map(ord, look)) * 13 + sum(map(ord, side))
    # The edge is covered on most of its length and gives way on some of it, so two
    # bands interlock instead of stacking into a stripe. Deterministic: the sparse
    # materials give way every fifth pixel, the loose ones every seventh.
    covered = [along for along in range(16) if h32(along, 0, salt) % 16 >= 3]
    if len(covered) < 9:  # deterministic, but never a fringe of a band
        covered = [along for along in range(16) if along % 5 != 0]
    # A band covers most of its edge and never all of it: the far side of a seam has to
    # give the other material somewhere to interlock.
    covered = covered[:15]
    for along in covered:
        for depth in range(max_depth + 2):
            if depth and h32(along, depth, salt + 2) % 100 > 46:
                break
            if side == "l":
                x, y = depth, along
            elif side == "r":
                x, y = 15 - depth, along
            elif side == "t":
                x, y = along, depth
            else:
                x, y = along, 15 - depth
            img.putpixel((x, y), tile.getpixel((x, y)))
    return img


# --------------------------------------------------------------------------------------
# Fluids
# --------------------------------------------------------------------------------------

#: A fluid body is one picture this many cells square, cut into `FLUID_BLOCK ** 2` cells
#: that the renderer indexes by the tile's own position. `crate::view::FLUID_BLOCK` is
#: the same number from the other side; the two are one decision and are pinned together
#: by `view::tests::fluid_bodies_tile_as_one_four_by_four_picture`.
FLUID_BLOCK = 4


def fluid_block(kind: str) -> list[Image.Image]:
    """A fluid body as one 64-pixel picture cut into sixteen cells.

    A pool has a tile boundary at every cell, so the body cannot be *varied* per tile:
    it is one continuous picture four tiles square, and the renderer samples the cell
    that matches the tile's position. That is what makes stacked water one body rather
    than sixteen trays, and it is why this is generated whole and cut afterwards.
    """
    size = CELL * FLUID_BLOCK
    ramp = WATER if kind == "water" else LAVA
    # The body is `mid`, and it stays `mid`.
    #
    # Volume is somebody else's job here, and that is the whole lesson of this pass.
    # The surface cell ([`fluid_surface`]) fades into `mid` at its foot and the bed
    # ([`fluid_bed`]) shades the last rows against the floor, so the body only has to
    # *continue* that tone. Painting tone fields into it — which the earlier version
    # did, at four tiles across — put big pale and dark patches directly under a
    # clean crest. Three things went wrong at once: the patches were a second material
    # under the water rather than water, the join with the surface was a hard tone
    # step, and because the block repeats every four tiles the patches came back, in
    # the same places, a few tiles along. A pool that looked like camouflage at four
    # tiles and like wallpaper at twelve.
    #
    # What the body gets instead is *detail*: thin broken lines for water and even,
    # clustered grain for lava, a couple of percent of the pixels each, so there is
    # something for the eye to find under the surface but nothing with a silhouette
    # to repeat.
    big = Image.new("RGBA", (size, size), ramp.mid)

    def dot(x: int, y: int, tone) -> None:
        big.putpixel((x % size, y % size), tone)

    if kind == "water":
        for y in range(size):
            for x in range(size):
                # Two periodic fields: one broad enough to carry a long caustic, one
                # tighter so no two strokes run parallel. Both wrap with the block.
                broad = 0.62 * vnoise(x, y, 61, 2, size) + 0.38 * vnoise(x, y, 63, 5, size)
                fine = 0.60 * vnoise(x, y, 67, 3, size) + 0.40 * vnoise(x, y, 69, 7, size)
                # A caustic: the contour of `broad`, one pixel of `lit`, and broken
                # into dashes in groups of two so it flickers instead of drawing a
                # line. Pale enough to read as light on a body of water, not as a
                # second, lighter fluid.
                if abs(broad - 0.60) < 0.010 and h32(x // 2, y // 2, 71) % 5 < 3:
                    dot(x, y, ramp.lit)
                # The shadow side of the same light: quieter, one step down, mostly
                # along the bottom of the field rather than scattered through it.
                elif abs(fine - 0.34) < 0.008 and h32(x // 3, y // 3, 73) % 4 < 2:
                    dot(x, y, ramp.sh)
                # Suspended motes: one pixel, well under one in a hundred, so a pool
                # glints when the surface above moves. More than this and it is sand.
                elif h32(x, y, 75) % 149 == 0:
                    dot(x, y, ramp.lit if h32(x, y, 77) % 3 else ramp.hi)
        # Bubbles: a highlight with a shadow under it, so a two-pixel mark reads as a
        # sphere. Kept two pixels inside the block so no cell boundary cuts one.
        for index in range(5):
            bx = 2 + int(noise(index, 0, 51, wrap=size) * (size - 6))
            by = 2 + int(noise(index, 1, 53, wrap=size) * (size - 6))
            dot(bx, by, ramp.hi)
            dot(bx + 1, by, ramp.lit)
            dot(bx + 1, by + 1, ramp.sh)
    else:
        # Lava is the harder of the two, because a molten body *does* have a pattern —
        # a crust with cracks — and any pattern is a shape, and any shape comes back
        # every four tiles. Two earlier versions drew that crust as lines (the last one
        # as noise contours) and both came back as the same closed loops, three times
        # across a wide pool. So lava gets no line work at all: grain, a scatter of
        # `hi` embers, and nothing else. Grain has no outline to trace, so what repeats
        # is a *texture*, which the eye reads as the same lava rather than as the same
        # rock.
        for y in range(size):
            for x in range(size):
                # A skin of cooled rock read as grain rather than as plates: single
                # pixels, evenly spread, two or three percent of them. The two noise
                # fields before this clumped into islands — and an island is a shape
                # the eye meets again four tiles along. A hash has no neighbours, so
                # the density is the same everywhere in the block and the repeat has
                # nothing to catch on. It is also *sparse*: at one pixel in twenty the
                # body is glitter, and lava is not glitter.
                roll = h32(x, y, 105) % 1000
                tone = ramp.mid
                if roll < 21:
                    tone = ramp.lit
                elif roll < 30:
                    tone = ramp.sh
                # Embers: single bright pixels, rare, and never two together — a pair
                # is a shape, one is a spark.
                if h32(x, y, 97) % 197 == 0:
                    tone = ramp.hi
                big.putpixel((x, y), tone)
    cells: list[Image.Image] = []
    for row in range(FLUID_BLOCK):
        for col in range(FLUID_BLOCK):
            left, top = col * CELL, row * CELL
            cells.append(big.crop((left, top, left + CELL, top + CELL)))
    return cells


def fluid_surface(kind: str, frame: int) -> Image.Image:
    """A pool's surface: a crest with air above it, fading into the body below.

    The rows above the crest are transparent, and the bottom of the cell is the body's
    own tone, so the surface cell ends where the body cell under it begins.
    """
    ramp = WATER if kind == "water" else LAVA
    img = canvas()
    phase = frame * math.pi
    heights = [2 + round(1.3 * math.sin(2 * math.pi * x / 16.0 + phase)) for x in range(16)]
    for x in range(16):
        top = heights[x]
        crest = ramp.hi if kind == "water" else ramp.hi
        put(img, x, top, crest)
        put(img, x, top + 1, ramp.lit)
        if kind == "water":
            put(img, x, top + 2, ramp.mid)
            if (x + frame * 3) % 7 == 0:
                put(img, x, top + 1, WHITE)
        else:
            put(img, x, top + 2, ramp.lit)
            put(img, x, top + 3, ramp.mid)
        for y in range(top + 2 + (kind == "lava"), 16):
            depth = y - top
            if kind == "water":
                # A lit shoulder under the crest, then the body's own depth: the
                # surface has to look like the top of a volume, not a stripe on it.
                # End in the body's own mid tone, not its shadow: the body is a flat
                # field of `mid` with detail over it, and a surface that bottomed out
                # in `sh` put a dark band across the join — the hard line this pass is
                # fixing. `lit` near the crest still says the light is above.
                if depth <= 3:
                    tone = ramp.lit
                else:
                    tone = ramp.mid
                if depth in (2, 5) and h32(x, y, 3 + frame) % 100 < 12:
                    tone = ramp.hi
            else:
                # The same join rule as water: the molten body is `mid`, so the
                # surface fades to `mid` and the crack line stays at the crest, where
                # it belongs, instead of darkening the whole top of the body.
                tone = ramp.lit if depth < 4 else ramp.mid
                # Flecks only where the crust is still lit. They used to run all the
                # way to the cell's last row, which put a ten-percent sparkle band over
                # a body with almost none — so the *end of the surface* was visible as
                # a line, and that line is exactly what a second layer cannot hide. The
                # bottom half of the surface must be the body's own tone, undecorated.
                if depth < 4 and h32(x, y, 9 + frame) % 100 < 10:
                    tone = ramp.hi
            put(img, x, y, tone)
    return img


def fluid_bed(kind: str) -> Image.Image:
    """The shadow where a pool meets its floor, drawn over the body cell.

    A *gradient* by ordered dither, not a band with a ragged edge. The band that used
    to be here ended in a sawtooth row — one column per depth step — and because the
    bed is drawn on every floor cell of a pool, a wide pool grew a comb along its
    whole bottom. The dither starts at nothing on its first row, so the bed also joins
    the body above it without a line, and every cell draws the same one, so a pool's
    floor has no seams either.
    """
    ramp = WATER if kind == "water" else LAVA
    bayer = ((0, 8, 2, 10), (12, 4, 14, 6), (3, 11, 1, 9), (15, 7, 13, 5))
    img = canvas()
    # Six rows: nothing, a scatter, then the deepest tone against the floor. `sh`
    # first and `deep` underneath, so the shadow has two steps in it rather than one
    # flat slab of the darkest colour the ramp owns.
    for y in range(10, 16):
        for x in range(16):
            step = (y - 10) / 6.0
            threshold = (bayer[y % 4][x % 4] + 0.5) / 16.0
            if threshold < step * 0.85:
                put(img, x, y, ramp.sh)
            if threshold < (step - 0.28) / 0.72:
                put(img, x, y, ramp.deep)
    # One or two specks on the floor, so the gradient is not a perfect ramp.
    for x, y in ((3, 15), (4, 14), (11, 14), (12, 15)):
        put(img, x, y, ramp.deep if kind == "water" else ramp.sh)
    return img


def fluid_wall(kind: str, side: str) -> Image.Image:
    """The rim of a pool that meets open air: a dark edge with one lit reflection."""
    ramp = WATER if kind == "water" else LAVA
    img = canvas()
    # Optional art, and deliberately quiet: a pool's rim is a change of tone, not a
    # line. The first pass drew it at 235 alpha, which put a black-looking edge between
    # the water and the ground — the outline this pass exists to remove.
    for y in range(16):
        if side == "l":
            put(img, 0, y, tint(ramp.sh, 170))
            put(img, 1, y, tint(ramp.lit, 95))
            if y % 6 != 0:
                put(img, 2, y, tint(ramp.hi, 70))
        else:
            put(img, 15, y, tint(ramp.deep, 150))
            put(img, 14, y, tint(ramp.sh, 120))
            if y % 6 != 0:
                put(img, 13, y, tint(ramp.lit, 70))
    return img


def lava_ball() -> Image.Image:
    """One ball of lava: the cell is its hitbox, so the ball fills the cell."""
    img = canvas()
    draw = ImageDraw.Draw(img)
    draw.ellipse((1, 1, 14, 14), fill=LAVA.lit)
    draw.ellipse((2, 2, 12, 12), fill=LAVA.mid)
    draw.ellipse((4, 3, 9, 8), fill=LAVA.hi)
    draw.ellipse((9, 9, 12, 12), fill=LAVA.deep)
    for x, y in ((0, 7), (15, 8), (7, 0), (8, 15)):
        put(img, x, y, LAVA.hi)
    return img


# --------------------------------------------------------------------------------------
# Mechanical tiles
# --------------------------------------------------------------------------------------


def spike() -> Image.Image:
    """Four dark steel spikes on a stone footing.

    The top three rows are air: the sim's lethal box begins exactly there
    (`physics::SPIKE_TOP_CLEARANCE`), so a body can clear the drawing and clear the
    hazard. Anything drawn above row 3 would be an invisible kill band.

    The shape is a *spike*: one pixel at the tip, widening to a straight-sided shaft,
    with a dark canyon between neighbours. The first pass drew rounded columns, which
    read as organ pipes — a hazard has to look like it would go through you.
    """
    img = canvas()
    steel = Ramp("#C2CBD5", "#98A4B1", "#75818E", "#57626F", "#3E4854")
    # Three spikes, each a true triangle: the width grows every other row all the way to
    # the base, and five pixels of base is what lets the edge read as a slope instead of
    # as a post with a pointed cap.
    for centre in (3, 8, 13):
        for row in range(10):
            y = 3 + row
            width = 1 + row // 2
            x0 = centre - width // 2
            x1 = x0 + width - 1
            for x in range(x0, x1 + 1):
                if x == x0:
                    tone = steel.hi if row < 2 else steel.lit
                elif x == x1:
                    tone = steel.sh
                else:
                    tone = steel.mid
                put(img, x, y, tone)
            if width == 1:
                put(img, centre, y, steel.hi)
        # A shadow at the foot of each shaft, and one canyon between neighbours: the
        # separation that makes three triangles read as three spikes.
        put(img, centre, 12, steel.deep)
        if centre > 3:
            put(img, centre - 3, 12, steel.deep)
            put(img, centre - 3, 11, steel.sh)
    rect(img, 0, 13, 15, 15, STONE.sh)
    rect(img, 0, 13, 15, 13, STONE.lit)
    rect(img, 0, 15, 15, 15, STONE.deep)
    for x in (2, 6, 10, 14):
        put(img, x, 14, STONE.deep)
    return img


def spring() -> Image.Image:
    """A brass pad: the top four rows are air, matching `SPRING_TOP_CLEARANCE`."""
    img = canvas()
    # Top plate, lit along its leading edge so it reads as a surface that will throw
    # you upward rather than as a floor tile that happens to be yellow.
    rect(img, 2, 4, 13, 4, GOLD.hi)
    rect(img, 2, 5, 13, 5, GOLD.lit)
    rect(img, 2, 6, 13, 6, GOLD.mid)
    rect(img, 2, 6, 13, 6, GOLD.sh)
    rect(img, 1, 7, 14, 7, IRON.sh)
    # The coil: three full turns, each a chevron of brass with a shadow under it.
    for turn, y in enumerate((8, 9, 10, 11)):
        offset = turn % 2
        for x in range(3 + offset, 13 - offset, 4):
            put(img, x, y, GOLD.lit)
            put(img, x + 1, y, GOLD.mid)
            put(img, x + 2, y, GOLD.lit)
            put(img, x + 1, y + 1, GOLD.sh)
    rect(img, 1, 12, 14, 12, IRON.mid)
    rect(img, 2, 13, 13, 15, IRON.sh)
    rect(img, 2, 13, 13, 13, IRON.lit)
    for x in (4, 8, 11):
        put(img, x, 14, IRON.deep)
    return img


def oneway(part: str) -> Image.Image:
    """A one-way shelf: three plank rows across the top of the cell, brackets below."""
    img = canvas()
    rect(img, 0, 0, 15, 2, WOOD.mid)
    rect(img, 0, 0, 15, 0, WOOD.hi)
    rect(img, 0, 1, 15, 1, WOOD.lit)
    rect(img, 0, 2, 15, 2, WOOD.sh)
    for x in range(16):
        if x % 5 == 4:
            put(img, x, 1, WOOD.sh)
    # Iron brackets, inset from the cut ends.
    left = 2 if part != "l" else 0
    right = 13 if part != "r" else 15
    for x, y in ((left + 1, 3), (left + 1, 4), (right - 1, 3), (right - 1, 4)):
        put(img, x, y, IRON.sh)
    rect(img, left + 1, 5, left + 1, 5, IRON.deep)
    rect(img, right - 1, 5, right - 1, 5, IRON.deep)
    if part == "l":
        rect(img, 0, 0, 0, 2, WOOD.deep)
    if part == "r":
        rect(img, 15, 0, 15, 2, WOOD.deep)
    return img


def mover(part: str) -> Image.Image:
    """A moving shelf: a deck with an iron rail on top and a hanging link."""
    img = canvas()
    rect(img, 0, 0, 15, 2, IRON.mid)
    rect(img, 0, 0, 15, 0, IRON.hi)
    rect(img, 0, 1, 15, 1, IRON.lit)
    rect(img, 0, 2, 15, 2, IRON.sh)
    rect(img, 0, 3, 15, 4, WOOD.mid)
    rect(img, 0, 3, 15, 3, WOOD.lit)
    rect(img, 0, 4, 15, 4, WOOD.sh)
    for x in range(16):
        if x % 4 == 2:
            put(img, x, 1, IRON.mid)
    # Hanging link: the shelf's own cable, carried by the middle piece.
    if part == "m":
        rect(img, 7, 5, 8, 9, IRON.sh)
        rect(img, 7, 5, 7, 9, IRON.mid)
        rect(img, 6, 10, 9, 11, IRON.deep)
    if part == "l":
        rect(img, 0, 0, 0, 4, IRON.deep)
    if part == "r":
        rect(img, 15, 0, 15, 4, IRON.deep)
    return img


def belt(direction: str) -> Image.Image:
    """A conveyor: a timber tread on top, an iron chassis with chevrons below."""
    img = canvas()
    rect(img, 0, 0, 15, 3, WOOD.mid)
    rect(img, 0, 0, 15, 0, WOOD.hi)
    rect(img, 0, 1, 15, 1, WOOD.lit)
    rect(img, 0, 3, 15, 3, WOOD.sh)
    for x in range(16):
        if x % 6 == 5:
            put(img, x, 2, WOOD.deep)
    rect(img, 0, 4, 15, 15, IRON.sh)
    rect(img, 0, 4, 15, 4, IRON.mid)
    rect(img, 0, 15, 15, 15, IRON.deep)
    for x in (3, 8, 13):
        for i in range(4):
            yy = 7 + i
            xx = x + i
            put(img, xx, yy, GOLD.lit)
            put(img, xx, yy + 1, GOLD.sh)
    if direction == "l":
        img = img.transpose(Image.Transpose.FLIP_LEFT_RIGHT)
    return img


def boulder() -> Image.Image:
    """A natural stone: a faceted mass with a flattened base, lit from the upper left.

    The silhouette is a polygon, and the *inside* is drawn as planes rather than as a
    rim: a bright top-left facet, a mid face and a shadowed lower-right one, with the
    boundaries between them in the stone's own shadow tone. The earlier boulder was a
    flat disc with a lit rim, which reads as a grey ball rather than as rock.
    """
    img = canvas()
    outline = [(3, 15), (2, 10), (3, 6), (6, 3), (9, 2), (12, 3), (14, 7), (14, 11), (11, 15)]
    ImageDraw.Draw(img).polygon(outline, fill=STONE.mid)
    for y in range(16):
        for x in range(16):
            if img.getpixel((x, y))[3] == 0:
                continue
            above = img.getpixel((x, max(0, y - 1)))[3] == 0
            left = img.getpixel((max(0, x - 1), y))[3] == 0
            below = img.getpixel((x, min(15, y + 1)))[3] == 0
            right = img.getpixel((min(15, x + 1), y))[3] == 0
            if above:
                tone = STONE.hi
            elif left:
                tone = STONE.lit if y < 11 else STONE.mid
            elif below:
                tone = STONE.deep
            elif right:
                tone = STONE.sh
            elif x + y < 12:
                tone = STONE.lit
            elif x + y < 19:
                tone = STONE.mid
            else:
                tone = STONE.sh
            put(img, x, y, tone)
    # The facet edge: a shadow with a lit shoulder, so the planes meet on a ridge
    # instead of blending into one another.
    for x, y in ((8, 4), (8, 5), (7, 6), (7, 7), (8, 8), (9, 9), (10, 10), (10, 11)):
        put(img, x, y, STONE.sh)
        put(img, x - 1, y, STONE.hi)
    # Pits and a hairline crack, from the top-left face.
    for x, y in ((5, 6), (6, 6), (6, 7), (4, 10), (11, 12), (12, 12)):
        put(img, x, y, STONE.sh)
    put(img, 4, 5, STONE.deep)
    put(img, 4, 6, STONE.deep)
    put(img, 11, 8, STONE.lit)
    return img


def block() -> Image.Image:
    """A crate: planks with a cross batten, bound by an iron frame. Fills the cell."""
    img = canvas()
    rect(img, 0, 0, 15, 15, WOOD.mid)
    for y in range(16):
        if y % 4 == 3:
            rect(img, 0, y, 15, y, WOOD.sh)
    rect(img, 0, 0, 15, 0, WOOD.hi)
    for x in range(16):
        if x % 7 == 3:
            put(img, x, 1, WOOD.sh)
            put(img, x, 2, WOOD.sh)
    # Iron frame: four straps, lit on their outer edge.
    for x in range(16):
        put(img, x, 0, IRON.mid)
        put(img, x, 1, IRON.sh)
        put(img, x, 14, IRON.sh)
        put(img, x, 15, IRON.deep)
    for y in range(16):
        put(img, 0, y, IRON.mid)
        put(img, 1, y, IRON.sh)
        put(img, 14, y, IRON.sh)
        put(img, 15, y, IRON.deep)
    for x, y in ((0, 0), (15, 0), (0, 15), (15, 15)):
        put(img, x, y, IRON.hi if x == 0 else IRON.mid)
    # The batten, a diagonal strap across the door.
    for i in range(11):
        put(img, 3 + i, 4 + i, IRON.mid)
        put(img, 4 + i, 4 + i, IRON.sh)
    for x, y in ((3, 13), (4, 13), (11, 4), (12, 4)):
        put(img, x, y, IRON.hi)
    return img


def rope_icon() -> Image.Image:
    """The rope brush's palette picture: a cord, a knot and a loop."""
    img = canvas()
    for y in range(1, 10):
        put(img, 7, y, WOOD.lit)
        put(img, 8, y, WOOD.sh)
    for x, y in ((7, 2), (8, 4), (7, 6), (8, 8)):
        put(img, x, y, WOOD.hi)
    draw = ImageDraw.Draw(img)
    draw.arc((4, 9, 12, 15), 10, 340, fill=WOOD.lit, width=2)
    draw.arc((5, 10, 11, 14), 190, 160, fill=WOOD.sh, width=1)
    put(img, 8, 14, WOOD.deep)
    return img


def spike_ball() -> Image.Image:
    """An iron ball with eight spikes; the cell is the hitbox, so it fills the cell."""
    img = canvas()
    for angle in range(0, 360, 45):
        dx = round(math.cos(math.radians(angle)) * 7)
        dy = round(math.sin(math.radians(angle)) * 7)
        for step in range(2, 6):
            put(img, 8 + round(dx * step / 5), 8 + round(dy * step / 5), IRON.mid)
        put(img, 8 + dx, 8 + dy, IRON.hi)
    draw = ImageDraw.Draw(img)
    draw.ellipse((3, 3, 12, 12), fill=IRON.mid)
    draw.ellipse((4, 4, 10, 10), fill=IRON.lit)
    draw.ellipse((5, 5, 8, 8), fill=IRON.hi)
    draw.ellipse((9, 10, 12, 12), fill=IRON.sh)
    for x, y in ((6, 8), (8, 6), (10, 9)):
        put(img, x, y, IRON.deep)
    return img


def vine_tile() -> Image.Image:
    """One cell of climbable vine: a stem continuous top to bottom, leaves alternating.

    The stem wanders by a pixel so it reads as a plant rather than as a ladder, and
    every leaf is a diamond with a lit upper face and a shadowed underside — the same
    one-light rule as the canopy. The ends stay in the middle columns: two stacked
    cells have to meet, and a vine that stepped sideways at every tile would break.
    """
    img = canvas()
    xs = [7 + round(0.9 * math.sin(2 * math.pi * y / 16.0)) for y in range(16)]
    for y in range(16):
        x = xs[y]
        put(img, x, y, LEAF.mid)
        put(img, x + 1, y, LEAF.sh)
        if y % 5 == 2:
            put(img, x, y, LEAF.lit)
    for y, side in ((3, -1), (7, 1), (11, -1), (15, 1)):
        x = xs[y]
        # A four-pixel leaf, angled down and away from the stem.
        tip = x + side * 4
        put(img, x + side, y, LEAF.lit)
        put(img, x + side * 2, y - 1, LEAF.mid)
        put(img, x + side * 3, y - 1, LEAF.lit)
        put(img, tip, y, LEAF.hi)
        put(img, x + side * 2, y, LEAF.mid)
        put(img, x + side, y + 1, LEAF.sh)
        put(img, x + side * 3, y, LEAF.sh)
    # Two tendrils, so the stem is something a plant does rather than a line with
    # leaves glued to it.
    for x, y, dx in ((xs[5] + 1, 5, 1), (xs[13] - 1, 13, -1)):
        put(img, x + dx, y - 1, LEAF.lit)
        put(img, x + dx * 2, y - 2, LEAF.mid)
        put(img, x + dx * 2, y - 3, LEAF.lit)
    return img


def cracked(name_material: str) -> Image.Image:
    return crack_overlay(name_material, 0)
